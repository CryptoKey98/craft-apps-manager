use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const APPS: [&str; 7] = [
    "designcraft",
    "effectcraft",
    "filmcraft",
    "lightcraft",
    "photocraft",
    "printcraft",
    "vectorcraft",
];
pub const SOURCES: [&str; 8] = [
    "designcraft",
    "effectcraft",
    "filmcraft",
    "lightcraft",
    "photocraft",
    "printcraft",
    "vectorcraft",
    "artcraftx",
];
pub fn title(name: &str) -> String {
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
            release_format: "installer".into(),
            architecture: "x64".into(),
            selected_apps: APPS.iter().map(|s| s.to_string()).collect(),
            selected_sources: SOURCES.iter().map(|s| s.to_string()).collect(),
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
    "x64".into()
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub apps_root: String,
    pub apps: Vec<Installed>,
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
        if self.at("settings.json").exists() {
            let config: Config = crate::files::read_json(&self.at("settings.json"))?;
            if !Path::new(&config.apps_root).eq(&self.root) {
                bail!("settings.json points to a different data folder. Select that folder in Settings.");
            }
            return Ok(config);
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
                path: if path.is_empty() && direct.join(format!("{name}.exe")).exists() {
                    direct.to_string_lossy().into_owned()
                } else {
                    path
                },
                architecture: default_arch(),
                install_kind: String::new(),
                product_code: String::new(),
            });
        }
        Ok(Config {
            apps_root: self.root.to_string_lossy().into_owned(),
            apps,
        })
    }
    pub fn preferences(&self) -> Result<Preferences> {
        let mut p =
            crate::files::read_or_default::<Preferences>(&self.at("updater-settings.json"))?;
        p.validate()?;
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
