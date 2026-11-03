use super::*;
use serde_json::json;
use std::{fs, path::PathBuf};
fn fixture() -> (PathBuf, ConfigStore) {
    let root =
        std::env::temp_dir().join(format!("craft-config-migration-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let store = ConfigStore::new(root.clone());
    (root, store)
}
fn write(root: &std::path::Path, file: &str, value: serde_json::Value) {
    crate::files::write_json(&root.join(file), &value).unwrap();
}

#[test]
fn legacy_startup_alias_migrates_and_remains_editable() {
    for source in ["manager-settings.json", "updater-settings.json"] {
        for initial in [false, true] {
            let (root, store) = fixture();
            write(
                &root,
                source,
                json!({"checkUpdaterOnStartup":initial,"future":{"keep":7}}),
            );
            let snapshot = ensure_migrated(&store).unwrap().snapshot;
            let before = snapshot.manager().unwrap();
            assert_eq!(before.check_manager_on_startup, initial);
            let mut after = before.clone();
            after.check_manager_on_startup = !initial;
            let saved = store
                .patch_section_typed("manager", &before, &after)
                .unwrap();
            assert_eq!(saved.manager().unwrap().check_manager_on_startup, !initial);
            assert_eq!(saved.document["manager"]["future"], json!({"keep":7}));
            assert!(saved.document["manager"]
                .get("checkUpdaterOnStartup")
                .is_none());
            assert!(!root.join(source).exists());
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[test]
fn production_migration_deletes_only_verified_imports_and_reopens_without_sources() {
    let (root, store) = fixture();
    for &(name, section) in SOURCES {
        let value = match section {
            "custom_apps" => json!([]),
            "launch" | "build_launch" => {
                json!({"fixture":{"executable":"", "arguments":[], "futurePreserved":{"value":name}}})
            }
            _ => json!({"futurePreserved":{"value":name}}),
        };
        write(&root, name, value);
    }
    fs::write(root.join("data-root.json"), b"locator must remain").unwrap();
    fs::write(root.join("installations.json"), b"inventory must remain").unwrap();
    let imported = ensure_migrated(&store).unwrap();
    let reopened = ConfigStore::new(root.clone()).load().unwrap();
    assert_eq!(imported.snapshot.document, reopened.document);
    for &(name, section) in SOURCES {
        if section != "custom_apps" {
            let value = if name == "updater-settings.json" {
                &reopened.document["legacy_extensions"][name]
            } else {
                &reopened.document[section]
            };
            let value = if section == "launch" || section == "build_launch" {
                &value["fixture"]
            } else {
                value
            };
            assert_eq!(value["futurePreserved"]["value"], name);
        }
        assert!(
            !root.join(name).exists(),
            "Verified obsolete source retained: {name}"
        );
    }
    assert!(imported.warnings.is_empty());
    assert_eq!(
        fs::read(root.join("data-root.json")).unwrap(),
        b"locator must remain"
    );
    assert_eq!(
        fs::read(root.join("installations.json")).unwrap(),
        b"inventory must remain"
    );
    assert!(!root.join(JOURNAL).exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cleanup_rechecks_actual_yaml_instead_of_trusting_an_old_snapshot() {
    let (root, store) = fixture();
    write(&root, "manager-settings.json", json!({"theme":"light"}));
    let imported = migrate_locked(&store, false).unwrap();
    let journal: Journal = serde_json::from_slice(&fs::read(root.join(JOURNAL)).unwrap()).unwrap();
    let mut changed = imported.snapshot.document.clone();
    changed["manager"] = json!({});
    store.publish_locked(changed).unwrap();
    let warnings = cleanup(&store, &imported.snapshot, &journal, true);
    assert!(!warnings.is_empty());
    assert!(
        root.join("manager-settings.json").exists(),
        "Changed destination cannot authorize deletion"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn verified_cleanup_survives_settings_edits_and_restart_without_rewriting_yaml() {
    let (root, store) = fixture();
    for &(name, section) in SOURCES {
        let value = match section {
            "custom_apps" => json!([]),
            "launch" | "build_launch" => json!({}),
            _ => json!({"theme":"light","future":{"keep":name}}),
        };
        write(&root, name, value);
    }
    fs::write(root.join("installations.json"), b"keep inventory").unwrap();
    fs::write(root.join("data-root.json"), b"keep locator").unwrap();
    let imported = migrate_locked(&store, false).unwrap().snapshot;
    let before = imported.manager().unwrap();
    let mut edited = before.clone();
    edited.theme = crate::model::Theme::Dark;
    store
        .patch_section_typed("manager", &before, &edited)
        .unwrap();
    let saved = fs::read(store.path()).unwrap();
    let journal: Journal = serde_json::from_slice(&fs::read(root.join(JOURNAL)).unwrap()).unwrap();
    assert!(journal.verified);
    assert_ne!(
        journal.candidate_hash,
        semantic_hash(&store.load().unwrap().document).unwrap()
    );
    let restarted = ensure_migrated(&ConfigStore::new(root.clone())).unwrap();
    assert!(restarted.warnings.is_empty(), "{:?}", restarted.warnings);
    assert_eq!(
        restarted.snapshot.manager().unwrap().theme,
        crate::model::Theme::Dark
    );
    assert_eq!(
        restarted.snapshot.document["manager"]["future"],
        imported.document["manager"]["future"]
    );
    assert_eq!(fs::read(store.path()).unwrap(), saved);
    for &(name, _) in SOURCES {
        assert!(!root.join(name).exists(), "Retained {name}");
    }
    assert!(!root.join(JOURNAL).exists());
    assert_eq!(
        fs::read(root.join("installations.json")).unwrap(),
        b"keep inventory"
    );
    assert_eq!(
        fs::read(root.join("data-root.json")).unwrap(),
        b"keep locator"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn settings_edits_never_authorize_unverified_or_malformed_migration_receipts() {
    for malformed_hash in [false, true] {
        let (root, store) = fixture();
        write(&root, "manager-settings.json", json!({"theme":"light"}));
        let imported = migrate_locked(&store, false).unwrap().snapshot;
        let mut journal: Journal =
            serde_json::from_slice(&fs::read(root.join(JOURNAL)).unwrap()).unwrap();
        if malformed_hash {
            journal.candidate_hash = "invalid".into();
        } else {
            journal.verified = false;
        }
        write_journal(&store, &journal).unwrap();
        let before = imported.manager().unwrap();
        let mut edited = before.clone();
        edited.theme = crate::model::Theme::Dark;
        store
            .patch_section_typed("manager", &before, &edited)
            .unwrap();
        let result = ensure_migrated(&store).unwrap();
        assert!(!result.warnings.is_empty());
        assert!(root.join("manager-settings.json").exists());
        assert_eq!(
            result.snapshot.manager().unwrap().theme,
            crate::model::Theme::Dark
        );
        fs::remove_dir_all(root).unwrap();
    }
}
#[test]
fn migration_imports_sections_unknowns_and_manager_precedence_before_cleanup() {
    let (root, store) = fixture();
    write(
        &root,
        "manager-settings.json",
        json!({"theme":"light","future":{"keep":1}}),
    );
    write(
        &root,
        "updater-settings.json",
        json!({"theme":"dark","superseded":{"keep":2}}),
    );
    write(
        &root,
        "builder-settings.json",
        json!({"futureBuilder":{"keep":3}}),
    );
    write(
        &root,
        "runtime/launch-settings.json",
        json!({"filmcraft|C:\\a.b": {"executable":"", "arguments":["--test"], "futureLaunch":4}}),
    );
    let result = ensure_migrated(&store).unwrap();
    assert_eq!(result.snapshot.document["manager"]["theme"], "light");
    assert_eq!(result.snapshot.document["manager"]["future"]["keep"], 1);
    assert_eq!(
        result.snapshot.document["builder"]["futureBuilder"]["keep"],
        3
    );
    assert_eq!(
        result.snapshot.document["launch"]["filmcraft|C:\\a.b"]["futureLaunch"],
        4
    );
    assert_eq!(
        result.snapshot.document["legacy_extensions"]["updater-settings.json"]["superseded"]
            ["keep"],
        2
    );
    assert!(!root.join("manager-settings.json").exists());
    assert!(result.warnings.is_empty());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn migration_cleanup_retains_operational_files() {
    let (root, store) = fixture();
    write(&root, "manager-settings.json", json!({"theme":"light"}));
    fs::write(root.join("installations.json"), b"operational inventory").unwrap();
    fs::write(root.join("data-root.json"), b"locator").unwrap();
    let result = migrate_locked(&store, true).unwrap();
    assert!(result.warnings.is_empty());
    assert!(!root.join("manager-settings.json").exists());
    assert!(!root.join("runtime/config-migration.json").exists());
    assert_eq!(
        fs::read(root.join("installations.json")).unwrap(),
        b"operational inventory"
    );
    assert_eq!(fs::read(root.join("data-root.json")).unwrap(), b"locator");
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn migration_corrupt_source_never_publishes_or_removes_inputs() {
    let (root, store) = fixture();
    fs::write(root.join("builder-settings.json"), b"{broken").unwrap();
    assert!(ensure_migrated(&store).is_err());
    assert!(!root.join("config.yaml").exists());
    assert_eq!(
        fs::read(root.join("builder-settings.json")).unwrap(),
        b"{broken"
    );
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn migration_yaml_is_authoritative_and_corruption_never_reimports_json() {
    let (root, store) = fixture();
    write(&root, "manager-settings.json", json!({"theme":"light"}));
    let imported = ensure_migrated(&store).unwrap();
    let yaml = fs::read(store.path()).unwrap();
    write(&root, "manager-settings.json", json!({"theme":"dark"}));
    let again = ensure_migrated(&store).unwrap();
    assert_eq!(again.snapshot.document, imported.snapshot.document);
    assert_eq!(fs::read(store.path()).unwrap(), yaml);
    fs::write(store.path(), b"schema_version: 999\n").unwrap();
    assert!(ensure_migrated(&store).is_err());
    assert_eq!(fs::read(store.path()).unwrap(), b"schema_version: 999\n");
    assert!(root.join("manager-settings.json").exists());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn migration_changed_sources_remain_protected_after_yaml_edits() {
    let (root, store) = fixture();
    write(&root, "manager-settings.json", json!({"theme":"light"}));
    migrate_locked(&store, false).unwrap();
    write(&root, "manager-settings.json", json!({"theme":"dark"}));
    let result = migrate_locked(&store, true).unwrap();
    assert!(!result.warnings.is_empty());
    assert!(root.join("manager-settings.json").exists());
    let mut edited = result.snapshot.document;
    edited["manager"]["futureManual"] = json!(true);
    store.publish_locked(edited.clone()).unwrap();
    let result = migrate_locked(&store, true).unwrap();
    assert_eq!(result.snapshot.document, edited);
    assert!(!result.warnings.is_empty());
    assert!(root.join("runtime/config-migration.json").exists());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn migration_interrupted_journal_without_yaml_restarts_but_unverified_yaml_never_deletes() {
    let (root, store) = fixture();
    write(
        &root,
        "manager-settings.json",
        json!({"theme":"light", "futureSecretValue":"do-not-put-in-journal"}),
    );
    let (candidate, sources) = candidate(&store).unwrap();
    let journal = Journal {
        schema_version: 1,
        candidate_hash: semantic_hash(&candidate).unwrap(),
        verified: false,
        sources,
    };
    write_journal(&store, &journal).unwrap();
    let bytes = fs::read(root.join(JOURNAL)).unwrap();
    assert!(!String::from_utf8(bytes)
        .unwrap()
        .contains("do-not-put-in-journal"));
    assert!(!store.path().exists());
    // Crash after YAML publication, before verification marker publication.
    store.publish_locked(candidate).unwrap();
    let result = migrate_locked(&store, true).unwrap();
    assert!(!result.warnings.is_empty());
    assert!(root.join("manager-settings.json").exists());
    // A journal before publication alone never permits cleanup and is rebuilt.
    fs::remove_file(store.path()).unwrap();
    let result = migrate_locked(&store, true).unwrap();
    assert!(result.warnings.is_empty());
    assert!(!root.join("manager-settings.json").exists());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn migration_verified_journal_resumes_after_publication_without_reimport() {
    let (root, store) = fixture();
    write(
        &root,
        "updater-settings.json",
        json!({"theme":"dark","unknownLegacy":{"keep":4}}),
    );
    let first = migrate_locked(&store, false).unwrap();
    let result = migrate_locked(&store, true).unwrap();
    assert_eq!(result.snapshot.document, first.snapshot.document);
    assert_eq!(
        result.snapshot.document["manager"]["unknownLegacy"]["keep"],
        4
    );
    assert!(!root.join("updater-settings.json").exists());
    assert!(!root.join(JOURNAL).exists());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn migration_rejects_forged_cleanup_paths_and_preserves_yaml() {
    let (root, store) = fixture();
    write(&root, "manager-settings.json", json!({"theme":"light"}));
    let result = ensure_migrated(&store).unwrap();
    let protected = root.join("installations.json");
    fs::write(&protected, b"inventory").unwrap();
    let forged = Journal {
        schema_version: 1,
        candidate_hash: semantic_hash(&result.snapshot.document).unwrap(),
        verified: true,
        sources: vec![Source {
            name: "installations.json".into(),
            sha256: hash(b"inventory"),
        }],
    };
    write_journal(&store, &forged).unwrap();
    let result = migrate_locked(&store, true).unwrap();
    assert!(!result.warnings.is_empty());
    assert_eq!(fs::read(protected).unwrap(), b"inventory");
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn migration_custom_choices_and_build_selection_survive_without_catalog_adoption() {
    let (root, store) = fixture();
    write(
        &root,
        "manager-settings.json",
        json!({"selectedApps":["not-loaded-yet"],"appCheckIntervalMinutes":1,"backupVersions":99}),
    );
    write(
        &root,
        "builder-settings.json",
        json!({"logSizeMB":0,"logArchives":99}),
    );
    write(
        &root,
        "runtime/build-launch-settings.json",
        json!({"not-loaded-yet":{"executable":"","arguments":["a.b"],"selectedBuild":root.join("missing-build"),"futureChoice":{"keep":true}}}),
    );
    let result = ensure_migrated(&store).unwrap();
    assert_eq!(
        result.snapshot.document["manager"]["selectedApps"],
        json!(["not-loaded-yet"])
    );
    assert_eq!(
        result.snapshot.document["manager"]["appCheckIntervalMinutes"],
        10
    );
    assert_eq!(
        result.snapshot.document["manager"]["backupVersions"], 99,
        "retired keys remain untouched"
    );
    assert_eq!(result.snapshot.document["builder"]["logSizeMB"], 1);
    assert_eq!(result.snapshot.document["builder"]["logArchives"], 5);
    assert_eq!(
        result.snapshot.document["build_launch"]["not-loaded-yet"]["futureChoice"]["keep"],
        true
    );
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn migration_linked_source_is_rejected_without_touching_link_target() {
    let (root, store) = fixture();
    let outside = root.with_extension("outside");
    fs::create_dir_all(&outside).unwrap();
    let link = root.join("runtime");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    #[cfg(windows)]
    assert!(std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&link)
        .arg(&outside)
        .output()
        .unwrap()
        .status
        .success());
    fs::write(outside.join("custom-apps.json"), b"[]").unwrap();
    assert!(ensure_migrated(&store).is_err());
    assert!(!store.path().exists());
    assert_eq!(fs::read(outside.join("custom-apps.json")).unwrap(), b"[]");
    #[cfg(unix)]
    fs::remove_file(link).unwrap();
    #[cfg(windows)]
    fs::remove_dir(link).unwrap();
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(outside).unwrap();
}
