use super::*;

#[test]
fn build_selection_survives_restart_and_keeps_latest_independent() {
    let root = std::env::temp_dir().join(format!("craft-build-choice-{}", uuid::Uuid::new_v4()));
    let paths = Paths::new(root.clone(), None);
    let make = |name: &str, time: &str| {
        let folder = paths.at(format!("builds/filmcraft/{name}"));
        files::write_json(
            &folder.join("build-info.json"),
            &BuildInfo {
                app: "filmcraft".into(),
                commit: "a".repeat(40),
                source_branch: "main".into(),
                built_at: time.into(),
                profile: "release".into(),
                log: String::new(),
            },
        )
        .unwrap();
        fs::write(
            folder.join(crate::model::build_executable_name("filmcraft")),
            b"test executable",
        )
        .unwrap();
        folder
    };
    let older = make("older", "2026-10-10T12:00:00Z");
    let newer = make("newer", "2026-10-11T12:00:00Z");
    assert_eq!(
        selected_build(&paths, "filmcraft").unwrap(),
        Some(newer.clone())
    );
    save_build_options(
        &paths,
        "filmcraft",
        &BuildLaunchOptions {
            launch: crate::apps::LaunchSettings {
                executable: String::new(),
                arguments: vec!["--test".into()],
            },
            selected_build: Some(older.clone()),
        },
    )
    .unwrap();
    let restarted = Paths::new(root.clone(), None);
    assert_eq!(
        selected_build(&restarted, "filmcraft").unwrap(),
        Some(older.clone())
    );
    assert_eq!(history(&paths, "filmcraft"), Some(newer.clone()));
    let newest = make("newest", "2026-10-12T12:00:00Z");
    assert_eq!(
        selected_build(&paths, "filmcraft").unwrap(),
        Some(older.clone())
    );
    assert_eq!(completed_builds(&paths, "filmcraft").len(), 3);
    fs::remove_dir_all(&older).unwrap();
    assert_eq!(
        selected_build(&paths, "filmcraft").unwrap(),
        Some(newest.clone())
    );
    assert!(save_build_options(
        &paths,
        "filmcraft",
        &BuildLaunchOptions {
            selected_build: Some(root.join("outside")),
            ..Default::default()
        }
    )
    .is_err());
    save_build_options(&paths, "filmcraft", &BuildLaunchOptions::default()).unwrap();
    assert_eq!(selected_build(&paths, "filmcraft").unwrap(), Some(newest));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn legacy_launch_settings_load_without_a_build_selection() {
    let root = std::env::temp_dir().join(format!("craft-build-legacy-{}", uuid::Uuid::new_v4()));
    let paths = Paths::new(root.clone(), None);
    files::write_json(
        &paths.at("runtime/build-launch-settings.json"),
        &serde_json::json!({"filmcraft":{"executable":"","arguments":["--legacy"]}}),
    )
    .unwrap();
    let options = build_options(&paths, "filmcraft").unwrap();
    assert!(options.selected_build.is_none());
    assert_eq!(options.launch.arguments, ["--legacy"]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cache_availability_follows_each_build_commit() {
    let root = std::env::temp_dir().join(format!("craft-cache-choice-{}", uuid::Uuid::new_v4()));
    let paths = Paths::new(root.clone(), None);
    let make = |name: &str, commit: String| {
        let folder = paths.at(format!("builds/filmcraft/{name}"));
        files::write_json(
            &folder.join("build-info.json"),
            &BuildInfo {
                app: "filmcraft".into(),
                commit,
                source_branch: "main".into(),
                built_at: chrono::Utc::now().to_rfc3339(),
                profile: "release".into(),
                log: String::new(),
            },
        )
        .unwrap();
        folder
    };
    let older = make("older", "a".repeat(40));
    let newer = make("newer", "b".repeat(40));
    let workspace = paths.at("workspace/filmcraft/older");
    fs::create_dir_all(&workspace).unwrap();
    fs::write(workspace.join(".extracted"), "a".repeat(40)).unwrap();
    assert!(has_local_cache(&paths, "filmcraft", &older).unwrap());
    assert!(!has_local_cache(&paths, "filmcraft", &newer).unwrap());
    fs::remove_dir_all(&workspace).unwrap();
    assert!(!has_local_cache(&paths, "filmcraft", &older).unwrap());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn selected_build_cleanup_keeps_outputs_sources_and_other_workspaces() {
    let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let paths = Paths::new(root.clone(), None);
    let commit = "a".repeat(40);
    let folder = paths.at("builds/filmcraft/selected");
    files::write_json(
        &folder.join("build-info.json"),
        &BuildInfo {
            app: "filmcraft".into(),
            commit: commit.clone(),
            source_branch: "main".into(),
            built_at: chrono::Utc::now().to_rfc3339(),
            profile: "release".into(),
            log: String::new(),
        },
    )
    .unwrap();
    fs::write(
        folder.join(crate::model::build_executable_name("filmcraft")),
        b"compiled",
    )
    .unwrap();
    for name in [
        "workspace/filmcraft/selected",
        "workspace/filmcraft/other",
        "workspace/cache/filmcraft",
        "workspace/cache/soundcraft",
        "sources",
        "builds/filmcraft/other",
    ] {
        fs::create_dir_all(paths.at(name)).unwrap();
        fs::write(paths.at(format!("{name}/keep.txt")), b"data").unwrap();
    }
    fs::write(paths.at("workspace/filmcraft/selected/.extracted"), &commit).unwrap();
    fs::write(
        paths.at("workspace/filmcraft/other/.extracted"),
        "b".repeat(40),
    )
    .unwrap();
    assert!(clean_local(&paths, "soundcraft", &folder).is_err());
    assert!(paths.at("workspace/cache/filmcraft").exists());
    assert!(has_local_cache(&paths, "filmcraft", &folder).unwrap());
    clean_local(&paths, "filmcraft", &folder).unwrap();
    assert!(!has_local_cache(&paths, "filmcraft", &folder).unwrap());
    assert!(!paths.at("workspace/cache/filmcraft").exists());
    assert!(!paths.at("workspace/filmcraft/selected").exists());
    for name in [
        "workspace/filmcraft/other",
        "workspace/cache/soundcraft",
        "sources",
        "builds/filmcraft/other",
    ] {
        assert!(paths.at(format!("{name}/keep.txt")).is_file());
    }
    assert!(folder
        .join(crate::model::build_executable_name("filmcraft"))
        .is_file());
    clean_local(&paths, "filmcraft", &folder).unwrap();
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn interrupted_build_cache_cleanup_is_locked_and_sources_can_be_extracted_again() {
    use std::io::Write;

    let root =
        std::env::temp_dir().join(format!("craft-interrupted-cache-{}", uuid::Uuid::new_v4()));
    let paths = Paths::new(root.clone(), None);
    let commit = "a".repeat(40);
    let work = paths.at(format!("workspace/filmcraft/{}", &commit[..12]));
    let cache = paths.at("workspace/cache/filmcraft");
    let archive = paths.at("sources/filmcraft-source.zip");
    fs::create_dir_all(archive.parent().unwrap()).unwrap();
    let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
    zip.start_file(
        format!("filmcraft-{commit}/Cargo.toml"),
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(b"[package]\nname = 'filmcraft'\nversion = '0.1.0'\n")
        .unwrap();
    zip.finish().unwrap();
    let hash = files::hash(&archive).unwrap();
    let job = Job::new(
        paths.at("logs/test.log"),
        &crate::model::BuilderPreferences::default(),
    );
    let extract = || {
        files::verify_source(&archive, "filmcraft", &commit).unwrap();
        files::extract_zip(&archive, &work, &job).unwrap();
        fs::rename(work.join(format!("filmcraft-{commit}")), work.join("src")).unwrap();
        fs::write(work.join(".extracted"), &commit).unwrap();
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("intermediate"), b"partial compilation").unwrap();
    };
    extract();
    for name in [
        "workspace/cache/soundcraft",
        "workspace/filmcraft/user-files",
        "builds/filmcraft/completed",
    ] {
        fs::create_dir_all(paths.at(name)).unwrap();
        fs::write(paths.at(format!("{name}/keep.txt")), b"keep").unwrap();
    }
    // No completed build record is required to discover these intermediates.
    assert!(completed_builds(&paths, "filmcraft").is_empty());
    assert!(has_app_cache(&paths, "filmcraft").unwrap());
    {
        let _active_builder = platform::Lock::take(r"Local\CraftAppsSourceBuilder").unwrap();
        // Windows mutexes are reentrant on their owner thread: simulate a
        // separate cleanup worker, as used by both windows.
        let cleanup_paths = paths.clone();
        assert!(
            std::thread::spawn(move || clean_app_cache(&cleanup_paths, "filmcraft"))
                .join()
                .unwrap()
                .is_err()
        );
        assert!(cache.join("intermediate").is_file());
        assert!(work.join("src/Cargo.toml").is_file());
    }
    clean_app_cache(&paths, "filmcraft").unwrap();
    assert!(!has_app_cache(&paths, "filmcraft").unwrap());
    assert!(!work.exists());
    assert_eq!(files::hash(&archive).unwrap(), hash);
    for name in [
        "workspace/cache/soundcraft",
        "workspace/filmcraft/user-files",
        "builds/filmcraft/completed",
    ] {
        assert!(paths.at(format!("{name}/keep.txt")).is_file());
    }
    // Exercise the same extraction and compiler-cache creation used by
    // build_owned; a fresh build can recreate both after cleanup.
    extract();
    assert!(work.join("src/Cargo.toml").is_file());
    assert!(cache.join("intermediate").is_file());
    clean_app_cache(&paths, "filmcraft").unwrap();
    clean_app_cache(&paths, "filmcraft").unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn build_launch_options_are_persisted_separately_from_release_options() {
    let temp = std::env::temp_dir().join(format!("craft-build-options-{}", uuid::Uuid::new_v4()));
    let paths = Paths::new(temp.clone(), None);
    let release = crate::apps::LaunchSettings {
        executable: String::new(),
        arguments: vec!["--release".into()],
    };
    crate::apps::save(&paths, "filmcraft", &release).unwrap();
    let build = crate::apps::LaunchSettings {
        executable: String::new(),
        arguments: vec!["--build".into(), "a value with spaces".into()],
    };
    save_launch_options(&paths, "filmcraft", &build).unwrap();
    assert_eq!(
        launch_options(&paths, "filmcraft").unwrap().arguments,
        build.arguments
    );
    assert_eq!(
        crate::apps::settings(&paths, "filmcraft")
            .unwrap()
            .arguments,
        release.arguments
    );
    assert!(launch_options(&paths, "soundcraft")
        .unwrap()
        .arguments
        .is_empty());
    let invalid = crate::apps::LaunchSettings {
        executable: "../other.exe".into(),
        arguments: vec![],
    };
    assert!(save_launch_options(&paths, "filmcraft", &invalid).is_err());
    fs::remove_dir_all(temp).unwrap();
}
#[test]
fn local_build_actions_reject_missing_and_unrelated_outputs() {
    let temp = std::env::temp_dir().join(format!("craft-build-actions-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&temp).unwrap();
    let paths = Paths::new(temp.join("library"), None);
    let outside = temp.join("unrelated");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("keep.txt"), "keep").unwrap();
    assert!(launch_local(&paths, "filmcraft").is_err());
    assert!(delete_local(&paths, "filmcraft", &outside).is_err());
    let wrong = paths.at("builds/filmcraft/wrong");
    fs::create_dir_all(&wrong).unwrap();
    files::write_json(
        &wrong.join("build-info.json"),
        &BuildInfo {
            app: "soundcraft".into(),
            commit: "abcdef".into(),
            source_branch: "main".into(),
            built_at: chrono::Utc::now().to_rfc3339(),
            profile: "release".into(),
            log: String::new(),
        },
    )
    .unwrap();
    assert!(delete_local(&paths, "filmcraft", &wrong).is_err());
    assert!(wrong.exists());
    assert!(outside.join("keep.txt").exists());
    fs::remove_dir_all(temp).unwrap();
}
