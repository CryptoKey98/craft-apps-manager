#[cfg(target_os = "windows")]
#[test]
fn legacy_path_aliases_are_one_installation_after_refresh_and_save() {
    use super::*;
    let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let paths = Paths::new(root.clone(), None);
    let folder = root.join("copies/filmcraft");
    release_fixture(&folder, "filmcraft");
    let first = Installed {
        name: "filmcraft".into(),
        path: folder.display().to_string(),
        version: "0.6.0".into(),
        architecture: "x64".into(),
        install_kind: "portable".into(),
        installation_label: "A".into(),
        ..Default::default()
    };
    let mut alias = first.clone();
    alias.path = alias.path.replace('\\', "/").to_uppercase();
    alias.installation_label = "B".into();
    let config = Config {
        apps_root: root.display().to_string(),
        apps: vec![alias.clone()],
        installations: vec![first, alias],
        portable_label_counters: Default::default(),
    };
    crate::files::write_json(&paths.at("settings.json"), &config).unwrap();
    let mut refreshed = paths.config_with_detector(|_| Ok(None)).unwrap();
    assert_eq!(
        refreshed
            .installations
            .iter()
            .filter(|copy| copy.name == "filmcraft")
            .count(),
        1
    );
    let selected = refreshed
        .apps
        .iter()
        .find(|copy| copy.name == "filmcraft")
        .unwrap()
        .clone();
    assert_eq!(selected.installation_label, "A");
    paths.save_config(&refreshed).unwrap();
    crate::apps::forget_installation(&mut refreshed, &selected);
    assert!(!refreshed
        .installations
        .iter()
        .any(|copy| copy.name == "filmcraft"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn settings_save_preserves_same_architecture_installations_in_other_folders() {
    use super::*;
    let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let paths = Paths::new(root.clone(), None);
    let copies: Vec<_> = ["first", "second", "third"]
        .into_iter()
        .map(|name| {
            let folder = root.join(name);
            release_fixture(&folder, "filmcraft");
            Installed {
                name: "filmcraft".into(),
                path: folder.display().to_string(),
                version: "0.6.0".into(),
                architecture: "x64".into(),
                install_kind: "portable".into(),
                ..Default::default()
            }
        })
        .collect();
    paths
        .save_config(&Config {
            portable_label_counters: Default::default(),
            apps_root: root.display().to_string(),
            apps: vec![copies[0].clone()],
            installations: copies.clone(),
        })
        .unwrap();
    crate::settings::save(
        &paths,
        &root.join("locations.json"),
        &(),
        &root,
        &paths.tools,
        Some(&Preferences::default()),
        None,
    )
    .unwrap();
    let saved: Config = crate::files::read_json(&paths.at("settings.json")).unwrap();
    for copy in &copies {
        assert!(
            saved
                .installations
                .iter()
                .any(|a| a.name == copy.name && a.path == copy.path),
            "Settings save lost installation {}",
            copy.path
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn legacy_versioned_portable_recovers_package_type() {
    use super::*;
    let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let paths = Paths::new(root.clone(), None);
    let folder = paths.at(format!("releases/filmcraft-0.6.0-{}-x64", release_os()));
    release_fixture(&folder, "filmcraft");
    let config = paths.config_with_detector(|_| Ok(None)).unwrap();
    let installed = config.apps.iter().find(|a| a.name == "filmcraft").unwrap();
    assert_eq!(Path::new(&installed.path), folder);
    assert_eq!(installed.install_kind, "portable");
    // A folder name is not sufficient proof of the executable architecture.
    #[cfg(not(target_os = "macos"))]
    assert!(installed.architecture.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn discovered_portable_has_consistent_installed_state_without_settings() {
    use super::*;
    let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let paths = Paths::new(root.clone(), None);
    let folder = paths.at("releases/designcraft");
    release_fixture(&folder, "designcraft");
    // A minimal PE header: architecture is evidence from the executable, not preferences.
    #[cfg(target_os = "windows")]
    {
        let mut bytes = vec![0u8; 256];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[60..64].copy_from_slice(&128u32.to_le_bytes());
        bytes[128..132].copy_from_slice(b"PE\0\0");
        bytes[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
        bytes[148..150].copy_from_slice(&70u16.to_le_bytes());
        bytes[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
        std::fs::write(folder.join(executable_name("designcraft")), bytes).unwrap();
    }
    for _ in 0..2 {
        let config = paths.config_with_detector(|_| Ok(None)).unwrap();
        let installed = config
            .apps
            .iter()
            .find(|a| a.name == "designcraft")
            .unwrap();
        assert!(
            !installed.version.is_empty(),
            "An existing executable must not display Get"
        );
        assert_eq!(installed.install_kind, "portable");
        #[cfg(target_os = "windows")]
        assert!(crate::install_options::InstallOptions::installed(installed).is_some());
        paths.save_config(&config).unwrap();
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn same_architecture_copies_can_be_selected_and_uninstalled_independently() {
    use super::*;
    let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let paths = Paths::new(root.clone(), None);
    let copies: Vec<_> = ["first", "second"]
        .into_iter()
        .map(|name| {
            let folder = root.join(name);
            release_fixture(&folder, "filmcraft");
            Installed {
                name: "filmcraft".into(),
                path: folder.display().to_string(),
                version: "0.6.0".into(),
                architecture: "x64".into(),
                install_kind: "portable".into(),
                ..Default::default()
            }
        })
        .collect();
    for selected in &copies {
        let config = Config {
            portable_label_counters: Default::default(),
            apps_root: root.display().to_string(),
            apps: vec![selected.clone()],
            installations: copies.clone(),
        };
        paths.save_config(&config).unwrap();
        let restarted = paths.config_with_detector(|_| Ok(None)).unwrap();
        assert_eq!(
            restarted
                .apps
                .iter()
                .find(|copy| copy.name == "filmcraft")
                .unwrap()
                .path,
            selected.path
        );
        assert_eq!(
            restarted
                .installations
                .iter()
                .filter(|copy| copy.name == "filmcraft")
                .count(),
            2
        );
    }
    std::fs::remove_dir_all(&copies[1].path).unwrap();
    let mut config = paths.config_with_detector(|_| Ok(None)).unwrap();
    crate::apps::forget_installation(&mut config, &copies[1]);
    paths.save_config(&config).unwrap();
    let restarted = paths.config_with_detector(|_| Ok(None)).unwrap();
    assert_eq!(
        restarted
            .apps
            .iter()
            .find(|copy| copy.name == "filmcraft")
            .unwrap()
            .path,
        copies[0].path
    );
    assert_eq!(
        restarted
            .installations
            .iter()
            .filter(|copy| copy.name == "filmcraft")
            .count(),
        1
    );
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn visibility_defaults_visible_and_preserves_update_selection_and_order() {
    let mut preferences: super::Preferences = serde_json::from_str("{}").unwrap();
    assert!(preferences.hidden_apps.is_empty());
    preferences.validate().unwrap();
    let selection = preferences.selected_apps.clone();
    let order = preferences.app_order.clone();
    preferences.hidden_apps.push("photocraft".into());
    preferences.validate().unwrap();
    let reopened: super::Preferences =
        serde_json::from_slice(&serde_json::to_vec(&preferences).unwrap()).unwrap();
    assert_eq!(reopened.hidden_apps, ["photocraft"]);
    assert_eq!(reopened.selected_apps, selection);
    assert_eq!(reopened.app_order, order);
}
use super::*;
#[test]
fn backup_defaults_are_off_and_saved_choices_are_preserved() {
    let fresh = Preferences::default();
    assert!(!fresh.keep_app_backups);
    assert!(!fresh.keep_source_backups);
    let saved: Preferences =
        serde_json::from_str(r#"{"keepAppBackups":true,"keepSourceBackups":true}"#).unwrap();
    assert!(saved.keep_app_backups);
    assert!(saved.keep_source_backups);
}
#[test]
fn theme_defaults_for_existing_settings_and_survives_restart() {
    let legacy: Preferences = serde_json::from_str(r#"{"selectedApps":["filmcraft"]}"#).unwrap();
    assert_eq!(legacy.theme, Theme::System);
    let root = std::env::temp_dir().join(format!("craft-theme-{}", uuid::Uuid::new_v4()));
    let paths = Paths::new(root.clone(), None);
    let mut preferences = legacy;
    preferences.theme = Theme::Light;
    // Written after the newest apps were found, so none are adopted on reopening.
    preferences.known_apps = crate::catalog::all().into_iter().map(|e| e.key).collect();
    crate::files::write_json(&paths.at("manager-settings.json"), &preferences).unwrap();
    let reopened = paths.read_preferences().unwrap();
    assert_eq!(reopened.theme, Theme::Light);
    assert_eq!(reopened.selected_apps, ["filmcraft"]);
    std::fs::remove_dir_all(root).unwrap();
}
fn release_fixture(folder: &Path, app: &str) {
    std::fs::create_dir_all(folder).unwrap();
    let executable = folder.join(executable_name(app));
    if cfg!(target_os = "macos") {
        let contents = executable.join("Contents");
        std::fs::create_dir_all(&contents).unwrap();
        std::fs::write(contents.join("Info.plist"), format!("<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>ai.storyteller.{app}</string><key>CFBundleShortVersionString</key><string>0.4.0</string></dict></plist>")).unwrap();
    } else {
        std::fs::write(executable, b"fixture").unwrap();
    }
}
#[test]
fn uninstalling_one_format_preserves_other_copy_across_restart() {
    for removed_kind in ["installer", "portable"] {
        let root =
            std::env::temp_dir().join(format!("craft-uninstall-copies-{}", uuid::Uuid::new_v4()));
        let paths = Paths::new(root.clone(), None);
        let copies: Vec<_> = ["installer", "portable"]
            .into_iter()
            .map(|kind| {
                let folder = root.join(kind);
                release_fixture(&folder, "filmcraft");
                Installed {
                    name: "filmcraft".into(),
                    version: "0.6.0".into(),
                    architecture: "x64".into(),
                    install_kind: kind.into(),
                    path: folder.display().to_string(),
                    ..Default::default()
                }
            })
            .collect();
        let removed = copies
            .iter()
            .find(|copy| copy.install_kind == removed_kind)
            .unwrap();
        let remaining = copies
            .iter()
            .find(|copy| copy.install_kind != removed_kind)
            .unwrap();
        let prefs = Preferences {
            app_install_options: [(
                crate::install_options::identity("filmcraft"),
                crate::install_options::InstallOptions {
                    release_format: removed_kind.into(),
                    architecture: "x64".into(),
                },
            )]
            .into(),
            ..Default::default()
        };
        crate::files::write_json(&paths.at("manager-settings.json"), &prefs).unwrap();
        let mut config = Config {
            portable_label_counters: Default::default(),
            apps_root: root.display().to_string(),
            apps: vec![removed.clone()],
            installations: copies.clone(),
        };
        // Simulate only the selected uninstall; never invoke an installer.
        std::fs::remove_dir_all(&removed.path).unwrap();
        crate::apps::forget_installation(&mut config, removed);
        assert_eq!(config.apps[0].path, remaining.path);
        paths.save_config(&config).unwrap();
        let reopened = paths
            .config_with_detector(|name| {
                Ok(
                    (name == "filmcraft" && remaining.install_kind == "installer")
                        .then(|| remaining.clone()),
                )
            })
            .unwrap();
        let active = reopened
            .apps
            .iter()
            .find(|app| app.name == "filmcraft")
            .unwrap();
        assert_eq!(active.path, remaining.path);
        assert_eq!(active.install_kind, remaining.install_kind);
        assert!(reopened
            .installations
            .iter()
            .any(|copy| copy.path == remaining.path));
        assert!(!reopened
            .installations
            .iter()
            .any(|copy| copy.path == removed.path));
        assert!(installed_executable(Path::new(&remaining.path), "filmcraft").is_some());
        std::fs::remove_dir_all(root).unwrap();
    }
}
#[test]
fn missing_old_portable_record_is_rediscovered_after_installer_removal() {
    let root =
        std::env::temp_dir().join(format!("craft-portable-recovery-{}", uuid::Uuid::new_v4()));
    let paths = Paths::new(root.clone(), None);
    let folder = paths.at("releases/filmcraft");
    release_fixture(&folder, "filmcraft");
    let prefs = Preferences {
        app_install_options: [(
            crate::install_options::identity("filmcraft"),
            crate::install_options::InstallOptions {
                release_format: "installer".into(),
                architecture: "x64".into(),
            },
        )]
        .into(),
        ..Default::default()
    };
    crate::files::write_json(&paths.at("manager-settings.json"), &prefs).unwrap();
    crate::files::write_json(
        &paths.at("settings.json"),
        &Config {
            portable_label_counters: Default::default(),
            apps_root: root.display().to_string(),
            apps: vec![Installed {
                name: "filmcraft".into(),
                install_kind: String::new(),
                ..Default::default()
            }],
            installations: Vec::new(),
        },
    )
    .unwrap();
    let recovered = paths.config_with_detector(|_| Ok(None)).unwrap();
    let app = recovered
        .apps
        .iter()
        .find(|app| app.name == "filmcraft")
        .unwrap();
    assert_eq!(app.path, folder.display().to_string());
    assert_eq!(app.install_kind, "portable");
    paths.save_config(&recovered).unwrap();
    assert_eq!(
        paths
            .config_with_detector(|_| Ok(None))
            .unwrap()
            .apps
            .iter()
            .find(|app| app.name == "filmcraft")
            .unwrap()
            .path,
        app.path
    );
    assert!(folder.exists());
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn native_install_in_library_is_not_a_second_portable_installation() {
    let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let paths = Paths::new(root.clone(), None);
    let folder = paths.at("releases/filmcraft");
    release_fixture(&folder, "filmcraft");
    let native = Installed {
        name: "filmcraft".into(),
        path: folder.display().to_string(),
        version: "0.5.0".into(),
        install_kind: "installer".into(),
        architecture: "x64".into(),
        ..Default::default()
    };
    let mut config = Config {
        portable_label_counters: Default::default(),
        apps: vec![native.clone()],
        installations: vec![native.clone()],
        apps_root: root.display().to_string(),
    };
    assert!(paths
        .alternates_with_detector(&config, |_| Ok(None))
        .unwrap()
        .is_empty());
    let mut stale = native;
    stale.install_kind = "portable".into();
    config.installations.push(stale);
    assert!(paths
        .alternates_with_detector(&config, |_| Ok(None))
        .unwrap()
        .is_empty());
    let separate = root.join("separate-portable");
    release_fixture(&separate, "filmcraft");
    config.installations.last_mut().unwrap().path = separate.display().to_string();
    assert_eq!(
        paths
            .alternates_with_detector(&config, |_| Ok(None))
            .unwrap()
            .len(),
        1
    );
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn alternate_snapshot_detects_fresh_installers_and_forgets_removed_records_without_writes() {
    let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let paths = Paths::new(root.clone(), None);
    crate::files::write_json(
        &paths.at("updater-settings.json"),
        &Preferences {
            release_format: "portable".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let config = paths.config_with_detector(|_| Ok(None)).unwrap();
    let alternate = root.join("Applications");
    release_fixture(&alternate, "photocraft");
    let detect = |name: &str| -> Result<Option<Installed>> {
        Ok(
            (name == "photocraft" && installed_executable(&alternate, name).is_some()).then(|| {
                Installed {
                    name: name.into(),
                    path: alternate.display().to_string(),
                    version: "0.4.0".into(),
                    install_kind: "installer".into(),
                    ..Default::default()
                }
            }),
        )
    };
    let facts = paths.alternates_with_detector(&config, detect).unwrap();
    assert_eq!(facts.len(), 1);
    assert!(config.apps.iter().all(|a| a.path.is_empty()));
    std::fs::remove_dir_all(&alternate).unwrap();
    assert!(paths
        .alternates_with_detector(&config, detect)
        .unwrap()
        .is_empty());
    assert!(!paths.at("settings.json").exists());
    assert!(!paths.at("manager-settings.json").exists());
    assert!(paths.at("updater-settings.json").exists());
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn installer_snapshot_verifies_portable_records_and_does_not_route_actions_to_them() {
    let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let paths = Paths::new(root.clone(), None);
    crate::files::write_json(
        &paths.at("manager-settings.json"),
        &Preferences {
            release_format: "installer".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let portable = paths.at("releases/photocraft");
    release_fixture(&portable, "photocraft");
    let config = paths.config_with_detector(|_| Ok(None)).unwrap();
    assert_eq!(
        config
            .apps
            .iter()
            .find(|app| app.name == "photocraft")
            .unwrap()
            .path,
        portable.display().to_string()
    );
    assert!(paths
        .alternates_with_detector(&config, |_| Ok(None))
        .unwrap()
        .is_empty());
    std::fs::remove_dir_all(portable).unwrap();
    assert!(paths
        .alternates_with_detector(&config, |_| Ok(None))
        .unwrap()
        .is_empty());
    assert!(!paths.at("settings.json").exists());
    std::fs::remove_dir_all(root).unwrap();
}
#[cfg(target_os = "windows")]
#[test]
fn installer_architecture_conflicts_block_native_installs_only() {
    let installed = Installed {
        name: "designcraft".into(),
        path: r"C:\Program Files\designcraft".into(),
        version: "0.5.0".into(),
        architecture: "x86".into(),
        install_kind: "installer".into(),
        ..Default::default()
    };
    let prefs = Preferences {
        release_format: "installer".into(),
        architecture: "x64".into(),
        ..Default::default()
    };
    assert!(installer_architecture_conflict(&installed, &prefs));
    assert!(installer_architecture_conflict_message(&installed, &prefs)
        .contains("Uninstall the x86 version before installing x64"));
    assert!(!installer_architecture_conflict(
        &installed,
        &Preferences {
            architecture: "x86".into(),
            ..prefs.clone()
        }
    ));
    assert!(!installer_architecture_conflict(
        &installed,
        &Preferences {
            release_format: "portable".into(),
            ..prefs.clone()
        }
    ));
    assert!(!installer_architecture_conflict(
        &Installed {
            install_kind: "portable".into(),
            ..installed.clone()
        },
        &prefs
    ));
    assert!(!installer_architecture_conflict(
        &Installed {
            path: String::new(),
            ..installed
        },
        &prefs
    ));
}
#[cfg(target_os = "windows")]
#[test]
fn selected_copy_survives_pending_architecture_changes_and_preserves_other_copy() {
    let root = std::env::temp_dir().join(format!("craft-arch-records-{}", uuid::Uuid::new_v4()));
    let paths = Paths::new(root.clone(), None);
    let copies: Vec<_> = ["x64", "x86"]
        .into_iter()
        .map(|arch| {
            let folder = root.join(arch);
            release_fixture(&folder, "photocraft");
            Installed {
                name: "photocraft".into(),
                architecture: arch.into(),
                version: "0.1.0".into(),
                path: folder.display().to_string(),
                install_kind: "portable".into(),
                ..Default::default()
            }
        })
        .collect();
    crate::files::write_json(
        &paths.at("settings.json"),
        &Config {
            portable_label_counters: Default::default(),
            apps: vec![copies[0].clone()],
            installations: copies.clone(),
            apps_root: root.display().to_string(),
        },
    )
    .unwrap();
    for selected in ["x86", "x64", "arm64"] {
        let prefs = Preferences {
            app_install_options: [(
                crate::install_options::identity("photocraft"),
                crate::install_options::InstallOptions {
                    release_format: "portable".into(),
                    architecture: selected.into(),
                },
            )]
            .into(),
            release_format: "portable".into(),
            architecture: selected.into(),
            ..Default::default()
        };
        crate::files::write_json(&paths.at("manager-settings.json"), &prefs).unwrap();
        let config = paths.config_with_detector(|_| Ok(None)).unwrap();
        let installed = config.apps.iter().find(|a| a.name == "photocraft").unwrap();
        assert_eq!(installed.architecture, "x64");
        assert_eq!(
            installed_architecture_matches(installed, &prefs),
            selected == "x64"
        );
        paths.save_config(&config).unwrap();
        assert_eq!(
            paths
                .config_with_detector(|_| Ok(None))
                .unwrap()
                .installations
                .iter()
                .filter(|a| a.name == "photocraft")
                .count(),
            2
        );
    }
    for copy in copies {
        assert!(Path::new(&copy.path).exists());
    }
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn effective_architecture_preserves_legacy_macos_choices() {
    for architecture in ["x86", "x64", "arm64"] {
        let mut prefs = Preferences {
            architecture: architecture.into(),
            ..Default::default()
        };
        prefs.validate().unwrap();
        if cfg!(target_os = "macos") {
            assert_eq!(release_arch(architecture), release_arch("universal"));
            assert_eq!(
                architecture_label(architecture),
                "Universal (Intel + Apple silicon)"
            );
        } else {
            assert_ne!(release_arch(architecture), "universal");
            assert_eq!(architecture_label(architecture), architecture);
        }
    }
}
#[test]
fn existing_libraries_gain_new_apps_without_resetting_preferences() {
    let root = std::env::temp_dir().join(format!("craft-catalog-{}", uuid::Uuid::new_v4()));
    let paths = Paths::new(root.clone(), None);
    let prefs = Preferences {
        release_format: "installer".into(),
        selected_apps: vec!["filmcraft".into()],
        selected_sources: vec!["filmcraft".into()],
        app_order: vec!["filmcraft".into()],
        ..Default::default()
    };
    crate::files::write_json(&paths.at("manager-settings.json"), &prefs).unwrap();
    let config = Config {
        portable_label_counters: Default::default(),
        apps_root: root.display().to_string(),
        apps: vec![Installed {
            name: "filmcraft".into(),
            ..Default::default()
        }],
        installations: vec![],
    };
    crate::files::write_json(&paths.at("settings.json"), &config).unwrap();
    let detect = |name: &str| {
        Ok((name == "wordcraft").then(|| Installed {
            name: name.into(),
            path: "system/WordCraft".into(),
            version: "0.3.0".into(),
            install_kind: "installer".into(),
            ..Default::default()
        }))
    };
    let migrated = paths.config_with_detector(detect).unwrap();
    assert_eq!(migrated.apps.len(), apps().len());
    assert_eq!(
        migrated
            .apps
            .iter()
            .find(|a| a.name == "wordcraft")
            .unwrap()
            .version,
        "0.3.0"
    );
    paths.save_config(&migrated).unwrap();
    assert_eq!(
        paths.config_with_detector(detect).unwrap().apps.len(),
        apps().len()
    );
    let preserved = paths.preferences().unwrap();
    assert_eq!(preserved.selected_apps, ["filmcraft"]);
    assert_eq!(preserved.selected_sources, ["filmcraft"]);
    assert_eq!(preserved.app_order[0], "filmcraft");
    assert_eq!(preserved.app_order.len(), apps().len());
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn first_launch_detects_preexisting_installers_and_keeps_portable_inventory() {
    let root = std::env::temp_dir().join(format!("craft-first-launch-{}", uuid::Uuid::new_v4()));
    let paths = Paths::new(root.clone(), None);
    let portable = paths.at("releases/photocraft");
    std::fs::create_dir_all(&portable).unwrap();
    #[cfg(not(target_os = "macos"))]
    std::fs::write(portable.join(executable_name("photocraft")), b"portable").unwrap();
    #[cfg(target_os = "macos")]
    {
        let contents = portable
            .join(executable_name("photocraft"))
            .join("Contents");
        std::fs::create_dir_all(&contents).unwrap();
        std::fs::write(contents.join("Info.plist"), r#"<plist version="1.0"><dict><key>CFBundleIdentifier</key><string>ai.storyteller.photocraft</string><key>CFBundleShortVersionString</key><string>0.2.0</string></dict></plist>"#).unwrap();
    }
    crate::files::write_json(
        &paths.at("manager-settings.json"),
        &Preferences {
            release_format: "installer".into(),
            app_install_options: ["photocraft", "printcraft"]
                .into_iter()
                .map(|app| {
                    (
                        crate::install_options::identity(app),
                        crate::install_options::InstallOptions {
                            release_format: "installer".into(),
                            architecture: "x64".into(),
                        },
                    )
                })
                .collect(),
            ..Default::default()
        },
    )
    .unwrap();
    let detect = |name: &str| -> Result<Option<Installed>> {
        Ok(["photocraft", "printcraft"]
            .contains(&name)
            .then(|| Installed {
                installation_label: Default::default(),
                package_type: Default::default(),
                name: name.into(),
                path: root.join("system").join(name).display().to_string(),
                version: "0.2.1".into(),
                architecture: "x64".into(),
                install_kind: "installer".into(),
                product_code: "existing-product".into(),
            }))
    };
    assert!(!paths.at("settings.json").exists());
    let first = paths.config_with_detector(detect).unwrap();
    for name in ["photocraft", "printcraft"] {
        let app = first.apps.iter().find(|a| a.name == name).unwrap();
        assert_eq!(app.install_kind, "installer");
        assert_eq!(app.version, "0.2.1");
        assert!(first
            .installations
            .iter()
            .any(|a| a.name == name && a.install_kind == "installer"));
    }
    paths.save_config(&first).unwrap();
    let reopened = paths.config_with_detector(detect).unwrap();
    assert_eq!(
        reopened
            .apps
            .iter()
            .find(|a| a.name == "photocraft")
            .unwrap()
            .path,
        first
            .apps
            .iter()
            .find(|a| a.name == "photocraft")
            .unwrap()
            .path
    );
    crate::files::write_json(
        &paths.at("manager-settings.json"),
        &Preferences {
            release_format: "portable".into(),
            ..paths.read_preferences().unwrap()
        },
    )
    .unwrap();
    let switched = paths.config_with_detector(detect).unwrap();
    assert_eq!(
        switched
            .apps
            .iter()
            .find(|a| a.name == "photocraft")
            .unwrap()
            .path,
        first
            .apps
            .iter()
            .find(|app| app.name == "photocraft")
            .unwrap()
            .path
    );
    std::fs::remove_dir_all(root).unwrap();
}
