use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MANAGER_ARCH: &str = if cfg!(target_arch = "x86") {
    "x86"
} else if cfg!(target_arch = "aarch64") {
    "arm64"
} else {
    "x64"
};

/// Universal macOS bundles match all saved architecture selections.
pub fn installed_architecture_matches(installed: &Installed, prefs: &Preferences) -> bool {
    if installed.architecture.is_empty() {
        return false;
    }
    let arch = &installed.architecture;
    release_arch(arch) == release_arch(&prefs.architecture)
}
/// Native installer architectures are mutually exclusive in the manager.
pub fn installer_architecture_conflict(installed: &Installed, prefs: &Preferences) -> bool {
    prefs.release_format == "installer"
        && installed.install_kind == "installer"
        && !installed.path.is_empty()
        && !installed.version.is_empty()
        && !installed_architecture_matches(installed, prefs)
}
pub fn installer_architecture_conflict_message(
    installed: &Installed,
    prefs: &Preferences,
) -> String {
    let arch = if installed.architecture.is_empty() {
        MANAGER_ARCH
    } else {
        &installed.architecture
    };
    format!(
        "Uninstall the {} version before installing {}. Select {} to manage it.",
        architecture_label(arch),
        architecture_label(&prefs.architecture),
        architecture_label(arch)
    )
}
/// Apps with installable releases, from the current catalog.
pub fn apps() -> Vec<String> {
    crate::catalog::release_keys()
}
/// Keeps saved ordering across formats, displaying only apps registered for this format.
pub fn ordered_release_apps(order: &[String], format: &str) -> Vec<String> {
    let _ = format;
    let eligible: std::collections::BTreeSet<_> = crate::catalog::known_entries()
        .into_iter()
        .filter(|entry| entry.release)
        .map(|entry| entry.key)
        .collect();
    order
        .iter()
        .filter(|name| eligible.contains(*name))
        .cloned()
        .collect()
}
/// Apps whose source can be downloaded and built, from the current catalog.
pub fn sources() -> Vec<String> {
    crate::catalog::source_keys()
}
pub fn title(name: &str) -> String {
    crate::catalog::get(name)
        .map(|e| e.title)
        .unwrap_or_else(|| crate::catalog::pretty(name))
}
/// The upstream repository as an `owner/name` slug. Storytold apps store a
/// bare name in the catalog; third-party apps store a full slug.
pub fn repository(name: &str) -> String {
    crate::catalog::get(name)
        .map(|e| {
            if e.repository.contains('/') {
                e.repository
            } else {
                format!("{}/{}", crate::catalog::ORG, e.repository)
            }
        })
        .unwrap_or_else(|| format!("{}/{}", crate::catalog::ORG, name))
}
/// The bare repository name, without any `owner/` prefix. Used where a single
/// path segment or file name is required, such as source ZIP folder names.
pub fn repository_name(name: &str) -> String {
    crate::catalog::get(name)
        .map(|e| crate::catalog::repository_name(&e.repository).to_string())
        .unwrap_or_else(|| name.into())
}
/// Short category shown under each app, taken from the upstream repository description.
pub fn category(name: &str) -> String {
    crate::catalog::get(name)
        .map(|e| e.category)
        .unwrap_or_default()
}
/// The upstream repository description, shown on each app page.
pub fn description(name: &str) -> String {
    crate::catalog::get(name)
        .map(|e| e.description)
        .unwrap_or_default()
}
pub fn valid_app(name: &str) -> Result<()> {
    if crate::catalog::get(name).is_none() {
        bail!("Unknown app: {name}");
    }
    Ok(())
}
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
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BuilderPreferences {
    pub delete_cache_after_success: bool,
    pub delete_workspace_after_success: bool,
    #[serde(rename = "logSizeMB")]
    pub log_size_mb: u64,
    pub log_archives: usize,
}
impl Default for BuilderPreferences {
    fn default() -> Self {
        Self {
            delete_cache_after_success: true,
            delete_workspace_after_success: true,
            log_size_mb: 10,
            log_archives: 2,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Installed {
    pub name: String,
    pub version: String,
    pub path: String,
    #[serde(default)]
    pub architecture: String,
    #[serde(default)]
    pub install_kind: String,
    #[serde(default)]
    pub product_code: String,
}
fn default_arch() -> String {
    MANAGER_ARCH.into()
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub apps_root: String,
    pub apps: Vec<Installed>,
    #[serde(default)]
    pub installations: Vec<Installed>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub sha: String,
    pub branch: String,
    pub repository: String,
    pub archive_sha256: String,
    pub downloaded_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub browser_download_url: String,
    #[serde(default)]
    pub digest: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    pub draft: bool,
    pub prerelease: bool,
    pub assets: Vec<Asset>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildInfo {
    pub app: String,
    pub commit: String,
    pub source_branch: String,
    pub built_at: String,
    pub profile: String,
    pub log: String,
}
#[derive(Clone)]
pub struct Paths {
    pub root: PathBuf,
    pub tools: PathBuf,
}
impl Paths {
    pub fn new(root: PathBuf, tools: Option<PathBuf>) -> Self {
        let tools = tools.unwrap_or_else(|| root.join("workspace/tools"));
        Self { root, tools }
    }
    pub fn at(&self, name: impl AsRef<Path>) -> PathBuf {
        self.root.join(name)
    }
    pub fn config(&self) -> Result<Config> {
        #[cfg(target_os = "windows")]
        {
            let preferences = self.read_preferences()?;
            self.config_with_detector(|app| {
                crate::installers::detect_for_architecture(
                    app,
                    Some(&crate::install_options::preferred(&preferences, app).architecture),
                )?
                .map_or_else(
                    || crate::installers::detect(app),
                    |installed| Ok(Some(installed)),
                )
            })
        }
        #[cfg(not(target_os = "windows"))]
        self.config_with_detector(crate::installers::detect)
    }
    fn config_with_detector(
        &self,
        detect: impl Fn(&str) -> Result<Option<Installed>>,
    ) -> Result<Config> {
        if self.at("settings.json").exists() {
            let config: Config = crate::files::read_json(&self.at("settings.json"))?;
            if !Path::new(&config.apps_root).eq(&self.root) {
                bail!("settings.json points to a different data folder. Select that folder in Settings.");
            }
            return self.refresh_config(config, &detect);
        }
        let mut apps = Vec::new();
        for name in self::apps() {
            let name = name.as_str();
            let mut found: Vec<_> = std::fs::read_dir(self.at("releases"))
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| {
                    e.path().is_dir()
                        && e.file_name()
                            .to_string_lossy()
                            .starts_with(&format!("{name}-"))
                })
                .filter_map(|e| {
                    let label = e.file_name().to_string_lossy().into_owned();
                    let version = label
                        .strip_prefix(&format!("{name}-"))?
                        .split(&format!("-{}-", release_os()))
                        .next()?
                        .to_string();
                    crate::updates::version(&version)
                        .ok()
                        .map(|v| (v, version, e.path()))
                })
                .collect();
            found.sort_by_key(|v| v.0);
            let (version, path) = found
                .pop()
                .map(|(_, v, p)| (v, p.to_string_lossy().into_owned()))
                .unwrap_or_default();
            let direct = self.at(format!("releases/{name}"));
            apps.push(Installed {
                name: name.into(),
                version,
                path: if path.is_empty() && installed_executable(&direct, name).is_some() {
                    direct.to_string_lossy().into_owned()
                } else {
                    path
                },
                architecture: String::new(),
                install_kind: String::new(),
                product_code: String::new(),
            });
        }
        self.refresh_config(
            Config {
                apps_root: self.root.to_string_lossy().into_owned(),
                apps,
                installations: Vec::new(),
            },
            &detect,
        )
    }
    fn refresh_config(
        &self,
        mut config: Config,
        detect: &impl Fn(&str) -> Result<Option<Installed>>,
    ) -> Result<Config> {
        let preferences = self.read_preferences()?;

        // Add catalog entries when upgrading an existing library without
        // changing saved selections, installations, or the user's app order.
        for name in apps() {
            if !config.apps.iter().any(|app| app.name == name) {
                config.apps.push(Installed {
                    name,
                    architecture: default_arch(),
                    ..Default::default()
                });
            }
        }
        for app in &config.apps {
            if !app.path.is_empty()
                && !config.installations.iter().any(|a| {
                    a.name == app.name
                        && a.path == app.path
                        && (a.install_kind == "installer") == (app.install_kind == "installer")
                })
            {
                config.installations.push(app.clone());
            }
        }
        for app in &mut config.apps {
            let selected_path = app.path.clone();
            let selected_kind = app.install_kind.clone();
            let preferred = crate::install_options::preferred(&preferences, &app.name);
            let has_portable = config.installations.iter().any(|copy| {
                copy.name == app.name
                    && copy.install_kind != "installer"
                    && installed_executable(Path::new(&copy.path), &copy.name).is_some()
            });
            let detected = detect(&app.name)?;
            if let Some(native) = &detected {
                config
                    .installations
                    .retain(|copy| !(copy.name == app.name && copy.install_kind == "installer"));
                config.installations.push(native.clone());
            }
            // Legacy discovery supplied a path but no package type/version. Recover
            // only verified portable files: unknown external folders stay unknown,
            // and a detected native installation must never become portable.
            for copy in config.installations.iter_mut().filter(|copy| {
                copy.name == app.name
                    && !copy.path.is_empty()
                    && (copy.install_kind == "portable"
                        || (copy.install_kind.is_empty()
                            && Path::new(&copy.path).starts_with(self.at("releases"))))
                    && detected
                        .as_ref()
                        .is_none_or(|native| Path::new(&native.path) != Path::new(&copy.path))
            }) {
                if let Some(executable) = installed_executable(Path::new(&copy.path), &copy.name) {
                    copy.install_kind = "portable".into();
                    if copy.version.is_empty() {
                        copy.version = crate::platform::executable_version(&executable)
                            .unwrap_or_else(|| "0.0.0".into());
                    }
                    if copy.architecture.is_empty() {
                        copy.architecture =
                            crate::install_options::executable_architecture(&executable)
                                .unwrap_or_default();
                    }
                }
            }
            let library = self.at(format!("releases/{}", app.name));
            if !config
                .installations
                .iter()
                .any(|copy| copy.name == app.name && Path::new(&copy.path) == library)
            {
                if let Some(executable) = installed_executable(&library, &app.name) {
                    config.installations.push(Installed {
                        name: app.name.clone(),
                        path: library.display().to_string(),
                        version: crate::platform::executable_version(&executable)
                            .unwrap_or_else(|| "0.0.0".into()),
                        architecture: crate::install_options::executable_architecture(&executable)
                            .unwrap_or_default(),
                        install_kind: "portable".into(),
                        product_code: String::new(),
                    });
                }
            }
            let library_portable = detected.is_none()
                && installed_executable(&self.at(format!("releases/{}", app.name)), &app.name)
                    .is_some();
            // Native inventory must reflect detection even when a portable
            // installation is selected for display.
            if detected.is_none() {
                config.installations.retain(|record| {
                    !(record.name == app.name && record.install_kind == "installer")
                });
            }
            // The selected copy is installation identity, not pending package preferences.
            // Changing install options must never silently switch an existing copy.
            if let Some(selected) = config.installations.iter().find(|copy| {
                !selected_kind.is_empty()
                    && copy.name == app.name
                    && copy.path == selected_path
                    && copy.install_kind == selected_kind
                    && !copy.path.is_empty()
                    && installed_executable(Path::new(&copy.path), &copy.name).is_some()
            }) {
                *app = selected.clone();
                continue;
            }
            let installer =
                preferred.release_format == "installer" || (!has_portable && detected.is_some());
            if installer && (detected.is_some() || (!has_portable && !library_portable)) {
                if let Some(record) = detected {
                    config.installations.retain(|a| {
                        !(a.name == app.name
                            && a.install_kind == "installer"
                            && (a.architecture == record.architecture || a.path == record.path))
                    });
                    config.installations.push(record.clone());
                    *app = record;
                } else {
                    config.installations.retain(|record| {
                        !(record.name == app.name && record.install_kind == "installer")
                    });
                    app.path.clear();
                    app.version.clear();
                    app.product_code.clear();
                    app.install_kind = "installer".into();
                }
            } else if let Some(record) = config
                .installations
                .iter()
                .filter(|a| {
                    a.name == app.name
                        && a.install_kind != "installer"
                        && installed_executable(Path::new(&a.path), &a.name).is_some()
                })
                .max_by_key(|a| installed_architecture_matches(a, &preferred))
            {
                *app = record.clone();
            } else {
                let root = self.at(format!("releases/{}", app.name));
                if let Some(executable) = installed_executable(&root, &app.name) {
                    app.path = root.display().to_string();
                    app.version = crate::platform::executable_version(&executable)
                        .unwrap_or_else(|| "0.0.0".into());
                    app.install_kind = "portable".into();
                    app.product_code.clear();
                    config.installations.push(app.clone());
                } else {
                    app.path.clear();
                    app.version.clear();
                    app.install_kind = "portable".into();
                    app.product_code.clear();
                }
            }
        }
        for app in &mut config.apps {
            if app.architecture.is_empty() && !app.path.is_empty() {
                app.architecture = installed_executable(Path::new(&app.path), &app.name)
                    .and_then(|exe| crate::install_options::executable_architecture(&exe))
                    .unwrap_or_default();
            }
        }
        Ok(config)
    }
    pub fn save_config(&self, config: &Config) -> Result<()> {
        let mut config = config.clone();
        for app in &config.apps {
            let installer = app.install_kind == "installer";
            config.installations.retain(|a| {
                !(a.name == app.name
                    && (a.install_kind == "installer") == installer
                    && a.path == app.path)
            });
            if !app.path.is_empty() {
                config.installations.push(app.clone());
            }
        }
        crate::files::write_json(&self.at("settings.json"), &config)
    }
    /// Read preferences without migrating or changing any file.
    pub fn read_preferences(&self) -> Result<Preferences> {
        let target = self.at("manager-settings.json");
        let legacy = self.at("updater-settings.json");
        let mut p =
            crate::files::read_or_default::<Preferences>(if !target.exists() && legacy.exists() {
                &legacy
            } else {
                &target
            })?;
        p.validate()?;
        Ok(p)
    }
    /// Verified alternate-format facts for display only; action inventory remains unchanged.
    pub fn alternate_installations(&self, config: &Config) -> Result<Vec<Installed>> {
        self.alternates_with_detector(config, crate::installers::detect)
    }
    fn alternates_with_detector(
        &self,
        config: &Config,
        detect: impl Fn(&str) -> Result<Option<Installed>>,
    ) -> Result<Vec<Installed>> {
        let mut records = Vec::new();
        for name in apps() {
            let name = name.as_str();
            let installer = config.apps.iter().any(|app| {
                app.name == name && !app.path.is_empty() && app.install_kind == "installer"
            });
            if !installer {
                if let Some(record) = detect(name)? {
                    records.push(record);
                }
            } else {
                // Native installers may legitimately target the portable library.
                // A directory location alone is not evidence of another installation.
                let native_owns = |folder: &Path| {
                    config
                        .apps
                        .iter()
                        .chain(config.installations.iter())
                        .any(|record| {
                            record.name == name
                                && record.install_kind == "installer"
                                && !record.path.is_empty()
                                && (Path::new(&record.path) == folder
                                    || std::fs::canonicalize(&record.path)
                                        .ok()
                                        .zip(std::fs::canonicalize(folder).ok())
                                        .is_some_and(|(native, candidate)| native == candidate))
                        })
                };
                let recorded = config.installations.iter().find(|a| {
                    a.name == name
                        && a.install_kind != "installer"
                        && !a.path.is_empty()
                        && !native_owns(Path::new(&a.path))
                        && installed_executable(Path::new(&a.path), name).is_some()
                });
                if let Some(record) = recorded {
                    records.push(record.clone());
                } else {
                    let folder = self.at(format!("releases/{name}"));
                    if native_owns(&folder) {
                        continue;
                    }
                    if let Some(executable) = installed_executable(&folder, name) {
                        records.push(Installed {
                            name: name.into(),
                            path: folder.display().to_string(),
                            version: crate::platform::executable_version(&executable)
                                .unwrap_or_else(|| "0.0.0".into()),
                            install_kind: "portable".into(),
                            architecture: default_arch(),
                            ..Default::default()
                        });
                    }
                }
            }
        }
        Ok(records)
    }
    pub fn preferences(&self) -> Result<Preferences> {
        let target = self.at("manager-settings.json");
        let legacy = self.at("updater-settings.json");
        let migrate = !target.exists() && legacy.exists();
        let mut p =
            crate::files::read_or_default::<Preferences>(if migrate { &legacy } else { &target })?;
        let adopted = p.adopt_new_apps()?;
        if migrate {
            crate::files::write_json(&target, &p)?;
            std::fs::remove_file(legacy)?;
        } else if adopted && target.exists() {
            // Remember discovered apps without changing update selections.
            crate::files::write_json(&target, &p)?;
        }
        Ok(p)
    }
    pub fn builder_preferences(&self) -> Result<BuilderPreferences> {
        let mut p =
            crate::files::read_or_default::<BuilderPreferences>(&self.at("builder-settings.json"))?;
        p.log_size_mb = p.log_size_mb.clamp(1, 100);
        p.log_archives = p.log_archives.min(5);
        Ok(p)
    }
}

/// The launchable item a release installs. On macOS this is an `.app` bundle
/// directory such as `PhotoCraft.app`; elsewhere it is a single executable file.
pub fn executable_name(app: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{app}.exe")
    } else if cfg!(target_os = "macos") {
        if app == "cadcraft" {
            return "CADCraft.app".into();
        }
        format!(
            "{}{}.app",
            app[..1].to_uppercase(),
            app[1..].replace("craft", "Craft")
        )
    } else {
        app.into()
    }
}
/// The bare executable produced by a source build.
pub fn build_executable_name(app: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{app}.exe")
    } else {
        app.into()
    }
}
/// Whether a release item exists. macOS app bundles are directories.
pub fn is_executable(path: &Path) -> bool {
    if cfg!(target_os = "macos") {
        path.exists()
    } else {
        path.is_file()
    }
}
pub fn executable_names(app: &str) -> Vec<String> {
    let mut names: Vec<String> = crate::catalog::names(app)
        .iter()
        .map(|name| executable_name(name))
        .collect();
    if crate::catalog::get(app).is_some_and(|entry| entry.scheme == crate::catalog::Scheme::Generic)
    {
        names = desktop_executable_names(names);
    }
    names.dedup();
    // Prefer the current release name on Unix; keep older names as fallbacks.
    // Windows installer resolution is handled separately.
    if cfg!(any(target_os = "macos", target_os = "linux")) {
        names.reverse();
    }
    names
}
// Generic desktop projects often suffix the GUI binary while shipping separate CLI tools.
// Keep the accepted suffixes explicit so daemon/helper binaries are never selected.
fn desktop_executable_names(mut names: Vec<String>) -> Vec<String> {
    let originals = names.clone();
    for name in originals {
        let (stem, extension) = name
            .strip_suffix(".exe")
            .map(|stem| (stem, ".exe"))
            .unwrap_or((&name, ""));
        for suffix in [
            "-qt",
            "-qt5",
            "-qt6",
            "-gui",
            "-desktop",
            "-client",
            ".Launcher",
            "-launcher",
        ] {
            names.push(format!("{stem}{suffix}{extension}"));
        }
    }
    names
}
#[cfg(test)]
mod desktop_executable_tests {
    #[test]
    fn desktop_variants_keep_exact_names_and_exclude_background_tools() {
        let names = super::desktop_executable_names(vec!["transmission.exe".into()]);
        assert_eq!(names[0], "transmission.exe");
        assert!(names.contains(&"transmission-qt.exe".into()));
        assert!(!names.contains(&"transmission-daemon.exe".into()));
        assert!(!names.contains(&"transmission-remote.exe".into()));
        let launchers = super::desktop_executable_names(vec!["everythingtoolbar.exe".into()]);
        assert!(launchers.contains(&"everythingtoolbar.Launcher.exe".into()));
        assert!(launchers.contains(&"everythingtoolbar-launcher.exe".into()));
        assert!(!launchers.contains(&"unins000.exe".into()));
        assert!(!launchers.contains(&"everythingtoolbar-helper.exe".into()));
        let unix = super::desktop_executable_names(vec!["example".into()]);
        assert!(unix.contains(&"example-gui".into()));
    }
}
pub fn installed_executable(folder: &Path, app: &str) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        crate::installers::resolve_bundle(folder, app)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let executable = executable_names(app)
            .into_iter()
            .map(|name| folder.join(name))
            .find(|path| is_executable(path));
        #[cfg(target_os = "windows")]
        let executable =
            executable.or_else(|| crate::installers::registered_launch_executable(folder, app));
        executable.or_else(|| {
            if crate::catalog::get(app)?.scheme != crate::catalog::Scheme::Generic {
                return None;
            }
            let names = executable_names(app);
            let matches: Vec<_> = walkdir::WalkDir::new(folder)
                .max_depth(4)
                .into_iter()
                .filter_map(std::result::Result::ok)
                .filter(|entry| {
                    entry.file_type().is_file()
                        && names.iter().any(|name| {
                            entry
                                .file_name()
                                .to_string_lossy()
                                .eq_ignore_ascii_case(name)
                        })
                })
                .map(|entry| entry.into_path())
                .collect();
            (matches.len() == 1).then(|| matches[0].clone())
        })
    }
}
pub fn release_os() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}
pub fn architecture_label(architecture: &str) -> &str {
    if cfg!(target_os = "macos") {
        "Universal (Intel + Apple silicon)"
    } else {
        architecture
    }
}
pub fn release_arch(architecture: &str) -> &str {
    if cfg!(target_os = "macos") {
        // Upstream publishes one universal DMG for Intel and Apple silicon.
        "universal"
    } else if cfg!(target_os = "linux") {
        match architecture {
            "x64" => "x86_64",
            "x86" => "i686",
            "arm64" => "aarch64",
            other => other,
        }
    } else {
        architecture
    }
}

#[cfg(test)]
mod detection_tests {
    #[test]
    fn settings_save_preserves_same_architecture_installations_in_other_folders() {
        use super::*;
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let paths = Paths::new(root.clone(), None);
        let copies: Vec<_> = ["first", "second", "third"]
            .into_iter()
            .map(|name| {
                let folder = root.join(name);
                release_fixture(&folder, "filmcraft");
                Installed {
                    name: "filmcraft".into(),
                    path: folder.display().to_string(),
                    version: "0.6.0".into(),
                    architecture: "x64".into(),
                    install_kind: "portable".into(),
                    ..Default::default()
                }
            })
            .collect();
        paths
            .save_config(&Config {
                apps_root: root.display().to_string(),
                apps: vec![copies[0].clone()],
                installations: copies.clone(),
            })
            .unwrap();
        crate::settings::save(
            &paths,
            &root.join("locations.json"),
            &(),
            &root,
            &paths.tools,
            Some(&Preferences::default()),
            None,
        )
        .unwrap();
        let saved: Config = crate::files::read_json(&paths.at("settings.json")).unwrap();
        for copy in &copies {
            assert!(
                saved
                    .installations
                    .iter()
                    .any(|a| a.name == copy.name && a.path == copy.path),
                "Settings save lost installation {}",
                copy.path
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_versioned_portable_recovers_package_type() {
        use super::*;
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let paths = Paths::new(root.clone(), None);
        let folder = paths.at(format!("releases/filmcraft-0.6.0-{}-x64", release_os()));
        release_fixture(&folder, "filmcraft");
        let config = paths.config_with_detector(|_| Ok(None)).unwrap();
        let installed = config.apps.iter().find(|a| a.name == "filmcraft").unwrap();
        assert_eq!(Path::new(&installed.path), folder);
        assert_eq!(installed.install_kind, "portable");
        // A folder name is not sufficient proof of the executable architecture.
        #[cfg(not(target_os = "macos"))]
        assert!(installed.architecture.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discovered_portable_has_consistent_installed_state_without_settings() {
        use super::*;
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let paths = Paths::new(root.clone(), None);
        let folder = paths.at("releases/designcraft");
        release_fixture(&folder, "designcraft");
        // A minimal PE header: architecture is evidence from the executable, not preferences.
        #[cfg(target_os = "windows")]
        {
            let mut bytes = vec![0u8; 256];
            bytes[..2].copy_from_slice(b"MZ");
            bytes[60..64].copy_from_slice(&128u32.to_le_bytes());
            bytes[128..132].copy_from_slice(b"PE\0\0");
            bytes[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
            bytes[148..150].copy_from_slice(&70u16.to_le_bytes());
            bytes[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
            std::fs::write(folder.join(executable_name("designcraft")), bytes).unwrap();
        }
        for _ in 0..2 {
            let config = paths.config_with_detector(|_| Ok(None)).unwrap();
            let installed = config
                .apps
                .iter()
                .find(|a| a.name == "designcraft")
                .unwrap();
            assert!(
                !installed.version.is_empty(),
                "An existing executable must not display Get"
            );
            assert_eq!(installed.install_kind, "portable");
            #[cfg(target_os = "windows")]
            assert!(crate::install_options::InstallOptions::installed(installed).is_some());
            paths.save_config(&config).unwrap();
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn same_architecture_copies_can_be_selected_and_uninstalled_independently() {
        use super::*;
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let paths = Paths::new(root.clone(), None);
        let copies: Vec<_> = ["first", "second"]
            .into_iter()
            .map(|name| {
                let folder = root.join(name);
                release_fixture(&folder, "filmcraft");
                Installed {
                    name: "filmcraft".into(),
                    path: folder.display().to_string(),
                    version: "0.6.0".into(),
                    architecture: "x64".into(),
                    install_kind: "portable".into(),
                    ..Default::default()
                }
            })
            .collect();
        for selected in &copies {
            let config = Config {
                apps_root: root.display().to_string(),
                apps: vec![selected.clone()],
                installations: copies.clone(),
            };
            paths.save_config(&config).unwrap();
            let restarted = paths.config_with_detector(|_| Ok(None)).unwrap();
            assert_eq!(
                restarted
                    .apps
                    .iter()
                    .find(|copy| copy.name == "filmcraft")
                    .unwrap()
                    .path,
                selected.path
            );
            assert_eq!(
                restarted
                    .installations
                    .iter()
                    .filter(|copy| copy.name == "filmcraft")
                    .count(),
                2
            );
        }
        std::fs::remove_dir_all(&copies[1].path).unwrap();
        let mut config = paths.config_with_detector(|_| Ok(None)).unwrap();
        crate::apps::forget_installation(&mut config, &copies[1]);
        paths.save_config(&config).unwrap();
        let restarted = paths.config_with_detector(|_| Ok(None)).unwrap();
        assert_eq!(
            restarted
                .apps
                .iter()
                .find(|copy| copy.name == "filmcraft")
                .unwrap()
                .path,
            copies[0].path
        );
        assert_eq!(
            restarted
                .installations
                .iter()
                .filter(|copy| copy.name == "filmcraft")
                .count(),
            1
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn visibility_defaults_visible_and_preserves_update_selection_and_order() {
        let mut preferences: super::Preferences = serde_json::from_str("{}").unwrap();
        assert!(preferences.hidden_apps.is_empty());
        preferences.validate().unwrap();
        let selection = preferences.selected_apps.clone();
        let order = preferences.app_order.clone();
        preferences.hidden_apps.push("photocraft".into());
        preferences.validate().unwrap();
        let reopened: super::Preferences =
            serde_json::from_slice(&serde_json::to_vec(&preferences).unwrap()).unwrap();
        assert_eq!(reopened.hidden_apps, ["photocraft"]);
        assert_eq!(reopened.selected_apps, selection);
        assert_eq!(reopened.app_order, order);
    }
    use super::*;
    #[test]
    fn backup_defaults_are_off_and_saved_choices_are_preserved() {
        let fresh = Preferences::default();
        assert!(!fresh.keep_app_backups);
        assert!(!fresh.keep_source_backups);
        let saved: Preferences =
            serde_json::from_str(r#"{"keepAppBackups":true,"keepSourceBackups":true}"#).unwrap();
        assert!(saved.keep_app_backups);
        assert!(saved.keep_source_backups);
    }
    #[test]
    fn theme_defaults_for_existing_settings_and_survives_restart() {
        let legacy: Preferences =
            serde_json::from_str(r#"{"selectedApps":["filmcraft"]}"#).unwrap();
        assert_eq!(legacy.theme, Theme::System);
        let root = std::env::temp_dir().join(format!("craft-theme-{}", uuid::Uuid::new_v4()));
        let paths = Paths::new(root.clone(), None);
        let mut preferences = legacy;
        preferences.theme = Theme::Light;
        // Written after the newest apps were found, so none are adopted on reopening.
        preferences.known_apps = crate::catalog::all().into_iter().map(|e| e.key).collect();
        crate::files::write_json(&paths.at("manager-settings.json"), &preferences).unwrap();
        let reopened = paths.read_preferences().unwrap();
        assert_eq!(reopened.theme, Theme::Light);
        assert_eq!(reopened.selected_apps, ["filmcraft"]);
        std::fs::remove_dir_all(root).unwrap();
    }
    fn release_fixture(folder: &Path, app: &str) {
        std::fs::create_dir_all(folder).unwrap();
        let executable = folder.join(executable_name(app));
        if cfg!(target_os = "macos") {
            let contents = executable.join("Contents");
            std::fs::create_dir_all(&contents).unwrap();
            std::fs::write(contents.join("Info.plist"), format!("<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>ai.storyteller.{app}</string><key>CFBundleShortVersionString</key><string>0.4.0</string></dict></plist>")).unwrap();
        } else {
            std::fs::write(executable, b"fixture").unwrap();
        }
    }
    #[test]
    fn uninstalling_one_format_preserves_other_copy_across_restart() {
        for removed_kind in ["installer", "portable"] {
            let root = std::env::temp_dir()
                .join(format!("craft-uninstall-copies-{}", uuid::Uuid::new_v4()));
            let paths = Paths::new(root.clone(), None);
            let copies: Vec<_> = ["installer", "portable"]
                .into_iter()
                .map(|kind| {
                    let folder = root.join(kind);
                    release_fixture(&folder, "filmcraft");
                    Installed {
                        name: "filmcraft".into(),
                        version: "0.6.0".into(),
                        architecture: "x64".into(),
                        install_kind: kind.into(),
                        path: folder.display().to_string(),
                        ..Default::default()
                    }
                })
                .collect();
            let removed = copies
                .iter()
                .find(|copy| copy.install_kind == removed_kind)
                .unwrap();
            let remaining = copies
                .iter()
                .find(|copy| copy.install_kind != removed_kind)
                .unwrap();
            let prefs = Preferences {
                app_install_options: [(
                    crate::install_options::identity("filmcraft"),
                    crate::install_options::InstallOptions {
                        release_format: removed_kind.into(),
                        architecture: "x64".into(),
                    },
                )]
                .into(),
                ..Default::default()
            };
            crate::files::write_json(&paths.at("manager-settings.json"), &prefs).unwrap();
            let mut config = Config {
                apps_root: root.display().to_string(),
                apps: vec![removed.clone()],
                installations: copies.clone(),
            };
            // Simulate only the selected uninstall; never invoke an installer.
            std::fs::remove_dir_all(&removed.path).unwrap();
            crate::apps::forget_installation(&mut config, removed);
            assert_eq!(config.apps[0].path, remaining.path);
            paths.save_config(&config).unwrap();
            let reopened = paths
                .config_with_detector(|name| {
                    Ok(
                        (name == "filmcraft" && remaining.install_kind == "installer")
                            .then(|| remaining.clone()),
                    )
                })
                .unwrap();
            let active = reopened
                .apps
                .iter()
                .find(|app| app.name == "filmcraft")
                .unwrap();
            assert_eq!(active.path, remaining.path);
            assert_eq!(active.install_kind, remaining.install_kind);
            assert!(reopened
                .installations
                .iter()
                .any(|copy| copy.path == remaining.path));
            assert!(!reopened
                .installations
                .iter()
                .any(|copy| copy.path == removed.path));
            assert!(installed_executable(Path::new(&remaining.path), "filmcraft").is_some());
            std::fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn missing_old_portable_record_is_rediscovered_after_installer_removal() {
        let root =
            std::env::temp_dir().join(format!("craft-portable-recovery-{}", uuid::Uuid::new_v4()));
        let paths = Paths::new(root.clone(), None);
        let folder = paths.at("releases/filmcraft");
        release_fixture(&folder, "filmcraft");
        let prefs = Preferences {
            app_install_options: [(
                crate::install_options::identity("filmcraft"),
                crate::install_options::InstallOptions {
                    release_format: "installer".into(),
                    architecture: "x64".into(),
                },
            )]
            .into(),
            ..Default::default()
        };
        crate::files::write_json(&paths.at("manager-settings.json"), &prefs).unwrap();
        crate::files::write_json(
            &paths.at("settings.json"),
            &Config {
                apps_root: root.display().to_string(),
                apps: vec![Installed {
                    name: "filmcraft".into(),
                    install_kind: String::new(),
                    ..Default::default()
                }],
                installations: Vec::new(),
            },
        )
        .unwrap();
        let recovered = paths.config_with_detector(|_| Ok(None)).unwrap();
        let app = recovered
            .apps
            .iter()
            .find(|app| app.name == "filmcraft")
            .unwrap();
        assert_eq!(app.path, folder.display().to_string());
        assert_eq!(app.install_kind, "portable");
        paths.save_config(&recovered).unwrap();
        assert_eq!(
            paths
                .config_with_detector(|_| Ok(None))
                .unwrap()
                .apps
                .iter()
                .find(|app| app.name == "filmcraft")
                .unwrap()
                .path,
            app.path
        );
        assert!(folder.exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn native_install_in_library_is_not_a_second_portable_installation() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let paths = Paths::new(root.clone(), None);
        let folder = paths.at("releases/filmcraft");
        release_fixture(&folder, "filmcraft");
        let native = Installed {
            name: "filmcraft".into(),
            path: folder.display().to_string(),
            version: "0.5.0".into(),
            install_kind: "installer".into(),
            architecture: "x64".into(),
            ..Default::default()
        };
        let mut config = Config {
            apps: vec![native.clone()],
            installations: vec![native.clone()],
            apps_root: root.display().to_string(),
        };
        assert!(paths
            .alternates_with_detector(&config, |_| Ok(None))
            .unwrap()
            .is_empty());
        let mut stale = native;
        stale.install_kind = "portable".into();
        config.installations.push(stale);
        assert!(paths
            .alternates_with_detector(&config, |_| Ok(None))
            .unwrap()
            .is_empty());
        let separate = root.join("separate-portable");
        release_fixture(&separate, "filmcraft");
        config.installations.last_mut().unwrap().path = separate.display().to_string();
        assert_eq!(
            paths
                .alternates_with_detector(&config, |_| Ok(None))
                .unwrap()
                .len(),
            1
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn alternate_snapshot_detects_fresh_installers_and_forgets_removed_records_without_writes() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let paths = Paths::new(root.clone(), None);
        crate::files::write_json(
            &paths.at("updater-settings.json"),
            &Preferences {
                release_format: "portable".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let config = paths.config_with_detector(|_| Ok(None)).unwrap();
        let alternate = root.join("Applications");
        release_fixture(&alternate, "photocraft");
        let detect = |name: &str| -> Result<Option<Installed>> {
            Ok(
                (name == "photocraft" && installed_executable(&alternate, name).is_some()).then(
                    || Installed {
                        name: name.into(),
                        path: alternate.display().to_string(),
                        version: "0.4.0".into(),
                        install_kind: "installer".into(),
                        ..Default::default()
                    },
                ),
            )
        };
        let facts = paths.alternates_with_detector(&config, detect).unwrap();
        assert_eq!(facts.len(), 1);
        assert!(config.apps.iter().all(|a| a.path.is_empty()));
        std::fs::remove_dir_all(&alternate).unwrap();
        assert!(paths
            .alternates_with_detector(&config, detect)
            .unwrap()
            .is_empty());
        assert!(!paths.at("settings.json").exists());
        assert!(!paths.at("manager-settings.json").exists());
        assert!(paths.at("updater-settings.json").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn installer_snapshot_verifies_portable_records_and_does_not_route_actions_to_them() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let paths = Paths::new(root.clone(), None);
        crate::files::write_json(
            &paths.at("manager-settings.json"),
            &Preferences {
                release_format: "installer".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let portable = paths.at("releases/photocraft");
        release_fixture(&portable, "photocraft");
        let config = paths.config_with_detector(|_| Ok(None)).unwrap();
        assert_eq!(
            config
                .apps
                .iter()
                .find(|app| app.name == "photocraft")
                .unwrap()
                .path,
            portable.display().to_string()
        );
        assert!(paths
            .alternates_with_detector(&config, |_| Ok(None))
            .unwrap()
            .is_empty());
        std::fs::remove_dir_all(portable).unwrap();
        assert!(paths
            .alternates_with_detector(&config, |_| Ok(None))
            .unwrap()
            .is_empty());
        assert!(!paths.at("settings.json").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[cfg(target_os = "windows")]
    #[test]
    fn installer_architecture_conflicts_block_native_installs_only() {
        let installed = Installed {
            name: "designcraft".into(),
            path: r"C:\Program Files\designcraft".into(),
            version: "0.5.0".into(),
            architecture: "x86".into(),
            install_kind: "installer".into(),
            ..Default::default()
        };
        let prefs = Preferences {
            release_format: "installer".into(),
            architecture: "x64".into(),
            ..Default::default()
        };
        assert!(installer_architecture_conflict(&installed, &prefs));
        assert!(installer_architecture_conflict_message(&installed, &prefs)
            .contains("Uninstall the x86 version before installing x64"));
        assert!(!installer_architecture_conflict(
            &installed,
            &Preferences {
                architecture: "x86".into(),
                ..prefs.clone()
            }
        ));
        assert!(!installer_architecture_conflict(
            &installed,
            &Preferences {
                release_format: "portable".into(),
                ..prefs.clone()
            }
        ));
        assert!(!installer_architecture_conflict(
            &Installed {
                install_kind: "portable".into(),
                ..installed.clone()
            },
            &prefs
        ));
        assert!(!installer_architecture_conflict(
            &Installed {
                path: String::new(),
                ..installed
            },
            &prefs
        ));
    }
    #[cfg(target_os = "windows")]
    #[test]
    fn selected_copy_survives_pending_architecture_changes_and_preserves_other_copy() {
        let root =
            std::env::temp_dir().join(format!("craft-arch-records-{}", uuid::Uuid::new_v4()));
        let paths = Paths::new(root.clone(), None);
        let copies: Vec<_> = ["x64", "x86"]
            .into_iter()
            .map(|arch| {
                let folder = root.join(arch);
                release_fixture(&folder, "photocraft");
                Installed {
                    name: "photocraft".into(),
                    architecture: arch.into(),
                    version: "0.1.0".into(),
                    path: folder.display().to_string(),
                    install_kind: "portable".into(),
                    ..Default::default()
                }
            })
            .collect();
        crate::files::write_json(
            &paths.at("settings.json"),
            &Config {
                apps: vec![copies[0].clone()],
                installations: copies.clone(),
                apps_root: root.display().to_string(),
            },
        )
        .unwrap();
        for selected in ["x86", "x64", "arm64"] {
            let prefs = Preferences {
                app_install_options: [(
                    crate::install_options::identity("photocraft"),
                    crate::install_options::InstallOptions {
                        release_format: "portable".into(),
                        architecture: selected.into(),
                    },
                )]
                .into(),
                release_format: "portable".into(),
                architecture: selected.into(),
                ..Default::default()
            };
            crate::files::write_json(&paths.at("manager-settings.json"), &prefs).unwrap();
            let config = paths.config_with_detector(|_| Ok(None)).unwrap();
            let installed = config.apps.iter().find(|a| a.name == "photocraft").unwrap();
            assert_eq!(installed.architecture, "x64");
            assert_eq!(
                installed_architecture_matches(installed, &prefs),
                selected == "x64"
            );
            paths.save_config(&config).unwrap();
            assert_eq!(
                paths
                    .config_with_detector(|_| Ok(None))
                    .unwrap()
                    .installations
                    .iter()
                    .filter(|a| a.name == "photocraft")
                    .count(),
                2
            );
        }
        for copy in copies {
            assert!(Path::new(&copy.path).exists());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn effective_architecture_preserves_legacy_macos_choices() {
        for architecture in ["x86", "x64", "arm64"] {
            let mut prefs = Preferences {
                architecture: architecture.into(),
                ..Default::default()
            };
            prefs.validate().unwrap();
            if cfg!(target_os = "macos") {
                assert_eq!(release_arch(architecture), release_arch("universal"));
                assert_eq!(
                    architecture_label(architecture),
                    "Universal (Intel + Apple silicon)"
                );
            } else {
                assert_ne!(release_arch(architecture), "universal");
                assert_eq!(architecture_label(architecture), architecture);
            }
        }
    }
    #[test]
    fn existing_libraries_gain_new_apps_without_resetting_preferences() {
        let root = std::env::temp_dir().join(format!("craft-catalog-{}", uuid::Uuid::new_v4()));
        let paths = Paths::new(root.clone(), None);
        let prefs = Preferences {
            release_format: "installer".into(),
            selected_apps: vec!["filmcraft".into()],
            selected_sources: vec!["filmcraft".into()],
            app_order: vec!["filmcraft".into()],
            ..Default::default()
        };
        crate::files::write_json(&paths.at("manager-settings.json"), &prefs).unwrap();
        let config = Config {
            apps_root: root.display().to_string(),
            apps: vec![Installed {
                name: "filmcraft".into(),
                ..Default::default()
            }],
            installations: vec![],
        };
        crate::files::write_json(&paths.at("settings.json"), &config).unwrap();
        let detect = |name: &str| {
            Ok((name == "wordcraft").then(|| Installed {
                name: name.into(),
                path: "system/WordCraft".into(),
                version: "0.3.0".into(),
                install_kind: "installer".into(),
                ..Default::default()
            }))
        };
        let migrated = paths.config_with_detector(detect).unwrap();
        assert_eq!(migrated.apps.len(), apps().len());
        assert_eq!(
            migrated
                .apps
                .iter()
                .find(|a| a.name == "wordcraft")
                .unwrap()
                .version,
            "0.3.0"
        );
        paths.save_config(&migrated).unwrap();
        assert_eq!(
            paths.config_with_detector(detect).unwrap().apps.len(),
            apps().len()
        );
        let preserved = paths.preferences().unwrap();
        assert_eq!(preserved.selected_apps, ["filmcraft"]);
        assert_eq!(preserved.selected_sources, ["filmcraft"]);
        assert_eq!(preserved.app_order[0], "filmcraft");
        assert_eq!(preserved.app_order.len(), apps().len());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn first_launch_detects_preexisting_installers_and_keeps_portable_inventory() {
        let root =
            std::env::temp_dir().join(format!("craft-first-launch-{}", uuid::Uuid::new_v4()));
        let paths = Paths::new(root.clone(), None);
        let portable = paths.at("releases/photocraft");
        std::fs::create_dir_all(&portable).unwrap();
        #[cfg(not(target_os = "macos"))]
        std::fs::write(portable.join(executable_name("photocraft")), b"portable").unwrap();
        #[cfg(target_os = "macos")]
        {
            let contents = portable
                .join(executable_name("photocraft"))
                .join("Contents");
            std::fs::create_dir_all(&contents).unwrap();
            std::fs::write(contents.join("Info.plist"), r#"<plist version="1.0"><dict><key>CFBundleIdentifier</key><string>ai.storyteller.photocraft</string><key>CFBundleShortVersionString</key><string>0.2.0</string></dict></plist>"#).unwrap();
        }
        crate::files::write_json(
            &paths.at("manager-settings.json"),
            &Preferences {
                release_format: "installer".into(),
                app_install_options: ["photocraft", "printcraft"]
                    .into_iter()
                    .map(|app| {
                        (
                            crate::install_options::identity(app),
                            crate::install_options::InstallOptions {
                                release_format: "installer".into(),
                                architecture: "x64".into(),
                            },
                        )
                    })
                    .collect(),
                ..Default::default()
            },
        )
        .unwrap();
        let detect = |name: &str| -> Result<Option<Installed>> {
            Ok(["photocraft", "printcraft"]
                .contains(&name)
                .then(|| Installed {
                    name: name.into(),
                    path: root.join("system").join(name).display().to_string(),
                    version: "0.2.1".into(),
                    architecture: "x64".into(),
                    install_kind: "installer".into(),
                    product_code: "existing-product".into(),
                }))
        };
        assert!(!paths.at("settings.json").exists());
        let first = paths.config_with_detector(detect).unwrap();
        for name in ["photocraft", "printcraft"] {
            let app = first.apps.iter().find(|a| a.name == name).unwrap();
            assert_eq!(app.install_kind, "installer");
            assert_eq!(app.version, "0.2.1");
            assert!(first
                .installations
                .iter()
                .any(|a| a.name == name && a.install_kind == "installer"));
        }
        paths.save_config(&first).unwrap();
        let reopened = paths.config_with_detector(detect).unwrap();
        assert_eq!(
            reopened
                .apps
                .iter()
                .find(|a| a.name == "photocraft")
                .unwrap()
                .path,
            first
                .apps
                .iter()
                .find(|a| a.name == "photocraft")
                .unwrap()
                .path
        );
        crate::files::write_json(
            &paths.at("manager-settings.json"),
            &Preferences {
                release_format: "portable".into(),
                ..paths.read_preferences().unwrap()
            },
        )
        .unwrap();
        let switched = paths.config_with_detector(detect).unwrap();
        assert_eq!(
            switched
                .apps
                .iter()
                .find(|a| a.name == "photocraft")
                .unwrap()
                .path,
            first
                .apps
                .iter()
                .find(|app| app.name == "photocraft")
                .unwrap()
                .path
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
