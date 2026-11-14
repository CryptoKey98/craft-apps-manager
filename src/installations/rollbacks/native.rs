//! Verified native packages are stored separately from disposable setup downloads.
use super::*;
use crate::model::Asset;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PackageIdentity {
    pub format: String,
    pub identity: String,
    pub product: String,
    pub version: String,
    pub architecture: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct NativePoint {
    schema: u32,
    repository: String,
    platform: String,
    active: Installed,
    // Read-only legacy guard: never execute old cross-scope recovery as an ordinary rollback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    original_migration: Option<Installed>,
    asset: Asset,
    tag: String,
    identity: PackageIdentity,
}

pub(crate) struct Prepared {
    #[cfg(target_os = "windows")]
    pub(crate) windows_msi_scope: Option<String>,
    pub(crate) asset: Asset,
    pub(crate) package: PathBuf,
    pub(super) tag: String,
    pub(crate) identity: PackageIdentity,
}

pub(crate) fn enabled(paths: &Paths, installed: &Installed) -> Result<bool> {
    let prefs = paths.read_preferences_locked()?;
    Ok(prefs
        .rollback_overrides
        .get(&key(installed)?)
        .copied()
        .unwrap_or(prefs.rollback_enabled()))
}

pub(crate) fn supported(installed: &Installed) -> Result<()> {
    key(installed)?;
    ensure!(
        !installed.product_code.is_empty(),
        "Native rollback requires a verified package identity"
    );
    #[cfg(target_os = "windows")]
    uuid::Uuid::parse_str(&installed.product_code).map_err(|_| anyhow::anyhow!("Rollback supports MSI installers; this installation does not have a verified MSI identity"))?;
    Ok(())
}

pub(crate) fn prepare_current(
    paths: &Paths,
    installed: &Installed,
    job: &Job,
    approved: Option<&BTreeMap<String, Asset>>,
) -> Result<Prepared> {
    supported(installed)?;
    job.check()?;
    job.stage(
        "Preparing previous installer",
        None,
        "Verifying a recovery package for the installed version",
    );
    let (asset, tag) = if let Some(approved) = approved {
        let asset = approved
            .get(&installed.name)
            .context("No previous installer was approved for this operation")?
            .clone();
        let url = reqwest::Url::parse(&asset.browser_download_url)?;
        let segments: Vec<_> = url
            .path_segments()
            .context("Invalid recovery URL")?
            .collect();
        let tag = segments
            .windows(2)
            .position(|s| s == ["releases", "download"])
            .and_then(|i| segments.get(i + 2))
            .context("Missing recovery tag")?
            .to_string();
        (asset, tag)
    } else {
        let network = crate::network::Network::new(&paths.root)?;
        let releases: Vec<crate::model::Release> = network.release_json(&format!(
            "https://api.github.com/repos/{}/releases?per_page=100",
            crate::model::repository(&installed.name)
        ))?;
        let release = crate::updates::recovery_release(releases, &installed.version)?;
        let prefs =
            crate::install_options::for_installed(&paths.read_preferences_locked()?, installed)?;
        (
            crate::updates::select_asset(&release, &installed.name, &prefs)?.clone(),
            release.tag_name,
        )
    };
    files::safe_relative(&asset.name)?;
    ensure!(
        Path::new(&asset.name).components().count() == 1
            && crate::updates::expected_asset_url(
                &asset.browser_download_url,
                &crate::model::repository(&installed.name)
            ),
        "Previous installer source is invalid"
    );
    let package = crate::updates::installer_package_path(paths, &installed.name, &asset)?;
    files::inside(&package, &paths.at("releases/installers"))?;
    let downloaded = !package.exists();
    if downloaded {
        ensure!(approved.is_none(), "Approved previous installer is missing");
        if let Err(error) = crate::network::Network::new(&paths.root)?.asset(&asset, &package, job)
        {
            let _ = discard_new_download(paths, &package);
            return Err(error);
        }
    }
    let verified = (|| {
        files::no_links(&package)?;
        crate::network::verify_asset(&package, &asset)?;
        let identity = native_package::inspect(paths, installed, &package, job)?;
        validate_identity(&identity, installed)?;
        #[cfg(target_os = "windows")]
        let windows_msi_scope = {
            let snapshot = crate::installers::snapshot_msi_scope(installed)?;
            crate::installers::validate_msi_replacement_scope(&package, &snapshot)?;
            snapshot.windows_msi_scope
        };
        Ok(Prepared {
            #[cfg(target_os = "windows")]
            windows_msi_scope,
            asset,
            package: package.clone(),
            tag,
            identity,
        })
    })();
    if verified.is_err() && downloaded {
        if let Err(error) = discard_new_download(paths, &package) {
            job.log(&format!(
                "Unused recovery download cleanup warning: {error:#}"
            ));
        }
    }
    verified
}

/// Only a caller that created this disposable cache file may request this cleanup.
pub(crate) fn discard_new_download(paths: &Paths, file: &Path) -> Result<()> {
    let cache = paths.at("releases/installers");
    files::inside(file, &cache)?;
    ensure!(
        file.parent().and_then(Path::parent) == Some(cache.as_path()),
        "Invalid installer staging ownership"
    );
    if file.exists() {
        files::no_links(file)?;
        fs::remove_file(file)?;
    }
    for folder in [file.parent().unwrap(), cache.as_path()] {
        match fs::remove_dir(folder) {
            Ok(()) => (),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::DirectoryNotEmpty | std::io::ErrorKind::NotFound
                ) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub(crate) fn retain(
    paths: &Paths,
    previous: &Installed,
    updated: &Installed,
    prepared: &Prepared,
    new_package: &Path,
    job: &Job,
) -> Result<()> {
    let next = native_package::inspect(paths, updated, new_package, job)?;
    validate_identity(&next, updated)?;
    ensure!(
        next.format == prepared.identity.format
            && same_identifier(&next.identity, &prepared.identity.identity),
        "Installer update changed the native package family"
    );
    publish(paths, previous, updated, prepared, job)
}

pub(crate) fn recover(
    paths: &Paths,
    previous: &Installed,
    prepared: &Prepared,
    job: &Job,
) -> Result<()> {
    #[cfg(target_os = "windows")]
    let scoped_previous = {
        let mut snapshot = previous.clone();
        if prepared.windows_msi_scope.is_some() {
            snapshot.windows_msi_scope = prepared.windows_msi_scope.clone();
        }
        snapshot
    };
    #[cfg(target_os = "windows")]
    let previous = &scoped_previous;
    let recovery_job = Job::new(job.log_path.clone(), &Default::default());
    crate::network::verify_asset(&prepared.package, &prepared.asset)?;
    validate_identity(
        &native_package::inspect(paths, previous, &prepared.package, &recovery_job)?,
        previous,
    )?;
    job.stage(
        "Recovering installation",
        None,
        "Restoring the verified current-version installer",
    );
    #[cfg(target_os = "windows")]
    let current = crate::installers::detect_rollback_copy(previous)?;
    #[cfg(not(target_os = "windows"))]
    let current = crate::installers::detect(&previous.name)?;
    if current
        .as_ref()
        .is_some_and(|current| same_active(current, previous))
    {
        return Ok(());
    }
    if let Some(current) = &current {
        ensure!(
            location(current) == location(previous),
            "Recovery detected a different installed copy; no other copy was removed"
        );
    }
    #[cfg(target_os = "windows")]
    let restored = {
        if let Some(current) = current {
            crate::installers::uninstall(&current)?;
        }
        crate::installers::run_replacement(&prepared.package, previous, None)?
    };
    #[cfg(target_os = "linux")]
    let restored = crate::installers::run_selected_with_job(
        &prepared.package,
        &previous.name,
        None,
        current.as_ref().is_some_and(|current| {
            crate::updates::version(&current.version).ok()
                > crate::updates::version(&previous.version).ok()
        }),
    )?;
    #[cfg(target_os = "macos")]
    let restored = crate::installers::run_with_job(&prepared.package, &previous.name, None)?;
    ensure!(
        location(&restored) == location(previous)
            && same_identifier(&restored.product_code, &previous.product_code)
            && crate::updates::version(&restored.version)?
                == crate::updates::version(&previous.version)?,
        "Recovery installer did not restore the selected installation"
    );
    Ok(())
}

fn location(installed: &Installed) -> String {
    crate::installations::labels::path_identity(&installed.path)
}

fn same_identifier(a: &str, b: &str) -> bool {
    if cfg!(target_os = "windows") {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

pub(super) fn status(
    paths: &Paths,
    installed: &Installed,
    prefs: &crate::model::Preferences,
    verify_installer: bool,
) -> Result<RollbackStatus> {
    let unsupported = supported(installed).err().map(|error| format!("{error:#}"));
    if unsupported.is_some() {
        return Ok(RollbackStatus {
            installed: installed.clone(),
            previous: None,
            preference: None,
            global_enabled: prefs.rollback_enabled(),
            compression_preference: None,
            global_compression_enabled: prefs.compress_rollbacks,
            unsupported,
        });
    }
    let id = key(installed)?;
    let previous = if unsupported.is_none() {
        read_point(paths, &id)?
    } else {
        None
    };
    if let Some(point) = &previous {
        if verify_installer {
            verify(paths, installed, point)?;
        } else {
            let (package, asset) = recorded_package(paths, installed, point)?;
            let metadata = fs::metadata(&package)?;
            ensure!(
                metadata.is_file() && metadata.len() == asset.size,
                "Retained installer size mismatch"
            );
        }
    }
    Ok(RollbackStatus {
        installed: installed.clone(),
        previous,
        preference: prefs.rollback_overrides.get(&id).copied(),
        global_enabled: prefs.rollback_enabled(),
        compression_preference: None,
        global_compression_enabled: prefs.compress_rollbacks,
        unsupported,
    })
}

pub(super) fn remove(paths: &Paths, expected: &Installed, point: &RollbackPoint) -> Result<()> {
    let folder = bound_folder(paths, expected, point)?;
    if folder.exists() {
        files::no_links(&folder)?;
    }
    let directory = root(paths, &point.installation_id)?;
    if folder.exists() {
        files::remove_managed(&folder, &directory)?;
    }
    fs::remove_file(directory.join("point.json"))?;
    cleanup::prune_empty(paths)
}

pub(crate) fn consume(paths: &Paths, expected: &Installed, point_id: &str) -> Result<()> {
    let point =
        read_point(paths, &key(expected)?)?.context("Previous installer no longer exists")?;
    ensure!(point.id == point_id, "Rollback selection changed");
    remove(paths, expected, &point)
}

pub(crate) fn target_path(
    paths: &Paths,
    app: &str,
    point_id: &str,
    asset: &Asset,
) -> Result<PathBuf> {
    uuid::Uuid::parse_str(point_id)?;
    let installed = paths
        .read_inventory()?
        .context("Missing installation inventory")?
        .apps
        .into_iter()
        .find(|a| a.name == app)
        .context("Native installation is no longer selected")?;
    let point =
        read_point(paths, &key(&installed)?)?.context("Previous installer no longer exists")?;
    let native = point.native.as_ref().context("Not an installer rollback")?;
    ensure!(
        point.id == point_id
            && serde_json::to_value(&native.asset)? == serde_json::to_value(asset)?,
        "Retained installer changed"
    );
    verify(paths, &installed, &point)
}

pub(crate) fn validate_plan(paths: &Paths, plan: &crate::updates::ReleasePlan) -> Result<()> {
    let id = plan
        .native_rollback
        .as_ref()
        .context("Missing rollback authorization")?;
    ensure!(
        plan.entries.len() == 1 && !plan.additional_installation,
        "Rollback requires one selected installation"
    );
    let entry = &plan.entries[0];
    ensure!(
        entry.preferences.release_format == "installer"
            && entry.action == "Update"
            && entry.error.is_none(),
        "Invalid rollback operation"
    );
    let asset = entry.asset.as_ref().context("Missing retained installer")?;
    target_path(paths, &entry.app, id, asset)?;
    let installed = paths
        .read_inventory()?
        .context("Installation inventory no longer exists")?
        .apps
        .into_iter()
        .find(|a| a.name == entry.app)
        .context("Missing installed app")?;
    let point = read_point(paths, &key(&installed)?)?.context("Missing rollback point")?;
    let native = point.native.as_ref().context("Not an installer rollback")?;
    ensure!(
        entry.version == point.version
            && plan.requested_tag.as_deref() == Some(native.tag.as_str())
            && crate::model::release_arch(&entry.preferences.architecture)
                == crate::model::release_arch(&installed.architecture),
        "Rollback package selection changed"
    );
    Ok(())
}

pub(crate) fn validate_replacement(
    paths: &Paths,
    previous: &Installed,
    id: &str,
    installed: &Installed,
) -> Result<()> {
    let point =
        read_point(paths, &key(previous)?)?.context("Previous installer no longer exists")?;
    let native = point.native.as_ref().context("Not a native rollback")?;
    ensure!(
        point.id == id
            && location(previous) == location(installed)
            && previous.windows_msi_scope == installed.windows_msi_scope,
        "Rollback changed installation scope or location"
    );
    validate_identity(&native.identity, installed)
}

pub(super) fn restore(
    paths: &Paths,
    expected: &Installed,
    point: &RollbackPoint,
    job: &Job,
) -> Result<()> {
    let plan = {
        let _lock = crate::platform::Lock::take("Local\\CraftAppsManager")?;
        selected(paths, expected)?;
        let file = verify(paths, expected, point)?;
        let native = point.native.as_ref().context("Not an installer rollback")?;
        let inspected = native_package::inspect(paths, expected, &file, job)?;
        ensure!(
            inspected == native.identity,
            "Retained installer identity changed"
        );
        // Reject a retained package with incompatible scope before staging recovery.
        #[cfg(target_os = "windows")]
        let expected = &crate::installers::snapshot_msi_scope(expected)?;
        #[cfg(target_os = "windows")]
        crate::installers::validate_msi_replacement_scope(&file, expected)?;
        // Stage verified current-version recovery before authorizing replacement.
        prepare_current(paths, expected, job, None)?;
        crate::updates::plan_native_rollback(
            paths,
            expected,
            &point.id,
            &point.version,
            &native.tag,
            native.asset.clone(),
        )?
    };
    crate::updates::execute_plan(paths, &plan, job)
}

pub(crate) fn discard_uninstalled(paths: &Paths, removed: &Installed) -> Result<()> {
    let id = key(removed)?;
    let config = paths
        .read_inventory()?
        .context("Missing uninstall inventory")?;
    ensure!(
        !config
            .apps
            .iter()
            .chain(&config.installations)
            .filter(|a| a.install_kind == "installer" && !a.path.is_empty())
            .any(|a| key(a).is_ok_and(|key| key == id)),
        "Native installation is still present in inventory"
    );
    if let Some(point) = read_point(paths, &id)? {
        remove(paths, removed, &point)?;
    }
    cleanup::prune_empty(paths)
}

pub(super) fn key(installed: &Installed) -> Result<String> {
    crate::model::valid_app(&installed.name)?;
    ensure!(
        installed.install_kind == "installer"
            && !installed.path.is_empty()
            && !installed.architecture.is_empty(),
        "Native installation identity is incomplete"
    );
    let binding = serde_json::to_vec(&(
        "native-rollback-v1",
        crate::model::repository(&installed.name).to_ascii_lowercase(),
        crate::model::release_os(),
        crate::model::release_arch(&installed.architecture),
        location(installed),
    ))?;
    let hash = Sha256::digest(binding);
    let bytes: [u8; 16] = hash[..16].try_into().unwrap();
    Ok(uuid::Uuid::from_bytes(bytes).to_string())
}

fn same_active(a: &Installed, b: &Installed) -> bool {
    a.name == b.name
        && a.install_kind == b.install_kind
        && location(a) == location(b)
        && crate::model::release_arch(&a.architecture)
            == crate::model::release_arch(&b.architecture)
        && crate::updates::version(&a.version)
            .ok()
            .zip(crate::updates::version(&b.version).ok())
            .is_some_and(|(a, b)| a == b)
        && same_identifier(&a.product_code, &b.product_code)
        && a.windows_msi_scope
            .as_deref()
            .is_none_or(|scope| b.windows_msi_scope.as_deref() == Some(scope))
}

pub(crate) fn same_version(package: &PackageIdentity, installed: &str) -> Result<bool> {
    let version = if matches!(package.format.as_str(), "deb" | "rpm" | "arch") {
        let text = package
            .version
            .split_once(':')
            .map_or(package.version.as_str(), |(_, v)| v);
        text.rsplit_once('-').map_or(text, |(v, _)| v)
    } else {
        &package.version
    };
    Ok(crate::updates::version(version)? == crate::updates::version(installed)?)
}

pub(super) fn validate_identity(identity: &PackageIdentity, installed: &Installed) -> Result<()> {
    validate_identity_runtime(identity, installed, installed)
}

fn validate_identity_runtime(
    identity: &PackageIdentity,
    installed: &Installed,
    runtime: &Installed,
) -> Result<()> {
    #[cfg(not(target_os = "windows"))]
    let _ = runtime;
    ensure!(
        !identity.identity.is_empty() && !identity.product.is_empty(),
        "Installer identity is incomplete"
    );
    ensure!(
        same_identifier(&identity.product, &installed.product_code),
        "Installer does not belong to this installed application"
    );
    ensure!(
        same_version(identity, &installed.version)?,
        "Installer version does not match the installed version"
    );
    let architecture_matches = crate::model::release_arch(&identity.architecture)
        == crate::model::release_arch(&installed.architecture);
    // MSI Template describes installer/component bitness, not an AnyCPU CLR
    // application's runtime. Keep the package's physical architecture recorded;
    // accept this mismatch only with executable evidence for the selected app.
    #[cfg(target_os = "windows")]
    let architecture_matches = architecture_matches
        || (identity.format == "msi"
            && identity.architecture == "x86"
            && crate::model::release_arch(&runtime.architecture)
                == crate::model::release_arch(&installed.architecture)
            && verified_anycpu_runtime(runtime)?);
    ensure!(
        architecture_matches,
        "Installer architecture does not match this installation"
    );
    Ok(())
}

#[cfg(target_os = "windows")]
fn verified_anycpu_runtime(installed: &Installed) -> Result<bool> {
    let root = Path::new(&installed.path);
    let Some(executable) = crate::model::installed_executable(root, &installed.name) else {
        return Ok(false);
    };
    files::inside(&executable, root)?;
    files::no_links(&executable)?;
    Ok(
        crate::downloads::windows_anycpu_architecture(&executable)?.is_some_and(|actual| {
            crate::model::release_arch(actual)
                == crate::model::release_arch(&installed.architecture)
        }),
    )
}

pub(super) fn publish(
    paths: &Paths,
    previous: &Installed,
    updated: &Installed,
    prepared: &Prepared,
    job: &Job,
) -> Result<()> {
    publish_with(paths, previous, updated, prepared, job, |path, point| {
        files::write_json(path, point)
    })
}

pub(super) fn publish_with(
    paths: &Paths,
    previous: &Installed,
    updated: &Installed,
    prepared: &Prepared,
    job: &Job,
    write: impl FnOnce(&Path, &RollbackPoint) -> Result<()>,
) -> Result<()> {
    let asset = &prepared.asset;
    let package = &prepared.package;
    validate_identity(&prepared.identity, previous)?;
    ensure!(
        key(previous)? == key(updated)?
            && previous.windows_msi_scope == updated.windows_msi_scope
            && previous.name == updated.name,
        "Native update changed installation location or architecture"
    );
    files::safe_relative(&asset.name)?;
    ensure!(
        Path::new(&asset.name).components().count() == 1,
        "Invalid native package filename"
    );
    ensure!(
        crate::updates::expected_asset_url(
            &asset.browser_download_url,
            &crate::model::repository(&previous.name)
        ),
        "Native package repository mismatch"
    );
    files::no_links(package)?;
    crate::network::verify_asset(package, asset)?;
    let id = key(updated)?;
    let directory = root(paths, &id)?;
    let old = read_point(paths, &id)?;
    if let Some(old) = &old {
        ensure!(
            old.app == previous.name
                && old.native.as_ref().is_some_and(|native| native.repository
                    == crate::model::repository(&previous.name).to_ascii_lowercase()
                    && native.platform == crate::model::release_os()),
            "Previous installer storage belongs to another application"
        );
    }
    let point_id = uuid::Uuid::new_v4().to_string();
    let destination = directory.join(&point_id);
    files::inside(&destination, &paths.at("rollbacks"))?;
    let copied = destination.join(&asset.name);
    let result = (|| {
        fs::create_dir_all(&destination)?;
        fs::copy(package, &copied)?;
        crate::network::verify_asset(&copied, asset)?;
        let point = RollbackPoint {
            id: point_id,
            version: previous.version.clone(),
            bytes: asset.size,
            architecture: previous.architecture.clone(),
            created_at: chrono::Utc::now().to_rfc3339(),
            installation_id: id,
            app: previous.name.clone(),
            package_type: previous.package_type.clone(),
            executable: asset.name.clone().into(),
            files: manifest(&destination)?,
            directories: BTreeMap::new(),
            compressed: false,
            native: Some(NativePoint {
                schema: 1,
                repository: crate::model::repository(&previous.name).to_ascii_lowercase(),
                platform: crate::model::release_os().into(),
                active: updated.clone(),
                original_migration: None,
                asset: asset.clone(),
                tag: prepared.tag.clone(),
                identity: prepared.identity.clone(),
            }),
        };
        write(&directory.join("point.json"), &point)?;
        Ok::<_, anyhow::Error>(())
    })();
    if result.is_err() {
        let _ = files::remove_managed(&destination, &directory);
        cleanup_empty(paths, job);
        return result;
    }
    if let Some(old) = old {
        let old_folder = directory.join(old.id);
        let cleanup = (|| {
            ensure!(
                manifest(&old_folder)? == old.files,
                "Older rollback files changed; preserved for manual review"
            );
            files::remove_managed(&old_folder, &directory)
        })();
        if let Err(error) = cleanup {
            job.log(&format!("Previous installer cleanup warning: {error:#}"));
        }
    }
    Ok(())
}

pub(super) fn bound_folder(
    paths: &Paths,
    expected: &Installed,
    point: &RollbackPoint,
) -> Result<PathBuf> {
    let id = key(expected)?;
    let current = read_point(paths, &id)?.context("Previous installer no longer exists")?;
    ensure!(
        serde_json::to_value(&current)? == serde_json::to_value(point)?,
        "Previous installer changed; reopen Rollback"
    );
    let native = point
        .native
        .as_ref()
        .context("Previous version is not a native installer")?;
    ensure!(
        native.schema == 1
            && native.platform == crate::model::release_os()
            && native.repository == crate::model::repository(&expected.name).to_ascii_lowercase()
            && point.installation_id == id
            && point.app == expected.name
            && same_active(&native.active, expected),
        "Previous installer does not match the current installation"
    );
    let folder = root(paths, &id)?.join(&point.id);
    files::inside(&folder, &paths.at("rollbacks"))?;
    Ok(folder)
}

fn recorded_package<'a>(
    paths: &Paths,
    expected: &Installed,
    point: &'a RollbackPoint,
) -> Result<(PathBuf, &'a Asset)> {
    let folder = bound_folder(paths, expected, point)?;
    let native = point.native.as_ref().context("Not an installer rollback")?;
    ensure!(native.original_migration.is_none(), "This rollback requires an installation scope change, which is no longer supported. Its retained installer is preserved for manual recovery; you can also remove this rollback.");
    ensure!(
        point.executable == Path::new(&native.asset.name) && point.files.len() == 1,
        "Retained installer metadata is invalid"
    );
    ensure!(
        crate::updates::expected_asset_url(&native.asset.browser_download_url, &native.repository),
        "Retained installer repository mismatch"
    );
    let package = folder.join(&point.executable);
    files::inside(&package, &folder)?;
    Ok((package, &native.asset))
}

pub(super) fn verify(
    paths: &Paths,
    expected: &Installed,
    point: &RollbackPoint,
) -> Result<PathBuf> {
    let (package, asset) = recorded_package(paths, expected, point)?;
    let folder = package
        .parent()
        .context("Missing retained installer folder")?;
    ensure!(
        manifest(folder)? == point.files,
        "Retained installer files have changed"
    );
    crate::network::verify_asset(&package, asset)?;
    Ok(package)
}
