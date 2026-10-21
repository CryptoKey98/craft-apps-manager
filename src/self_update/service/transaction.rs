use super::{
    catalog::{TrustContext, VerifiedCatalog},
    model::*,
    trust::lock_name,
};
use crate::{
    files,
    jobs::Job,
    model::{Asset, Paths},
    self_update,
};
use anyhow::Context;
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Debug, Clone)]
pub struct SignedSelection {
    pub catalog: VerifiedCatalog,
    pub offer: UpdateOffer,
}
pub use super::api::ExpectedIdentity;
#[derive(Serialize, Deserialize)]
pub struct TransactionPlanV2 {
    pub schema: u32,
    pub source_hash: String,
    pub profile: ClientProfile,
    pub offer: UpdateOffer,
    pub envelope: Vec<u8>,
    pub legacy: self_update::Plan,
}
pub struct CommitAuthorization {
    _guard: super::trust::TrustGuard,
}
pub fn authorize_commit(
    root: &Path,
    selection: &SignedSelection,
    profile: &ClientProfile,
    trust: &TrustContext,
    now: i64,
) -> Result<CommitAuthorization> {
    let store = super::trust::TrustStore::new(root, &trust.repository, &trust.channel);
    let guard = store.guard()?;
    let catalog = super::catalog::verify_catalog(
        selection.catalog.envelope(),
        trust,
        now,
        store.load()?.as_ref(),
    )?;
    validate_offer(&catalog, profile, &selection.offer)?;
    Ok(CommitAuthorization { _guard: guard })
}
pub(crate) fn authorize_plan(
    path: &Path,
    legacy: &self_update::Plan,
) -> Result<CommitAuthorization> {
    let plan = read_plan(path)?;
    ensure!(
        serde_json::to_value(&plan.legacy)? == serde_json::to_value(legacy)?,
        "Commit plan changed after verified staging"
    );
    validate_source(
        &self_update::staged_executable(&legacy.target),
        &plan.source_hash,
    )?;
    let trust = production_trust()?;
    let catalog = super::catalog::verify_catalog(
        &plan.envelope,
        &trust,
        chrono::Utc::now().timestamp(),
        None,
    )?;
    let profile = profile_for(
        crate::model::release_os(),
        crate::model::MANAGER_ARCH,
        plan.profile.installation.clone(),
    );
    authorize_commit(
        &legacy.root,
        &SignedSelection {
            catalog,
            offer: plan.offer,
        },
        &profile,
        &trust,
        chrono::Utc::now().timestamp(),
    )
}
pub fn validate_build_identity(bytes: &[u8], expected: &ExpectedIdentity) -> Result<()> {
    #[derive(Deserialize)]
    struct Identity {
        version: String,
        revision: u32,
        target: String,
    }
    ensure!(bytes.len() <= 4096, "Build identity output too large");
    let identity: Identity = serde_json::from_slice(bytes)?;
    let architecture = match expected.architecture.as_str() {
        "x64" => "x86_64-",
        "x86" => "i686-",
        "arm64" => "aarch64-",
        _ => anyhow::bail!("Unsupported manager architecture"),
    };
    let platform = match expected.platform.as_str() {
        "windows" => "windows",
        "linux" => "linux",
        "macos" => "apple-darwin",
        _ => anyhow::bail!("Unsupported manager platform"),
    };
    ensure!(
        crate::updates::version(&identity.version)? == crate::updates::version(&expected.version)?
            && identity.revision == expected.revision
            && identity.target.starts_with(architecture)
            && identity.target.contains(platform),
        "Manager payload identity/version/revision mismatch"
    );
    Ok(())
}
pub fn asset(package: &PackageRecord) -> Asset {
    Asset {
        name: package.asset.clone(),
        size: package.size,
        digest: Some(format!("sha256:{}", package.sha256)),
        browser_download_url: package.url.clone(),
    }
}
pub fn verify_executable(
    executable: &Path,
    expected: &ExpectedIdentity,
    stage: &Path,
) -> Result<()> {
    files::no_links(executable)?;
    let output = stage.join(format!("identity-{}.json", uuid::Uuid::new_v4().simple()));
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output)?;
    let mut child = crate::platform::hidden(
        Command::new(executable)
            .arg("--build-identity")
            .stdin(Stdio::null())
            .stdout(file)
            .stderr(Stdio::null()),
    )
    .spawn()?;
    let result = (|| {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            ensure!(
                fs::metadata(&output)?.len() <= 4096,
                "Build identity output too large"
            );
            if let Some(status) = child.try_wait()? {
                ensure!(status.success(), "Manager build identity failed");
                break;
            }
            ensure!(
                Instant::now() < deadline,
                "Manager build identity timed out"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        validate_build_identity(&fs::read(&output)?, expected)
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = fs::remove_file(output);
    result
}
pub(crate) fn expected(c: &VerifiedCatalog, p: &PackageRecord) -> ExpectedIdentity {
    ExpectedIdentity {
        version: c.catalog().release(p).version.clone(),
        revision: p.revision,
        architecture: p.architecture.clone(),
        platform: p.platform.clone(),
    }
}
fn prepare_payload(
    p: &PackageRecord,
    download: &Path,
    stage: &Path,
    job: &Job,
) -> Result<super::handlers::StagedPayload> {
    match p.handler.as_str() {
        ZIP | TAR => super::handlers::archive::stage(p, download, &stage.join("payload"), job),
        #[cfg(target_os = "macos")]
        DMG => super::handlers::dmg::stage(p, download, &stage.join("payload"), job),
        _ => anyhow::bail!("Unsupported update transport for this installation"),
    }
}
fn payload_plan(
    p: &PackageRecord,
    download: &Path,
    stage: &Path,
    c: &VerifiedCatalog,
    job: &Job,
) -> Result<(PathBuf, String)> {
    #[cfg(target_os = "windows")]
    if p.handler == "native.msi.v1" {
        super::handlers::native::validate_msi(
            download,
            p.native_version
                .as_deref()
                .context("Missing native package version")?,
            &p.architecture,
        )?;
        return Ok((download.into(), files::hash(download)?));
    }
    let payload = prepare_payload(p, download, stage, job)?;
    #[cfg(target_os = "macos")]
    {
        self_update::verify_bundle(&payload.root)?;
        ensure!(
            self_update::staged_executable(&payload.root) == payload.entrypoint,
            "Declared entrypoint is not the bundle's main executable"
        );
        ensure!(
            crate::platform::bundle_value(&payload.root, "CFBundleShortVersionString").as_deref()
                == Some(c.catalog().release(p).version.as_str())
                && crate::platform::bundle_value(&payload.root, "CFBundleVersion")
                    .and_then(|s| s.parse::<u32>().ok())
                    == Some(p.revision),
            "Bundle version/revision mismatch"
        );
    }
    verify_executable(&payload.entrypoint, &expected(c, p), stage)?;
    job.check()?;
    #[cfg(target_os = "macos")]
    {
        let package = stage.join("package");
        fs::create_dir(&package)?;
        let bundle = package.join("Craft Apps Manager.app");
        fs::rename(payload.root, &bundle)?;
        Ok((bundle, payload.hash))
    }
    #[cfg(not(target_os = "macos"))]
    Ok((payload.entrypoint, payload.hash))
}
pub fn prepare_offer(
    paths: &Paths,
    catalog: &VerifiedCatalog,
    offer: &UpdateOffer,
    job: &Job,
) -> Result<PathBuf> {
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    super::native_admin::revoke(&paths.root);
    super::bridge::revoke(&paths.root)?;
    let journey = if offer.route_ids.len() > 1 {
        Some(super::bridge::Journey::approve(
            &SignedSelection {
                catalog: catalog.clone(),
                offer: offer.clone(),
            },
            &installed_profile(paths)?,
            &production_trust()?,
            chrono::Utc::now().timestamp(),
        )?)
    } else {
        None
    };
    let result = prepare_approved(paths, catalog, offer, job, journey);
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    if result.is_err() {
        super::native_admin::revoke(&paths.root);
    }
    result
}
pub fn prepare_continuation(
    paths: &Paths,
    selection: &SignedSelection,
    journey: super::bridge::Journey,
    job: &Job,
) -> Result<PathBuf> {
    // A permit is bound to this exact freshly checked offer, not an arbitrary later selection.
    journey.validate_selection(
        selection,
        &installed_profile(paths)?,
        &production_trust()?,
        chrono::Utc::now().timestamp(),
    )?;
    super::bridge::revoke(&paths.root)?;
    let result = prepare_approved(
        paths,
        &selection.catalog,
        &selection.offer,
        job,
        Some(journey),
    );
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    if result.is_err() {
        super::native_admin::revoke(&paths.root);
    }
    result
}
fn prepare_approved(
    paths: &Paths,
    catalog: &VerifiedCatalog,
    offer: &UpdateOffer,
    job: &Job,
    journey: Option<super::bridge::Journey>,
) -> Result<PathBuf> {
    super::bridge::ensure_idle(&paths.root, job.operation_id().as_deref())?;
    // Existing execution locks also cover a Builder in a different process.
    // In particular, native packages must not replace its executable while it builds.
    let _execution = super::bridge::execution_gate()?;
    let profile = installed_profile(paths)?;
    let trust = production_trust()?;
    let prior =
        super::trust::TrustStore::new(&paths.root, &trust.repository, &trust.channel).load()?;
    let catalog = super::catalog::verify_catalog(
        catalog.envelope(),
        &trust,
        chrono::Utc::now().timestamp(),
        prior.as_ref(),
    )?;
    validate_offer(&catalog, &profile, offer)?;
    let p = catalog
        .catalog()
        .packages
        .iter()
        .find(|p| p.id == offer.immediate_package_id)
        .context("Missing selected package")?;
    let executable = std::env::current_exe()?.canonicalize()?;
    let _lock = target_lock(&executable)?;
    job.check()?;
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    super::native_admin::prepare(
        paths,
        &SignedSelection {
            catalog: catalog.clone(),
            offer: offer.clone(),
        },
        &profile,
        super::native_admin::exists(&paths.root),
        journey
            .as_ref()
            .map(super::bridge::Journey::approval_expires),
        job,
    )?;
    #[cfg(target_os = "linux")]
    if matches!(
        profile.installation,
        InstallationKind::Deb | InstallationKind::Rpm | InstallationKind::Arch
    ) {
        ensure!(
            p.native_version.is_some(),
            "Native package version required"
        );
        let plan = self_update::prepare_native_v2(
            paths,
            &self_update::Available {
                version: catalog.catalog().release(p).version.clone(),
                revision: p.revision,
                package_version: p.native_version.clone(),
                asset: asset(p),
                selection: Some(SignedSelection {
                    catalog: catalog.clone(),
                    offer: offer.clone(),
                }),
            },
            job,
        )?;
        super::coordinator::record_bridge(paths, &catalog, offer)?;
        let legacy: self_update::Plan = files::read_json(&plan)?;
        job.check()?;
        super::bridge::ensure_idle(&paths.root, job.operation_id().as_deref())?;
        super::bridge::record_prepared(&legacy, &plan, journey.as_ref())?;
        return Ok(plan);
    }
    let target = executable.clone();
    #[cfg(target_os = "macos")]
    let target = self_update::app_bundle(&target)?;
    let stage_home = if profile.installation == InstallationKind::Msi {
        paths.root.as_path()
    } else {
        target.parent().context("Missing target parent")?
    };
    #[cfg(target_os = "macos")]
    let protected = super::native_admin::exists(&paths.root);
    #[cfg(target_os = "macos")]
    let stage = if protected {
        self_update::create_macos_admin_stage(&paths.root)?
    } else {
        self_update::create_macos_stage(stage_home)?
    };
    #[cfg(not(target_os = "macos"))]
    let stage = {
        let root = stage_home.join("runtime/self-update");
        fs::create_dir_all(&root)?;
        files::no_links(&root)?;
        let stage = root.join(uuid::Uuid::new_v4().simple().to_string());
        fs::create_dir(&stage)?;
        stage
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&stage, fs::Permissions::from_mode(0o700))?;
    }
    let download = stage.join(if profile.installation == InstallationKind::Msi {
        "release.msi"
    } else {
        "download.package"
    });
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    let cached = super::native_admin::stage(paths, p, &download)?;
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    let cached = false;
    if !cached {
        super::transport::download_asset(p, &download, job)?;
    }
    let (staged, hash) = payload_plan(p, &download, &stage, &catalog, job)?;
    let source_hash = files::hash(&executable)?;
    fs::copy(&executable, stage.join(self_update::helper_name()))?;
    #[cfg(target_os = "macos")]
    self_update::clear_quarantine(&stage.join(self_update::helper_name()))?;
    let plan = TransactionPlanV2 {
        schema: 2,
        source_hash,
        profile: profile.clone(),
        offer: offer.clone(),
        envelope: catalog.envelope().to_vec(),
        legacy: self_update::Plan {
            expected_identity: Some(expected(&catalog, p)),
            layout: {
                #[cfg(target_os = "macos")]
                {
                    if protected {
                        2
                    } else {
                        1
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    0
                }
            },
            msi: profile.installation == InstallationKind::Msi,
            linux_package: false,
            parent_pid: std::process::id(),
            target,
            staged,
            hash,
            root: paths.root.clone(),
            tools: paths.tools.clone(),
        },
    };
    let path = stage.join("authorization.json");
    files::write_bytes(&path, &serde_json::to_vec(&plan)?)?;
    super::coordinator::record_bridge(paths, &catalog, offer)?;
    job.check()?;
    super::bridge::ensure_idle(&paths.root, job.operation_id().as_deref())?;
    super::bridge::record_prepared(&plan.legacy, &path, journey.as_ref())?;
    Ok(path)
}
#[cfg(target_os = "windows")]
pub(crate) fn install_native_batch(
    plan: &self_update::Plan,
    authorization: Option<&Path>,
) -> Result<Option<(i32, String)>> {
    let Some(path) = authorization else {
        return Ok(None);
    };
    let signed = read_plan(path)?;
    ensure!(
        serde_json::to_value(&signed.legacy)? == serde_json::to_value(plan)?,
        "Native MSI plan changed"
    );
    let selection = SignedSelection {
        catalog: verify_for_native(&signed.envelope)?,
        offer: signed.offer,
    };
    super::native_admin::install(
        &Paths::new(plan.root.clone(), Some(plan.tools.clone())),
        &selection,
        &signed.profile,
        None,
    )
}
#[cfg(target_os = "windows")]
fn verify_for_native(envelope: &[u8]) -> Result<VerifiedCatalog> {
    super::catalog::verify_catalog(
        envelope,
        &production_trust()?,
        chrono::Utc::now().timestamp(),
        None,
    )
}
#[cfg(target_os = "windows")]
pub(crate) fn acknowledge_native_batch(
    plan: &self_update::Plan,
    authorization: Option<&Path>,
) -> Result<()> {
    if let Some(path) = authorization {
        let signed = read_plan(path)?;
        let catalog = verify_for_native(&signed.envelope)?;
        let package = catalog
            .catalog()
            .packages
            .iter()
            .find(|p| p.id == signed.offer.immediate_package_id)
            .context("Missing native MSI package")?;
        super::native_admin::acknowledge(&plan.root, package)?;
    }
    Ok(())
}
#[cfg(target_os = "macos")]
pub(crate) fn verify_macos_admin_plan(
    plan: &self_update::Plan,
    authorization: &Path,
) -> Result<()> {
    let signed = read_plan(authorization)?;
    ensure!(
        authorization
            == authorization
                .parent()
                .context("No authorization parent")?
                .join("authorization.json")
            && serde_json::to_value(&signed.legacy)? == serde_json::to_value(plan)?,
        "Protected authorization differs from plan"
    );
    super::native_admin::verify_macos_session(&signed)
}
#[cfg(target_os = "macos")]
pub(crate) fn install_macos_batch(plan: &self_update::Plan, authorization: &Path) -> Result<()> {
    verify_macos_admin_plan(plan, authorization)?;
    let signed = read_plan(authorization)?;
    let selection = SignedSelection {
        catalog: super::catalog::verify_catalog(
            &signed.envelope,
            &production_trust()?,
            chrono::Utc::now().timestamp(),
            None,
        )?,
        offer: signed.offer,
    };
    let result = super::native_admin::install(
        &Paths::new(plan.root.clone(), Some(plan.tools.clone())),
        &selection,
        &signed.profile,
        None,
    )?
    .context("Protected update lost administrator approval")?;
    ensure!(result.0 == 0, "Privileged bundle replacement failed");
    Ok(())
}
#[cfg(target_os = "macos")]
pub(crate) fn acknowledge_macos_batch(
    plan: &self_update::Plan,
    authorization: &Path,
) -> Result<()> {
    let signed = read_plan(authorization)?;
    let catalog = super::catalog::verify_catalog(
        &signed.envelope,
        &production_trust()?,
        chrono::Utc::now().timestamp(),
        None,
    )?;
    let package = catalog
        .catalog()
        .packages
        .iter()
        .find(|p| p.id == signed.offer.immediate_package_id)
        .context("Missing bundle package")?;
    super::native_admin::acknowledge(&plan.root, package)
}

fn read_plan(path: &Path) -> Result<TransactionPlanV2> {
    files::no_links(path)?;
    ensure!(
        fs::metadata(path)?.len() <= 12 * 1024 * 1024,
        "Update authorization too large"
    );
    let plan: TransactionPlanV2 = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(plan.schema == 2, "Unsupported update plan schema");
    Ok(plan)
}
pub fn launch_v2(path: &Path) -> Result<()> {
    let plan = read_plan(path)?;
    validate_source(
        &self_update::staged_executable(&plan.legacy.target),
        &plan.source_hash,
    )?;
    let helper = path
        .parent()
        .context("No update stage")?
        .join(self_update::helper_name());
    ensure!(
        files::hash(&helper)? == plan.source_hash,
        "Update helper mismatch"
    );
    crate::platform::hidden(Command::new(helper).arg("--apply-self-update").arg(path)).spawn()?;
    Ok(())
}
pub fn apply_v2(path: &Path) -> Result<()> {
    let mut plan = read_plan(path)?;
    let stage = path
        .parent()
        .context("Missing update stage")?
        .canonicalize()?;
    let source = self_update::staged_executable(&plan.legacy.target);
    let _lock = target_lock(&source)?;
    validate_source(&source, &plan.source_hash)?;
    ensure!(
        std::env::current_exe()?.canonicalize()?
            == stage.join(self_update::helper_name()).canonicalize()?
            && files::hash(&std::env::current_exe()?)? == plan.source_hash,
        "Update helper ownership mismatch"
    );
    let profile = installed_profile(&Paths::new(
        plan.legacy.root.clone(),
        Some(plan.legacy.tools.clone()),
    ))?;
    // The helper runs beside the package, not the MSI installation marker.
    let profile = profile_for(
        &profile.platform,
        &profile.architecture,
        plan.profile.installation.clone(),
    );
    ensure!(
        serde_json::to_value(&profile)? == serde_json::to_value(&plan.profile)?,
        "Update source profile mismatch"
    );
    let trust = production_trust()?;
    let prior = super::trust::TrustStore::new(&plan.legacy.root, &trust.repository, &trust.channel)
        .load()?;
    let catalog = super::catalog::verify_catalog(
        &plan.envelope,
        &trust,
        chrono::Utc::now().timestamp(),
        prior.as_ref(),
    )?;
    validate_offer(&catalog, &profile, &plan.offer)?;
    let p = catalog
        .catalog()
        .packages
        .iter()
        .find(|p| p.id == plan.offer.immediate_package_id)
        .context("Missing package")?;
    ensure!(
        plan.legacy.msi == (profile.installation == InstallationKind::Msi)
            && !plan.legacy.linux_package,
        "Conflicting update method"
    );
    let home = plan
        .legacy
        .target
        .parent()
        .context("Missing target folder")?;
    #[cfg(not(target_os = "macos"))]
    {
        let root = if plan.legacy.msi {
            plan.legacy.root.as_path()
        } else {
            home
        };
        validate_executable_stage(&stage, &root.join("runtime/self-update"))?;
    }
    #[cfg(target_os = "macos")]
    if plan.legacy.layout == 2 {
        verify_macos_admin_plan(&plan.legacy, path)?;
        self_update::validate_macos_admin_layout(
            &plan.legacy,
            &stage.join("plan.json"),
            &stage.join(self_update::helper_name()),
        )?;
    } else {
        ensure!(
            stage.parent() == Some(home)
                && stage
                    .file_name()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.starts_with(".craft-manager-update-")),
            "Invalid macOS update stage"
        );
    }
    let download = stage.join(if plan.legacy.msi {
        "release.msi"
    } else {
        "download.package"
    });
    crate::network::verify_asset(&download, &asset(p))?;
    // Rebuild from authenticated bytes; hashes in a writable local plan are not authority.
    for name in ["payload", "package"] {
        let old = stage.join(name);
        if old.exists() {
            files::remove_managed(&old, &stage)?;
        }
    }
    let job = Job::new(
        stage.join("verify.log"),
        &crate::model::BuilderPreferences::default(),
    );
    let (staged, hash) = payload_plan(p, &download, &stage, &catalog, &job)?;
    plan.legacy.staged = staged;
    plan.legacy.hash = hash;
    plan.legacy.expected_identity = Some(expected(&catalog, p));
    let legacy = stage.join("plan.json");
    // Bind the final gate to the rebuilt, normalized payload rather than stale
    // pre-extraction paths (notably Windows MSI paths gaining the UNC prefix).
    files::write_bytes(path, &serde_json::to_vec(&plan)?)?;
    files::write_bytes(&legacy, &serde_json::to_vec(&plan.legacy)?)?;
    self_update::apply_authorized(&legacy, path)
}

#[cfg(not(target_os = "macos"))]
fn validate_executable_stage(stage: &Path, root: &Path) -> Result<()> {
    // Windows canonicalization adds a verbatim prefix to the stage. Compare
    // against the same representation, after rejecting links in the raw root.
    files::no_links(root)?;
    files::inside(stage, &root.canonicalize()?)
}

pub fn target_lock(target: &Path) -> Result<crate::platform::Lock> {
    let target = target.canonicalize()?;
    #[cfg(target_os = "windows")]
    let name = target.to_string_lossy().to_lowercase();
    #[cfg(not(target_os = "windows"))]
    let name = target.to_string_lossy().into_owned();
    crate::platform::Lock::take(&lock_name("Target", name.as_bytes()))
}
pub fn validate_source(target: &Path, hash: &str) -> Result<()> {
    crate::files::no_links(target)?;
    ensure!(
        crate::files::hash(target)? == hash,
        "Installed manager changed after preparation"
    );
    Ok(())
}
pub fn production_trust() -> Result<TrustContext> {
    let bytes = super::catalog::decode_hex(env!("CRAFT_UPDATE_PUBLIC_KEY_HEX"), 32)?;
    Ok(TrustContext {
        repository: crate::self_update::REPOSITORY_NAME.into(),
        channel: env!("CRAFT_UPDATE_CHANNEL").into(),
        keys: vec![bytes.try_into().expect("32-byte key")],
    })
}
pub fn profile_for(
    platform: &str,
    architecture: &str,
    installation: InstallationKind,
) -> ClientProfile {
    let mut capabilities = std::collections::BTreeSet::from([LAYOUT.into()]);
    for handler in [
        ZIP,
        TAR,
        DMG,
        "native.msi.v1",
        "native.deb.v1",
        "native.rpm.v1",
        "native.arch.v1",
    ] {
        if super::routing::known_handler(handler, platform, &installation) {
            capabilities.insert(handler.into());
        }
    }
    if platform == "macos" && installation == InstallationKind::Bundle {
        capabilities.insert("admin.bundle-replace.v1".into());
    }
    ClientProfile {
        repository: crate::self_update::REPOSITORY_NAME.into(),
        channel: env!("CRAFT_UPDATE_CHANNEL").into(),
        version: env!("CARGO_PKG_VERSION").into(),
        revision: crate::update_revisions::BUILD_REVISION,
        platform: platform.into(),
        architecture: architecture.into(),
        installation,
        capabilities,
    }
}
pub fn installed_profile(_paths: &crate::model::Paths) -> Result<ClientProfile> {
    #[allow(unused_mut)]
    let mut installation = if cfg!(target_os = "macos") {
        InstallationKind::Bundle
    } else if crate::self_update::installed_with_msi() {
        InstallationKind::Msi
    } else {
        InstallationKind::Portable
    };
    #[cfg(target_os = "linux")]
    if crate::self_update::installed_with_linux_package() {
        installation = match crate::installers::package_kind()? {
            crate::installers::PackageKind::Debian => InstallationKind::Deb,
            crate::installers::PackageKind::Rpm => InstallationKind::Rpm,
            crate::installers::PackageKind::Arch => InstallationKind::Arch,
        };
    }
    Ok(profile_for(
        crate::model::release_os(),
        crate::model::MANAGER_ARCH,
        installation,
    ))
}
pub fn validate_offer(
    catalog: &VerifiedCatalog,
    profile: &ClientProfile,
    offer: &UpdateOffer,
) -> Result<()> {
    super::routing::validate_selection(catalog, profile, offer)
}

#[cfg(all(test, target_os = "windows"))]
mod windows_stage_tests {
    use super::*;

    #[test]
    fn native_stage_accepts_canonical_child_of_normal_windows_library() {
        let temp = std::env::temp_dir().join(format!("craft-msi-stage-{}", uuid::Uuid::new_v4()));
        let root = temp.join("library/runtime/self-update");
        let stage = root.join("owned-stage");
        fs::create_dir_all(&stage).unwrap();
        let canonical_stage = stage.canonicalize().unwrap();
        let result = validate_executable_stage(&canonical_stage, &root);
        fs::remove_dir_all(&temp).unwrap();
        result.unwrap();
    }

    #[test]
    fn native_stage_still_rejects_root_and_unrelated_windows_folder() {
        let temp =
            std::env::temp_dir().join(format!("craft-msi-boundary-{}", uuid::Uuid::new_v4()));
        let root = temp.join("library/runtime/self-update");
        let outside = temp.join("unrelated");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        assert!(validate_executable_stage(&root.canonicalize().unwrap(), &root).is_err());
        assert!(validate_executable_stage(&outside.canonicalize().unwrap(), &root).is_err());
        fs::remove_dir_all(&temp).unwrap();
    }
}
