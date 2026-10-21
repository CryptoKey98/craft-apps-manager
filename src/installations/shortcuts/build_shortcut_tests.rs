use super::*;
#[test]
fn legacy_build_shortcut_cleanup_survives_portable_library_move() {
    let temp =
        std::env::temp_dir().join(format!("craft-portable-shortcut-{}", uuid::Uuid::new_v4()));
    let base = temp.join("original");
    let paths = Paths::new(base.join("data"), None);
    let folder = paths.at("builds/filmcraft/first");
    let desktop = temp.join("desktop");
    files::write_json(
        &folder.join("build-info.json"),
        &crate::model::BuildInfo {
            app: "filmcraft".into(),
            commit: "a".repeat(40),
            source_branch: "main".into(),
            built_at: "2026-10-12T12:00:00Z".into(),
            profile: "release".into(),
            log: String::new(),
        },
    )
    .unwrap();
    std::fs::write(folder.join("filmcraft.exe"), b"test build").unwrap();
    // Create the shortcut before portable mode, exercising the old absolute digest.
    build_desktop_shortcut_at(&paths, "filmcraft", &folder, &desktop).unwrap();
    assert_eq!(std::fs::read_dir(&desktop).unwrap().count(), 1);
    std::fs::write(base.join(crate::portable_mode::MARKER), b"").unwrap();
    crate::portable_mode::initialize(&base).unwrap();
    let moved = temp.join("moved");
    std::fs::rename(&base, &moved).unwrap();
    crate::portable_mode::initialize(&moved).unwrap();
    let paths = Paths::new(moved.join("data"), None);
    remove_build_shortcut_at(
        &paths,
        "filmcraft",
        &paths.at("builds/filmcraft/first"),
        &desktop,
    )
    .unwrap();
    assert_eq!(std::fs::read_dir(&desktop).unwrap().count(), 0);
    std::fs::remove_dir_all(temp).unwrap();
}
#[test]
fn multiple_build_shortcuts_coexist_and_remove_independently() {
    let root = std::env::temp_dir().join(format!(
        "craft-build-shortcuts-many-{}",
        uuid::Uuid::new_v4()
    ));
    let paths = Paths::new(root.join("library"), None);
    let desktop = root.join("desktop");
    let make = |name: &str| {
        let folder = paths.at(format!("builds/filmcraft/{name}"));
        files::write_json(
            &folder.join("build-info.json"),
            &crate::model::BuildInfo {
                app: "filmcraft".into(),
                commit: "a".repeat(40),
                source_branch: "main".into(),
                built_at: "2026-10-12T12:00:00Z".into(),
                profile: "release".into(),
                log: String::new(),
            },
        )
        .unwrap();
        std::fs::write(folder.join("filmcraft.exe"), b"test build").unwrap();
        build_desktop_shortcut_at(&paths, "filmcraft", &folder, &desktop).unwrap();
        let (marker, title) = build_shortcut_identity(&paths, "filmcraft", &folder).unwrap();
        (folder, marker, desktop.join(format!("{title}.lnk")))
    };
    // Even the same commit and timestamp need separate shortcuts.
    let first = make("first");
    let second = make("second");
    assert_ne!(first.2, second.2);
    assert!(first.2.is_file() && second.2.is_file());
    assert_eq!(
        PathBuf::from(crate::platform::shortcut_target(&first.2).unwrap())
            .canonicalize()
            .unwrap(),
        first.0.join("filmcraft.exe").canonicalize().unwrap()
    );
    assert_eq!(
        PathBuf::from(crate::platform::shortcut_target(&second.2).unwrap())
            .canonicalize()
            .unwrap(),
        second.0.join("filmcraft.exe").canonicalize().unwrap()
    );
    build_desktop_shortcut_at(&paths, "filmcraft", &first.0, &desktop).unwrap();
    assert_eq!(std::fs::read_dir(&desktop).unwrap().count(), 2);
    remove_build_shortcut_at(&paths, "filmcraft", &first.0, &desktop).unwrap();
    assert!(!first.2.exists() && !first.1.exists());
    assert!(second.2.exists() && second.1.exists());
    std::fs::write(&second.2, b"User edited shortcut").unwrap();
    remove_build_shortcut_at(&paths, "filmcraft", &second.0, &desktop).unwrap();
    assert_eq!(std::fs::read(&second.2).unwrap(), b"User edited shortcut");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn removing_build_shortcuts_preserves_other_builds_and_user_edits() {
    for (legacy, other_build, edited) in [
        (false, false, false),
        (true, false, false),
        (false, true, false),
        (true, true, false),
        (false, false, true),
    ] {
        let root =
            std::env::temp_dir().join(format!("craft-build-shortcut-{}", uuid::Uuid::new_v4()));
        let paths = Paths::new(root.join("library"), None);
        let desktop = root.join("desktop");
        let selected = root.join("selected");
        let other = root.join("other");
        std::fs::create_dir_all(&selected).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        for folder in [&selected, &other] {
            std::fs::write(folder.join("filmcraft.exe"), b"test build").unwrap();
        }
        let marker = paths.at("runtime/build-shortcuts/filmcraft.json");
        let target = if other_build { &other } else { &selected };
        create_named_shortcut(
            &marker,
            Ownership {
                app: "filmcraft".into(),
                library: std::path::absolute(&paths.root).unwrap(),
                shortcut: None,
            },
            target,
            &desktop,
            "FilmCraft Build",
        )
        .unwrap();
        if !legacy {
            let owner = files::read_json(&marker).unwrap();
            files::write_json(
                &marker,
                &BuildShortcutOwnership {
                    owner,
                    build_folder: Some(target.clone()),
                    shortcut_title: None,
                },
            )
            .unwrap();
        }
        let shortcut = desktop.join("FilmCraft Build.lnk");
        if edited {
            std::fs::write(&shortcut, b"user changed shortcut").unwrap();
        }
        let original = std::fs::read(&shortcut).unwrap();
        remove_build_shortcut_at(&paths, "filmcraft", &selected, &desktop).unwrap();
        if other_build || edited {
            assert_eq!(std::fs::read(&shortcut).unwrap(), original);
        } else {
            assert!(!shortcut.exists());
        }
        if other_build {
            assert!(marker.exists());
        }
        assert!(other.join("filmcraft.exe").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
