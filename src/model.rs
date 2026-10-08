use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MANAGER_ARCH: &str = if cfg!(target_arch = "x86") {
    "x86"
} else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    "arm64"
} else {
    "x64"
};

pub const APPS: [&str; 12] = [
    "designcraft",
    "effectcraft",
    "filmcraft",
    "lightcraft",
    "photocraft",
    "printcraft",
    "vectorcraft",
    "wordcraft",
    "gridcraft",
    "deckcraft",
    "cadcraft",
    "soundcraft",
];
pub const SOURCES: [&str; 13] = [
    "designcraft",
    "effectcraft",
    "filmcraft",
    "lightcraft",
    "photocraft",
    "printcraft",
    "vectorcraft",
    "wordcraft",
    "gridcraft",
    "deckcraft",
    "cadcraft",
    "soundcraft",
    "artcraftx",
];
pub fn title(name: &str) -> String {
    if name == "cadcraft" {
        return "CADCraft".into();
    }
    if name == "printcraft" {
        return "PDFCraft".into();
    }
    if name == "artcraftx" {
        return "ArtCraft X".into();
    }
    format!(
        "{}{}",
        name[..1].to_uppercase(),
        name[1..].replace("craft", "Craft")
    )
}
/// Upstream repository names can differ from the published binary names.
pub fn repository(name: &str) -> &str {
    if name == "printcraft" {
        "pdfcraft"
    } else {
        name
    }
}
pub fn valid_app(name: &str) -> Result<()> {
    if !SOURCES.contains(&name) {
        bail!("Unknown app: {name}");
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Preferences {
    pub keep_app_backups: bool,
    pub keep_source_backups: bool,
    pub compress_backups: bool,
    pub compress_source_backups: bool,
    pub notify_updates: bool,
    pub backup_versions: usize,
    pub release_format: String,
    pub architecture: String,
    pub selected_apps: Vec<String>,
    pub selected_sources: Vec<String>,
    pub app_order: Vec<String>,
    #[serde(alias = "checkUpdaterOnStartup")]
    pub check_manager_on_startup: bool,
    pub check_installed_apps_on_startup: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            keep_app_backups: true,
            keep_source_backups: true,
            compress_backups: true,
            compress_source_backups: true,
            notify_updates: true,
            backup_versions: 1,
            release_format: if cfg!(any(target_os = "windows", target_os = "macos")) {
                "installer"
            } else {
                "portable"
            }
            .into(),
            architecture: MANAGER_ARCH.into(),
            selected_apps: APPS.iter().map(|s| s.to_string()).collect(),
            selected_sources: SOURCES.iter().map(|s| s.to_string()).collect(),
            app_order: APPS.iter().map(|s| s.to_string()).collect(),
            check_manager_on_startup: false,
            check_installed_apps_on_startup: false,
        }
    }
}
impl Preferences {
    pub fn validate(&mut self) -> Result<()> {
        if !["portable", "installer"].contains(&self.release_format.as_str())
            || !["x64", "x86", "arm64"].contains(&self.architecture.as_str())
        {
            bail!("Unsupported release format or architecture");
        }
        self.backup_versions = self.backup_versions.clamp(1, 10);
        self.selected_apps.retain(|s| APPS.contains(&s.as_str()));
        self.selected_apps.sort();
        self.selected_apps.dedup();
        self.selected_sources
            .retain(|s| SOURCES.contains(&s.as_str()));
        self.selected_sources.sort();
        self.selected_sources.dedup();
        let mut seen = std::collections::BTreeSet::new();
        self.app_order
            .retain(|s| APPS.contains(&s.as_str()) && seen.insert(s.clone()));
        for name in APPS {
            if seen.insert(name.to_owned()) {
                self.app_order.push(name.to_owned());
            }
        }
        Ok(())
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
    #[serde(default = "default_arch")]
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
        for name in APPS {
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
                        .split("-windows-")
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
                architecture: default_arch(),
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
        let installer = self.read_preferences()?.release_format == "installer";
        // Add catalog entries when upgrading an existing library without
        // changing saved selections, installations, or the user's app order.
        for name in APPS {
            if !config.apps.iter().any(|app| app.name == name) {
                config.apps.push(Installed {
                    name: name.into(),
                    architecture: default_arch(),
                    ..Default::default()
                });
            }
        }
        for app in &config.apps {
            if !app.path.is_empty()
                && !config.installations.iter().any(|a| {
                    a.name == app.name
                        && (a.install_kind == "installer") == (app.install_kind == "installer")
                })
            {
                config.installations.push(app.clone());
            }
        }
        for app in &mut config.apps {
            if installer {
                let detected = detect(&app.name)?;
                config
                    .installations
                    .retain(|a| !(a.name == app.name && a.install_kind == "installer"));
                if let Some(record) = detected {
                    config.installations.push(record.clone());
                    *app = record;
                } else {
                    app.path.clear();
                    app.version.clear();
                    app.product_code.clear();
                    app.install_kind = "installer".into();
                }
            } else if let Some(record) = config.installations.iter().find(|a| {
                a.name == app.name
                    && a.install_kind != "installer"
                    && installed_executable(Path::new(&a.path), &a.name).is_some()
            }) {
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
        Ok(config)
    }
    pub fn save_config(&self, config: &Config) -> Result<()> {
        let mut config = config.clone();
        let installer = self.read_preferences()?.release_format == "installer";
        for app in &config.apps {
            config
                .installations
                .retain(|a| !(a.name == app.name && (a.install_kind == "installer") == installer));
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
        let installer = self.read_preferences()?.release_format == "installer";
        let mut records = Vec::new();
        for name in APPS {
            if !installer {
                if let Some(record) = detect(name)? {
                    records.push(record);
                }
            } else {
                let recorded = config.installations.iter().find(|a| {
                    a.name == name
                        && a.install_kind != "installer"
                        && !a.path.is_empty()
                        && installed_executable(Path::new(&a.path), name).is_some()
                });
                if let Some(record) = recorded {
                    records.push(record.clone());
                } else {
                    let folder = self.at(format!("releases/{name}"));
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
        p.validate()?;
        if migrate {
            crate::files::write_json(&target, &p)?;
            std::fs::remove_file(legacy)?;
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
    let mut names = vec![executable_name(app)];
    let renamed = repository(app);
    if renamed != app {
        names.push(executable_name(renamed));
    }
    names
}
pub fn installed_executable(folder: &Path, app: &str) -> Option<PathBuf> {
    executable_names(app)
        .into_iter()
        .map(|name| folder.join(name))
        .find(|path| is_executable(path))
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
    use super::*;
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
        let config = paths
            .config_with_detector(|_| {
                panic!("portable action inventory must not detect installers")
            })
            .unwrap();
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
        assert!(config.apps.iter().all(|a| a.path.is_empty()));
        assert_eq!(
            paths
                .alternates_with_detector(&config, |_| Ok(None))
                .unwrap()
                .len(),
            1
        );
        std::fs::remove_dir_all(portable).unwrap();
        assert!(paths
            .alternates_with_detector(&config, |_| Ok(None))
            .unwrap()
            .is_empty());
        assert!(!paths.at("settings.json").exists());
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
        assert_eq!(migrated.apps.len(), APPS.len());
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
            APPS.len()
        );
        let preserved = paths.preferences().unwrap();
        assert_eq!(preserved.selected_apps, ["filmcraft"]);
        assert_eq!(preserved.selected_sources, ["filmcraft"]);
        assert_eq!(preserved.app_order[0], "filmcraft");
        assert_eq!(preserved.app_order.len(), APPS.len());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn first_launch_detects_preexisting_installers_and_keeps_portable_inventory() {
        let root =
            std::env::temp_dir().join(format!("craft-first-launch-{}", uuid::Uuid::new_v4()));
        let paths = Paths::new(root.clone(), None);
        let portable = paths.at("releases/photocraft");
        std::fs::create_dir_all(&portable).unwrap();
        std::fs::write(portable.join(executable_name("photocraft")), b"portable").unwrap();
        crate::files::write_json(
            &paths.at("manager-settings.json"),
            &Preferences {
                release_format: "installer".into(),
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
                ..Default::default()
            },
        )
        .unwrap();
        let switched = paths
            .config_with_detector(|_| panic!("Portable mode must not query installer records"))
            .unwrap();
        assert_eq!(
            switched
                .apps
                .iter()
                .find(|a| a.name == "photocraft")
                .unwrap()
                .path,
            portable.display().to_string()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
