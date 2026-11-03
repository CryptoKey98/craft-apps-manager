use crate::catalog::metadata::{apps, sources};
use crate::installations::architecture::MANAGER_ARCH;
use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Apps offered before the catalog was read from GitHub. Settings written by
/// those versions treat every later app as new.
fn legacy_known_apps() -> Vec<String> {
    [
        "artcraftx",
        "cadcraft",
        "deckcraft",
        "designcraft",
        "effectcraft",
        "filmcraft",
        "gridcraft",
        "lightcraft",
        "photocraft",
        "printcraft",
        "soundcraft",
        "vectorcraft",
        "wordcraft",
    ]
    .map(String::from)
    .to_vec()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Dark,
    Light,
}

impl Theme {
    pub fn toggled(self) -> Self {
        match self {
            Self::System | Self::Dark => Self::Light,
            Self::Light => Self::Dark,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Preferences {
    pub theme: Theme,
    pub close_to_tray: bool,
    /// Presentation only; unread activity is still tracked and highlighted.
    pub hide_activity_badge: bool,
    pub batch_admin: bool,
    /// Legacy setting retained only to migrate the user's retention preference.
    pub keep_app_backups: bool,
    /// None means an older configuration: honor its explicit keepAppBackups value.
    #[serde(default)]
    pub rollback_enabled: Option<bool>,
    #[serde(default)]
    pub rollback_overrides: std::collections::BTreeMap<String, bool>,
    pub notify_updates: bool,
    /// Legacy global fields are read for compatibility, never installation authority.
    pub release_format: String,
    pub app_install_options:
        std::collections::BTreeMap<String, crate::install_options::InstallOptions>,
    pub architecture: String,
    pub selected_apps: Vec<String>,
    pub selected_sources: Vec<String>,
    pub app_order: Vec<String>,
    /// Independent ordering for Home tiles; never follows sidebar reordering.
    pub home_app_order: Vec<String>,
    /// Presentation only: does not affect installation or update selection.
    pub hidden_apps: Vec<String>,
    #[serde(alias = "checkUpdaterOnStartup")]
    pub check_manager_on_startup: bool,
    pub check_installed_apps_on_startup: bool,
    pub check_installed_apps_periodically: bool,
    pub app_check_interval_minutes: u64,
    /// Apps already offered to the user; newer catalog apps are adopted once.
    #[serde(default = "legacy_known_apps")]
    pub known_apps: Vec<String>,
    /// Discover new apps at startup, using the shared catalog cache.
    pub check_catalog_on_startup: bool,
    /// Discover new apps during scheduled app availability checks.
    pub check_catalog_with_app_updates: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        let order = default_release_order();
        Self {
            theme: Theme::System,
            close_to_tray: false,
            hide_activity_badge: false,
            batch_admin: true,
            keep_app_backups: false,
            rollback_enabled: Some(true),
            rollback_overrides: Default::default(),
            notify_updates: true,
            app_install_options: Default::default(),
            release_format: if cfg!(any(target_os = "windows", target_os = "macos")) {
                "installer"
            } else {
                "portable"
            }
            .into(),
            architecture: MANAGER_ARCH.into(),
            selected_apps: apps(),
            selected_sources: sources(),
            app_order: order.clone(),
            home_app_order: order,
            hidden_apps: Vec::new(),
            check_manager_on_startup: true,
            check_installed_apps_on_startup: true,
            check_installed_apps_periodically: true,
            app_check_interval_minutes: 20,
            known_apps: crate::catalog::all().into_iter().map(|e| e.key).collect(),
            check_catalog_on_startup: true,
            check_catalog_with_app_updates: false,
        }
    }
}

/// Defaults may be read after the discovery cache is loaded. Give discoveries
/// their initial position here too, without pinning any explicitly saved order.
fn default_release_order() -> Vec<String> {
    let builtins: std::collections::BTreeSet<_> = crate::catalog::builtin()
        .into_iter()
        .map(|entry| entry.key)
        .collect();
    let discoveries: std::collections::BTreeSet<_> = crate::catalog::all()
        .into_iter()
        .filter(|entry| {
            entry.release && !entry.repository.contains('/') && !builtins.contains(&entry.key)
        })
        .map(|entry| entry.key)
        .collect();
    let mut order = apps();
    order.sort_by_key(|name| !discoveries.contains(name));
    order
}

impl Preferences {
    pub fn rollback_enabled(&self) -> bool {
        self.rollback_enabled.unwrap_or(self.keep_app_backups)
    }

    pub fn validate(&mut self) -> Result<()> {
        self.adopt_new_apps()?;
        Ok(())
    }
    /// Normalizes the settings against the current catalog. Returns `true`
    /// when newly published apps were adopted and the settings should be saved.
    pub fn adopt_new_apps(&mut self) -> Result<bool> {
        self.app_check_interval_minutes = self.app_check_interval_minutes.clamp(10, 60);
        for options in self.app_install_options.values() {
            options.validate()?;
        }
        let apps: Vec<_> = crate::catalog::known_entries()
            .into_iter()
            .filter(|e| e.release)
            .map(|e| e.key)
            .collect();
        let sources: Vec<_> = crate::catalog::known_entries()
            .into_iter()
            .filter(|e| e.source)
            .map(|e| e.key)
            .collect();
        let builtins: std::collections::BTreeSet<_> = crate::catalog::builtin()
            .into_iter()
            .map(|entry| entry.key)
            .collect();
        let new_discoveries: std::collections::BTreeSet<_> = crate::catalog::all()
            .into_iter()
            .filter(|entry| {
                entry.release && !entry.repository.contains('/') && !builtins.contains(&entry.key)
            })
            .map(|entry| entry.key)
            .collect();
        let mut adopted = false;
        for entry in crate::catalog::all() {
            if self.known_apps.contains(&entry.key) {
                continue;
            }
            adopted = true;
            self.known_apps.push(entry.key);
        }
        self.known_apps.sort();
        self.known_apps.dedup();
        self.selected_apps.retain(|s| apps.contains(s));
        self.selected_apps.sort();
        self.selected_apps.dedup();
        self.selected_sources.retain(|s| sources.contains(s));
        self.selected_sources.sort();
        self.selected_sources.dedup();
        let mut seen = std::collections::BTreeSet::new();
        self.app_order
            .retain(|s| apps.contains(s) && seen.insert(s.clone()));
        let mut home_seen = std::collections::BTreeSet::new();
        self.home_app_order
            .retain(|s| apps.contains(s) && home_seen.insert(s.clone()));
        let mut sidebar_front = 0;
        let mut home_front = 0;
        // Only missing order entries are promoted, including formerly source-only
        // apps gaining their first release. Existing manual positions stay intact.
        for name in apps {
            if home_seen.insert(name.clone()) {
                if new_discoveries.contains(&name) {
                    self.home_app_order.insert(home_front, name.clone());
                    home_front += 1;
                } else {
                    self.home_app_order.push(name.clone());
                }
            }
            if seen.insert(name.clone()) {
                if new_discoveries.contains(&name) {
                    self.app_order.insert(sidebar_front, name);
                    sidebar_front += 1;
                } else {
                    self.app_order.push(name);
                }
            }
        }
        Ok(adopted)
    }
}

#[cfg(test)]
mod rollback_preference_tests {
    use super::*;
    #[test]
    fn fresh_rollback_default_and_explicit_legacy_preferences_are_preserved() {
        assert!(Preferences::default().rollback_enabled());
        let disabled: Preferences =
            serde_json::from_str(r#"{"keepAppBackups":false,"compressBackups":true}"#).unwrap();
        assert!(!disabled.rollback_enabled());
        let enabled: Preferences = serde_json::from_str(r#"{"keepAppBackups":true}"#).unwrap();
        assert!(enabled.rollback_enabled());
        let explicit: Preferences =
            serde_json::from_str(r#"{"keepAppBackups":false,"rollbackEnabled":true}"#).unwrap();
        assert!(explicit.rollback_enabled());
        assert!(explicit.rollback_overrides.is_empty());
    }
    #[test]
    fn fresh_configuration_enables_rollback_and_override_roundtrips() {
        let root =
            std::env::temp_dir().join(format!("rollback-preferences-{}", uuid::Uuid::new_v4()));
        let paths = crate::model::Paths::new(root.clone(), None);
        let mut preferences = paths.read_preferences().unwrap();
        assert!(preferences.rollback_enabled());
        preferences
            .rollback_overrides
            .insert(uuid::Uuid::nil().to_string(), false);
        let decoded: Preferences =
            serde_json::from_value(serde_json::to_value(&preferences).unwrap()).unwrap();
        assert_eq!(decoded.rollback_overrides, preferences.rollback_overrides);
        let _ = std::fs::remove_dir_all(root);
    }
}
