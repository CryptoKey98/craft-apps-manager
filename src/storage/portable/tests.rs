#[test]
fn shared_parent_attaches_legacy_paths_without_creating_or_rewriting_metadata() {
    let base = std::env::temp_dir().join(format!("craft-shared-readonly-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(base.join("data")).unwrap();
    std::fs::write(base.join(super::MARKER), b"legacy marker").unwrap();
    let original = br#"{"root":"@portable/data"}"#;
    std::fs::write(base.join("data-root.json"), original).unwrap();
    super::initialize_read_only(&base).unwrap();
    assert!(
        !base.join(super::ORIGIN).exists(),
        "Shared startup must not create portable state"
    );
    assert_eq!(
        std::fs::read(base.join("data-root.json")).unwrap(),
        original
    );
    assert_eq!(
        std::fs::read(base.join(super::MARKER)).unwrap(),
        b"legacy marker"
    );
    let location: serde_json::Value =
        crate::files::read_json(&base.join("data-root.json")).unwrap();
    assert_eq!(
        std::path::Path::new(location["root"].as_str().unwrap()),
        base.join("data")
    );
    std::fs::remove_dir_all(base).unwrap();
}
#[test]
#[cfg(target_os = "macos")]
fn shared_app_parent_does_not_allow_adjacent_portable_metadata() {
    assert!(!super::adjacent_metadata_writable(std::path::Path::new("/Applications")).unwrap());
}
use super::*;
#[test]
fn portable_adoption_ignores_unrelated_link_but_rejects_linked_metadata() {
    let temp = std::env::temp_dir().join(format!("craft-adoption-links-{}", uuid::Uuid::new_v4()));
    let base = temp.join("Applications");
    let other = temp.join("unrelated");
    fs::create_dir_all(&base).unwrap();
    fs::create_dir_all(&other).unwrap();
    let link = base.join("Unrelated-Test-Link");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&other, &link).unwrap();
    #[cfg(windows)]
    assert!(std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&link)
        .arg(&other)
        .output()
        .unwrap()
        .status
        .success());
    // Reproduce the old whole-folder rejection before testing scoped adoption.
    assert!(crate::files::no_links(&base).is_err());
    adopt_updated_portable(&base, &temp.join("library"), None).unwrap();
    assert!(local_storage(&base));
    assert!(link.exists());
    fs::remove_file(base.join(MARKER)).unwrap();
    fs::remove_file(base.join("data-root.json")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&other, base.join("data-root.json")).unwrap();
    #[cfg(windows)]
    assert!(std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(base.join("data-root.json"))
        .arg(&other)
        .output()
        .unwrap()
        .status
        .success());
    assert!(adopt_updated_portable(&base, &temp.join("library"), None).is_err());
    assert!(!local_storage(&base));
    #[cfg(unix)]
    fs::remove_file(base.join("data-root.json")).unwrap();
    #[cfg(windows)]
    fs::remove_dir(base.join("data-root.json")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&other, base.join(MARKER)).unwrap();
    #[cfg(windows)]
    assert!(std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(base.join(MARKER))
        .arg(&other)
        .output()
        .unwrap()
        .status
        .success());
    assert!(adopt_updated_portable(&base, &temp.join("library"), None).is_err());
    // The containing folder itself must also remain unlinked.
    assert!(adopt_updated_portable(&link, &temp.join("library"), None).is_err());
    assert!(fs::read_dir(&other).unwrap().next().is_none());
    fs::remove_dir_all(temp).unwrap();
}

#[test]
fn old_portable_update_preserves_library_and_migrates_on_restart() {
    let temp = std::env::temp_dir().join(format!("craft-old-update-{}", uuid::Uuid::new_v4()));
    let base = temp.join("manager");
    let library = base.join("legacy-library");
    let tools = base.join("workspace").join("tools");
    fs::create_dir_all(&library).unwrap();
    let settings = library.join("settings.json");
    fs::write(
        &settings,
        serde_json::to_vec(&serde_json::json!({"appsRoot":library,"apps":[],"installations":[]}))
            .unwrap(),
    )
    .unwrap();
    let old = fs::read(&settings).unwrap();
    adopt_updated_portable(&base, &library, Some(&tools)).unwrap();
    assert_eq!(fs::read(&settings).unwrap(), old);
    initialize(&base).unwrap();
    let moved = temp.join("moved-manager");
    fs::rename(&base, &moved).unwrap();
    initialize(&moved).unwrap();
    let locations: Value = crate::files::read_json(&moved.join("data-root.json")).unwrap();
    assert_eq!(
        locations["root"],
        moved.join("legacy-library").display().to_string()
    );
    assert_eq!(
        locations["tools"],
        moved.join("workspace").join("tools").display().to_string()
    );
    let config: crate::model::Config =
        crate::files::read_json(&moved.join("legacy-library/settings.json")).unwrap();
    assert_eq!(
        config.apps_root,
        moved.join("legacy-library").display().to_string()
    );
    // Existing marker/location choices are never overwritten by another update.
    let old_locations = fs::read(moved.join("data-root.json")).unwrap();
    adopt_updated_portable(&moved, &temp.join("another-library"), None).unwrap();
    assert_eq!(
        fs::read(moved.join("data-root.json")).unwrap(),
        old_locations
    );
    fs::remove_dir_all(temp).unwrap();
}

#[test]
fn old_portable_update_keeps_external_library_absolute() {
    let temp = std::env::temp_dir().join(format!("craft-external-update-{}", uuid::Uuid::new_v4()));
    let base = temp.join("manager");
    let library = temp.join("external-library");
    fs::create_dir_all(&base).unwrap();
    fs::create_dir_all(&library).unwrap();
    let settings = library.join("settings.json");
    fs::write(&settings, b"external settings are never rewritten").unwrap();
    adopt_updated_portable(&base, &library, None).unwrap();
    initialize(&base).unwrap();
    let moved = temp.join("moved");
    fs::rename(&base, &moved).unwrap();
    initialize(&moved).unwrap();
    let locations: Value = crate::files::read_json(&moved.join("data-root.json")).unwrap();
    assert_eq!(locations["root"], library.display().to_string());
    assert_eq!(
        fs::read(&settings).unwrap(),
        b"external settings are never rewritten"
    );
    fs::remove_dir_all(temp).unwrap();
}

#[test]
fn portable_worker_attachment_decodes_without_migrating_or_writing() {
    let temp = std::env::temp_dir().join(format!("craft-worker-storage-{}", uuid::Uuid::new_v4()));
    let library = temp.join("custom").join("library");
    fs::create_dir_all(&library).unwrap();
    fs::write(temp.join(MARKER), b"").unwrap();
    fs::write(
        temp.join("data-root.json"),
        serde_json::to_vec(&serde_json::json!({"root":library})).unwrap(),
    )
    .unwrap();
    initialize(&temp).unwrap();
    let settings = library.join("settings.json");
    crate::files::write_json(
        &settings,
        &serde_json::json!({"appsRoot":library,"apps":[],"installations":[]}),
    )
    .unwrap();
    let before = fs::read(&settings).unwrap();
    let origin = fs::read(temp.join(ORIGIN)).unwrap();
    assert_eq!(base_for_library(&library), Some(temp.clone()));
    STORAGE
        .get()
        .unwrap()
        .write()
        .unwrap()
        .retain(|store| store.base != temp);
    let undecoded: crate::model::Config = crate::files::read_json(&settings).unwrap();
    assert!(undecoded.apps_root.starts_with(PREFIX));
    attach_existing(&temp, &library).unwrap();
    let decoded: crate::model::Config = crate::files::read_json(&settings).unwrap();
    assert_eq!(decoded.apps_root, library.display().to_string());
    assert_eq!(fs::read(&settings).unwrap(), before);
    assert_eq!(fs::read(temp.join(ORIGIN)).unwrap(), origin);
    assert!(attach_existing(&temp, &temp.parent().unwrap().join("external")).is_err());
    fs::remove_dir_all(temp).unwrap();
}

#[test]
fn disabled_mode_retains_external_ownership_and_restores_tagged_backups() {
    let temp = std::env::temp_dir().join(format!("craft-fixed-compat-{}", uuid::Uuid::new_v4()));
    let first = temp.join("first");
    fs::create_dir_all(first.join("data")).unwrap();
    fs::write(first.join(MARKER), b"").unwrap();
    initialize(&first).unwrap();
    let external = temp.join("external/filmcraft");
    fs::create_dir_all(&external).unwrap();
    fs::write(
        external.join(".craft-manager-install.json"),
        serde_json::to_vec(&serde_json::json!({"app":"filmcraft","library":first.join("data")}))
            .unwrap(),
    )
    .unwrap();
    let moved = temp.join("moved");
    fs::rename(&first, &moved).unwrap();
    initialize(&moved).unwrap();
    let library = moved.join("data");
    let paths = crate::model::Paths::new(library.clone(), None);
    crate::files::write_json(
        &paths.at("settings.json"),
        &crate::model::Config {
            portable_label_counters: Default::default(),
            apps_root: library.display().to_string(),
            apps: Vec::new(),
            installations: Vec::new(),
        },
    )
    .unwrap();
    crate::portable::validate(&paths, "filmcraft", &external).unwrap();
    deactivate(&moved, false).unwrap();
    initialize(&moved).unwrap();
    crate::portable::validate(&paths, "filmcraft", &external).unwrap();
    assert!(!same_library(&temp.join("unrelated/data"), &library));
    let stage = paths.at("runtime/downloads/restore/contents/filmcraft");
    fs::create_dir_all(&stage).unwrap();
    // An archived ownership marker must remain decodable after Disable and restart.
    fs::write(
        stage.join(".craft-manager-install.json"),
        br#"{"app":"filmcraft","library":"@portable/data"}"#,
    )
    .unwrap();
    crate::portable::mark_restored(&paths, "filmcraft", &stage).unwrap();
    let raw: Value =
        serde_json::from_slice(&fs::read(stage.join(".craft-manager-install.json")).unwrap())
            .unwrap();
    assert_eq!(Path::new(raw["library"].as_str().unwrap()), library);
    fs::remove_dir_all(temp).unwrap();
}

#[test]
fn deeply_nested_library_under_excluded_name_migrates_completely() {
    let temp = std::env::temp_dir().join(format!("craft-deep-library-{}", uuid::Uuid::new_v4()));
    let first = temp.join("first");
    let relative = Path::new("workspace/one/two/three/four/five/six/seven/eight/library");
    let library = first.join(relative);
    fs::create_dir_all(&library).unwrap();
    fs::write(first.join(MARKER), b"").unwrap();
    fs::write(
        first.join("data-root.json"),
        serde_json::to_vec(&serde_json::json!({"root":library})).unwrap(),
    )
    .unwrap();
    crate::files::write_json(
        &library.join("settings.json"),
        &serde_json::json!({"appsRoot":library,"apps":[],"installations":[]}),
    )
    .unwrap();
    let chosen = library.join("builds/filmcraft/selected");
    crate::files::write_json(
        &library.join("runtime/build-launch-settings.json"),
        &serde_json::json!({"filmcraft":{"selectedBuild":chosen}}),
    )
    .unwrap();
    let unrelated = first.join("workspace/tools/settings.json");
    fs::create_dir_all(unrelated.parent().unwrap()).unwrap();
    fs::write(&unrelated, b"unrelated application settings").unwrap();
    initialize(&first).unwrap();
    let raw: Value =
        serde_json::from_slice(&fs::read(library.join("settings.json")).unwrap()).unwrap();
    assert!(raw["appsRoot"].as_str().unwrap().starts_with(PREFIX));
    let moved = temp.join("moved");
    fs::rename(&first, &moved).unwrap();
    initialize(&moved).unwrap();
    let library = moved.join(relative);
    let config: Value = crate::files::read_json(&library.join("settings.json")).unwrap();
    assert_eq!(Path::new(config["appsRoot"].as_str().unwrap()), library);
    deactivate(&moved, false).unwrap();
    initialize(&moved).unwrap();
    let raw: Value = serde_json::from_slice(
        &fs::read(library.join("runtime/build-launch-settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        Path::new(raw["filmcraft"]["selectedBuild"].as_str().unwrap()),
        library.join("builds/filmcraft/selected")
    );
    assert_eq!(
        fs::read(moved.join("workspace/tools/settings.json")).unwrap(),
        b"unrelated application settings"
    );
    fs::remove_dir_all(temp).unwrap();
}
#[test]
fn disabling_after_move_fixes_paths_and_preserves_library() {
    let temp = std::env::temp_dir().join(format!("craft-disable-{}", uuid::Uuid::new_v4()));
    let first = temp.join("first");
    fs::create_dir_all(first.join("data")).unwrap();
    fs::write(first.join(MARKER), b"").unwrap();
    fs::write(
        first.join("data-root.json"),
        serde_json::to_vec(&serde_json::json!({"root": first.join("data")})).unwrap(),
    )
    .unwrap();
    initialize(&first).unwrap();
    let moved = temp.join("moved");
    fs::rename(&first, &moved).unwrap();
    initialize(&moved).unwrap();
    assert!(deactivate(&moved, true).is_err());
    assert!(enabled(&moved));
    deactivate(&moved, false).unwrap();
    assert!(!enabled(&moved));
    assert!(local_storage(&moved));
    assert!(initialized(&moved)); // Compatibility decoding survives disabling.
    let value: Value = crate::files::read_or_default(&moved.join("data-root.json")).unwrap();
    assert_eq!(
        Path::new(value["root"].as_str().unwrap()),
        moved.join("data")
    );
    assert!(moved.join("data").is_dir());
    initialize(&moved).unwrap();
    let next = temp.join("next");
    fs::rename(&moved, &next).unwrap();
    initialize(&next).unwrap();
    let value: Value = crate::files::read_or_default(&next.join("data-root.json")).unwrap();
    assert_eq!(
        Path::new(value["root"].as_str().unwrap()),
        moved.join("data")
    );
    activate(&next, &next.join("data"), false).unwrap();
    initialize(&next).unwrap();
    let value: Value = crate::files::read_or_default(&next.join("data-root.json")).unwrap();
    assert_eq!(
        Path::new(value["root"].as_str().unwrap()),
        next.join("data")
    );
    fs::remove_dir_all(temp).unwrap();
}
#[test]
fn builds_operations_and_transactional_settings_survive_repeated_moves() {
    let temp = std::env::temp_dir().join(format!("craft-portable-state-{}", uuid::Uuid::new_v4()));
    let first = temp.join("first");
    fs::create_dir_all(&first).unwrap();
    fs::write(first.join(MARKER), b"").unwrap();
    initialize(&first).unwrap();
    let library = first.join("data");
    let paths = crate::model::Paths::new(library.clone(), None);
    let output = paths.at("builds/filmcraft/selected");
    fs::create_dir_all(&output).unwrap();
    fs::write(
        output.join(crate::model::build_executable_name("filmcraft")),
        b"test build",
    )
    .unwrap();
    crate::files::write_json(
        &output.join("build-info.json"),
        &crate::model::BuildInfo {
            app: "filmcraft".into(),
            commit: "a".repeat(40),
            source_branch: "main".into(),
            built_at: "2026-10-12T12:00:00Z".into(),
            profile: "release".into(),
            log: paths.at("logs/build.log").display().to_string(),
        },
    )
    .unwrap();
    crate::builder::save_build_options(
        &paths,
        "filmcraft",
        &crate::builder::BuildLaunchOptions {
            selected_build: Some(output.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    let id_before = identity(&output).unwrap();
    let operation = crate::operations::Manager::new(&library).register("Build", "filmcraft", None);
    operation.report(
        "Log file",
        &paths.at("logs/build.log").display().to_string(),
    );
    operation.finish(
        crate::operations::Lifecycle::Completed,
        "Build completed".into(),
    );
    crate::settings::save(
        &paths,
        &first.join("data-root.json"),
        &serde_json::json!({"root":library,"tools":paths.tools}),
        &paths.root,
        &paths.tools,
        None,
        None,
    )
    .unwrap();
    let raw: Value =
        serde_json::from_slice(&fs::read(first.join("data-root.json")).unwrap()).unwrap();
    assert_eq!(raw["root"], "@portable/data");
    let second = temp.join("second");
    fs::rename(&first, &second).unwrap();
    initialize(&second).unwrap();
    let third = temp.join("third");
    fs::rename(&second, &third).unwrap();
    initialize(&third).unwrap();
    let moved = crate::model::Paths::new(third.join("data"), None);
    let selected = crate::builder::selected_build(&moved, "filmcraft")
        .unwrap()
        .unwrap();
    assert_eq!(selected, moved.at("builds/filmcraft/selected"));
    assert_eq!(identity(&selected).unwrap(), id_before);
    let info: crate::model::BuildInfo =
        crate::files::read_json(&selected.join("build-info.json")).unwrap();
    assert_eq!(Path::new(&info.log), moved.at("logs/build.log"));
    let records = crate::operations::Manager::new(&moved.root).list();
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].lifecycle,
        crate::operations::Lifecycle::Completed
    );
    assert_eq!(records[0].facts["Log file"], info.log);
    assert!(same_library(&library, &moved.root));
    assert!(!same_library(&temp.join("unrelated"), &moved.root));
    assert_eq!(
        resolve_folder(&third, Path::new("./data")).unwrap(),
        moved.root
    );
    assert_eq!(
        resolve_folder(&third, Path::new("../external")).unwrap(),
        temp.join("external")
    );
    fs::remove_dir_all(temp).unwrap();
}

#[test]
fn failed_migration_preserves_originals_and_does_not_parse_application_data() {
    let temp =
        std::env::temp_dir().join(format!("craft-portable-failure-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(temp.join("data/releases/filmcraft")).unwrap();
    fs::write(temp.join(MARKER), b"").unwrap();
    fs::write(
        temp.join("data/releases/filmcraft/settings.json"),
        b"not manager JSON",
    )
    .unwrap();
    crate::files::write_json(
        &temp.join("settings.json"),
        &serde_json::json!({"appsRoot":temp,"apps":[],"installations":[]}),
    )
    .unwrap();
    let original = fs::read(temp.join("settings.json")).unwrap();
    fs::create_dir_all(temp.join("runtime")).unwrap();
    fs::write(
        temp.join("runtime/launch-settings.json"),
        b"corrupt manager JSON",
    )
    .unwrap();
    assert!(initialize(&temp).is_err());
    assert_eq!(fs::read(temp.join("settings.json")).unwrap(), original);
    assert!(!temp.join(ORIGIN).exists());
    fs::remove_file(temp.join("runtime/launch-settings.json")).unwrap();
    initialize(&temp).unwrap();
    assert_eq!(
        fs::read(temp.join("data/releases/filmcraft/settings.json")).unwrap(),
        b"not manager JSON"
    );
    fs::remove_dir_all(temp).unwrap();
}
#[test]
fn portable_paths_survive_moves_without_touching_external_installs_or_arguments() {
    let temp = std::env::temp_dir().join(format!("craft-portable-{}", uuid::Uuid::new_v4()));
    let old = temp.join("original");
    fs::create_dir_all(old.join("data/releases/filmcraft")).unwrap();
    fs::write(old.join(MARKER), b"").unwrap();
    let config = serde_json::json!({"appsRoot": old.join("data"), "apps": [
            {"name":"filmcraft", "path":old.join("data/releases/filmcraft"), "install_kind":"portable"},
            {"name":"designcraft", "path":temp.join("external"), "install_kind":"installer"}
        ], "installations": []});
    crate::files::write_json(&old.join("data/settings.json"), &config).unwrap();
    let settings = serde_json::json!({format!("filmcraft|{}",old.join("data/releases/filmcraft").display()): {
        "executable":old.join("data/releases/filmcraft/filmcraft.exe"), "arguments":[old.display().to_string()]
    }});
    crate::files::write_json(&old.join("data/runtime/launch-settings.json"), &settings).unwrap();
    let plan = serde_json::json!({"root":old, "target":old.join("data/releases/filmcraft")});
    crate::files::write_json(&old.join("runtime/self-update/plan.json"), &plan).unwrap();
    initialize(&old).unwrap();
    let raw: Value =
        serde_json::from_slice(&fs::read(old.join("data/settings.json")).unwrap()).unwrap();
    assert_eq!(raw["appsRoot"], "@portable/data");
    assert!(old.join("runtime/portable-migration").is_dir());
    let new = temp.join("moved folder");
    fs::rename(&old, &new).unwrap();
    initialize(&new).unwrap();
    let config: Value = crate::files::read_json(&new.join("data/settings.json")).unwrap();
    assert_eq!(
        Path::new(config["apps"][0]["path"].as_str().unwrap()),
        new.join("data/releases/filmcraft")
    );
    assert_eq!(
        config["apps"][1]["path"],
        temp.join("external").display().to_string()
    );
    let settings: Value =
        crate::files::read_json(&new.join("data/runtime/launch-settings.json")).unwrap();
    let settings = &settings[format!("filmcraft|{}", config["apps"][0]["path"].as_str().unwrap())];
    assert_eq!(settings["arguments"][0], old.display().to_string());
    assert_eq!(
        Path::new(settings["executable"].as_str().unwrap()),
        new.join("data/releases/filmcraft/filmcraft.exe")
    );
    assert_eq!(
        crate::files::read_json::<Value>(&new.join("runtime/self-update/plan.json")).unwrap(),
        plan
    );
    fs::remove_dir_all(temp).unwrap();
}
#[test]
fn tagged_paths_reject_traversal_and_mac_data_stays_outside_bundle() {
    let base = PathBuf::from(if cfg!(windows) {
        "C:\\portable"
    } else {
        "/portable"
    });
    let storage = Storage {
        base: base.clone(),
        previous: base.clone(),
        aliases: Vec::new(),
        fixed_base: None,
        libraries: vec![base.clone()],
    };
    for path in ["@portable/../external", "@portable//absolute"] {
        assert!(path_value(&mut Value::String(path.into()), &storage, false).is_err());
    }
    assert_eq!(
        executable_folder(&base.join("Manager.app/Contents/MacOS/manager")).unwrap(),
        base
    );
}
