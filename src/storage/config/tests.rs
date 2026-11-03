use super::*;
use serde_json::json;

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn store() -> (Fixture, ConfigStore) {
    let temp = Fixture(std::env::temp_dir().join(format!("craft-config-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir_all(temp.path()).unwrap();
    let store = ConfigStore::new(temp.path().to_owned());
    (temp, store)
}

#[cfg(unix)]
#[test]
fn fresh_config_can_be_read_inside_an_existing_operation_lock() {
    let (_temp, store) = store();
    let _lock = crate::platform::Lock::take("Local\\CraftAppsManager").unwrap();
    let snapshot = store.load().unwrap();
    assert_eq!(snapshot.document["schema_version"], 1);
    assert!(!store.path().exists());
}
fn change(path: &[&str], before: Option<Value>, after: Option<Value>) -> ConfigPatch {
    ConfigPatch {
        fields: vec![FieldPatch {
            path: path.iter().map(|s| (*s).into()).collect(),
            before,
            after,
        }],
    }
}

#[test]
fn theme_saves_while_an_installation_owns_the_operation_lock() {
    let (_temp, store) = store();
    let mut initial = document::defaults();
    initial["manager"]["theme"] = json!("light");
    initial["manager"]["future"] = json!({"keep": 42});
    store.publish_locked(initial).unwrap();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let operation = std::thread::spawn(move || {
        let _lock = crate::platform::Lock::take("Local\\CraftAppsManager").unwrap();
        ready_tx.send(()).unwrap();
        release_rx.recv().unwrap();
    });
    ready_rx.recv().unwrap();
    let paths = crate::model::Paths::new(store.root().to_owned(), None);
    let result = crate::settings::save_theme(&paths, crate::model::Theme::Dark);
    let ordinary_edit_blocked = store.commit(ConfigPatch::default()).is_err();
    // Always release the simulated operation, including when the assertion fails.
    release_tx.send(()).unwrap();
    operation.join().unwrap();
    assert!(
        ordinary_edit_blocked,
        "non-cosmetic edits must stay protected"
    );
    assert_eq!(result.unwrap().theme, crate::model::Theme::Dark);
    let saved = store.load().unwrap();
    assert_eq!(saved.manager().unwrap().theme, crate::model::Theme::Dark);
    assert_eq!(saved.document["manager"]["future"]["keep"], 42);
}

#[test]
fn theme_save_rejects_invalid_yaml_and_does_not_bypass_legacy_migration() {
    let (temp, store) = store();
    let paths = crate::model::Paths::new(temp.path().to_owned(), None);
    let legacy = temp.path().join("manager-settings.json");
    std::fs::write(&legacy, br#"{"theme":"light","future":42}"#).unwrap();
    assert!(crate::settings::save_theme(&paths, crate::model::Theme::Dark).is_err());
    assert!(!store.path().exists());
    assert!(legacy.exists());
    std::fs::write(store.path(), b"schema_version: [broken").unwrap();
    assert!(crate::settings::save_theme(&paths, crate::model::Theme::Dark).is_err());
    assert_eq!(
        std::fs::read(store.path()).unwrap(),
        b"schema_version: [broken"
    );
    assert!(legacy.exists());
}

#[test]
fn theme_save_respects_other_configuration_writers() {
    let (_temp, store) = store();
    store.publish_locked(document::defaults()).unwrap();
    let original = std::fs::read(store.path()).unwrap();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let writer = std::thread::spawn(move || {
        let _lock = super::write_lock().unwrap();
        ready_tx.send(()).unwrap();
        release_rx.recv().unwrap();
    });
    ready_rx.recv().unwrap();
    let paths = crate::model::Paths::new(store.root().to_owned(), None);
    let blocked = crate::settings::save_theme(&paths, crate::model::Theme::Dark).is_err();
    release_tx.send(()).unwrap();
    writer.join().unwrap();
    assert!(blocked);
    assert_eq!(std::fs::read(store.path()).unwrap(), original);
    crate::settings::save_theme(&paths, crate::model::Theme::Dark).unwrap();
}

#[test]
fn legacy_startup_alias_in_existing_yaml_supports_settings_patches() {
    for initial in [false, true] {
        let (_temp, store) = store();
        let mut raw = document::defaults();
        raw["manager"] = json!({"checkUpdaterOnStartup":initial,"future":{"keep":8}});
        let bytes = serde_saphyr::to_string(&raw).unwrap();
        std::fs::write(store.path(), bytes.as_bytes()).unwrap();
        let base = store.load().unwrap();
        // Loading must not rewrite files before the user saves.
        assert_eq!(std::fs::read(store.path()).unwrap(), bytes.as_bytes());
        let saved = store
            .commit(change(
                &["manager", "checkManagerOnStartup"],
                base.document["manager"]
                    .get("checkManagerOnStartup")
                    .cloned(),
                Some(json!(!initial)),
            ))
            .unwrap()
            .snapshot;
        assert_eq!(saved.manager().unwrap().check_manager_on_startup, !initial);
        assert_eq!(saved.document["manager"]["future"], json!({"keep":8}));
        assert!(saved.document["manager"]
            .get("checkUpdaterOnStartup")
            .is_none());
    }
}

#[test]
fn ambiguous_startup_alias_is_rejected_without_rewriting_yaml() {
    let (_temp, store) = store();
    let bytes = b"schema_version: 1\nmanager:\n  checkUpdaterOnStartup: true\n  checkManagerOnStartup: false\n";
    std::fs::write(store.path(), bytes).unwrap();
    assert!(store.load().is_err());
    assert_eq!(std::fs::read(store.path()).unwrap(), bytes);
}

#[test]
fn missing_config_is_read_only_until_a_changed_commit() {
    let (_temp, store) = store();
    let initial = store.load().unwrap();
    assert_eq!(initial.document["schema_version"], 1);
    assert!(!store.path().exists());
    assert!(!store.commit(ConfigPatch::default()).unwrap().changed);
    assert!(!store.path().exists());
    assert!(
        store
            .commit(change(&["future"], None, Some(json!({"a": 2}))))
            .unwrap()
            .changed
    );
    assert_eq!(store.load().unwrap().document["future"]["a"], 2);
}

#[test]
fn unrelated_edits_merge_identical_edits_are_noops_and_conflicts_fail() {
    let (_temp, store) = store();
    store
        .commit(change(&["future"], None, Some(json!({"x": 1, "y": 1}))))
        .unwrap();
    store
        .commit(change(&["future", "x"], Some(json!(1)), Some(json!(2))))
        .unwrap();
    store
        .commit(change(&["future", "y"], Some(json!(1)), Some(json!(3))))
        .unwrap();
    assert!(
        !store
            .commit(change(&["future", "x"], Some(json!(1)), Some(json!(2))))
            .unwrap()
            .changed
    );
    assert!(store
        .commit(change(&["future", "x"], Some(json!(1)), Some(json!(4))))
        .is_err());
    assert_eq!(
        store.load().unwrap().document["future"],
        json!({"x": 2, "y": 3})
    );
}

#[test]
fn literal_keys_unknowns_and_inventory_survive() {
    let (temp, store) = store();
    std::fs::write(temp.path().join("settings.json"), "inventory bytes").unwrap();
    store
        .commit(change(
            &["launch", "a.b/C:/x"],
            None,
            Some(json!({"executable":"", "arguments": ["--x"], "future": 4})),
        ))
        .unwrap();
    store
        .commit(change(
            &["launch", "a.b/C:/x", "arguments"],
            Some(json!(["--x"])),
            Some(json!(["--y"])),
        ))
        .unwrap();
    assert_eq!(
        store.load().unwrap().document["launch"]["a.b/C:/x"]["future"],
        4
    );
    assert_eq!(
        std::fs::read(temp.path().join("settings.json")).unwrap(),
        b"inventory bytes"
    );
}

#[test]
fn rejected_yaml_never_defaults_or_overwrites() {
    let (_temp, store) = store();
    for yaml in [
        "schema_version: 99",
        "schema_version: 1\nmanager: [bad]",
        "schema_version: 1\na: 1\na: 2",
        "schema_version: 1\nx: &a 1",
        "schema_version: 1\nx: !custom hi",
        "schema_version: 1\nx: .nan",
        "schema_version: 1\n---\nschema_version: 1",
        "schema_version: 1\n? [a, b]\n: x",
        "schema_version: 1\nx: {<<: {a: 1}}",
    ] {
        std::fs::write(store.path(), yaml).unwrap();
        assert!(store.load().is_err(), "accepted {yaml}");
        assert!(store.commit(change(&["x"], None, Some(json!(1)))).is_err());
        assert_eq!(std::fs::read_to_string(store.path()).unwrap(), yaml);
    }
}

#[test]
fn yaml_budgets_reject_size_depth_and_nodes() {
    assert!(document::parse(&vec![b' '; document::MAX_BYTES + 1]).is_err());
    let yaml = format!(
        "schema_version: 1\nx: {}0{}",
        "[".repeat(70),
        "]".repeat(70)
    );
    assert!(document::parse(yaml.as_bytes()).is_err());
    let yaml = format!("schema_version: 1\nx: [{}]", "0,".repeat(100_001));
    assert!(document::parse(yaml.as_bytes()).is_err());
}

#[test]
fn typed_views_preserve_custom_choices_and_unknown_properties() {
    let (_temp, store) = store();
    store
        .commit(change(
            &["manager", "selectedApps"],
            None,
            Some(json!(["custom-app"])),
        ))
        .unwrap();
    let snapshot = store.load().unwrap();
    assert_eq!(
        snapshot.manager().unwrap().selected_apps,
        vec!["custom-app"]
    );
    assert_eq!(snapshot.builder().unwrap().log_size_mb, 10);
}

#[test]
fn invalid_known_values_and_overlapping_patches_cannot_publish() {
    let (_temp, store) = store();
    assert!(store
        .commit(change(&["builder", "logSizeMB"], None, Some(json!(0))))
        .is_err());
    assert!(!store.path().exists());
    let patch = ConfigPatch {
        fields: vec![
            FieldPatch {
                path: vec!["launch".into()],
                before: Some(json!({})),
                after: Some(json!({})),
            },
            FieldPatch {
                path: vec!["launch".into(), "x".into()],
                before: None,
                after: Some(json!(1)),
            },
        ],
    };
    assert!(store.commit(patch).is_err());
}

#[test]
fn manual_yaml_edits_keep_unknowns_and_noop_keeps_exact_bytes() {
    let (_temp, store) = store();
    let original = b"# handwritten\nschema_version: 1\nmanager:\n  future: {flag: true}\n";
    std::fs::write(store.path(), original).unwrap();
    let base = store.load().unwrap();
    assert!(!store.commit(ConfigPatch::default()).unwrap().changed);
    assert_eq!(std::fs::read(store.path()).unwrap(), original);
    store
        .commit(change(&["manager", "theme"], None, Some(json!("dark"))))
        .unwrap();
    let latest = store.load().unwrap();
    assert_ne!(base.revision, latest.revision);
    assert_eq!(latest.document["manager"]["future"]["flag"], true);
}

#[test]
fn lists_are_atomic_and_missing_differs_from_explicit_null() {
    let (_temp, store) = store();
    store
        .commit(change(
            &["future"],
            None,
            Some(json!({"list":[1], "nullable":null})),
        ))
        .unwrap();
    store
        .commit(change(
            &["future", "list"],
            Some(json!([1])),
            Some(json!([1, 2])),
        ))
        .unwrap();
    assert!(store
        .commit(change(
            &["future", "list"],
            Some(json!([1])),
            Some(json!([1, 3]))
        ))
        .is_err());
    assert!(store
        .commit(change(&["future", "nullable"], None, Some(json!(2))))
        .is_err());
    store
        .commit(change(&["future", "nullable"], Some(Value::Null), None))
        .unwrap();
    assert!(store.load().unwrap().document["future"]
        .get("nullable")
        .is_none());
}

#[test]
fn custom_app_list_edits_preserve_retained_raw_metadata() {
    for mode in ["patch", "mutate", "commit"] {
        for operation in ["add", "remove"] {
            let (_temp, store) = store();
            let retained = json!({"key":"keepapp","repository":"owner/keepapp","title":"Keep App", "scheme":"generic","assetPrefix":"keepapp","release":true,"source":false,
            "future":{"flag":true}, "downloadRule":{"template":"app-{version}.zip","os":"windows","architecture":"x64","format":"portable","futureRule":7},
            "downloadRules":[{"template":"app-{version}.zip","os":"windows","architecture":"x64","format":"portable","futureArrayRule":8}]});
            let removed = json!({"key":"otherapp","repository":"owner/otherapp","title":"Other App","scheme":"none","release":false,"source":true});
            let mut initial = document::defaults();
            initial["custom_apps"] = json!([retained.clone(), removed.clone()]);
            store.publish_locked(initial).unwrap();
            let base = store.load().unwrap().custom_apps().unwrap();
            let mut edited = if operation == "remove" {
                base[..1].to_vec()
            } else {
                let mut added = base.clone();
                let mut extra = base[1].clone();
                extra.key = "thirdapp".into();
                extra.repository = "owner/thirdapp".into();
                added.push(extra);
                added
            };
            edited[0].title = "Renamed".into();
            edited[0].download_rule.as_mut().unwrap().architecture = "arm64".into();
            match mode {
                "patch" => {
                    store
                        .patch_section_typed("custom_apps", &base, &edited)
                        .unwrap();
                }
                "mutate" => {
                    store
                        .mutate_section::<Vec<crate::catalog::Entry>>("custom_apps", |apps| {
                            *apps = edited.clone();
                            Ok(())
                        })
                        .unwrap();
                }
                _ => {
                    store
                        .commit(change(
                            &["custom_apps"],
                            Some(store.load().unwrap().document["custom_apps"].clone()),
                            Some(json!(edited)),
                        ))
                        .unwrap();
                }
            }
            let actual = store.load().unwrap().document["custom_apps"][0].clone();
            assert_eq!(actual["future"], retained["future"], "{mode}");
            assert_eq!(actual["downloadRule"]["futureRule"], 7, "{mode}");
            assert_eq!(actual["downloadRule"]["architecture"], "arm64");
            assert_eq!(actual["downloadRules"], retained["downloadRules"], "{mode}");
            assert_eq!(actual["title"], "Renamed");
        }
    }
}

#[test]
fn custom_app_rule_list_edits_preserve_surviving_rule_metadata() {
    for mode in ["patch", "mutate", "commit"] {
        let (_temp, store) = store();
        let mut initial = document::defaults();
        initial["custom_apps"] = json!([{"key":"keepapp","repository":"owner/keepapp","title":"Keep App", "scheme":"generic","assetPrefix":"keepapp","release":true,"source":false,
            "downloadRules":[{"template":"app-{version}-x64.zip","os":"windows","architecture":"x64","format":"portable","futureRule":{"keep":"x64"}}]}]);
        store.publish_locked(initial).unwrap();
        for operation in ["append", "reorder", "edit", "remove"] {
            let snapshot = store.load().unwrap();
            let base = snapshot.custom_apps().unwrap();
            let mut edited = base.clone();
            let rules = &mut edited[0].download_rules;
            match operation {
                "append" => {
                    let mut second = rules[0].clone();
                    second.architecture = "arm64".into();
                    second.template = "app-{version}-arm64.zip".into();
                    rules.push(second);
                }
                "reorder" => rules.reverse(),
                "edit" => {
                    rules
                        .iter_mut()
                        .find(|rule| rule.architecture == "x64")
                        .unwrap()
                        .template = "new-{version}-x64.zip".into();
                }
                _ => rules.retain(|rule| rule.architecture == "x64"),
            }
            match mode {
                "patch" => {
                    store
                        .patch_section_typed("custom_apps", &base, &edited)
                        .unwrap();
                }
                "mutate" => {
                    store
                        .mutate_section::<Vec<crate::catalog::Entry>>("custom_apps", |apps| {
                            *apps = edited.clone();
                            Ok(())
                        })
                        .unwrap();
                }
                _ => {
                    store
                        .commit(change(
                            &["custom_apps"],
                            Some(snapshot.document["custom_apps"].clone()),
                            Some(json!(edited)),
                        ))
                        .unwrap();
                }
            }
            let actual = store.load().unwrap().document["custom_apps"][0]["downloadRules"].clone();
            let retained = actual
                .as_array()
                .unwrap()
                .iter()
                .find(|rule| rule["architecture"] == "x64")
                .unwrap();
            assert_eq!(
                retained["futureRule"],
                json!({"keep":"x64"}),
                "{mode}: {operation}"
            );
            for added in actual
                .as_array()
                .unwrap()
                .iter()
                .filter(|rule| rule["architecture"] == "arm64")
            {
                assert!(
                    added.get("futureRule").is_none(),
                    "metadata must not move to the new rule"
                );
            }
            if operation == "edit" || operation == "remove" {
                assert_eq!(retained["template"], "new-{version}-x64.zip");
            }
        }
    }
}
