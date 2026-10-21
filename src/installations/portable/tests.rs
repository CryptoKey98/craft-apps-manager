#[test]
fn shortcut_migration_continues_after_errors_and_visits_each_copy_once() {
    let copy = |path: &str| crate::model::Installed {
        name: "filmcraft".into(),
        path: path.into(),
        install_kind: "portable".into(),
        ..Default::default()
    };
    let config = crate::model::Config {
        apps_root: String::new(),
        installations: vec![copy("damaged"), copy("conflict"), copy("healthy")],
        apps: vec![copy("healthy")],
        portable_label_counters: Default::default(),
    };
    let mut visited = Vec::new();
    let result = super::migrate_shortcut_records(&config, |copy| {
        visited.push(copy.path.clone());
        if copy.path != "healthy" {
            anyhow::bail!("fixture migration failure");
        }
        Ok(())
    });
    assert_eq!(visited, ["damaged", "conflict", "healthy"]);
    let message = result.unwrap_err().to_string();
    assert!(message.contains("damaged") && message.contains("conflict"));
}

#[test]
fn empty_generated_installation_parents_are_removed_but_user_folders_are_kept() {
    let (root, paths) = fixture();
    let parent = paths.at(format!(
        "releases/filmcraft-x64-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&parent).unwrap();
    cleanup_empty_installation_parent(&paths, "filmcraft", &parent.join("filmcraft")).unwrap();
    assert!(!parent.exists());
    assert!(paths.at("releases").exists());

    // The install dialog historically omitted the architecture component.
    let dialog_parent = paths.at(format!(
        "releases/filmcraft-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&dialog_parent).unwrap();
    cleanup_empty_installation_parent(&paths, "filmcraft", &dialog_parent.join("filmcraft"))
        .unwrap();
    assert!(!dialog_parent.exists());

    std::fs::create_dir_all(&parent).unwrap();
    std::fs::write(parent.join("notes.txt"), b"keep").unwrap();
    cleanup_empty_installation_parent(&paths, "filmcraft", &parent.join("filmcraft")).unwrap();
    assert_eq!(std::fs::read(parent.join("notes.txt")).unwrap(), b"keep");

    let custom = root.join("my-apps");
    std::fs::create_dir_all(&custom).unwrap();
    cleanup_empty_installation_parent(&paths, "filmcraft", &custom.join("filmcraft")).unwrap();
    assert!(custom.exists());
    let ordinary = paths.at("releases/my-folder");
    std::fs::create_dir_all(&ordinary).unwrap();
    cleanup_empty_installation_parent(&paths, "filmcraft", &ordinary.join("filmcraft")).unwrap();
    assert!(ordinary.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn installation_shortcut_names_distinguish_copies_and_validate_ownership() {
    let root = std::env::temp_dir().join("shortcut-names");
    let desktop = root.join("desktop");
    let first =
        installation_shortcut_title("filmcraft", &root.join("first"), "Portable", "x64", "A")
            .unwrap();
    let second =
        installation_shortcut_title("filmcraft", &root.join("second"), "Portable", "x64", "B")
            .unwrap();
    let installer =
        installation_shortcut_title("filmcraft", &root.join("first"), "Installer", "x64", "")
            .unwrap();
    assert_ne!(first, second);
    assert_ne!(first, installer);
    assert_eq!(installer, "FilmCraft (x64)");
    for title in ["FilmCraft (x64)", "FilmCraft (Installer x64)"] {
        let shortcut = desktop.join(format!("{title}.{}", crate::platform::shortcut_extension()));
        assert_eq!(
            owned_installation_title("filmcraft", &shortcut, &desktop).as_deref(),
            Some(title)
        );
    }
    let shortcut = desktop.join(format!("{first}.{}", crate::platform::shortcut_extension()));
    assert_eq!(
        owned_installation_title("filmcraft", &shortcut, &desktop),
        Some(first)
    );
    assert!(owned_installation_title("photocraft", &shortcut, &desktop).is_none());
    assert!(owned_installation_title("filmcraft", &shortcut, &root).is_none());
}

#[cfg(target_os = "windows")]
#[test]
fn multiple_installation_shortcuts_survive_updates_moves_and_individual_removal() {
    let (root, paths) = fixture();
    let desktop = root.join("desktop");
    let mut copies = Vec::new();
    let mut config = paths.config().unwrap();
    for (name, kind) in [
        ("first", "Portable"),
        ("second", "Portable"),
        ("native", "Installer"),
    ] {
        let target = root.join(name);
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("filmcraft.exe"), b"fixture").unwrap();
        let record = crate::model::Installed {
            name: "filmcraft".into(),
            path: target.display().to_string(),
            install_kind: if kind == "Installer" {
                "installer"
            } else {
                "portable"
            }
            .into(),
            architecture: "x64".into(),
            ..Default::default()
        };
        *config
            .apps
            .iter_mut()
            .find(|copy| copy.name == "filmcraft")
            .unwrap() = record.clone();
        config.installations.push(record);
        paths.save_config(&config).unwrap();
        config = files::read_json(&paths.at("settings.json")).unwrap();
        let marker = target.join(MARKER);
        let owner = Ownership {
            app: "filmcraft".into(),
            library: paths.root.clone(),
            shortcut: None,
        };
        let owner = if name == "first" {
            create_named_shortcut(
                &marker,
                owner,
                &target,
                &desktop,
                "FilmCraft (Portable x64 · 1234abcd)",
            )
            .unwrap();
            files::read_json(&marker).unwrap()
        } else {
            owner
        };
        create_installation_shortcut(&paths, &marker, owner, &target, &desktop, kind).unwrap();
        let owner: Ownership = files::read_json(&marker).unwrap();
        let shortcut = owner.shortcut.as_ref().unwrap().0.clone();
        assert_eq!(
            PathBuf::from(crate::platform::shortcut_target(&shortcut).unwrap())
                .canonicalize()
                .unwrap(),
            target.join("filmcraft.exe").canonicalize().unwrap()
        );
        copies.push((target, marker, shortcut));
    }
    assert_eq!(std::fs::read_dir(&desktop).unwrap().count(), 3);
    let (target, marker, shortcut) = &copies[0];
    let owner: Ownership = files::read_json(marker).unwrap();
    create_installation_shortcut(&paths, marker, owner, target, &desktop, "Portable").unwrap();
    assert_eq!(std::fs::read_dir(&desktop).unwrap().count(), 3);
    let moved = root.join("moved");
    std::fs::rename(target, &moved).unwrap();
    config
        .installations
        .iter_mut()
        .find(|copy| Path::new(&copy.path) == target)
        .unwrap()
        .path = moved.display().to_string();
    paths.save_config(&config).unwrap();
    let marker = moved.join(MARKER);
    let owner: Ownership = files::read_json(&marker).unwrap();
    create_installation_shortcut(&paths, &marker, owner, &moved, &desktop, "Portable").unwrap();
    assert_eq!(
        PathBuf::from(crate::platform::shortcut_target(shortcut).unwrap())
            .canonicalize()
            .unwrap(),
        moved.join("filmcraft.exe").canonicalize().unwrap()
    );
    let owner: Ownership = files::read_json(&marker).unwrap();
    remove_owned_shortcut(owner, &desktop).unwrap();
    assert!(!shortcut.exists());
    assert!(copies[1].2.exists() && copies[2].2.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(target_os = "windows")]
#[test]
fn legacy_shortcut_is_replaced_only_when_still_owned() {
    for edited in [false, true] {
        let (root, paths) = fixture();
        let target = root.join("app");
        let desktop = root.join("desktop");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("filmcraft.exe"), b"fixture").unwrap();
        // A pre-label installation record must still be usable.
        files::write_json(&paths.at("settings.json"), &serde_json::json!({"appsRoot":paths.root, "apps":[{"name":"filmcraft","path":target,"version":"0.1.0","architecture":"x64","install_kind":"portable"}]})).unwrap();
        let marker = target.join(MARKER);
        create_shortcut(
            &marker,
            Ownership {
                app: "filmcraft".into(),
                library: paths.root.clone(),
                shortcut: None,
            },
            &target,
            &desktop,
        )
        .unwrap();
        let legacy = desktop.join("FilmCraft.lnk");
        if edited {
            std::fs::write(&legacy, b"user edit").unwrap();
        }
        let owner: Ownership = files::read_json(&marker).unwrap();
        create_installation_shortcut(&paths, &marker, owner, &target, &desktop, "Portable")
            .unwrap();
        assert_eq!(legacy.exists(), edited);
        if edited {
            assert_eq!(std::fs::read(&legacy).unwrap(), b"user edit");
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(target_os = "windows")]
#[test]
fn installer_shortcuts_are_tracked_without_writing_to_app_folder() {
    use super::*;
    let root = std::env::temp_dir().join(format!("craft-shortcut-{}", uuid::Uuid::new_v4()));
    let target = root.join("installed-app");
    let desktop = root.join("desktop");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("photocraft.exe"), b"fixture").unwrap();
    let marker = root.join("runtime/desktop-shortcuts/photocraft.json");
    let owner = Ownership {
        app: "photocraft".into(),
        library: root.clone(),
        shortcut: None,
    };
    create_shortcut(&marker, owner, &target, &desktop).unwrap();
    assert!(!target.join(MARKER).exists());
    let shortcut = desktop.join("PhotoCraft.lnk");
    assert!(shortcut.is_file());
    let owner: Ownership = files::read_json(&marker).unwrap();
    create_shortcut(&marker, owner, &target, &desktop).unwrap();
    std::fs::write(&shortcut, b"user-edited shortcut").unwrap();
    let owner: Ownership = files::read_json(&marker).unwrap();
    remove_owned_shortcut(owner, &desktop).unwrap();
    assert_eq!(std::fs::read(&shortcut).unwrap(), b"user-edited shortcut");
    let owner: Ownership = files::read_json(&marker).unwrap();
    assert!(create_shortcut(&marker, owner, &target, &desktop).is_err());
    std::fs::remove_file(&shortcut).unwrap();
    let owner: Ownership = files::read_json(&marker).unwrap();
    create_shortcut(&marker, owner, &target, &desktop).unwrap();
    let owner: Ownership = files::read_json(&marker).unwrap();
    remove_owned_shortcut(owner, &desktop).unwrap();
    assert!(!shortcut.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(target_os = "windows")]
#[test]
fn shortcuts_repair_when_the_target_still_points_at_the_app() {
    use super::*;
    let root = std::env::temp_dir().join(format!("craft-shortcut-{}", uuid::Uuid::new_v4()));
    let target = root.join("installed-app");
    let desktop = root.join("desktop");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("photocraft.exe"), b"fixture").unwrap();
    let marker = root.join("runtime/desktop-shortcuts/photocraft.json");
    let owner = Ownership {
        app: "photocraft".into(),
        library: root.clone(),
        shortcut: None,
    };
    create_shortcut(&marker, owner, &target, &desktop).unwrap();
    let shortcut = desktop.join("PhotoCraft.lnk");
    // Rewriting the shortcut with different metadata keeps the same target
    // but breaks the recorded proof hash, as an update or repair would.
    crate::platform::shortcut(
        &shortcut,
        &target.join("photocraft.exe"),
        "",
        &target.join("elsewhere"),
    )
    .unwrap();
    let owner: Ownership = files::read_json(&marker).unwrap();
    create_shortcut(&marker, owner, &target, &desktop).unwrap();
    let owner: Ownership = files::read_json(&marker).unwrap();
    remove_owned_shortcut(owner, &desktop).unwrap();
    assert!(!shortcut.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(target_os = "windows")]
#[test]
fn installer_cache_path_alone_does_not_prove_shortcut_ownership() {
    use super::*;
    let root = std::env::temp_dir().join(format!("craft-shortcut-{}", uuid::Uuid::new_v4()));
    let target = root.join("installed-app");
    let desktop = root.join("desktop");
    std::fs::create_dir_all(&target).unwrap();
    let exe = target.join("photocraft.exe");
    std::fs::write(&exe, b"fixture").unwrap();
    // Installer updates leave advertised shortcuts whose target lives in
    // the Windows Installer cache and breaks on the next major upgrade.
    let advertised = PathBuf::from(
        r"C:\Windows\Installer\{01234567-89AB-CDEF-0123-456789ABCDEF}\PhotocraftIcon.ico",
    );
    std::fs::create_dir_all(&desktop).unwrap();
    let shortcut = desktop.join("PhotoCraft.lnk");
    crate::platform::shortcut(&shortcut, &advertised, "", &target).unwrap();
    let marker = root.join("runtime/desktop-shortcuts/photocraft.json");
    let owner = Ownership {
        app: "photocraft".into(),
        library: root.clone(),
        shortcut: None,
    };
    let before = std::fs::read(&shortcut).unwrap();
    assert!(create_shortcut(&marker, owner, &target, &desktop).is_err());
    assert_eq!(std::fs::read(&shortcut).unwrap(), before);
    assert!(crate::platform::advertised_shortcut_target(&shortcut).is_none());
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(target_os = "windows")]
#[test]
fn shortcuts_pointing_elsewhere_are_left_unchanged() {
    use super::*;
    let root = std::env::temp_dir().join(format!("craft-shortcut-{}", uuid::Uuid::new_v4()));
    let target = root.join("installed-app");
    let desktop = root.join("desktop");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("photocraft.exe"), b"fixture").unwrap();
    std::fs::write(target.join("other.exe"), b"other").unwrap();
    std::fs::create_dir_all(&desktop).unwrap();
    let shortcut = desktop.join("PhotoCraft.lnk");
    crate::platform::shortcut(&shortcut, &target.join("other.exe"), "", &target).unwrap();
    let before = std::fs::read(&shortcut).unwrap();
    let marker = root.join("runtime/desktop-shortcuts/photocraft.json");
    let owner = Ownership {
        app: "photocraft".into(),
        library: root.clone(),
        shortcut: None,
    };
    assert!(create_shortcut(&marker, owner, &target, &desktop).is_err());
    assert_eq!(std::fs::read(&shortcut).unwrap(), before);
    crate::platform::shortcut(
        &shortcut,
        &target.join("photocraft.exe"),
        "--user-option",
        &target,
    )
    .unwrap();
    let before = std::fs::read(&shortcut).unwrap();
    let owner = Ownership {
        app: "photocraft".into(),
        library: root.clone(),
        shortcut: None,
    };
    assert!(create_shortcut(&marker, owner, &target, &desktop).is_err());
    assert_eq!(std::fs::read(&shortcut).unwrap(), before);
    std::fs::remove_dir_all(root).unwrap();
}

use super::*;
use crate::model::Preferences;
#[cfg(unix)]
#[test]
fn destination_rejects_custom_links_even_below_system_temp_directory() {
    let root = std::env::temp_dir().join(format!("craft-linked-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join("real")).unwrap();
    std::os::unix::fs::symlink(root.join("real"), root.join("linked")).unwrap();
    assert!(destination(&root.join("real"), "photocraft").is_ok());
    assert!(destination(&root.join("linked"), "photocraft").is_err());
    std::fs::remove_dir_all(root).unwrap();
}
fn fixture() -> (PathBuf, Paths) {
    let root = std::env::temp_dir().join(format!("craft-custom-{}", uuid::Uuid::new_v4()));
    let paths = Paths::new(root.join("library"), None);
    files::write_json(
        &paths.at("manager-settings.json"),
        &Preferences {
            release_format: "portable".into(),
            ..Default::default()
        },
    )
    .unwrap();
    (root, paths)
}
#[test]
fn destinations_reject_relative_traversal_internal_and_existing_unmanaged_folders() {
    let (root, paths) = fixture();
    assert!(destination(Path::new("relative"), "photocraft").is_err());
    assert!(destination(&root.join(".."), "photocraft").is_err());
    assert!(validate_install(&paths, "photocraft", &paths.at("sources/photocraft")).is_err());
    let target = destination(&root.join("apps"), "photocraft").unwrap();
    assert!(!target.exists());
    validate_install(&paths, "photocraft", &target).unwrap();
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("work.png"), b"personal").unwrap();
    assert!(validate_install(&paths, "photocraft", &target).is_err());
    assert_eq!(std::fs::read(target.join("work.png")).unwrap(), b"personal");
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn additional_library_copies_are_flat_numbered_and_never_replace_existing_files() {
    let (root, paths) = fixture();
    let library = paths.at("releases");
    assert_eq!(
        additional_destination(&paths, &library, "filmcraft").unwrap(),
        library.join("filmcraft")
    );
    std::fs::create_dir_all(library.join("filmcraft")).unwrap();
    std::fs::write(library.join("filmcraft/keep.txt"), b"original").unwrap();
    let second = additional_destination(&paths, &library, "filmcraft").unwrap();
    assert_eq!(second, library.join("filmcraft-2"));
    validate_install(&paths, "filmcraft", &second).unwrap();
    std::fs::write(&second, b"unrelated file").unwrap();
    assert!(validate_install(&paths, "filmcraft", &second).is_err());
    assert_eq!(
        additional_destination(&paths, &library, "filmcraft").unwrap(),
        library.join("filmcraft-3")
    );
    let custom_parent = root.join("custom");
    assert_eq!(
        additional_destination(&paths, &custom_parent, "filmcraft").unwrap(),
        custom_parent.join("filmcraft")
    );
    assert!(validate_install(&paths, "filmcraft", &custom_parent.join("filmcraft-2")).is_err());
    assert_eq!(
        std::fs::read(library.join("filmcraft/keep.txt")).unwrap(),
        b"original"
    );
    assert_eq!(std::fs::read(&second).unwrap(), b"unrelated file");
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn custom_folder_ownership_is_bound_to_app_and_library() {
    let (root, paths) = fixture();
    let target = destination(&root.join("apps"), "photocraft").unwrap();
    std::fs::create_dir_all(&target).unwrap();
    mark(&paths, "photocraft", &target).unwrap();
    validate(&paths, "photocraft", &target).unwrap();
    assert!(validate(&paths, "filmcraft", &target).is_err());
    let other = Paths::new(root.join("other-library"), None);
    assert!(validate(&other, "photocraft", &target).is_err());
    std::fs::remove_dir_all(root).unwrap();
}
#[cfg(not(target_os = "macos"))]
#[test]
fn custom_portable_uninstall_keeps_projects_and_retains_photo_data() {
    let (root, paths) = fixture();
    let parent = root.join("apps");
    let target = destination(&parent, "photocraft").unwrap();
    std::fs::create_dir_all(target.join("PhotoCraftData")).unwrap();
    let exe = target.join(crate::model::executable_name("photocraft"));
    std::fs::write(&exe, b"fixture").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::write(target.join("PhotoCraftData/recovery.pcraft"), b"recovery").unwrap();
    std::fs::write(parent.join("my-image.png"), b"image").unwrap();
    mark(&paths, "photocraft", &target).unwrap();
    let mut config = paths.config().unwrap();
    let app = config
        .apps
        .iter_mut()
        .find(|a| a.name == "photocraft")
        .unwrap();
    app.path = target.display().to_string();
    app.version = "0.3.0".into();
    app.install_kind = "portable".into();
    paths.save_config(&config).unwrap();
    assert_eq!(
        crate::apps::executables(&paths, "photocraft")
            .unwrap()
            .len(),
        1
    );
    validate_install(&paths, "photocraft", &target).unwrap();
    crate::apps::uninstall_with_profile(&paths, "photocraft", false).unwrap();
    assert!(!target.exists());
    assert_eq!(
        std::fs::read(parent.join("my-image.png")).unwrap(),
        b"image"
    );
    assert_eq!(
        std::fs::read(paths.at("runtime/app-profiles/photocraft/recovery.pcraft")).unwrap(),
        b"recovery"
    );
    assert!(!crate::apps::installed(&paths, "photocraft").is_ok());
    std::fs::remove_dir_all(root).unwrap();
}
