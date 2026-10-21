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
    pub keep_app_backups: bool,
    pub keep_source_backups: bool,
    pub compress_backups: bool,
    pub compress_source_backups: bool,
    pub notify_updates: bool,
    pub backup_versions: usize,
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
        Self {
            theme: Theme::System,
            close_to_tray: false,
            hide_activity_badge: false,
            batch_admin: true,
            keep_app_backups: false,
            keep_source_backups: false,
            compress_backups: true,
            compress_source_backups: true,
            notify_updates: true,
            backup_versions: 1,
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
            app_order: apps(),
            home_app_order: apps(),
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

impl Preferences {
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
        self.backup_versions = self.backup_versions.clamp(1, 10);
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
        for name in &apps {
            if home_seen.insert(name.clone()) {
                self.home_app_order.push(name.clone());
            }
        }
        for name in apps {
            if seen.insert(name.clone()) {
                self.app_order.push(name);
            }
        }
        Ok(adopted)
    }
}
