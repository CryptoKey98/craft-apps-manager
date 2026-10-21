#[test]
fn discovery_rejects_non_packages_unrelated_and_ambiguous_assets() {
    for filename in [
        "mapcraft-1.0.0-windows-x64-portable.zip.sha256",
        "mapcraft-1.0.0-windows-x64.pdb",
        "mapcraft-1.0.0-linux-x64.msi",
        "mapcraft-1.0.0-windows-nonsense-portable.zip",
    ] {
        assert!(parse_asset(filename).is_none(), "{filename}");
        assert!(!verified_discovery(&classify(
            "mapcraft",
            Some(&release("v1.0.0", "", &[filename]))
        )));
    }
    assert!(!verified_discovery(&classify("mapcraft", None)));
    let wrong = release("v1.0.0", "", &["othercraft-1.0.0-windows-x64-portable.zip"]);
    assert!(!verified_discovery(&classify("mapcraft", Some(&wrong))));
    let ambiguous = release(
        "v1.0.0",
        "",
        &[
            "mapcraft-1.0.0-windows-x64-portable.zip",
            "othercraft-1.0.0-windows-x64-portable.zip",
        ],
    );
    assert!(!verified_discovery(&classify("mapcraft", Some(&ambiguous))));
    let valid = release(
        "v1.0.0",
        "",
        &[
            "mapcraft-1.0.0-windows-x64-portable.zip",
            "mapcraft-1.0.0-linux-x86_64.AppImage",
            "mapcraft-1.0.0-macos-universal.dmg",
        ],
    );
    assert!(verified_discovery(&classify("mapcraft", Some(&valid))));
}

#[test]
fn refreshing_discovery_preserves_missing_apps_and_stable_identity() {
    let missing = craft("oldcraft", "OldCraft", "", "");
    let mut renamed = craft("savedkey", "MapCraft", "", "");
    renamed.repository = "mapcraft".into();
    let mut entries = vec![missing.clone(), renamed];
    retain_discovery(&mut entries, craft("mapcraft", "Updated title", "", ""));
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0], missing);
    assert_eq!(entries[1].key, "savedkey");
    assert_eq!(entries[1].title, "Updated title");
    retain_discovery(&mut entries, craft("newcraft", "NewCraft", "", ""));
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0], missing);
}

#[test]
fn discovered_subtitles_use_publisher_descriptions() {
    assert_eq!(
        repository_subtitle(
            "FigCraft",
            "FigCraft: interface design and prototyping; an open-source app in pure Rust"
        ),
        "interface design and prototyping"
    );
    assert_eq!(repository_subtitle("Unknown", "  "), "Desktop application");
    let long = repository_subtitle("App", &"界".repeat(100));
    assert!(long.chars().count() <= 72);
}

#[test]
fn repository_creation_date_survives_cache_and_metadata_refresh() {
    let mut entry = craft("figcraft", "FigCraft", "", "");
    let mut repository: Repository = serde_json::from_value(serde_json::json!({
        "name": "figcraft", "created_at": "2026-10-01T12:00:00Z"
    }))
    .unwrap();
    apply_repository_metadata(&mut entry, &repository);
    let expected = chrono::DateTime::parse_from_rfc3339("2026-10-01T12:00:00Z")
        .unwrap()
        .timestamp();
    assert_eq!(entry.created_at, Some(expected));
    let mut cached: Entry = serde_json::from_slice(&serde_json::to_vec(&entry).unwrap()).unwrap();
    repository.created_at = Some("invalid".into());
    apply_repository_metadata(&mut cached, &repository);
    assert_eq!(cached.created_at, Some(expected));
    repository.created_at = None;
    apply_repository_metadata(&mut cached, &repository);
    assert_eq!(cached.created_at, Some(expected));
    let mut legacy = serde_json::to_value(entry).unwrap();
    legacy.as_object_mut().unwrap().remove("createdAt");
    assert!(serde_json::from_value::<Entry>(legacy)
        .unwrap()
        .created_at
        .is_none());
}

#[test]
fn cached_discovery_refreshes_metadata_without_reclassifying() {
    let mut entry = craft("figcraft", "FigCraft", "", "Old description");
    let scheme = entry.scheme;
    let repository = Repository {
        created_at: None,
        name: "figcraft".into(),
        topics: Vec::new(),
        description: Some("FigCraft: interface design and prototyping; open source".into()),
        archived: false,
        fork: false,
        disabled: false,
    };
    apply_repository_metadata(&mut entry, &repository);
    assert_eq!(entry.category, "interface design and prototyping");
    assert_eq!(entry.scheme, scheme);
    assert_eq!(entry.key, "figcraft");
    let loaded = merge(vec![entry], &BTreeMap::new(), &[]);
    assert_eq!(
        loaded
            .iter()
            .find(|e| e.key == "figcraft")
            .unwrap()
            .category,
        "interface design and prototyping"
    );
}

#[test]
fn old_catalogs_gain_subtitles_and_curated_labels_are_preserved() {
    let discovered = craft(
        "figcraft",
        "FigCraft",
        "",
        "FigCraft: interface design and prototyping; open source",
    );
    let entries = merge(vec![discovered], &BTreeMap::new(), &[]);
    assert_eq!(
        entries
            .iter()
            .find(|e| e.key == "figcraft")
            .unwrap()
            .category,
        "interface design and prototyping"
    );
    assert_eq!(
        entries
            .iter()
            .find(|e| e.key == "soundcraft")
            .unwrap()
            .category,
        "Audio production"
    );
}

#[test]
fn fresh_catalog_uses_cache_without_a_network_request() {
    let root = std::env::temp_dir().join(format!("craft-catalog-cache-{}", uuid::Uuid::new_v4()));
    files::write_json(
        &cache_path(&root),
        &Cache {
            checked_at: chrono::Utc::now().timestamp(),
            entries: vec![craft("mapcraft", "MapCraft", "Maps", "")],
            ..Default::default()
        },
    )
    .unwrap();
    let result = refresh_if_older(&root, REFRESH_INTERVAL).unwrap();
    assert!(result.iter().any(|e| e.key == "mapcraft"));
    // Restore the process-wide catalog for the remaining serial tests.
    load(&root.join("unused"));
    std::fs::remove_dir_all(root).unwrap();
}
use super::*;
use crate::model::Asset;
fn release(tag: &str, name: &str, assets: &[&str]) -> Release {
    Release {
        tag_name: tag.into(),
        name: Some(name.into()),
        draft: false,
        prerelease: false,
        assets: assets
            .iter()
            .map(|a| Asset {
                name: (*a).into(),
                size: 1,
                browser_download_url: String::new(),
                digest: None,
            })
            .collect(),
    }
}
#[test]
fn parses_both_release_schemes() {
    let a = parse_asset("printcraft-0.2.1-windows-x64-portable.zip").unwrap();
    assert_eq!((a.prefix.as_str(), a.os), ("printcraft", "windows"));
    assert_eq!(
        parse_asset("cadcraft-0.3.0-macos-universal.dmg")
            .unwrap()
            .os,
        "macos"
    );
    assert_eq!(
        parse_asset("cadcraft-0.3.0-linux-x86_64.AppImage")
            .unwrap()
            .os,
        "linux"
    );
    let a = parse_asset("ArtCraft_0.41.0_x64-setup.exe").unwrap();
    assert_eq!(
        (a.scheme, a.prefix.as_str(), a.os),
        (Scheme::Tauri, "ArtCraft", "windows")
    );
    assert_eq!(
        parse_asset("ArtCraft_0.41.0_x64_en-US.msi").unwrap().os,
        "windows"
    );
    assert_eq!(
        parse_asset("ArtCraft_0.41.0_universal.dmg").unwrap().os,
        "macos"
    );
    for ignored in [
        "cadcraft-cli-0.3.0-macos-universal.zip",
        "cadcraft-web-0.3.0.zip",
        "SHA256SUMS.txt",
        "ArtCraft_universal.app.tar.gz",
    ] {
        assert!(parse_asset(ignored).is_none(), "{ignored}");
    }
}
#[test]
fn classifies_new_apps_from_their_release() {
    let e = classify(
        "mapcraft",
        Some(&release(
            "v1.2.0",
            "MapCraft v1.2.0",
            &[
                "mapcraft-1.2.0-windows-x64-portable.zip",
                "mapcraft-1.2.0-macos-universal.dmg",
            ],
        )),
    );
    assert_eq!(
        (e.key.as_str(), e.title.as_str(), e.scheme),
        ("mapcraft", "MapCraft", Scheme::Craft)
    );
    assert!(e.release && e.source);
    let renamed = classify(
        "notecraft",
        Some(&release(
            "v0.1.0",
            "",
            &["jotcraft-0.1.0-windows-x64-portable.zip"],
        )),
    );
    assert_eq!(
        (renamed.key.as_str(), renamed.asset_prefix.as_str()),
        ("jotcraft", "jotcraft")
    );
    assert_eq!(renamed.title, "NoteCraft");
    let tauri = classify(
        "studiocraft",
        Some(&release(
            "studiocraft-v2.0.0",
            "Studio v2.0.0",
            &["Studio_2.0.0_universal.dmg"],
        )),
    );
    assert_eq!(
        (tauri.key.as_str(), tauri.scheme, tauri.source),
        ("studiocraft", Scheme::Tauri, false)
    );
    let source = classify("labcraft", None);
    assert_eq!(
        (source.scheme, source.release, source.source),
        (Scheme::None, false, true)
    );
    let stale = classify(
        "oldcraft",
        Some(&release(
            "v2.0.0",
            "",
            &["oldcraft-1.0.0-windows-x64-portable.zip"],
        )),
    );
    assert!(!stale.release);
}
#[test]
fn builtin_apps_stay_authoritative() {
    let mut duplicate = craft("printcraft", "Wrong title", "", "");
    duplicate.release = false;
    let learned = std::collections::BTreeMap::from([(
        "lightcraft".to_string(),
        vec!["lumencraft".to_string(), "Bad Name".to_string()],
    )]);
    let merged = merge(
        vec![
            duplicate,
            craft("mapcraft", "MapCraft", "", ""),
            craft("../x", "X", "", ""),
        ],
        &learned,
        &[],
    );
    let light = merged.iter().find(|e| e.key == "lightcraft").unwrap();
    assert_eq!(light.aliases, ["lumencraft"]);
    let pdf = merged.iter().find(|e| e.key == "printcraft").unwrap();
    assert_eq!(pdf.title, "PDFCraft");
    assert!(pdf.release);
    assert!(merged.iter().any(|e| e.key == "mapcraft"));
    assert!(!merged.iter().any(|e| e.key == "../x"));
    assert_eq!(merged[0].key, "designcraft");
    assert_eq!(merged.last().unwrap().key, "mapcraft");
    assert_eq!(pretty("cadcraft"), "CadCraft");
    assert_eq!(names("printcraft"), ["printcraft", "pdfcraft"]);
}
#[test]
fn parses_github_links_to_owner_slugs() {
    assert_eq!(
        parse_github_slug("https://github.com/Bherbruck/SolveCraft").unwrap(),
        "bherbruck/solvecraft"
    );
    assert_eq!(
        parse_github_slug("github.com/jub0t/concat/").unwrap(),
        "jub0t/concat"
    );
    assert_eq!(parse_github_slug("jub0t/concat").unwrap(), "jub0t/concat");
    assert_eq!(
        parse_github_slug("https://github.com/jub0t/concat.git").unwrap(),
        "jub0t/concat"
    );
    assert_eq!(
        parse_github_slug("https://github.com/jub0t/concat?tab=releases").unwrap(),
        "jub0t/concat"
    );
    for bad in [
        "",
        "not a link",
        "https://gitlab.com/owner/repo",
        "https://github.com/just-an-owner",
        "https://github.com/owner/repo/releases",
        "https://github.com/owner/repo/blob/main/README.md",
        "owner/repo/extra",
        "https://github.com/BAD OWNER/repo",
    ] {
        assert!(parse_github_slug(bad).is_err(), "{bad}");
    }
}
#[test]
fn classifies_custom_apps_from_their_release() {
    fn platform_asset() -> (&'static str, &'static str) {
        if cfg!(target_os = "macos") {
            ("zookraft-1.2.0-macos-universal.dmg", "portable")
        } else if cfg!(target_os = "linux") {
            ("zookraft-1.2.0-linux-x86_64.AppImage", "portable")
        } else {
            ("zookraft-1.2.0-windows-x64.msi", "installer")
        }
    }
    let repo = Repository {
        created_at: None,
        topics: Vec::new(),
        name: "ZooKraft".into(),
        description: Some(" Puzzles ".into()),
        archived: false,
        fork: false,
        disabled: false,
    };
    let (asset, format) = platform_asset();
    let craft_release = release("v1.2.0", "ZooKraft v1.2.0", &[asset]);
    let entry = classify_custom("someone/zookraft", &repo, &craft_release, format, "x64").unwrap();
    assert_eq!(
        (entry.key.as_str(), entry.repository.as_str()),
        ("zookraft", "someone/zookraft")
    );
    assert_eq!(
        (entry.scheme, entry.release, entry.source),
        (Scheme::Craft, true, false)
    );
    assert_eq!(entry.asset_prefix.as_str(), "zookraft");
    assert_eq!(entry.title.as_str(), "ZooKraft");
    assert_eq!(entry.description.as_str(), "Puzzles");
    if cfg!(target_os = "macos") {
        let tauri_release = release("1.2.0", "", &["Zoo_1.2.0_universal.dmg"]);
        let entry =
            classify_custom("someone/zookraft", &repo, &tauri_release, "portable", "x64").unwrap();
        assert_eq!(
            (entry.scheme, entry.asset_prefix.as_str()),
            (Scheme::Tauri, "Zoo")
        );
    } else if cfg!(target_os = "windows") {
        let tauri_release = release("1.2.0", "", &["Zoo_1.2.0_x64-setup.exe"]);
        let entry = classify_custom(
            "someone/zookraft",
            &repo,
            &tauri_release,
            "installer",
            "x64",
        )
        .unwrap();
        assert_eq!(
            (entry.scheme, entry.asset_prefix.as_str()),
            (Scheme::Tauri, "Zoo")
        );
    }
    let web = release("v1.2.0", "", &["zookraft-1.2.0-web.zip"]);
    assert!(classify_custom("someone/zookraft", &repo, &web, format, "x64").is_err());
    let draft = Release {
        draft: true,
        ..release("v1.2.0", "", &[asset])
    };
    assert!(classify_custom("someone/zookraft", &repo, &draft, format, "x64").is_err());
}
#[test]
fn custom_apps_persist_and_win_over_discovery() {
    let root = std::env::temp_dir().join(format!("craft-catalog-custom-{}", uuid::Uuid::new_v4()));
    let mine = Entry {
        key: "zookraft".into(),
        repository: "someone/zookraft".into(),
        title: "Zoo".into(),
        category: String::new(),
        description: String::new(),
        scheme: Scheme::Craft,
        download_rule: None,
        download_rules: Vec::new(),
        registered_formats: Vec::new(),
        asset_prefix: "zookraft".into(),
        aliases: Vec::new(),
        created_at: None,
        bundle_ids: Vec::new(),
        release: true,
        source: false,
    };
    files::write_json(&custom_path(&root), &vec![mine.clone()]).unwrap();
    load(&root);
    assert!(all()
        .iter()
        .any(|e| e.key == "zookraft" && e.repository == "someone/zookraft"));
    // A discovered entry for the same repository is dropped, never duplicated.
    let merged = merge(
        vec![mine.clone()],
        &std::collections::BTreeMap::new(),
        &custom_apps(&root),
    );
    assert_eq!(merged.iter().filter(|e| e.key == "zookraft").count(), 1);
    let removed = remove_custom_app(&root, "zookraft").unwrap();
    assert_eq!(removed.repository, "someone/zookraft");
    assert!(remove_custom_app(&root, "zookraft").is_err());
    assert!(remove_custom_app(&root, "designcraft").is_err());
    // Restore the process-wide catalog for the remaining serial tests.
    load(&root.join("unused"));
    assert!(all().iter().all(|e| e.key != "zookraft"));
    std::fs::remove_dir_all(root).unwrap();
}
#[cfg(target_os = "windows")]
#[test]
fn common_custom_downloads_support_single_and_ambiguous_releases() {
    let repository = Repository {
        created_at: None,
        topics: Vec::new(),
        name: "obs-studio".into(),
        description: None,
        archived: false,
        fork: false,
        disabled: false,
    };
    let asset = crate::model::Asset {
        name: "OBS-Studio-32.2.2-Windows-x64-Installer.exe".into(),
        size: 1,
        digest: None,
        browser_download_url: String::new(),
    };
    let mut release = Release {
        tag_name: "32.2.2".into(),
        name: Some("OBS Studio 32.2.2".into()),
        draft: false,
        prerelease: false,
        assets: vec![asset.clone()],
    };
    let entry = classify_custom(
        "obsproject/obs-studio",
        &repository,
        &release,
        "installer",
        "x64",
    )
    .unwrap();
    assert_eq!(entry.scheme, Scheme::Generic);
    assert_eq!(entry.title, "OBS Studio");
    assert!(entry.aliases.contains(&"obs64".into()));
    assert!(entry.download_rule.is_some());
    release.assets.push(crate::model::Asset {
        name: "OBS-Studio-32.2.2-x64-Setup.exe".into(),
        ..asset
    });
    let error = classify_custom(
        "obsproject/obs-studio",
        &repository,
        &release,
        "installer",
        "x64",
    )
    .unwrap_err();
    let choice = error.downcast_ref::<DownloadChoice>().unwrap();
    assert_eq!(choice.assets.len(), 2);
    assert!(choice.entry.download_rule.is_none());
    let root = std::env::temp_dir().join(format!("craft-choice-{}", uuid::Uuid::new_v4()));
    let saved = choose_custom_download(&root, choice, 1).unwrap();
    assert_eq!(
        saved.download_rule.as_ref().unwrap().template,
        "OBS-Studio-{version}-x64-Setup.exe"
    );
    assert_eq!(custom_apps(&root).len(), 1);
    let mut second = choice.clone();
    second.preferences.architecture = "x86".into();
    second.assets = vec![crate::model::Asset {
        name: "OBS-Studio-32.2.2-Windows-x86-Installer.exe".into(),
        ..second.assets[0].clone()
    }];
    let updated = choose_custom_download(&root, &second, 0).unwrap();
    assert_eq!(custom_apps(&root).len(), 1);
    assert_eq!(updated.download_rules.len(), 2);
    let original =
        crate::downloads::select_for_entry(&release, &updated, &choice.preferences).unwrap();
    assert_eq!(original.name, choice.assets[1].name);
    assert_eq!(custom_apps(&root)[0].download_rules, updated.download_rules);
    load(&root.join("unused"));
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(target_os = "windows")]
#[test]
fn rejected_architecture_does_not_block_retry_with_compatible_preferences() {
    let repository = Repository {
        created_at: None,
        topics: Vec::new(),
        name: "exampletool".into(),
        description: None,
        archived: false,
        fork: false,
        disabled: false,
    };
    let installer = crate::model::Asset {
        name: "ExampleTool-1.2.3-Windows-x64-Installer.exe".into(),
        size: 1,
        digest: None,
        browser_download_url: String::new(),
    };
    let release = Release {
        tag_name: "v1.2.3".into(),
        name: None,
        draft: false,
        prerelease: false,
        assets: vec![
            installer.clone(),
            crate::model::Asset {
                name: "ExampleTool-1.2.3-Windows-x64.zip".into(),
                ..installer
            },
        ],
    };
    let error = classify_custom(
        "someone/exampletool",
        &repository,
        &release,
        "portable",
        "x86",
    )
    .unwrap_err();
    assert!(error.to_string().contains("portable"));
    assert!(error.to_string().contains("x86"));
    assert!(classify_custom(
        "someone/exampletool",
        &repository,
        &release,
        "installer",
        "x86"
    )
    .is_err());
    assert!(classify_custom(
        "someone/exampletool",
        &repository,
        &release,
        "installer",
        "x64"
    )
    .is_ok());
    assert!(classify_custom(
        "someone/exampletool",
        &repository,
        &release,
        "portable",
        "x64"
    )
    .is_ok());
    let root = std::env::temp_dir().join(format!("craft-availability-{}", uuid::Uuid::new_v4()));
    let paths = crate::model::Paths::new(root.clone(), None);
    let prefs = crate::model::Preferences {
        release_format: "installer".into(),
        architecture: "x86".into(),
        ..Default::default()
    };
    let entry = classify_custom(
        "someone/exampletool",
        &repository,
        &release,
        "installer",
        "x64",
    )
    .unwrap();
    write_custom_apps(&root, &vec![entry]).unwrap();
    files::write_json(&paths.at("manager-settings.json"), &prefs).unwrap();
    load(&root);
    assert!(get("exampletool").is_some());
    assert_eq!(
        crate::updates::cached_download_available(&paths, "exampletool", &prefs),
        None
    );
    use sha2::{Digest, Sha256};
    let url = "https://api.github.com/repos/someone/exampletool/releases/latest";
    let path = root
        .join("runtime/api-cache")
        .join(format!("{:x}.json", Sha256::digest(url.as_bytes())));
    files::write_json(
        &path,
        &serde_json::json!({"at":chrono::Utc::now().timestamp(),"etag":null,"value":release}),
    )
    .unwrap();
    assert_eq!(
        crate::updates::cached_download_available(&paths, "exampletool", &prefs),
        Some(true)
    );
    let x64 = crate::model::Preferences {
        architecture: "x64".into(),
        ..prefs
    };
    assert_eq!(
        crate::updates::cached_download_available(&paths, "exampletool", &x64),
        Some(true)
    );
    assert!(get("exampletool").is_some());
    load(&root.join("unused"));
    std::fs::remove_dir_all(root).unwrap();
}
#[cfg(target_os = "windows")]
#[test]
fn format_registrations_are_staged_and_saved_together() {
    let root = std::env::temp_dir().join(format!("craft-formats-{}", uuid::Uuid::new_v4()));
    let paths = crate::model::Paths::new(root.clone(), None);
    load(&root);
    let repository = Repository {
        created_at: None,
        topics: Vec::new(),
        name: "exampletool".into(),
        description: None,
        archived: false,
        fork: false,
        disabled: false,
    };
    let installer = crate::model::Asset {
        name: "ExampleTool-1.2.3-Windows-x64-Setup.exe".into(),
        size: 1,
        digest: None,
        browser_download_url: String::new(),
    };
    let release = Release {
        tag_name: "v1.2.3".into(),
        name: None,
        draft: false,
        prerelease: false,
        assets: vec![
            installer.clone(),
            crate::model::Asset {
                name: "ExampleTool-1.2.3-Windows-x64.zip".into(),
                ..installer
            },
        ],
    };
    let installed = classify_custom(
        "someone/exampletool",
        &repository,
        &release,
        "installer",
        "x64",
    )
    .unwrap();
    let mut draft = Vec::new();
    merge_custom_registration(&mut draft, installed.clone()).unwrap();
    assert!(custom_apps(&root).is_empty());
    assert!(get("exampletool").is_none());
    assert!(!draft[0].registered_formats.contains(&"portable".into()));
    // Cancelling discards the draft without touching the saved catalog.
    let cancelled = draft.clone();
    drop(cancelled);
    assert!(!custom_path(&root).exists());
    let portable = classify_custom(
        "someone/exampletool",
        &repository,
        &release,
        "portable",
        "x64",
    )
    .unwrap();
    merge_custom_registration(&mut draft, portable).unwrap();
    assert_eq!(draft.len(), 1);
    assert_eq!(draft[0].registered_formats.len(), 2);
    let prefs = crate::model::Preferences {
        release_format: "installer".into(),
        architecture: "x64".into(),
        ..Default::default()
    };
    crate::settings::save_with_custom_apps(
        &paths,
        &root.join("locations.json"),
        &serde_json::json!({}),
        &root,
        &root.join("tools"),
        Some(&prefs),
        None,
        Some(&draft),
    )
    .unwrap();
    load(&root);
    let selected = get("exampletool").unwrap();
    assert!(
        crate::downloads::select_for_entry(&release, &selected, &prefs)
            .unwrap()
            .name
            .ends_with(".exe")
    );
    // An app registered only for Installer is absent from the Portable catalog.
    let mut only_installer = installed;
    only_installer.key = "otherexample".into();
    only_installer.repository = "someone/otherexample".into();
    draft.push(only_installer);
    write_custom_apps(&root, &draft).unwrap();
    let portable_prefs = crate::model::Preferences {
        release_format: "portable".into(),
        ..prefs.clone()
    };
    files::write_json(&paths.at("manager-settings.json"), &portable_prefs).unwrap();
    load(&root);
    assert!(get("otherexample").is_some());
    // Sidebar and Home use saved orders that deliberately retain inactive apps.
    let sidebar_order = vec![
        "otherexample".into(),
        "photocraft".into(),
        "exampletool".into(),
    ];
    let home_order = vec![
        "exampletool".into(),
        "otherexample".into(),
        "photocraft".into(),
    ];
    assert_eq!(
        crate::model::ordered_release_apps(&sidebar_order, "portable"),
        sidebar_order
    );
    assert_eq!(
        crate::model::ordered_release_apps(&home_order, "portable"),
        home_order
    );
    assert_eq!(
        crate::model::ordered_release_apps(&sidebar_order, "installer"),
        sidebar_order
    );
    assert_eq!(
        crate::model::ordered_release_apps(&home_order, "installer"),
        home_order
    );
    let mut retained = portable_prefs.clone();
    retained.app_order = sidebar_order.clone();
    retained.home_app_order = home_order.clone();
    retained.validate().unwrap();
    assert!(retained.app_order.contains(&"otherexample".into()));
    assert!(retained.home_app_order.contains(&"otherexample".into()));
    let selected = get("exampletool").unwrap();
    assert!(
        crate::downloads::select_for_entry(&release, &selected, &portable_prefs)
            .unwrap()
            .name
            .ends_with(".zip")
    );
    files::write_json(&paths.at("manager-settings.json"), &prefs).unwrap();
    load(&root);
    assert!(get("otherexample").is_some());
    assert_eq!(custom_apps(&root).len(), 2);
    let incompatible = Release {
        assets: vec![],
        ..release
    };
    assert!(classify_custom(
        "someone/exampletool",
        &repository,
        &incompatible,
        "portable",
        "x64"
    )
    .is_err());
    assert_eq!(custom_apps(&root).len(), 2);
    load(&root.join("unused"));
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn third_party_apps_are_only_added_explicitly() {
    let defaults = builtin();
    assert!(defaults.iter().all(|e| !e.repository.contains('/')));
    assert!(!defaults
        .iter()
        .any(|e| matches!(e.key.as_str(), "solvecraft" | "concat")));

    let custom = Entry {
        repository: "someone/customcraft".into(),
        ..craft("customcraft", "CustomCraft", "Desktop application", "")
    };
    let merged = merge(Vec::new(), &BTreeMap::new(), std::slice::from_ref(&custom));
    assert!(merged.contains(&custom));
    assert_eq!(repository_name(&custom.repository), "customcraft");
}
