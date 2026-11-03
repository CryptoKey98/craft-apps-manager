use super::*;
use serde_json::json;
use std::{
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/recovery-fixtures")
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn store(&self) -> ConfigStore {
        ConfigStore::new(self.0.clone())
    }
    fn seed(&self) -> Vec<(String, Vec<u8>)> {
        let app = self.0.join("releases/fixture/app.exe");
        fs::create_dir_all(app.parent().unwrap()).unwrap();
        fs::write(&app, b"application preservation marker; never execute").unwrap();
        let values = [
            (
                "manager-settings.json",
                json!({"theme":"light", "closeToTray":true, "selectedApps":["fixture"], "future":{"keep":1}}),
            ),
            (
                "builder-settings.json",
                json!({"logSizeMB":42,"futureBuilder":{"keep":2}}),
            ),
            (
                "runtime/custom-apps.json",
                json!([{"key":"fixture","repository":"example/fixture","title":"Fixture","release":true,"source":false,"scheme":"generic","assetPrefix":"fixture","future":{"keep":3}}]),
            ),
            (
                "runtime/launch-settings.json",
                json!({"fixture":{"arguments":["--preserved"],"executable":app}}),
            ),
            (
                "runtime/build-launch-settings.json",
                json!({"fixture":{"selectedBuild":self.0.join("builds/fixture"),"executable":"","arguments":["--build-preserved"]}}),
            ),
            (
                "settings.json",
                json!({"appsRoot":self.0,"apps":[{"name":"fixture","version":"1.2.3","path":app,"install_kind":"portable","installation_label":"B"}],"installations":[],"futureInventory":{"keep":4}}),
            ),
            (
                "data-root.json",
                json!({"root":self.0,"tools":self.0.join("workspace/tools")}),
            ),
        ];
        for (name, value) in values {
            crate::files::write_json(&self.0.join(name), &value).unwrap();
        }
        [
            "manager-settings.json",
            "builder-settings.json",
            "runtime/custom-apps.json",
            "runtime/launch-settings.json",
            "runtime/build-launch-settings.json",
            "settings.json",
            "data-root.json",
            "releases/fixture/app.exe",
        ]
        .into_iter()
        .map(|name| (name.into(), fs::read(self.0.join(name)).unwrap()))
        .collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Child(std::process::Child);
impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "launched and terminated by migration_crash_boundaries_preserve_apps_and_preferences"]
fn migration_crash_child() {
    let root = PathBuf::from(std::env::var_os("CRAFT_RECOVERY_ROOT").unwrap());
    let stage = std::env::var("CRAFT_RECOVERY_STAGE").unwrap();
    let _lock = crate::platform::Lock::take("Local\\CraftAppsManager").unwrap();
    let store = ConfigStore::new(root.clone());
    let (value, sources) = candidate(&store).unwrap();
    let mut journal = Journal {
        schema_version: 1,
        candidate_hash: semantic_hash(&value).unwrap(),
        verified: false,
        sources,
    };
    // Leave genuinely incomplete temporary bytes, as an interrupted writer would.
    let mut temp = fs::File::create(root.join(".config-interrupted.tmp")).unwrap();
    temp.write_all(b"schema_version: 1\nmanager: {theme:")
        .unwrap();
    temp.sync_all().unwrap();
    if stage != "before-journal" {
        write_journal(&store, &journal).unwrap();
    }
    if stage == "after-yaml" || stage == "after-verification" {
        store.publish_locked(value).unwrap();
    }
    if stage == "after-verification" {
        journal.verified = true;
        write_journal(&store, &journal).unwrap();
    }
    fs::write(root.join("child-ready"), b"ready").unwrap();
    loop {
        std::thread::park();
    }
}

#[test]
fn migration_crash_boundaries_preserve_apps_and_preferences() {
    for stage in [
        "before-journal",
        "after-journal",
        "after-yaml",
        "after-verification",
    ] {
        let fixture = Fixture::new();
        let originals = fixture.seed();
        let mut child = Child(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    concat!(module_path!(), "::migration_crash_child")
                        .split_once("::")
                        .unwrap()
                        .1,
                    "--ignored",
                    "--nocapture",
                ])
                .env("CRAFT_RECOVERY_ROOT", &fixture.0)
                .env("CRAFT_RECOVERY_STAGE", stage)
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        while !fixture.0.join("child-ready").exists() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "Child exited at {stage}"
            );
            assert!(Instant::now() < deadline, "Child timed out at {stage}");
            std::thread::sleep(Duration::from_millis(10));
        }
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        let outcome = ensure_migrated(&fixture.store()).unwrap();
        let doc = &outcome.snapshot.document;
        assert_eq!(doc["manager"]["theme"], "light", "stage {stage}");
        assert_eq!(doc["manager"]["selectedApps"], json!(["fixture"]));
        assert_eq!(doc["builder"]["logSizeMB"], 42);
        assert_eq!(doc["custom_apps"][0]["future"]["keep"], 3);
        assert_eq!(
            doc["launch"]["fixture"]["arguments"],
            json!(["--preserved"])
        );
        assert_eq!(
            doc["build_launch"]["fixture"]["arguments"],
            json!(["--build-preserved"])
        );
        let yaml = fs::read(fixture.store().path()).unwrap();
        assert_eq!(
            ensure_migrated(&fixture.store()).unwrap().snapshot.document,
            *doc
        );
        assert_eq!(fs::read(fixture.store().path()).unwrap(), yaml);
        for (name, bytes) in &originals {
            if stage != "after-yaml" && SOURCES.iter().any(|(source, _)| source == name) {
                assert!(
                    !fixture.0.join(name).exists(),
                    "Retained verified source {name} after {stage}"
                );
                continue;
            }
            assert_eq!(
                fs::read(fixture.0.join(name)).unwrap(),
                *bytes,
                "Changed {name} after {stage}"
            );
        }
        let paths = crate::model::Paths::new(fixture.0.clone(), None);
        let inventory = paths.read_inventory().unwrap().unwrap();
        assert_eq!(inventory.apps[0].installation_label, "B");
        assert_eq!(inventory.apps[0].version, "1.2.3");
        assert!(Path::new(&inventory.apps[0].path).is_file());
        assert_eq!(
            crate::files::read_json::<Value>(&paths.at("installations.json")).unwrap()
                ["futureInventory"]["keep"],
            4
        );
    }
}

#[test]
fn invalid_legacy_sources_are_preserved_and_can_be_repaired_then_retried() {
    for &(name, section) in SOURCES {
        let fixture = Fixture::new();
        let originals = fixture.seed();
        crate::files::write_bytes(&fixture.0.join(name), b"{incomplete").unwrap();
        assert!(
            ensure_migrated(&fixture.store()).is_err(),
            "Accepted {name}"
        );
        assert!(!fixture.store().path().exists());
        assert!(!fixture.0.join(JOURNAL).exists());
        assert_eq!(fs::read(fixture.0.join(name)).unwrap(), b"{incomplete");
        for (other, bytes) in &originals {
            if other != name {
                assert_eq!(fs::read(fixture.0.join(other)).unwrap(), *bytes);
            }
        }
        crate::files::write_json(
            &fixture.0.join(name),
            &if section == "custom_apps" {
                json!([])
            } else {
                json!({})
            },
        )
        .unwrap();
        assert!(
            ensure_migrated(&fixture.store()).is_ok(),
            "Could not retry {name}"
        );
    }
}

#[cfg(windows)]
#[test]
fn blocked_config_replacement_preserves_original_and_retry_succeeds() {
    use std::os::windows::fs::OpenOptionsExt;
    let fixture = Fixture::new();
    let originals = fixture.seed();
    let snapshot = ensure_migrated(&fixture.store()).unwrap().snapshot;
    let yaml = fs::read(fixture.store().path()).unwrap();
    // Permit readers, but deny replacing the file while this handle is open.
    let blocked = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(fixture.store().path())
        .unwrap();
    let patch = super::super::ConfigPatch {
        fields: vec![super::super::FieldPatch {
            path: vec!["manager".into(), "theme".into()],
            before: Some(json!("light")),
            after: Some(json!("dark")),
        }],
    };
    assert!(fixture.store().commit(patch.clone()).is_err());
    assert_eq!(fs::read(fixture.store().path()).unwrap(), yaml);
    assert_eq!(fixture.store().load().unwrap().document, snapshot.document);
    drop(blocked);
    assert!(fixture.store().commit(patch).unwrap().changed);
    assert_eq!(
        fixture.store().load().unwrap().document["manager"]["theme"],
        "dark"
    );
    for (name, bytes) in originals {
        if SOURCES.iter().any(|(source, _)| *source == name) {
            assert!(!fixture.0.join(name).exists());
            continue;
        }
        assert_eq!(fs::read(fixture.0.join(name)).unwrap(), bytes);
    }
}

#[cfg(windows)]
#[test]
fn locked_legacy_source_is_retained_until_cleanup_can_retry() {
    use std::os::windows::fs::OpenOptionsExt;
    let fixture = Fixture::new();
    fixture.seed();
    let path = fixture.0.join("manager-settings.json");
    let original = fs::read(&path).unwrap();
    let blocked = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&path)
        .unwrap();
    let first = ensure_migrated(&fixture.store()).unwrap();
    assert_eq!(first.snapshot.document["manager"]["theme"], "light");
    assert!(first
        .warnings
        .iter()
        .any(|warning| warning.contains("manager-settings.json")));
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(fixture.0.join(JOURNAL).exists());
    assert!(!fixture.0.join("builder-settings.json").exists());
    drop(blocked);
    let retry = ensure_migrated(&fixture.store()).unwrap();
    assert_eq!(retry.snapshot.document, first.snapshot.document);
    assert!(retry.warnings.is_empty());
    assert!(!path.exists());
    assert!(!fixture.0.join(JOURNAL).exists());
}
