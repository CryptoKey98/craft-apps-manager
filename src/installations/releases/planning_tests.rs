#[cfg(target_os = "windows")]
fn approved_recovery_msi_fixture(file: &Path) {
    #[link(name = "msi")]
    unsafe extern "system" {
        fn MsiOpenDatabaseW(path: *const u16, mode: *const u16, database: *mut u32) -> u32;
        fn MsiDatabaseOpenViewW(database: u32, query: *const u16, view: *mut u32) -> u32;
        fn MsiViewExecute(view: u32, record: u32) -> u32;
        fn MsiDatabaseCommit(database: u32) -> u32;
        fn MsiCloseHandle(handle: u32) -> u32;
    }
    let mut database = 0;
    assert_eq!(
        unsafe {
            MsiOpenDatabaseW(
                crate::platform::wide(file).as_ptr(),
                3usize as *const u16,
                &mut database,
            )
        },
        0
    );
    let query = crate::platform::wide("CREATE TABLE `Property` (`Property` CHAR(72) NOT NULL, `Value` CHAR(0) LOCALIZABLE PRIMARY KEY `Property`)");
    let mut view = 0;
    assert_eq!(
        unsafe { MsiDatabaseOpenViewW(database, query.as_ptr(), &mut view) },
        0
    );
    assert_eq!(unsafe { MsiViewExecute(view, 0) }, 0);
    unsafe {
        MsiCloseHandle(view);
    }
    assert_eq!(unsafe { MsiDatabaseCommit(database) }, 0);
    unsafe {
        MsiCloseHandle(database);
    }
}

#[cfg(target_os = "windows")]
#[test]
fn approved_recovery_is_pinned_and_never_falls_back_to_release_discovery() {
    use sha2::{Digest, Sha256};
    let (paths, job) = fixture();
    let app = crate::model::Installed {
        name: "photocraft".into(),
        ..Default::default()
    };
    let file = paths.at("releases/installers/photocraft/approved.msi");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    approved_recovery_msi_fixture(&file);
    let bytes = fs::read(&file).unwrap();
    let asset = Asset {
        name: "approved.msi".into(),
        size: bytes.len() as u64,
        browser_download_url: "https://example.invalid/never-requested".into(),
        digest: Some(format!("sha256:{:x}", Sha256::digest(&bytes))),
    };
    let approved = std::collections::BTreeMap::from([("photocraft".into(), asset)]);
    assert_eq!(
        downgrade_recovery_path(&paths, &app, &job, Some(&approved)).unwrap(),
        file
    );
    // A checksum-approved source must still preserve the selected copy's scope.
    let machine_app = crate::model::Installed {
        windows_msi_scope: Some("machine".into()),
        ..app.clone()
    };
    assert!(
        downgrade_recovery_path(&paths, &machine_app, &job, Some(&approved))
            .unwrap_err()
            .to_string()
            .contains("installation scope")
    );
    assert!(
        downgrade_recovery_path(&paths, &app, &job, Some(&Default::default()))
            .unwrap_err()
            .to_string()
            .contains("Missing approved")
    );
    fs::write(&file, b"changed package").unwrap();
    assert!(downgrade_recovery_path(&paths, &app, &job, Some(&approved)).is_err());
    assert!(!paths.at("runtime/api-cache").exists());
    fs::remove_dir_all(paths.root).unwrap();
}
use super::*;
use std::sync::atomic::Ordering;
#[test]
#[ignore = "local release-mode install-dialog phase measurements"]
fn install_dialog_phase_timings() {
    use std::time::Instant;
    let project = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let session: serde_json::Value = serde_json::from_slice(
        &fs::read(project.join("target/install-dialog-timing-session.json")).unwrap(),
    )
    .unwrap();
    let root = PathBuf::from(session["Path"].as_str().unwrap()).join("data");
    assert!(root.starts_with(project.join("target")));
    let paths = Paths::new(root, None);
    crate::catalog::load(&paths.root);
    let job = Job::new(paths.at("logs/phase-benchmark.log"), &Default::default());
    let endpoint = format!(
        "https://api.github.com/repos/{}/releases/latest",
        crate::model::repository("filmcraft")
    );
    let release: Release =
        Network::cached_json(&paths.root, &endpoint).expect("Load FilmCraft preview once first");
    let observed = paths.config().unwrap();
    plan_install_preview(&paths, &job, "filmcraft", None, false, observed.clone()).unwrap();
    let mut rows = Vec::new();
    for _ in 0..8 {
        let start = Instant::now();
        let prefs = paths.read_preferences().unwrap();
        let preferences_ms = start.elapsed().as_secs_f64() * 1000.;
        let start = Instant::now();
        let config = paths.config().unwrap();
        let inventory_ms = start.elapsed().as_secs_f64() * 1000.;
        let start = Instant::now();
        let _: Release = Network::cached_json(&paths.root, &endpoint).unwrap();
        let cached_release_ms = start.elapsed().as_secs_f64() * 1000.;
        let start = Instant::now();
        let _ = state_snapshot(&paths, &prefs, &config).unwrap();
        let snapshot_ms = start.elapsed().as_secs_f64() * 1000.;
        let start = Instant::now();
        let plan = plan_from_config(&paths, &job, Some("filmcraft"), prefs, config, |_| {
            Ok(release.clone())
        })
        .unwrap();
        assert!(
            plan.entries[0].error.is_none(),
            "{:?}",
            plan.entries[0].error
        );
        let plan_ms = start.elapsed().as_secs_f64() * 1000.;
        let start = Instant::now();
        let preview =
            plan_install_preview(&paths, &job, "filmcraft", None, false, observed.clone()).unwrap();
        assert!(preview.entries[0].error.is_none());
        let cached_preview_ms = start.elapsed().as_secs_f64() * 1000.;
        rows.push(serde_json::json!({"preferences_ms":preferences_ms,"inventory_ms":inventory_ms,"cached_release_ms":cached_release_ms,"snapshot_ms":snapshot_ms,"plan_ms":plan_ms,"cached_preview_ms":cached_preview_ms}));
    }
    let report = serde_json::to_string_pretty(&rows).unwrap();
    fs::write(
        project.join("target/install-dialog-phase-timings.json"),
        &report,
    )
    .unwrap();
    println!("{report}");
}
fn fixture() -> (Paths, Job) {
    let root = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("craft-plan-{}", uuid::Uuid::new_v4()));
    let paths = Paths::new(root, None);
    files::write_json(
        &paths.at("updater-settings.json"),
        &Preferences {
            release_format: "portable".into(),
            selected_apps: vec!["photocraft".into(), "filmcraft".into()],
            ..Default::default()
        },
    )
    .unwrap();
    let job = Job::new(paths.at("log"), &Default::default());
    (paths, job)
}

#[test]
fn single_app_preview_uses_supplied_inventory_and_ignores_unrelated_changes() {
    let (paths, job) = fixture();
    let mut config = paths.config().unwrap();
    let original = config.clone();
    config.apps.retain(|app| app.name == "photocraft");
    let plan = plan_preview_with(&paths, &job, "photocraft", None, false, config, |_| {
        Ok(release("photocraft", "0.2.0"))
    })
    .unwrap();
    let (_, snapshot, _): (Preferences, crate::model::Config, serde_json::Value) =
        serde_json::from_slice(&plan.snapshot).unwrap();
    assert_eq!(
        snapshot.apps.len(),
        1,
        "Preview must reuse supplied inventory"
    );
    assert_eq!(plan.entries[0].action, "Install");
    paths.save_config(&original).unwrap();
    let base = paths.read_preferences().unwrap();
    let mut edited = base.clone();
    edited.app_order.reverse();
    edited.home_app_order.reverse();
    edited.app_install_options.insert(
        crate::install_options::identity("filmcraft"),
        crate::install_options::InstallOptions {
            release_format: "portable".into(),
            architecture: crate::model::MANAGER_ARCH.into(),
        },
    );
    crate::storage::settings::save_preferences_patch(&paths, &base, &edited).unwrap();
    execute_validated(&paths, &plan, |fresh| {
        assert!(fresh.apps.iter().any(|app| app.name == "filmcraft"));
        Ok(())
    })
    .unwrap();
    fs::remove_dir_all(paths.root).unwrap();
}

#[test]
fn portable_plan_survives_another_install_reordering_its_copy_records() {
    let (paths, job) = installed_fixture();
    let mut observed = paths.config().unwrap();
    let selected = observed
        .apps
        .iter()
        .find(|app| app.name == "photocraft")
        .unwrap()
        .clone();
    let alternate = paths.at("releases/photocraft-2");
    fs::create_dir_all(&alternate).unwrap();
    observed.installations.push(crate::model::Installed {
        path: alternate.display().to_string(),
        installation_label: "2".into(),
        ..selected
    });
    crate::installation_labels::assign(&mut observed).unwrap();
    let plan = plan_preview_with(
        &paths,
        &job,
        "photocraft",
        Some("0.0.9"),
        false,
        observed.clone(),
        |_| Ok(release("photocraft", "0.0.9")),
    )
    .unwrap();
    observed
        .apps
        .iter_mut()
        .find(|app| app.name == "filmcraft")
        .unwrap()
        .version = "0.2.0".into();
    paths.save_config(&observed).unwrap();
    execute_validated(&paths, &plan, |_| Ok(())).unwrap();
    fs::remove_dir_all(paths.root).unwrap();
}

#[test]
fn portable_downgrade_tolerates_app_settings_writes_but_rejects_payload_replacement() {
    let (paths, job) = installed_fixture();
    let folder = paths.at("releases/photocraft");
    crate::portable::mark(&paths, "photocraft", &folder).unwrap();
    let plan =
        plan_selected_version(&paths, &job, "photocraft", release("photocraft", "0.0.9")).unwrap();
    assert_eq!(plan.entries[0].action, "Update");
    fs::write(folder.join("app-settings.json"), "{}").unwrap();
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .access_mode(0x100) // FILE_WRITE_ATTRIBUTES
            .custom_flags(0x02000000); // FILE_FLAG_BACKUP_SEMANTICS (open directory)
    }
    options
        .open(&folder)
        .unwrap()
        .set_modified(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(86400))
        .unwrap();
    execute_validated(&paths, &plan, |_| Ok(())).unwrap();
    let marker = folder.join(crate::installations::portable::MARKER);
    let original_marker = fs::read(&marker).unwrap();
    fs::remove_file(&marker).unwrap();
    assert!(execute_validated(&paths, &plan, |_| panic!("ownership changed")).is_err());
    fs::write(marker, original_marker).unwrap();
    let plan =
        plan_selected_version(&paths, &job, "photocraft", release("photocraft", "0.0.9")).unwrap();
    let executable = crate::model::installed_executable(&folder, "photocraft").unwrap();
    if executable.is_dir() {
        fs::rename(&executable, folder.join("old-payload")).unwrap();
    } else {
        fs::write(executable, "different executable payload").unwrap();
    }
    assert!(execute_validated(&paths, &plan, |_| panic!("payload changed")).is_err());
    fs::remove_dir_all(paths.root).unwrap();
}

#[test]
fn single_app_downgrade_rejects_changed_selected_payload_and_preferences() {
    let (paths, job) = installed_fixture();
    let plan =
        plan_selected_version(&paths, &job, "photocraft", release("photocraft", "0.0.9")).unwrap();
    let base = paths.read_preferences().unwrap();
    let mut edited = base.clone();
    edited.batch_admin = !base.batch_admin;
    crate::storage::settings::save_preferences_patch(&paths, &base, &edited).unwrap();
    assert!(execute_validated(&paths, &plan, |_| panic!("approval settings changed")).is_err());
    crate::storage::settings::save_preferences_patch(&paths, &edited, &base).unwrap();
    let folder = paths.at("releases/photocraft");
    let executable = crate::model::installed_executable(&folder, "photocraft").unwrap();
    if executable.is_dir() {
        fs::write(executable.join("changed"), b"changed payload").unwrap();
    } else {
        fs::write(executable, b"replaced payload").unwrap();
    }
    assert!(execute_validated(&paths, &plan, |_| panic!("selected payload changed")).is_err());
    fs::remove_dir_all(paths.root).unwrap();
}

#[cfg(target_os = "windows")]
#[test]
fn installer_downgrade_ignores_unrelated_inventory_but_rejects_scope_changes() {
    let root = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("craft-native-plan-{}", uuid::Uuid::new_v4()));
    let paths = Paths::new(root, None);
    let job = Job::new(paths.at("log"), &Default::default());
    let mut config = paths.config().unwrap();
    let app = config
        .apps
        .iter_mut()
        .find(|app| app.name == "photocraft")
        .unwrap();
    app.path = paths.at("native-fixture").display().to_string();
    app.version = "0.2.0".into();
    app.install_kind = "installer".into();
    app.architecture = crate::model::MANAGER_ARCH.into();
    app.windows_msi_scope = Some("machine".into());
    fs::create_dir_all(&app.path).unwrap();
    fs::write(
        Path::new(&app.path).join(crate::model::executable_name(&app.name)),
        b"installed fixture",
    )
    .unwrap();
    config.installations.push(app.clone());
    let mut older = release("photocraft", "0.1.0");
    older.assets[0].name = format!(
        "photocraft-0.1.0-windows-{}.msi",
        crate::model::MANAGER_ARCH
    );
    older.assets[0].browser_download_url = format!(
        "https://github.com/storytold/photocraft/releases/download/v0.1.0/{}",
        older.assets[0].name
    );
    let plan = plan_preview_with(
        &paths,
        &job,
        "photocraft",
        Some("0.1.0"),
        false,
        config.clone(),
        |_| Ok(older.clone()),
    )
    .unwrap();
    assert!(plan.installer_downgrade(), "{:?}", plan.entries[0]);

    let prefs = paths.read_preferences().unwrap();
    config.apps.retain(|app| app.name == "photocraft");
    let current = state_snapshot(&paths, &prefs, &config).unwrap();
    assert_ne!(plan.snapshot, current);
    assert!(super::validation::single_app_state_matches(&plan, &current).unwrap());

    config.apps[0].windows_msi_scope = Some("user".into());
    config.installations.last_mut().unwrap().windows_msi_scope = Some("user".into());
    let changed = state_snapshot(&paths, &prefs, &config).unwrap();
    assert!(!super::validation::single_app_state_matches(&plan, &changed).unwrap());
    fs::remove_dir_all(paths.root).unwrap();
}

#[test]
fn observed_preview_preserves_version_and_additional_copy_plans() {
    let (paths, job) = installed_fixture();
    for additional in [false, true] {
        for tag in [None, Some("0.0.9")] {
            let release = release("photocraft", "0.0.9");
            let config = paths.config().unwrap();
            let preview =
                plan_preview_with(&paths, &job, "photocraft", tag, additional, config, |_| {
                    Ok(release.clone())
                })
                .unwrap();
            let mut expected = if additional {
                plan_additional_with(&paths, &job, "photocraft", |_| Ok(release.clone())).unwrap()
            } else if tag.is_some() {
                plan_selected_version(&paths, &job, "photocraft", release.clone()).unwrap()
            } else {
                plan_with(&paths, &job, Some("photocraft"), |_| Ok(release.clone())).unwrap()
            };
            expected.requested_tag = tag.map(str::to_owned);
            assert_eq!(
                serde_json::to_value(&preview).unwrap(),
                serde_json::to_value(&expected).unwrap()
            );
            let mut checked = false;
            execute_validated(&paths, &preview, |_| {
                checked = true;
                Ok(())
            })
            .unwrap();
            assert!(
                checked,
                "Unchanged observations must pass fresh execution checks"
            );
        }
    }
    fs::remove_dir_all(paths.root).unwrap();
}

#[test]
fn changing_format_for_an_uninstalled_app_keeps_the_preview_executable() {
    let (paths, job) = fixture();
    let base = paths.read_preferences().unwrap();
    let mut preferences = base.clone();
    preferences.app_install_options.insert(
        crate::install_options::identity("photocraft"),
        crate::install_options::InstallOptions {
            release_format: "portable".into(),
            architecture: crate::model::MANAGER_ARCH.into(),
        },
    );
    crate::storage::settings::save_preferences_patch(&paths, &base, &preferences).unwrap();
    let mut observed = paths.config().unwrap();
    let placeholder = observed
        .apps
        .iter_mut()
        .find(|app| app.name == "photocraft")
        .unwrap();
    assert!(placeholder.path.is_empty());
    // A window observation from before switching Installer to Portable still
    // carries the old preference-derived placeholder kind, not installed facts.
    placeholder.install_kind = "installer".into();
    let preview = plan_preview_with(&paths, &job, "photocraft", None, false, observed, |_| {
        Ok(release("photocraft", "0.2.0"))
    })
    .unwrap();
    assert!(preview.entries[0].error.is_none());
    let expected = plan_with(&paths, &job, Some("photocraft"), |_| {
        Ok(release("photocraft", "0.2.0"))
    })
    .unwrap();
    assert!(
        preview.snapshot == expected.snapshot,
        "Uninstalled placeholders must follow current options"
    );
    execute_validated(&paths, &preview, |_| Ok(())).unwrap();
    fs::remove_dir_all(paths.root).unwrap();
}

#[test]
fn observed_preview_rejects_foreign_inventory_and_changed_tags_and_missing_digests() {
    let (paths, job) = fixture();
    let config = paths.config().unwrap();
    let mut foreign = config.clone();
    foreign.apps_root = paths.at("another-library").display().to_string();
    assert!(plan_preview_with(
        &paths,
        &job,
        "photocraft",
        None,
        false,
        foreign,
        |_| panic!("Reject foreign inventory before requesting metadata")
    )
    .is_err());
    assert!(plan_preview_with(
        &paths,
        &job,
        "photocraft",
        Some("0.2.0"),
        false,
        config.clone(),
        |_| Ok(release("photocraft", "0.3.0"))
    )
    .is_err());
    let mut unverified = release("photocraft", "0.2.0");
    for asset in &mut unverified.assets {
        asset.digest = None;
    }
    let preview = plan_preview_with(&paths, &job, "photocraft", None, false, config, |_| {
        Ok(unverified.clone())
    })
    .unwrap();
    assert!(preview.entries[0].error.is_some());
    fs::remove_dir_all(paths.root).unwrap();
}

#[test]
fn additional_install_preview_preserves_existing_copy_and_rejects_collisions() {
    let (paths, job) = installed_fixture();
    let before = fs::read(paths.at("installations.json")).unwrap();
    let existing = crate::apps::installed(&paths, "photocraft").unwrap();
    let mut plan = plan_additional_with(&paths, &job, "photocraft", |name| {
        Ok(release(name, "0.2.0"))
    })
    .unwrap();
    assert!(plan.additional_installation);
    let default_target =
        crate::portable::additional_destination(&paths, &paths.at("releases"), "photocraft")
            .unwrap();
    assert_eq!(plan.entries[0].destination.as_ref(), Some(&default_target));
    assert_eq!(
        default_target.parent(),
        Some(paths.at("releases").as_path())
    );
    choose_destination(&paths, &mut plan, &paths.at("releases"), false).unwrap();
    assert_eq!(plan.entries[0].destination.as_ref(), Some(&default_target));
    execute_validated(&paths, &plan, |_| Ok(())).unwrap();
    assert_eq!(plan.entries[0].action, "Install");
    assert!(
        plan.entries[0].error.is_none(),
        "{:?}",
        plan.entries[0].error
    );
    assert_eq!(fs::read(paths.at("installations.json")).unwrap(), before);
    let parent = paths.at("releases/second-copy");
    choose_destination(&paths, &mut plan, &parent, false).unwrap();
    assert_eq!(plan.entries[0].destination, Some(parent.join("photocraft")));
    execute_validated(&paths, &plan, |config| {
        assert!(config
            .apps
            .iter()
            .find(|copy| copy.name == "photocraft")
            .unwrap()
            .path
            .is_empty());
        assert!(config
            .installations
            .iter()
            .any(|copy| copy.path == existing.path));
        Ok(())
    })
    .unwrap();
    assert_eq!(fs::read(paths.at("installations.json")).unwrap(), before);
    fs::create_dir_all(parent.join("photocraft")).unwrap();
    assert!(execute_validated(&paths, &plan, |_| panic!(
        "must reject collision before execution"
    ))
    .is_err());
    assert!(Path::new(&existing.path).exists());
    fs::remove_dir_all(paths.root).unwrap();
}

#[cfg(target_os = "windows")]
#[test]
fn installer_destinations_in_manager_storage_are_rejected_before_execution() {
    let (paths, job) = installed_fixture();
    for folder in [
        "releases/wrong-native",
        "releases/installers/wrong-native",
        "sources/wrong-native",
        "builds/wrong-native",
    ] {
        let mut plan = plan_with(&paths, &job, Some("photocraft"), |app| {
            Ok(release(app, "0.2.0"))
        })
        .unwrap();
        plan.installer_destination = Some(paths.at(folder));
        assert!(execute_validated(&paths, &plan, |_| panic!(
            "must reject native destination before starting an installer"
        ))
        .is_err());
    }
    let target = paths
        .root
        .parent()
        .unwrap()
        .join(format!("native-{}", uuid::Uuid::new_v4()))
        .join("photocraft");
    validate_installer_destination(&paths, &target).unwrap();
    let differently_cased = PathBuf::from(
        paths
            .at("releases/wrong-native")
            .to_string_lossy()
            .to_uppercase(),
    );
    assert!(validate_installer_destination(&paths, &differently_cased).is_err());
    fs::remove_dir_all(paths.root).unwrap();
}

#[cfg(target_os = "windows")]
#[test]
fn mixed_bulk_updates_pin_each_installed_package_and_report_missing_matches() {
    let (paths, job) = installed_fixture();
    let mut config = paths.config().unwrap();
    let native = config
        .apps
        .iter_mut()
        .find(|app| app.name == "filmcraft")
        .unwrap();
    native.install_kind = "installer".into();
    native.architecture = "x86".into();
    let mut prefs = paths.read_preferences().unwrap();
    // Pending preferences must not silently turn installed packages into other formats.
    prefs.app_install_options.insert(
        crate::install_options::identity("filmcraft"),
        crate::install_options::InstallOptions {
            release_format: "portable".into(),
            architecture: "x64".into(),
        },
    );
    let releases = |app: &str| {
        let mut r = release(app, "0.2.0");
        if app == "filmcraft" {
            for asset in &mut r.assets {
                asset.name = "filmcraft-0.2.0-windows-x86.msi".into();
            }
        }
        Ok(r)
    };
    let plan =
        plan_from_config(&paths, &job, None, prefs.clone(), config.clone(), releases).unwrap();
    assert_eq!(plan.executable_count(), 2);
    let photo = plan
        .entries
        .iter()
        .find(|entry| entry.app == "photocraft")
        .unwrap();
    let film = plan
        .entries
        .iter()
        .find(|entry| entry.app == "filmcraft")
        .unwrap();
    assert_eq!(photo.preferences.release_format, "portable");
    assert_eq!(photo.preferences.architecture, crate::model::MANAGER_ARCH);
    assert_eq!(film.preferences.release_format, "installer");
    assert_eq!(film.preferences.architecture, "x86");
    prefs.app_order = vec!["filmcraft".into(), "photocraft".into()];
    prefs.home_app_order = vec!["photocraft".into(), "filmcraft".into()];
    let missing = plan_from_config(&paths, &job, None, prefs, config, |app| {
        Ok(release(app, "0.2.0"))
    })
    .unwrap();
    assert_eq!(
        missing
            .entries
            .iter()
            .map(|entry| entry.app.as_str())
            .collect::<Vec<_>>(),
        ["filmcraft", "photocraft"]
    );
    assert_eq!(missing.executable_count(), 1);
    assert!(missing
        .entries
        .iter()
        .find(|entry| entry.app == "filmcraft")
        .unwrap()
        .error
        .is_some());
    assert!(paths.at("releases/filmcraft/filmcraft.exe").exists());
    fs::remove_dir_all(paths.root).unwrap();
}
#[test]
fn newly_installed_app_is_selected_once_without_resetting_other_preferences() {
    let (paths, _) = installed_fixture();
    let mut preferences = paths.read_preferences().unwrap();
    preferences.selected_apps.clear();
    preferences.close_to_tray = true;
    crate::storage::settings::save_preferences_patch(
        &paths,
        &paths.read_preferences().unwrap(),
        &preferences,
    )
    .unwrap();
    include_new_install(&paths, "photocraft").unwrap();
    include_new_install(&paths, "photocraft").unwrap();
    let saved = paths.read_preferences().unwrap();
    assert_eq!(saved.selected_apps, vec!["photocraft"]);
    assert!(saved.close_to_tray);
    std::fs::remove_dir_all(&paths.root).unwrap();
}

fn installed_fixture() -> (Paths, Job) {
    let (paths, job) = fixture();
    // Keep real native installations on the developer's machine from
    // replacing these portable test installations during config refresh.
    let mut prefs = paths.read_preferences().unwrap();
    for app in ["photocraft", "filmcraft"] {
        prefs.app_install_options.insert(
            crate::install_options::identity(app),
            crate::install_options::InstallOptions {
                release_format: "portable".into(),
                architecture: crate::model::MANAGER_ARCH.into(),
            },
        );
    }
    crate::storage::settings::save_preferences_patch(
        &paths,
        &paths.read_preferences().unwrap(),
        &prefs,
    )
    .unwrap();
    let apps = ["photocraft", "filmcraft"].into_iter().map(|name| {
            let folder = paths.at(format!("releases/{name}"));
            let executable = folder.join(crate::model::executable_name(name));
            fs::create_dir_all(&folder).unwrap();
            if cfg!(target_os = "macos") {
                fs::create_dir_all(executable.join("Contents")).unwrap();
                fs::write(executable.join("Contents/Info.plist"), format!("<plist><dict><key>CFBundleIdentifier</key><string>ai.storyteller.{name}</string><key>CFBundleShortVersionString</key><string>0.1.0</string></dict></plist>")).unwrap();
            } else { fs::write(executable, b"installed fixture").unwrap(); }
            crate::model::Installed { name: name.into(), version: "0.1.0".into(), path: folder.display().to_string(), architecture: crate::model::MANAGER_ARCH.into(), install_kind: "portable".into(), ..Default::default() }
        }).collect::<Vec<_>>();
    files::write_json(
        &paths.at("installations.json"),
        &crate::model::Config {
            portable_label_counters: Default::default(),
            apps_root: paths.root.display().to_string(),
            installations: apps.clone(),
            apps,
        },
    )
    .unwrap();
    (paths, job)
}
#[cfg(target_os = "windows")]
#[test]
fn preferred_architecture_change_requires_uninstall_but_bulk_keeps_installed_options() {
    let (paths, job) = installed_fixture();
    let other_architecture = if crate::model::MANAGER_ARCH == "x86" {
        "x64"
    } else {
        "x86"
    };
    let mut prefs = paths.read_preferences().unwrap();
    prefs.app_install_options.insert(
        crate::install_options::identity("photocraft"),
        crate::install_options::InstallOptions {
            release_format: "portable".into(),
            architecture: other_architecture.into(),
        },
    );
    crate::storage::settings::save_preferences_patch(
        &paths,
        &paths.read_preferences().unwrap(),
        &prefs,
    )
    .unwrap();
    let plan = plan_with(&paths, &job, Some("photocraft"), |app| {
        Ok(release(app, "0.1.0"))
    })
    .unwrap();
    assert!(plan.entries[0]
        .error
        .as_ref()
        .unwrap()
        .contains("Uninstall"));
    let bulk = plan_with(&paths, &job, None, |app| Ok(release(app, "0.2.0"))).unwrap();
    assert_eq!(bulk.entries.len(), 2);
    assert!(bulk.entries.iter().all(|entry| entry.action == "Update"
        && entry.preferences.architecture == crate::model::MANAGER_ARCH));
    assert!(paths.at("releases/photocraft/photocraft.exe").exists());
    fs::remove_dir_all(paths.root).unwrap();
}
#[test]
fn bulk_updates_never_install_missing_apps_but_explicit_install_still_works() {
    let (paths, job) = installed_fixture();
    fs::remove_dir_all(paths.at("releases/filmcraft")).unwrap();
    // Planning fixtures must not discover real packages installed on the test machine.
    let mut config: crate::model::Config =
        files::read_json(&paths.at("installations.json")).unwrap();
    config.installations.retain(|app| app.name != "filmcraft");
    *config
        .apps
        .iter_mut()
        .find(|app| app.name == "filmcraft")
        .unwrap() = crate::model::Installed {
        name: "filmcraft".into(),
        ..Default::default()
    };
    let prefs = paths.read_preferences().unwrap();
    let plan = plan_from_config(&paths, &job, None, prefs.clone(), config.clone(), |app| {
        assert_eq!(app, "photocraft");
        Ok(release(app, "0.4.0"))
    })
    .unwrap();
    assert_eq!(plan.entries.len(), 1);
    assert_eq!(plan.entries[0].action, "Update");
    let explicit = plan_from_config(&paths, &job, Some("filmcraft"), prefs, config, |app| {
        Ok(release(app, "0.4.0"))
    })
    .unwrap();
    assert_eq!(explicit.entries[0].action, "Install");
    fs::remove_dir_all(paths.root).unwrap();
}
#[test]
fn failed_or_cancelled_installer_replacement_restores_previous_version() {
    use std::cell::Cell;
    let restored = Cell::new(false);
    let error = replace_installer_transaction(
        || Ok(()),
        || Err::<(), _>(crate::jobs::Cancelled.into()),
        || {
            restored.set(true);
            Ok(())
        },
    )
    .unwrap_err();
    assert!(restored.get());
    assert!(crate::jobs::is_cancelled(&error));
    replace_installer_transaction(
        || Ok(()),
        || Ok(()),
        || panic!("successful install must not restore"),
    )
    .unwrap();
    let error = replace_installer_transaction(
        || bail!("remove failed"),
        || -> Result<()> { panic!("must not install after failed removal") },
        || Ok(()),
    )
    .unwrap_err();
    assert!(error.to_string().contains("Previous installation restored"));
    let error = replace_installer_transaction(
        || Ok(()),
        || Err::<(), _>(anyhow::anyhow!("install failed")),
        || bail!("restore failed"),
    )
    .unwrap_err();
    assert!(error.to_string().contains("restore failed"));
}
fn release(app: &str, tag: &str) -> Release {
    let suffix = if cfg!(target_os = "macos") {
        ".dmg"
    } else if cfg!(target_os = "linux") {
        ".AppImage"
    } else {
        "-portable.zip"
    };
    let name = format!(
        "{app}-{tag}-{}-{}{suffix}",
        crate::model::release_os(),
        crate::model::release_arch(crate::model::MANAGER_ARCH)
    );
    Release {
        tag_name: tag.into(),
        name: None,
        draft: false,
        prerelease: false,
        assets: vec![Asset {
            name: name.clone(),
            size: 1234,
            digest: Some(format!("sha256:{}", "a".repeat(64))),
            browser_download_url: format!(
                "https://github.com/storytold/{app}/releases/download/v{tag}/{name}"
            ),
        }],
    }
}
#[test]
fn downgrade_availability_reuses_cached_latest_and_survives_restart() {
    let (paths, _) = fixture();
    let prefs = Preferences {
        release_format: "portable".into(),
        ..Default::default()
    };
    let mut installed = crate::model::Installed {
        name: "photocraft".into(),
        version: "0.4.0".into(),
        path: paths.at("releases/photocraft").display().to_string(),
        install_kind: "portable".into(),
        architecture: crate::model::MANAGER_ARCH.into(),
        ..Default::default()
    };
    assert!(cached_installation_check(&paths, &installed, &prefs).is_none());
    let endpoint = format!(
        "https://api.github.com/repos/{}/releases/latest",
        crate::model::repository("photocraft")
    );
    crate::network::cache_tests::seed(
        &paths.root,
        &endpoint,
        0,
        serde_json::to_value(release("photocraft", "0.4.0")).unwrap(),
    );
    assert_eq!(
        cached_installation_check(&paths, &installed, &prefs)
            .unwrap()
            .unwrap(),
        None
    );
    installed.version = "0.3.0".into();
    assert_eq!(
        cached_installation_check(&paths, &installed, &prefs)
            .unwrap()
            .unwrap(),
        Some("0.4.0".into())
    );
    crate::hourly::refresh_after_install(&paths, &installed, &prefs).unwrap();
    let checks = crate::hourly::read(&Paths::new(paths.root.clone(), None)).unwrap();
    let effective = crate::install_options::for_installed(&prefs, &installed).unwrap();
    let state = &checks[&crate::hourly::key(&effective, "photocraft")];
    assert_eq!(state.installed, "0.3.0");
    assert_eq!(state.latest.as_deref(), Some("0.4.0"));
    installed.version = "0.4.0".into();
    crate::hourly::refresh_after_install(&paths, &installed, &prefs).unwrap();
    assert_eq!(
        crate::hourly::read(&paths).unwrap()[&crate::hourly::key(&effective, "photocraft")].latest,
        None
    );
    fs::remove_dir_all(paths.root).unwrap();
}
#[test]
fn cached_downgrade_check_requires_a_compatible_package() {
    let installed = crate::model::Installed {
        name: "photocraft".into(),
        version: "0.3.0".into(),
        path: "/test/photocraft".into(),
        architecture: crate::model::MANAGER_ARCH.into(),
        install_kind: "portable".into(),
        ..Default::default()
    };
    let mut latest = release("photocraft", "0.4.0");
    latest.assets.clear();
    assert!(
        check_installed_release(&latest, "photocraft", &installed, &Preferences::default())
            .is_err()
    );
}
#[test]
fn version_list_excludes_previews_incompatible_packages_and_unverified_assets() {
    let prefs = Preferences {
        release_format: "portable".into(),
        ..Default::default()
    };
    let stable = release("photocraft", "0.4.0");
    let older = release("photocraft", "0.3.0");
    let mut preview = stable.clone();
    preview.prerelease = true;
    let mut draft = stable.clone();
    draft.draft = true;
    let mut unverified = stable.clone();
    unverified.assets[0].digest = None;
    let mut invalid = stable.clone();
    invalid.tag_name = "beta".into();
    let mut incompatible = stable.clone();
    incompatible.assets.clear();
    let versions = compatible_versions(
        vec![
            older,
            preview,
            unverified,
            draft,
            incompatible,
            invalid,
            stable,
        ],
        "photocraft",
        &prefs,
    );
    assert_eq!(
        versions
            .iter()
            .map(|r| r.tag_name.as_str())
            .collect::<Vec<_>>(),
        vec!["0.4.0", "0.3.0"]
    );
}
#[test]
fn only_explicit_version_selection_can_plan_a_downgrade_and_stays_pinned() {
    let (paths, job) = fixture();
    let folder = paths.at("releases/photocraft");
    fs::create_dir_all(&folder).unwrap();
    let executable = folder.join(crate::model::executable_name("photocraft"));
    if cfg!(target_os = "macos") {
        fs::create_dir_all(executable.join("Contents")).unwrap();
        fs::write(executable.join("Contents/Info.plist"), "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>ai.storyteller.photocraft</string><key>CFBundleShortVersionString</key><string>9.0.0</string></dict></plist>").unwrap();
    } else {
        fs::write(&executable, "current version").unwrap();
    }
    let record = crate::model::Installed {
        name: "photocraft".into(),
        version: "9.0.0".into(),
        path: folder.display().to_string(),
        install_kind: "portable".into(),
        architecture: crate::model::MANAGER_ARCH.into(),
        ..Default::default()
    };
    files::write_json(
        &paths.at("installations.json"),
        &crate::model::Config {
            portable_label_counters: Default::default(),
            apps_root: paths.root.display().to_string(),
            apps: vec![record.clone()],
            installations: vec![record],
        },
    )
    .unwrap();
    let ordinary = plan_with(&paths, &job, Some("photocraft"), |_| {
        Ok(release("photocraft", "0.3.0"))
    })
    .unwrap();
    assert_eq!(ordinary.entries[0].action, "Skip");
    let mut prefs = paths.read_preferences().unwrap();
    prefs.selected_apps = vec!["filmcraft".into()];
    crate::storage::settings::save_preferences_patch(
        &paths,
        &paths.read_preferences().unwrap(),
        &prefs,
    )
    .unwrap();
    let explicit =
        plan_selected_version(&paths, &job, "photocraft", release("photocraft", "0.3.0")).unwrap();
    assert_eq!(explicit.entries.len(), 1);
    assert_eq!(explicit.entries[0].action, "Update");
    assert_eq!(explicit.entries[0].version, "0.3.0");
    assert_eq!(explicit.requested_tag.as_deref(), Some("0.3.0"));
    assert_eq!(explicit.executable_count(), 1);
    let mut unverified = release("photocraft", "0.3.0");
    unverified.assets[0].digest = None;
    let denied = plan_selected_version(&paths, &job, "photocraft", unverified).unwrap();
    assert_eq!(denied.executable_count(), 0);
    if cfg!(target_os = "macos") {
        fs::write(executable.join("Contents/Info.plist"), "changed").unwrap();
    } else {
        fs::write(&executable, "installation changed after review").unwrap();
    }
    assert!(execute_validated(&paths, &explicit, |_| panic!(
        "changed unselected app must invalidate version plan"
    ))
    .is_err());
    fs::remove_dir_all(paths.root).unwrap();
}
#[test]
fn invalid_digest_metadata_is_excluded_before_package_download() {
    let (paths, job) = installed_fixture();
    for digest in [None, Some("sha256:invalid".into())] {
        let plan = plan_with(&paths, &job, None, |app| {
            let mut release = release(app, "0.4.0");
            release.assets[0].digest = digest.clone();
            Ok(release)
        })
        .unwrap();
        assert_eq!(plan.executable_count(), 0);
        assert!(plan.entries.iter().all(|entry| entry.error.is_some()));
        execute_validated(&paths, &plan, |_| {
            execute_with(&plan, &job, |_, _| {
                panic!("invalid metadata must never download")
            })
        })
        .unwrap_err();
    }
    assert!(!paths.at("runtime/downloads").exists());
    fs::remove_dir_all(paths.root).unwrap();
}
#[test]
fn choosing_apps_rebuilds_review_without_unselected_operations() {
    let (paths, job) = installed_fixture();
    let original = plan_with(&paths, &job, None, |app| Ok(release(app, "0.4.0"))).unwrap();
    assert_eq!(original.entries.len(), 2);
    crate::settings::select_release_apps(&paths, &["filmcraft".into()]).unwrap();
    let changed = plan_with(&paths, &job, None, |app| {
        assert_eq!(
            app, "filmcraft",
            "unchecked apps must not be queried or planned"
        );
        Ok(release(app, "0.4.0"))
    })
    .unwrap();
    assert_eq!(changed.entries.len(), 1);
    assert_eq!(changed.entries[0].app, "filmcraft");
    assert!(execute_validated(&paths, &original, |_| panic!("stale review must not run")).is_err());
    crate::settings::select_release_apps(&paths, &[]).unwrap();
    let empty = plan_with(&paths, &job, None, |_| {
        panic!("empty selection must not query releases")
    })
    .unwrap();
    assert!(empty.entries.is_empty());
    assert_eq!(empty.executable_count(), 0);
    assert!(!paths.at("runtime/downloads").exists());
    fs::remove_dir_all(paths.root).unwrap();
}
#[test]
fn legacy_mac_architecture_is_current_for_same_universal_release() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let (paths, job) = installed_fixture();
    let folder = paths.at("releases/photocraft");
    let contents = folder
        .join(crate::model::executable_name("photocraft"))
        .join("Contents");
    fs::create_dir_all(&contents).unwrap();
    fs::write(contents.join("Info.plist"), "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>ai.storyteller.photocraft</string><key>CFBundleShortVersionString</key><string>0.4.0</string></dict></plist>").unwrap();
    let mut prefs = paths.read_preferences().unwrap();
    prefs.architecture = "arm64".into();
    prefs.selected_apps = vec!["photocraft".into()];
    crate::storage::settings::save_preferences_patch(
        &paths,
        &paths.read_preferences().unwrap(),
        &prefs,
    )
    .unwrap();
    let record = crate::model::Installed {
        name: "photocraft".into(),
        version: "0.4.0".into(),
        path: folder.display().to_string(),
        install_kind: "portable".into(),
        architecture: "x86".into(),
        ..Default::default()
    };
    let config = crate::model::Config {
        portable_label_counters: Default::default(),
        apps_root: paths.root.display().to_string(),
        apps: vec![record.clone()],
        installations: vec![record],
    };
    files::write_json(&paths.at("installations.json"), &config).unwrap();
    let plan = plan_with(&paths, &job, None, |app| Ok(release(app, "0.4.0"))).unwrap();
    assert_eq!(plan.entries[0].action, "Skip");
    assert_eq!(plan.executable_count(), 0);
    execute_validated(&paths, &plan, |_| {
        execute_with(&plan, &job, |_, _| {
            panic!("must not reinstall universal release")
        })
    })
    .unwrap();
    let plist = contents.join("Info.plist");
    let changed = fs::read_to_string(&plist)
        .unwrap()
        .replace("0.4.0", "0.5.0");
    fs::write(&plist, changed).unwrap();
    assert!(execute_validated(&paths, &plan, |_| panic!(
        "actual installed version changed without inventory write"
    ))
    .is_err());
    fs::remove_dir_all(paths.root).unwrap();
}
#[test]
fn planning_is_read_only_pins_assets_and_excludes_per_app_errors() {
    let (paths, job) = installed_fixture();
    let saved = fs::read(paths.at("installations.json")).unwrap();
    let preferences = fs::read(paths.at("config.yaml")).unwrap();
    let plan = plan_with(&paths, &job, None, |app| {
        if app == "filmcraft" {
            bail!("Metadata unavailable");
        } else {
            Ok(release(app, "0.4.0"))
        }
    })
    .unwrap();
    assert_eq!(plan.executable_count(), 1);
    assert!(plan
        .entries
        .iter()
        .any(|e| e.app == "filmcraft" && e.error.is_some()));
    execute_validated(&paths, &plan, |_| {
        execute_with(&plan, &job, |entry, _| {
            assert_eq!(entry.version, "0.4.0");
            assert_eq!(entry.asset.as_ref().unwrap().size, 1234);
            assert_eq!(
                entry.asset.as_ref().unwrap().digest.as_deref(),
                Some("sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            );
            Ok(true)
        })
    })
    .unwrap_err();
    assert_eq!(fs::read(paths.at("config.yaml")).unwrap(), preferences);
    assert_eq!(fs::read(paths.at("installations.json")).unwrap(), saved);
    for name in ["manager-settings.json", "runtime/downloads"] {
        assert!(!paths.at(name).exists(), "{name}");
    }
    fs::remove_dir_all(paths.root).unwrap();
}
#[test]
fn changed_preferences_destination_and_lock_reject_before_execution() {
    let (paths, job) = installed_fixture();
    let plan = plan_with(&paths, &job, None, |app| Ok(release(app, "0.4.0"))).unwrap();
    let lock = platform::Lock::take("Local\\CraftAppsManager").unwrap();
    // Windows mutexes are recursive on their owning thread. A competing
    // worker must acquire on another thread, as real GUI operations do.
    let blocked = std::thread::scope(|scope| {
        scope
            .spawn(|| execute_validated(&paths, &plan, |_| panic!("must not execute while locked")))
            .join()
            .unwrap()
    });
    assert!(blocked.is_err());
    drop(lock);
    fs::write(paths.at("releases/photocraft/changed.txt"), b"change").unwrap();
    assert!(execute_validated(&paths, &plan, |_| panic!(
        "must not execute after destination change"
    ))
    .is_err());
    fs::remove_file(paths.at("releases/photocraft/changed.txt")).unwrap();
    let mut prefs = paths.read_preferences().unwrap();
    prefs.selected_apps.clear();
    crate::storage::settings::save_preferences_patch(
        &paths,
        &paths.read_preferences().unwrap(),
        &prefs,
    )
    .unwrap();
    assert!(execute_validated(&paths, &plan, |_| panic!(
        "must not execute after selection change"
    ))
    .is_err());
    fs::remove_dir_all(paths.root).unwrap();
}
#[test]
fn custom_destination_keeps_pinned_release_and_rejects_a_late_folder_collision() {
    let (paths, job) = fixture();
    // The legacy global format no longer controls this app's installation intent.
    let mut preferences = paths.read_preferences().unwrap();
    preferences.app_install_options.insert(
        crate::install_options::identity("photocraft"),
        crate::install_options::InstallOptions {
            release_format: "portable".into(),
            architecture: crate::model::MANAGER_ARCH.into(),
        },
    );
    crate::storage::settings::save_preferences_patch(
        &paths,
        &paths.read_preferences().unwrap(),
        &preferences,
    )
    .unwrap();
    let parent = std::env::temp_dir().join(format!("craft-destination-{}", uuid::Uuid::new_v4()));
    let mut plan = plan_with(&paths, &job, Some("photocraft"), |app| {
        Ok(release(app, "0.4.0"))
    })
    .unwrap();
    let original_asset = plan.entries[0]
        .asset
        .as_ref()
        .unwrap()
        .browser_download_url
        .clone();
    choose_destination(&paths, &mut plan, &parent, true).unwrap();
    let target = parent.join("photocraft");
    assert_eq!(plan.entries[0].destination.as_ref(), Some(&target));
    assert_eq!(plan.entries[0].version, "0.4.0");
    assert_eq!(
        plan.entries[0].asset.as_ref().unwrap().browser_download_url,
        original_asset
    );
    assert!(plan.desktop_shortcut);
    assert!(!parent.exists());
    execute_validated(&paths, &plan, |_| Ok(())).unwrap();
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("work.png"), b"personal").unwrap();
    assert!(execute_validated(&paths, &plan, |_| panic!("must not replace a new folder")).is_err());
    assert_eq!(fs::read(target.join("work.png")).unwrap(), b"personal");
    fs::remove_dir_all(parent).unwrap();
    fs::remove_dir_all(paths.root).unwrap();
}
#[test]
#[cfg(target_os = "windows")]
fn native_staging_requires_confirmation_before_continuing_after_a_bad_checksum() {
    let (paths, mut job) = installed_fixture();
    let mut plan = plan_with(&paths, &job, None, |app| Ok(release(app, "0.4.0"))).unwrap();
    for entry in &mut plan.entries {
        entry.preferences.release_format = "installer".into();
        let asset = entry.asset.as_mut().unwrap();
        asset.name = format!("{}.msi", entry.app);
        let fixture = paths.at("fixture.msi");
        files::write_bytes(&fixture, b"good").unwrap();
        asset.digest = Some(format!("sha256:{}", files::hash(&fixture).unwrap()));
        asset.size = 4;
        let path = installer_cache_paths(&paths, &entry.app, asset).unwrap()[1].clone();
        let bytes = if entry.app == "filmcraft" {
            b"bad!"
        } else {
            b"good"
        };
        files::write_bytes(&path, bytes).unwrap();
    }
    job.enable_identity_choices();
    let entries: Vec<_> = plan.entries.iter().collect();
    std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            let staged = super::staging::stage_packages(&paths, &plan, &entries, &job)?;
            let mut verified_plan = plan.clone();
            staged.apply_failures(&mut verified_plan);
            assert_eq!(
                verified_plan
                    .entries
                    .iter()
                    .filter(|entry| entry.error.is_some())
                    .map(|entry| entry.app.as_str())
                    .collect::<Vec<_>>(),
                ["filmcraft"]
            );
            Ok::<_, anyhow::Error>(())
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(choice) = job.identity_choice() {
                assert!(choice.continue_batch);
                assert_eq!(choice.candidates.len(), 1);
                choice.answer(Some(0));
                break;
            }
            if std::time::Instant::now() >= deadline {
                job.cancel.store(true, Ordering::Relaxed);
                panic!("Partial batch confirmation did not appear");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        worker.join().unwrap().unwrap();
    });
    fs::remove_dir_all(paths.root).unwrap();
}

#[test]
fn bulk_packages_are_all_verified_before_install_and_cancel_clears_staging() {
    let (paths, job) = installed_fixture();
    let mut plan = plan_with(&paths, &job, None, |app| Ok(release(app, "0.4.0"))).unwrap();
    plan.entries.retain(|entry| entry.action == "Update");
    let mut third = plan.entries[0].clone();
    third.app = "designcraft".into();
    third.asset.as_mut().unwrap().browser_download_url =
        "https://github.com/storytold/designcraft/releases/download/v0.4.0/setup.zip".into();
    plan.entries.push(third);
    for entry in &mut plan.entries {
        let asset = entry.asset.as_mut().unwrap();
        asset.name = "setup.zip".into();
        let bytes = asset.browser_download_url.as_bytes();
        let fixture = paths.at("package-fixture");
        files::write_bytes(&fixture, bytes).unwrap();
        asset.size = bytes.len() as u64;
        asset.digest = Some(format!("sha256:{}", files::hash(&fixture).unwrap()));
    }
    let entries: Vec<_> = plan.entries.iter().collect();
    let verified = std::sync::atomic::AtomicUsize::new(0);
    let first_pair = std::sync::Barrier::new(2);
    let result = job.tracked("Bulk update", "Application library", |job| {
        let staged = super::staging::stage_packages_with(
            &paths,
            &plan,
            &entries,
            job,
            &|asset, archive, child| {
                if !asset.browser_download_url.contains("designcraft") {
                    first_pair.wait();
                } else {
                    assert_eq!(verified.load(Ordering::SeqCst), 2);
                }
                child.check()?;
                fs::write(archive, &asset.browser_download_url)?;
                crate::network::verify_asset(archive, asset)?;
                verified.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )?;
        assert_eq!(verified.load(Ordering::SeqCst), 3);
        let mut installed = Vec::new();
        execute_with(&plan, job, |entry, _| {
            assert_eq!(verified.load(Ordering::SeqCst), 3);
            assert!(staged.portable_archive(&entry.app).unwrap().is_file());
            installed.push(entry.app.as_str().to_owned());
            Ok(true)
        })?;
        assert!(installed
            .iter()
            .eq(plan.entries.iter().map(|entry| &entry.app)));
        for entry in &plan.entries {
            assert_eq!(
                fs::read_to_string(staged.portable_archive(&entry.app).unwrap())?,
                entry.asset.as_ref().unwrap().browser_download_url
            );
        }
        job.cancel.store(true, Ordering::Relaxed);
        execute_with(&plan, job, |_, _| {
            panic!("cancelled batch must not start installation")
        })
    });
    assert!(crate::jobs::is_cancelled(&result.unwrap_err()));
    assert!(!paths.at("runtime/downloads").exists());
    fs::remove_dir_all(paths.root).unwrap();
}

#[test]
fn cancellation_counts_completed_work_and_keeps_real_errors() {
    let (paths, job) = installed_fixture();
    let plan = plan_with(&paths, &job, None, |app| Ok(release(app, "0.4.0"))).unwrap();
    let result = execute_with(&plan, &job, |_, _| {
        job.cancel.store(true, Ordering::Relaxed);
        Ok(true)
    })
    .unwrap_err();
    assert!(crate::jobs::is_cancelled(&result));
    assert!(result.to_string().contains("1 completed"));
    assert!(result.to_string().contains("1 remaining"));
    job.cancel.store(false, Ordering::Relaxed);
    let error = execute_with(&plan, &job, |_, _| {
        job.cancel.store(true, Ordering::Relaxed);
        bail!("Actual installer failure");
    })
    .unwrap_err();
    assert!(!crate::jobs::is_cancelled(&error));
    assert!(error.to_string().contains("1 failed"));
    job.cancel.store(false, Ordering::Relaxed);
    let mut single = plan.clone();
    single.entries.truncate(1);
    assert!(execute_with(&single, &job, |_, _| {
        job.cancel.store(true, Ordering::Relaxed);
        Ok(true)
    })
    .is_ok());
    job.cancel.store(false, Ordering::Relaxed);
    execute_with(&single, &job, |_, _| Ok(false)).unwrap();
    assert!(job
        .state
        .lock()
        .unwrap()
        .log
        .contains("0 completed, 1 skipped"));
    fs::remove_dir_all(paths.root).unwrap();
}
#[test]
fn cancelled_planning_never_fetches_packages_and_preserves_prior_metadata_failure() {
    let (paths, job) = installed_fixture();
    job.cancel.store(true, Ordering::Relaxed);
    let error = plan_with(&paths, &job, None, |_| panic!("must not fetch metadata")).unwrap_err();
    assert!(crate::jobs::is_cancelled(&error));
    job.cancel.store(false, Ordering::Relaxed);
    let error = plan_with(&paths, &job, None, |_| {
        job.cancel.store(true, Ordering::Relaxed);
        bail!("Actual metadata error");
    })
    .unwrap_err();
    assert!(!crate::jobs::is_cancelled(&error));
    assert!(error.to_string().contains("Actual metadata error"));
    assert!(!paths.at("runtime/downloads").exists());
    fs::remove_dir_all(paths.root).unwrap();
}

#[cfg(target_os = "windows")]
#[test]
fn incompatible_msi_scope_is_rejected_before_identity_or_recovery_preparation() {
    let (paths, job) = fixture();
    struct FixtureRoot(PathBuf);
    impl Drop for FixtureRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _root = FixtureRoot(paths.root.clone());
    let package = paths.at("releases/installers/photocraft/previous.msi");
    fs::create_dir_all(package.parent().unwrap()).unwrap();
    // This metadata-only package has fixed per-user scope, no executable payload,
    // and no supported install directory: even a regressed preflight cannot run it.
    approved_recovery_msi_fixture(&package);
    let current_path = paths.at("current");
    fs::create_dir_all(&current_path).unwrap();
    fs::write(
        current_path.join("keep.txt"),
        b"preserved current installation",
    )
    .unwrap();
    let installed = crate::model::Installed {
        name: "photocraft".into(),
        path: current_path.display().to_string(),
        version: "2.0.0".into(),
        architecture: "x64".into(),
        install_kind: "installer".into(),
        windows_msi_scope: Some("machine".into()),
        ..Default::default()
    };
    let mut config = crate::model::Config {
        apps_root: paths.root.display().to_string(),
        apps: vec![installed.clone()],
        installations: vec![installed.clone()],
        portable_label_counters: Default::default(),
    };
    let preferences = Preferences {
        release_format: "installer".into(),
        architecture: "x64".into(),
        ..Default::default()
    };
    let plan = ReleasePlan {
        native_rollback: None,
        additional_installation: false,
        single_app: true,
        entries: vec![ReleaseEntry {
            app: installed.name.clone(),
            action: "Update".into(),
            version: "1.0.0".into(),
            asset: Some(Asset {
                name: "previous.msi".into(),
                size: fs::metadata(&package).unwrap().len(),
                browser_download_url: "https://example.invalid/not-requested".into(),
                digest: Some(format!("sha256:{}", files::hash(&package).unwrap())),
            }),
            destination: None,
            error: None,
            preferences: preferences.clone(),
        }],
        preferences,
        requested_tag: Some("1.0.0".into()),
        snapshot: vec![],
        desktop_shortcut: false,
        installer_destination: None,
    };
    assert!(execute_entries(&paths, &plan, &mut config, &job, None, None, None).is_err());
    let log = fs::read_to_string(paths.at("log")).unwrap();
    assert!(
        log.contains("current copy is installed for everyone"),
        "{log}"
    );
    assert!(!log.contains("Previous installation restored"), "{log}");
    assert!(!log.contains("Preparing previous installer"), "{log}");
    assert!(!paths.at("runtime/admin-batches").exists());
    assert!(!paths.at("runtime/api-cache").exists());
    assert_eq!(config.apps[0].path, installed.path);
    assert_eq!(
        fs::read(current_path.join("keep.txt")).unwrap(),
        b"preserved current installation"
    );
}
