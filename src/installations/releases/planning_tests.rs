#[cfg(target_os = "windows")]
#[test]
fn approved_recovery_is_pinned_and_never_falls_back_to_release_discovery() {
    use sha2::{Digest, Sha256};
    let (paths, job) = fixture();
    let app = crate::model::Installed {
        name: "photocraft".into(),
        ..Default::default()
    };
    let bytes = b"approved package fixture";
    let file = paths.at("releases/installers/photocraft/approved.msi");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, bytes).unwrap();
    let asset = Asset {
        name: "approved.msi".into(),
        size: bytes.len() as u64,
        browser_download_url: "https://example.invalid/never-requested".into(),
        digest: Some(format!("sha256:{:x}", Sha256::digest(bytes))),
    };
    let approved = std::collections::BTreeMap::from([("photocraft".into(), asset)]);
    assert_eq!(
        downgrade_recovery_path(&paths, &app, &job, Some(&approved)).unwrap(),
        file
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
fn fixture() -> (Paths, Job) {
    let root = std::env::temp_dir().join(format!("craft-plan-{}", uuid::Uuid::new_v4()));
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
fn additional_install_preview_preserves_existing_copy_and_rejects_collisions() {
    let (paths, job) = installed_fixture();
    let before = fs::read(paths.at("settings.json")).unwrap();
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
    assert_eq!(fs::read(paths.at("settings.json")).unwrap(), before);
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
    assert_eq!(fs::read(paths.at("settings.json")).unwrap(), before);
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
    let missing = plan_from_config(&paths, &job, None, prefs, config, |app| {
        Ok(release(app, "0.2.0"))
    })
    .unwrap();
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
    crate::files::write_json(&paths.at("manager-settings.json"), &preferences).unwrap();
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
    files::write_json(&paths.at("updater-settings.json"), &prefs).unwrap();
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
        &paths.at("settings.json"),
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
    files::write_json(&paths.at("manager-settings.json"), &prefs).unwrap();
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
    let mut config: crate::model::Config = files::read_json(&paths.at("settings.json")).unwrap();
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
    use sha2::Digest;
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
    let cache = paths.at(format!(
        "runtime/api-cache/{:x}.json",
        sha2::Sha256::digest(endpoint.as_bytes())
    ));
    files::write_json(
        &cache,
        &serde_json::json!({ "at": 0, "etag": null, "value": release("photocraft", "0.4.0") }),
    )
    .unwrap();
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
        &paths.at("settings.json"),
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
    files::write_json(&paths.at("manager-settings.json"), &prefs).unwrap();
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
    files::write_json(&paths.at("manager-settings.json"), &prefs).unwrap();
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
    files::write_json(&paths.at("settings.json"), &config).unwrap();
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
    let saved = fs::read(paths.at("settings.json")).unwrap();
    let legacy = fs::read(paths.at("updater-settings.json")).unwrap();
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
    assert_eq!(fs::read(paths.at("updater-settings.json")).unwrap(), legacy);
    assert_eq!(fs::read(paths.at("settings.json")).unwrap(), saved);
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
    files::write_json(&paths.at("manager-settings.json"), &prefs).unwrap();
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
    files::write_json(&paths.at("manager-settings.json"), &preferences).unwrap();
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
