use super::*;

pub(super) fn owner(paths: &Paths, installed: &Installed) -> Result<portable::Ownership> {
    ensure!(
        installed.install_kind == "portable" && !installed.path.is_empty(),
        "Previous versions require a portable installation"
    );
    let root = Path::new(&installed.path);
    portable::validate_path(root)?;
    portable::validate(paths, &installed.name, root)?;
    let marker = root.join(portable::MARKER);
    files::inside(&marker, root)?;
    let owner: portable::Ownership = files::read_json(&marker)?;
    ensure!(
        owner.app == installed.name
            && crate::portable_mode::same_library(
                &owner.library,
                &std::path::absolute(&paths.root)?
            ),
        "Installation belongs to another library"
    );
    if let Some(id) = &owner.installation_id {
        uuid::Uuid::parse_str(id)?;
    }
    Ok(owner)
}

pub(super) fn identity(paths: &Paths, installed: &Installed) -> Result<String> {
    let id = owner(paths, installed)?
        .installation_id
        .context("Installation has no rollback identity")?;
    if let Some(config) = paths.read_inventory()? {
        for copy in config.apps.iter().chain(&config.installations) {
            if copy.install_kind == "portable"
                && copy.path != installed.path
                && !copy.path.is_empty()
            {
                let marker = Path::new(&copy.path).join(portable::MARKER);
                if marker.exists() {
                    let other: portable::Ownership = files::read_json(&marker)?;
                    ensure!(
                        other.installation_id.as_ref() != Some(&id),
                        "Installation identity is ambiguous across multiple copies"
                    );
                }
            }
        }
    }
    Ok(id)
}

pub(super) fn ensure_identity(paths: &Paths, installed: &Installed) -> Result<String> {
    let mut ownership = owner(paths, installed)?;
    if ownership.installation_id.is_some() {
        return identity(paths, installed);
    }
    let id = uuid::Uuid::new_v4().to_string();
    ownership.installation_id = Some(id.clone());
    files::write_json(
        &Path::new(&installed.path).join(portable::MARKER),
        &ownership,
    )?;
    Ok(id)
}

pub(super) fn root(paths: &Paths, id: &str) -> Result<PathBuf> {
    uuid::Uuid::parse_str(id)?;
    let path = paths.at("rollbacks").join(id);
    files::inside(&path, &paths.at("rollbacks"))?;
    Ok(path)
}

pub(super) fn read_point(paths: &Paths, id: &str) -> Result<Option<RollbackPoint>> {
    let path = root(paths, id)?.join("point.json");
    if !path.exists() {
        return Ok(None);
    }
    files::inside(&path, &paths.at("rollbacks"))?;
    let point: RollbackPoint = files::read_json(&path)?;
    ensure!(point.installation_id == id, "Rollback identity mismatch");
    uuid::Uuid::parse_str(&point.id)?;
    Ok(Some(point))
}

pub(super) fn manifest(path: &Path) -> Result<BTreeMap<PathBuf, (u64, String)>> {
    files::no_links(path)?;
    let mut result = BTreeMap::new();
    for entry in walkdir::WalkDir::new(path).min_depth(1) {
        let entry = entry?;
        ensure!(
            entry.file_type().is_file() || entry.file_type().is_dir(),
            "Previous version contains a special file"
        );
        if entry.file_type().is_file() {
            result.insert(
                entry.path().strip_prefix(path)?.to_path_buf(),
                (entry.metadata()?.len(), files::hash(entry.path())?),
            );
        }
    }
    Ok(result)
}

fn same_selected_installation(current: &Installed, expected: &Installed, os: &str) -> Result<bool> {
    let mut current = current.clone();
    let mut expected = expected.clone();
    // Native macOS releases use a universal package selection. Older detection
    // recorded the manager host's CPU instead, without changing the installation.
    // Normalize only those known labels; every other inventory field stays exact.
    if os == "macos"
        && current.install_kind == "installer"
        && expected.install_kind == "installer"
        && [&current.architecture, &expected.architecture]
            .into_iter()
            .all(|architecture| matches!(architecture.as_str(), "universal" | "x64" | "arm64"))
    {
        current.architecture = "universal".into();
        expected.architecture = "universal".into();
    }
    Ok(serde_json::to_value(current)? == serde_json::to_value(expected)?)
}

pub(super) fn selected(paths: &Paths, expected: &Installed) -> Result<Installed> {
    let config = paths
        .read_inventory()?
        .context("Installation inventory is missing")?;
    ensure!(
        Path::new(&config.apps_root) == paths.root,
        "Installation inventory belongs to another library"
    );
    let current = config
        .apps
        .into_iter()
        .find(|a| a.name == expected.name)
        .context("Installation is no longer selected")?;
    ensure!(
        same_selected_installation(&current, expected, crate::model::release_os())?,
        "Installation changed. Reopen the Rollback dialog."
    );
    Ok(current)
}

pub fn record_update(
    paths: &Paths,
    previous: &Installed,
    updated: &Installed,
    old_folder: &Path,
    job: &Job,
) -> Result<()> {
    if !old_folder.exists() {
        return Ok(());
    }
    let transaction_root = paths.at("runtime/rollback-transactions");
    files::inside(old_folder, &transaction_root)?;
    ensure!(
        old_folder
            .file_name()
            .is_some_and(|name| name == "previous"),
        "Not an update recovery folder"
    );
    let transaction = old_folder.parent().context("Missing transaction folder")?;
    ensure!(
        transaction.parent() == Some(transaction_root.as_path()),
        "Unexpected update recovery location"
    );
    uuid::Uuid::parse_str(
        transaction
            .file_name()
            .and_then(|name| name.to_str())
            .context("Invalid transaction ID")?,
    )?;
    files::no_links(old_folder)?;
    let id = identity(paths, updated)?;
    ensure!(
        previous.name == updated.name
            && previous.path == updated.path
            && previous.architecture == updated.architecture
            && previous.package_type == updated.package_type,
        "Update changed installation format or architecture"
    );
    // Recovery snapshots are immutable and intentionally excluded from portable
    // metadata conversion. Decode their ownership using the active installation's
    // storage context without rewriting the retained bytes or their hashes.
    let mut old_marker = files::read_json(&old_folder.join(portable::MARKER))?;
    crate::portable_mode::convert(
        &Path::new(&updated.path).join(portable::MARKER),
        &mut old_marker,
        false,
    )?;
    let old_marker: portable::Ownership = serde_json::from_value(old_marker)?;
    ensure!(
        old_marker.installation_id.as_deref() == Some(&id)
            && old_marker.app == previous.name
            && crate::portable_mode::same_library(
                &old_marker.library,
                &std::path::absolute(&paths.root)?
            ),
        "Previous installation identity mismatch"
    );
    let prefs = crate::storage::config::ConfigStore::new(paths.root.clone())
        .load_locked()?
        .manager()?;
    if !prefs
        .rollback_overrides
        .get(&id)
        .copied()
        .unwrap_or(prefs.rollback_enabled())
    {
        files::remove_managed(old_folder, &transaction_root)?;
        return cleanup::prune_empty(paths);
    }
    let exe = crate::model::installed_executable(old_folder, &previous.name)
        .context("Previous executable is missing")?;
    let entries = manifest(old_folder)?;
    let point = RollbackPoint {
        id: uuid::Uuid::new_v4().to_string(),
        version: previous.version.clone(),
        bytes: entries.values().map(|(len, _)| len).sum(),
        architecture: previous.architecture.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
        installation_id: id.clone(),
        app: previous.name.clone(),
        package_type: previous.package_type.clone(),
        executable: exe.strip_prefix(old_folder)?.into(),
        files: entries,
        native: None,
    };
    let directory = root(paths, &id)?;
    fs::create_dir_all(&directory)?;
    let old = read_point(paths, &id)?;
    let destination = directory.join(&point.id);
    files::copy_directory_verified(old_folder, &destination)?;
    ensure!(
        manifest(&destination)? == point.files,
        "Retained copy verification failed"
    );
    files::write_json(&directory.join("point.json"), &point)?;
    if let Some(old) = old {
        if let Err(error) = files::remove_managed(&directory.join(old.id), &directory) {
            job.log(&format!("Previous version cleanup warning: {error:#}"));
        }
    }
    files::remove_managed(old_folder, &transaction_root)?;
    cleanup::prune_empty(paths)
}

pub(super) fn bound_folder(
    paths: &Paths,
    expected: &Installed,
    point: &RollbackPoint,
) -> Result<PathBuf> {
    ensure!(
        point.native.is_none(),
        "This is an installer rollback, not a portable snapshot"
    );
    let id = identity(paths, expected)?;
    let current = read_point(paths, &id)?.context("Previous version no longer exists")?;
    ensure!(
        serde_json::to_value(&current)? == serde_json::to_value(point)?,
        "Previous version changed. Reopen the dialog."
    );
    ensure!(
        point.app == expected.name
            && point.installation_id == id
            && point.architecture == expected.architecture
            && point.package_type == expected.package_type,
        "Previous version does not match this installation"
    );
    let path = root(paths, &id)?.join(&point.id);
    files::inside(&path, &paths.at("rollbacks"))?;
    Ok(path)
}

pub(super) fn verify_point(
    paths: &Paths,
    expected: &Installed,
    point: &RollbackPoint,
) -> Result<PathBuf> {
    let path = bound_folder(paths, expected, point)?;
    ensure!(
        manifest(&path)? == point.files,
        "Previous version files have changed"
    );
    files::safe_relative(&point.executable.to_string_lossy().replace('\\', "/"))?;
    ensure!(
        path.join(&point.executable).exists(),
        "Previous executable is missing"
    );
    Ok(path)
}

pub fn remove_previous(paths: &Paths, expected: &Installed, point: &RollbackPoint) -> Result<()> {
    let _lock = crate::platform::Lock::take("Local\\CraftAppsManager")?;
    selected(paths, expected)?;
    let candidate =
        super::removal_candidate(paths, expected)?.context("Previous version no longer exists")?;
    ensure!(
        serde_json::to_value(&candidate)? == serde_json::to_value(point)?,
        "Previous version changed. Reopen the dialog."
    );
    if expected.install_kind == "installer" {
        return native::remove(paths, expected, point);
    }
    let folder = bound_folder(paths, expected, point)?;
    if folder.exists() {
        files::no_links(&folder)?;
    }
    let directory = root(paths, &point.installation_id)?;
    // Clear the pointer first; failed deletion leaves an unadvertised recoverable copy.
    fs::remove_file(directory.join("point.json"))?;
    files::remove_managed(&folder, &directory)?;
    cleanup::prune_empty(paths)
}

#[cfg(test)]
mod selected_compatibility_tests {
    use super::*;
    fn installed() -> Installed {
        Installed {
            name: "designcraft".into(),
            path: "/Applications".into(),
            version: "0.13.0".into(),
            architecture: "universal".into(),
            install_kind: "installer".into(),
            product_code: "ai.storyteller.designcraft".into(),
            ..Default::default()
        }
    }
    #[test]
    fn mac_native_selection_accepts_only_known_legacy_architecture_labels() {
        let expected = installed();
        for architecture in ["x64", "arm64", "universal"] {
            let mut current = expected.clone();
            current.architecture = architecture.into();
            assert!(
                same_selected_installation(&current, &expected, "macos").unwrap(),
                "{architecture}"
            );
            assert!(
                same_selected_installation(&expected, &current, "macos").unwrap(),
                "{architecture}"
            );
        }
        for architecture in ["x86", "unknown", "", "aarch64", "amd64"] {
            let mut current = expected.clone();
            current.architecture = architecture.into();
            assert!(
                !same_selected_installation(&current, &expected, "macos").unwrap(),
                "{architecture}"
            );
        }
    }
    #[test]
    fn mac_native_selection_retains_every_other_installed_identity_check() {
        let expected = installed();
        let mut changed = expected.clone();
        changed.architecture = "x64".into();
        for field in [
            "name",
            "path",
            "version",
            "install_kind",
            "product_code",
            "package_type",
            "installation_label",
            "windows_msi_scope",
        ] {
            let mut value = serde_json::to_value(&changed).unwrap();
            value[field] = serde_json::Value::String("changed".into());
            let current: Installed = serde_json::from_value(value).unwrap();
            assert!(
                !same_selected_installation(&current, &expected, "macos").unwrap(),
                "{field}"
            );
        }
    }
    #[test]
    fn other_platform_and_portable_selections_keep_strict_architecture_checks() {
        let expected = installed();
        let mut current = expected.clone();
        current.architecture = "x64".into();
        for os in ["windows", "linux"] {
            assert!(!same_selected_installation(&current, &expected, os).unwrap());
        }
        let mut portable_expected = expected.clone();
        portable_expected.install_kind = "portable".into();
        current.install_kind = "portable".into();
        assert!(!same_selected_installation(&current, &portable_expected, "macos").unwrap());
    }
}
