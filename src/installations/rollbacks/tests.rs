use super::*;
#[test]
fn transaction_paths_are_distinct_and_confined() {
    let paths = Paths::new(std::env::temp_dir().join("rollback-test"), None);
    let a = staging_path(&paths);
    let b = staging_path(&paths);
    assert_ne!(a, b);
    assert!(a.starts_with(paths.at("runtime/rollback-transactions")));
}
#[test]
fn retained_tree_rejects_changed_content() {
    let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("binary"), b"old").unwrap();
    let before = manifest(&root).unwrap();
    std::fs::write(root.join("binary"), b"new").unwrap();
    assert_ne!(before, manifest(&root).unwrap());
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(not(target_os = "macos"))]
struct Fixture {
    paths: Paths,
    installed: Installed,
    job: Job,
}
#[cfg(not(target_os = "macos"))]
impl Fixture {
    fn new() -> Self {
        let paths = Paths::new(
            std::env::temp_dir().join(format!("craft-rollback-{}", uuid::Uuid::new_v4())),
            None,
        );
        let target = paths.at("releases/photocraft");
        fs::create_dir_all(&target).unwrap();
        fs::write(
            target.join(crate::model::executable_name("photocraft")),
            b"old executable",
        )
        .unwrap();
        let installed = Installed {
            name: "photocraft".into(),
            path: target.display().to_string(),
            version: "1.0.0".into(),
            install_kind: "portable".into(),
            architecture: "x64".into(),
            package_type: "zip".into(),
            ..Default::default()
        };
        let config = crate::model::Config {
            apps_root: paths.root.display().to_string(),
            apps: vec![installed.clone()],
            installations: vec![installed.clone()],
            portable_label_counters: Default::default(),
        };
        files::write_json(&paths.at("installations.json"), &config).unwrap();
        portable::mark(&paths, &installed.name, &target).unwrap();
        let job = Job::new(paths.at("logs/test.log"), &Default::default());
        Self {
            paths,
            installed,
            job,
        }
    }
    fn update(&mut self) -> RollbackPoint {
        let stage = self.paths.at("stage");
        fs::create_dir_all(&stage).unwrap();
        fs::write(
            stage.join(crate::model::executable_name("photocraft")),
            b"new executable",
        )
        .unwrap();
        portable::mark(&self.paths, &self.installed.name, &stage).unwrap();
        prepare_update(&self.paths, &self.installed, &stage).unwrap();
        let old = staging_path(&self.paths);
        let mut updated = self.installed.clone();
        updated.version = "2.0.0".into();
        let mut config = self.paths.read_inventory().unwrap().unwrap();
        config.apps[0] = updated.clone();
        crate::updates::replace_transaction(&stage, Path::new(&updated.path), Some(&old), || {
            self.paths.save_config(&config)
        })
        .unwrap();
        record_update(&self.paths, &self.installed, &updated, &old, &self.job).unwrap();
        self.installed = self.paths.read_inventory().unwrap().unwrap().apps.remove(0);
        status(&self.paths, &self.installed)
            .unwrap()
            .previous
            .unwrap()
    }
}
#[cfg(not(target_os = "macos"))]
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.paths.root);
    }
}

#[cfg(not(target_os = "macos"))]
#[test]
fn verified_rollback_preserves_current_profile_identity_and_location() {
    let mut f = Fixture::new();
    let identity = identity(&f.paths, &f.installed).unwrap();
    let point = f.update();
    let profile = Path::new(&f.installed.path).join("PhotoCraftData");
    fs::create_dir_all(&profile).unwrap();
    fs::write(profile.join("project"), b"current profile").unwrap();
    rollback(&f.paths, &f.installed, &point, &f.job).unwrap();
    assert_eq!(
        fs::read(Path::new(&f.installed.path).join(crate::model::executable_name("photocraft")))
            .unwrap(),
        b"old executable"
    );
    assert_eq!(
        fs::read(profile.join("project")).unwrap(),
        b"current profile"
    );
    assert_eq!(super::identity(&f.paths, &f.installed).unwrap(), identity);
    assert_eq!(
        f.paths.read_inventory().unwrap().unwrap().apps[0].version,
        "1.0.0"
    );
    assert!(status(&f.paths, &f.installed).unwrap().previous.is_none());
}
#[cfg(not(target_os = "macos"))]
#[test]
fn modified_snapshot_cannot_restore_but_can_be_deleted_after_ownership_checks() {
    let mut f = Fixture::new();
    let point = f.update();
    let retained = verify_point(&f.paths, &f.installed, &point).unwrap();
    fs::write(retained.join(&point.executable), b"tampered").unwrap();
    assert!(rollback(&f.paths, &f.installed, &point, &f.job).is_err());
    assert_eq!(
        f.paths.read_inventory().unwrap().unwrap().apps[0].version,
        "2.0.0"
    );
    let mut stale = f.installed.clone();
    stale.path.push_str("-other");
    assert!(remove_previous(&f.paths, &stale, &point).is_err());
    remove_previous(&f.paths, &f.installed, &point).unwrap();
    assert!(!retained.exists());
    assert_eq!(
        f.paths.read_inventory().unwrap().unwrap().apps[0].version,
        "2.0.0"
    );
}
#[cfg(not(target_os = "macos"))]
#[test]
fn settings_conflicts_and_disabling_retention_preserve_previous() {
    let mut f = Fixture::new();
    let point = f.update();
    save_override(&f.paths, &f.installed, None, Some(false)).unwrap();
    assert!(save_override(&f.paths, &f.installed, None, Some(true)).is_err());
    let before = status(&f.paths, &f.installed).unwrap();
    assert_eq!(before.preference, Some(false));
    let old = staging_path(&f.paths);
    files::copy_directory_verified(Path::new(&f.installed.path), &old).unwrap();
    record_update(&f.paths, &f.installed, &f.installed, &old, &f.job).unwrap();
    assert!(!old.exists());
    assert_eq!(
        status(&f.paths, &f.installed).unwrap().previous.unwrap().id,
        point.id
    );
}
#[cfg(not(target_os = "macos"))]
#[test]
fn distinct_copies_and_legacy_identity_are_isolated() {
    let f = Fixture::new();
    let first = identity(&f.paths, &f.installed).unwrap();
    let second = f.paths.at("releases/photocraft-2");
    fs::create_dir_all(&second).unwrap();
    portable::mark_additional(&f.paths, "photocraft", &second).unwrap();
    let mut copy = f.installed.clone();
    copy.path = second.display().to_string();
    assert_ne!(identity(&f.paths, &copy).unwrap(), first);
    let marker = Path::new(&f.installed.path).join(portable::MARKER);
    let mut owner: portable::Ownership = files::read_json(&marker).unwrap();
    owner.installation_id = None;
    files::write_json(&marker, &owner).unwrap();
    assert!(status(&f.paths, &f.installed).unwrap().previous.is_none());
    assert!(files::read_json::<portable::Ownership>(&marker)
        .unwrap()
        .installation_id
        .is_none());
    let id = ensure_identity(&f.paths, &f.installed).unwrap();
    assert_eq!(ensure_identity(&f.paths, &f.installed).unwrap(), id);
}
#[cfg(not(target_os = "macos"))]
#[test]
fn safe_removal_keeps_active_and_unknown_legacy_archives() {
    let mut f = Fixture::new();
    let point = f.update();
    let legacy = f.paths.at("backups/releases/personal.zip");
    fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    fs::write(&legacy, b"legacy").unwrap();
    remove_previous(&f.paths, &f.installed, &point).unwrap();
    assert!(legacy.exists());
    assert!(!root(&f.paths, &point.installation_id).unwrap().exists());
    assert!(Path::new(&f.installed.path).exists());
    assert!(status(&f.paths, &f.installed).unwrap().previous.is_none());
}

#[cfg(not(target_os = "macos"))]
#[test]
fn completed_retention_removes_empty_transaction_parents() {
    let mut f = Fixture::new();
    f.update();
    assert!(!f.paths.at("runtime/rollback-transactions").exists());
}

#[cfg(not(target_os = "macos"))]
#[test]
fn cancelled_restore_cleans_empty_staging_parent_and_keeps_point() {
    let mut f = Fixture::new();
    let point = f.update();
    f.job
        .cancel
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let error = rollback(&f.paths, &f.installed, &point, &f.job).unwrap_err();
    assert!(crate::jobs::is_cancelled(&error));
    assert!(!f.paths.at("runtime/rollback-transactions").exists());
    assert_eq!(
        status(&f.paths, &f.installed).unwrap().previous.unwrap().id,
        point.id
    );
    assert_eq!(
        f.paths.read_inventory().unwrap().unwrap().apps[0].version,
        "2.0.0"
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn empty_cleanup_preserves_recovery_unknown_and_hidden_content() {
    let f = Fixture::new();
    for base in [
        f.paths.at("rollbacks"),
        f.paths.at("runtime/rollback-transactions"),
    ] {
        let empty = base.join(uuid::Uuid::new_v4().to_string());
        let recovery = base.join(uuid::Uuid::new_v4().to_string());
        let unknown = base.join("personal-folder");
        for path in [&empty, &recovery, &unknown] {
            fs::create_dir_all(path).unwrap();
        }
        fs::write(recovery.join(".recovery"), b"keep").unwrap();
        cleanup::prune_empty(&f.paths).unwrap();
        assert!(!empty.exists());
        assert!(unknown.exists());
        assert_eq!(fs::read(recovery.join(".recovery")).unwrap(), b"keep");
    }
}
#[test]
fn native_status_is_honest_and_read_only() {
    let paths = Paths::new(
        std::env::temp_dir().join(format!("craft-rollback-native-{}", uuid::Uuid::new_v4())),
        None,
    );
    let installed = Installed {
        install_kind: "installer".into(),
        ..Default::default()
    };
    let state = status(&paths, &installed).unwrap();
    assert!(state.unsupported.is_some());
    assert!(state.previous.is_none());
    assert!(!paths.root.exists());
}
#[cfg(not(target_os = "macos"))]
#[test]
fn portable_manager_downgrade_retains_previous_version() {
    let mut f = Fixture::new();
    f.installed.version = "3.0.0".into();
    let mut config = f.paths.read_inventory().unwrap().unwrap();
    config.apps[0] = f.installed.clone();
    config.installations[0] = f.installed.clone();
    f.paths.save_config(&config).unwrap();
    crate::portable_mode::activate(&f.paths.root, &f.paths.root, false).unwrap();
    crate::portable_mode::initialize(&f.paths.root).unwrap();
    f.installed = f.paths.read_inventory().unwrap().unwrap().apps.remove(0);
    let point = f.update();
    assert_eq!(point.version, "3.0.0");
    assert_eq!(f.installed.version, "2.0.0");
    let retained = verify_point(&f.paths, &f.installed, &point).unwrap();
    let marker = fs::read_to_string(retained.join(portable::MARKER)).unwrap();
    assert!(marker.contains("@portable/"));
    rollback(&f.paths, &f.installed, &point, &f.job).unwrap();
    assert_eq!(
        f.paths.read_inventory().unwrap().unwrap().apps[0].version,
        "3.0.0"
    );
}
#[cfg(not(target_os = "macos"))]
#[test]
fn portable_recovery_marker_decoding_still_rejects_wrong_ownership() {
    let mut f = Fixture::new();
    crate::portable_mode::activate(&f.paths.root, &f.paths.root, false).unwrap();
    crate::portable_mode::initialize(&f.paths.root).unwrap();
    f.installed = f.paths.read_inventory().unwrap().unwrap().apps.remove(0);
    ensure_identity(&f.paths, &f.installed).unwrap();
    for (field, value) in [
        ("library", "@portable/other-library".to_owned()),
        ("library", "@portable/../outside".to_owned()),
        ("app", "another-app".to_owned()),
        ("installation_id", uuid::Uuid::new_v4().to_string()),
    ] {
        let old = staging_path(&f.paths);
        files::copy_directory_verified(Path::new(&f.installed.path), &old).unwrap();
        let marker = old.join(portable::MARKER);
        let mut raw: serde_json::Value =
            serde_json::from_slice(&fs::read(&marker).unwrap()).unwrap();
        raw[field] = value.into();
        let bytes = serde_json::to_vec_pretty(&raw).unwrap();
        fs::write(&marker, &bytes).unwrap();
        assert!(record_update(&f.paths, &f.installed, &f.installed, &old, &f.job).is_err());
        assert_eq!(fs::read(&marker).unwrap(), bytes);
        assert!(status(&f.paths, &f.installed).unwrap().previous.is_none());
    }
}
#[cfg(not(target_os = "macos"))]
#[test]
fn portable_root_move_keeps_immutable_snapshot_usable() {
    let mut f = Fixture::new();
    let point = f.update();
    let retained = verify_point(&f.paths, &f.installed, &point).unwrap();
    let before = manifest(&retained).unwrap();
    crate::portable_mode::activate(&f.paths.root, &f.paths.root, false).unwrap();
    crate::portable_mode::initialize(&f.paths.root).unwrap();
    assert_eq!(manifest(&retained).unwrap(), before);
    let moved = f.paths.root.with_extension("moved");
    fs::rename(&f.paths.root, &moved).unwrap();
    crate::portable_mode::initialize(&moved).unwrap();
    f.paths = Paths::new(moved, None);
    f.installed = f.paths.read_inventory().unwrap().unwrap().apps.remove(0);
    assert!(Path::new(&f.installed.path).starts_with(&f.paths.root));
    assert_eq!(
        status(&f.paths, &f.installed).unwrap().previous.unwrap().id,
        point.id
    );
    assert!(verify_point(&f.paths, &f.installed, &point).is_ok());
}
#[cfg(not(target_os = "macos"))]
#[test]
fn copied_uuid_is_rejected_as_ambiguous() {
    let mut f = Fixture::new();
    let point = f.update();
    let second = f.paths.at("releases/photocraft-2");
    files::copy_directory_verified(Path::new(&f.installed.path), &second).unwrap();
    let mut copy = f.installed.clone();
    copy.path = second.display().to_string();
    let mut config = f.paths.read_inventory().unwrap().unwrap();
    config.installations.push(copy);
    files::write_json(&f.paths.at("installations.json"), &config).unwrap();
    assert!(remove_previous(&f.paths, &f.installed, &point).is_err());
}
#[cfg(not(target_os = "macos"))]
#[test]
fn failed_inventory_commit_restores_current_and_keeps_point() {
    let mut f = Fixture::new();
    let point = f.update();
    let result = restore_with(&f.paths, &f.installed, &point, &f.job, |_| {
        anyhow::bail!("injected inventory save failure")
    });
    assert!(result.is_err());
    assert_eq!(
        fs::read(Path::new(&f.installed.path).join(crate::model::executable_name("photocraft")))
            .unwrap(),
        b"new executable"
    );
    assert_eq!(
        f.paths.read_inventory().unwrap().unwrap().apps[0].version,
        "2.0.0"
    );
    assert_eq!(
        status(&f.paths, &f.installed).unwrap().previous.unwrap().id,
        point.id
    );
}
#[cfg(not(target_os = "macos"))]
#[test]
fn uninstall_cleanup_requires_saved_removal_and_owned_transaction() {
    let mut f = Fixture::new();
    let point = f.update();
    assert!(discard_uninstalled(&f.paths, &f.installed, Path::new(&f.installed.path)).is_err());
    let temporary = Path::new(&f.installed.path)
        .parent()
        .unwrap()
        .join(format!(".uninstall-photocraft-{}", uuid::Uuid::new_v4()));
    fs::rename(&f.installed.path, &temporary).unwrap();
    assert!(discard_uninstalled(&f.paths, &f.installed, &temporary).is_err());
    let mut config = f.paths.read_inventory().unwrap().unwrap();
    crate::installations::actions::forget_installation(&mut config, &f.installed);
    f.paths.save_config(&config).unwrap();
    discard_uninstalled(&f.paths, &f.installed, &temporary).unwrap();
    assert!(read_point(&f.paths, &point.installation_id)
        .unwrap()
        .is_none());
    assert!(temporary.exists());
}
#[cfg(unix)]
#[test]
fn linked_snapshot_is_rejected() {
    let root = std::env::temp_dir().join(format!("craft-rollback-links-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("file"), b"original").unwrap();
    std::os::unix::fs::symlink(root.join("file"), root.join("link")).unwrap();
    assert!(manifest(&root).is_err());
    fs::remove_dir_all(root).unwrap();
}
