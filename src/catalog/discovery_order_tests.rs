use super::*;
use crate::model::{Paths, Preferences, Theme};

struct Library(std::path::PathBuf);
impl Library {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("craft-discovery-order-{}", uuid::Uuid::new_v4()));
        load(&root);
        Self(root)
    }
    fn catalog(&self, entries: Vec<Entry>) {
        files::write_json(
            &cache_path(&self.0),
            &Cache {
                entries,
                ..Default::default()
            },
        )
        .unwrap();
        load(&self.0);
    }
    fn paths(&self) -> Paths {
        Paths::new(self.0.clone(), None)
    }
}
impl Drop for Library {
    fn drop(&mut self) {
        load(&self.0.join("empty"));
        if self.0.exists() {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
}

fn recent_app(key: &str) -> Entry {
    Entry {
        discovery_added_at: Some(chrono::Utc::now().timestamp() - 3600),
        ..craft(key, key, "Desktop app", "")
    }
}

fn old_preferences() -> serde_json::Value {
    let mut value = serde_json::to_value(Preferences::default()).unwrap();
    // Both 1.0 JSON and the initial 1.1 YAML lack a placement checkpoint.
    value
        .as_object_mut()
        .unwrap()
        .remove("discoveryOrderedApps");
    value["appOrder"] = serde_json::json!([
        "filmcraft",
        "texturecraft",
        "designcraft",
        "figcraft",
        "materialcraft"
    ]);
    value["homeAppOrder"] = serde_json::json!([
        "designcraft",
        "filmcraft",
        "figcraft",
        "texturecraft",
        "materialcraft"
    ]);
    value["knownApps"] = serde_json::json!([
        "filmcraft",
        "designcraft",
        "figcraft",
        "texturecraft",
        "materialcraft"
    ]);
    value["theme"] = serde_json::json!("dark");
    value["hiddenApps"] = serde_json::json!(["figcraft"]);
    value
}

fn assert_upgrade_placement(preferences: &Preferences) {
    for order in [&preferences.app_order, &preferences.home_app_order] {
        assert_eq!(
            &order[..3],
            &["materialcraft", "solidcraft", "texturecraft"]
        );
        assert_eq!(
            order
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            order.len()
        );
    }
    assert_eq!(preferences.app_order[3], "filmcraft");
    assert_eq!(preferences.home_app_order[3], "designcraft");
    assert_eq!(preferences.theme, Theme::Dark);
    assert_eq!(preferences.hidden_apps, ["figcraft"]);
}

#[test]
fn upgrading_1_0_promotes_appended_discoveries_once_and_preserves_later_sorting() {
    let library = Library::new();
    files::write_json(&library.0.join("manager-settings.json"), &old_preferences()).unwrap();
    library.catalog(vec![
        craft("figcraft", "FigCraft", "", ""),
        recent_app("texturecraft"),
        recent_app("materialcraft"),
        recent_app("solidcraft"),
    ]);
    let paths = library.paths();
    let preferences = paths.preferences().unwrap();
    assert_upgrade_placement(&preferences);

    crate::settings::reorder(&paths, "texturecraft", "filmcraft", false).unwrap();
    let sidebar = crate::settings::reorder(&paths, "materialcraft", "designcraft", false).unwrap();
    let home = crate::settings::reorder_home(&paths, "solidcraft", "figcraft", false).unwrap();
    let expected_sidebar = sidebar.app_order;
    let expected_home = home.home_app_order;
    load(&library.0);
    let restarted = paths.preferences().unwrap();
    assert_eq!(restarted.app_order, expected_sidebar);
    assert_eq!(restarted.home_app_order, expected_home);
    assert_eq!(paths.preferences().unwrap().app_order, expected_sidebar);

    // Later discoveries are still promoted without undoing the earlier manual sort.
    library.catalog(vec![
        craft("figcraft", "FigCraft", "", ""),
        recent_app("texturecraft"),
        recent_app("materialcraft"),
        recent_app("solidcraft"),
        recent_app("futurecraft"),
    ]);
    let refreshed = paths.preferences().unwrap();
    assert_eq!(refreshed.app_order[0], "futurecraft");
    assert_eq!(refreshed.app_order[1..], expected_sidebar);
    assert_eq!(refreshed.home_app_order[0], "futurecraft");
    assert_eq!(refreshed.home_app_order[1..], expected_home);
}

#[test]
fn already_migrated_yaml_promotes_recent_discoveries_without_reordering_established_apps() {
    let library = Library::new();
    let mut old = old_preferences();
    old["futurePreference"] = serde_json::json!({"enabled": true});
    let store = crate::storage::config::ConfigStore::new(library.0.clone());
    store
        .mutate_section::<serde_json::Value>("manager", |manager| {
            *manager = old;
            Ok(())
        })
        .unwrap();
    library.catalog(vec![
        craft("figcraft", "FigCraft", "", ""),
        recent_app("texturecraft"),
        recent_app("materialcraft"),
        recent_app("solidcraft"),
    ]);
    assert_upgrade_placement(&library.paths().preferences().unwrap());
    assert_eq!(
        store.load().unwrap().document["manager"]["futurePreference"]["enabled"],
        true
    );
}

#[test]
fn legacy_order_keeps_established_and_custom_apps_and_waits_for_official_admission() {
    let library = Library::new();
    let mut old = old_preferences();
    let order = serde_json::json!([
        "filmcraft",
        "expiredcraft",
        "undatedcraft",
        "customcraft",
        "materialcraft"
    ]);
    old["appOrder"] = order.clone();
    old["homeAppOrder"] = order;
    let store = crate::storage::config::ConfigStore::new(library.0.clone());
    store
        .mutate_section::<serde_json::Value>("manager", |manager| {
            *manager = old;
            Ok(())
        })
        .unwrap();
    let mut custom = recent_app("customcraft");
    custom.repository = "someone/customcraft".into();
    store
        .mutate_section::<Vec<Entry>>("custom_apps", |entries| {
            entries.push(custom);
            Ok(())
        })
        .unwrap();
    let mut expired = recent_app("expiredcraft");
    expired.discovery_added_at = Some(chrono::Utc::now().timestamp() - 15 * 86400);
    let undated = craft("undatedcraft", "UndatedCraft", "", "");
    library.catalog(vec![expired.clone(), undated, recent_app("materialcraft")]);
    let preferences = library.paths().preferences().unwrap();
    let expected = [
        "materialcraft",
        "filmcraft",
        "expiredcraft",
        "undatedcraft",
        "customcraft",
    ];
    assert_eq!(&preferences.app_order[..5], &expected);
    assert_eq!(&preferences.home_app_order[..5], &expected);

    // Admission metadata can arrive after an older discovery cache is loaded.
    library.catalog(vec![
        expired,
        recent_app("undatedcraft"),
        recent_app("materialcraft"),
    ]);
    let preferences = library.paths().preferences().unwrap();
    let expected = [
        "undatedcraft",
        "materialcraft",
        "filmcraft",
        "expiredcraft",
        "customcraft",
    ];
    assert_eq!(&preferences.app_order[..5], &expected);
    assert_eq!(&preferences.home_app_order[..5], &expected);
}

#[test]
fn newly_installable_discovery_without_admission_metadata_is_placed_once() {
    let library = Library::new();
    let mut entry = craft("newreleasecraft", "NewReleaseCraft", "", "");
    entry.release = false;
    library.catalog(vec![entry.clone()]);
    let paths = library.paths();
    let preferences = paths.preferences().unwrap();
    assert!(preferences.known_apps.contains(&entry.key));
    assert!(!preferences.app_order.contains(&entry.key));
    entry.release = true;
    library.catalog(vec![entry]);
    let preferences = paths.preferences().unwrap();
    assert_eq!(preferences.app_order[0], "newreleasecraft");
    assert_eq!(preferences.home_app_order[0], "newreleasecraft");
    let sidebar = crate::settings::reorder(&paths, "newreleasecraft", "filmcraft", false).unwrap();
    let home =
        crate::settings::reorder_home(&paths, "newreleasecraft", "designcraft", false).unwrap();
    load(&library.0);
    let restarted = paths.preferences().unwrap();
    assert_eq!(restarted.app_order, sidebar.app_order);
    assert_eq!(restarted.home_app_order, home.home_app_order);
}
