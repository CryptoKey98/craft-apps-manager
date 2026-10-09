//! The Craft apps the manager knows about.
//!
//! A built-in list keeps the manager usable offline. `refresh` lists the
//! Storytold repositories on GitHub and classifies every new `*craft*`
//! repository from its latest release, so apps published after this build
//! appear without a manager update.
use crate::{files, model::Release, network::Network};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::RwLock};

pub const ORG: &str = "storytold";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scheme {
    /// `photocraft-0.5.0-windows-x64-portable.zip`, `photocraft-0.5.0-macos-universal.dmg`
    #[default]
    Craft,
    /// `ArtCraft_0.41.0_x64-setup.exe`, `ArtCraft_0.41.0_universal.dmg`
    Tauri,
    /// Source only: the repository has no installable release.
    None,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// Stable identity used in settings, folders and logs.
    pub key: String,
    pub repository: String,
    pub title: String,
    /// Short label under the app name.
    #[serde(default)]
    pub category: String,
    /// The repository description, shown on the app page.
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub scheme: Scheme,
    /// Release asset prefix; `printcraft` for the renamed `pdfcraft` repository.
    #[serde(default)]
    pub asset_prefix: String,
    /// Other names the app's files and programs have used (`pdfcraft` since 0.4.0).
    #[serde(default)]
    pub aliases: Vec<String>,
    /// macOS bundle identifiers beyond the usual `ai.storyteller.<name>`.
    #[serde(default)]
    pub bundle_ids: Vec<String>,
    pub release: bool,
    pub source: bool,
}

fn craft(key: &str, title: &str, category: &str, description: &str) -> Entry {
    Entry {
        key: key.into(),
        repository: key.into(),
        title: title.into(),
        category: category.into(),
        description: description.into(),
        scheme: Scheme::Craft,
        asset_prefix: key.into(),
        aliases: Vec::new(),
        bundle_ids: Vec::new(),
        release: true,
        source: true,
    }
}

/// Apps known when this manager was built, verified against their releases,
/// in the default sidebar order.
pub fn builtin() -> Vec<Entry> {
    vec![
        craft("designcraft", "DesignCraft", "Page layout", "Page layout and publishing; an open-source, clean-room reimplementation of Adobe InDesign, rebuilt in pure Rust."),
        craft("effectcraft", "EffectCraft", "Motion graphics", "Motion graphics and visual effects; an open-source, clean-room reimplementation of Adobe After Effects, rebuilt in pure Rust."),
        craft("filmcraft", "FilmCraft", "Video editing", "An open-source, clean-room reimplementation of Adobe Premiere Pro built in pure Rust."),
        craft("lightcraft", "LightCraft", "Photo workflow", "An open-source, clean-room reimplementation of Adobe Lightroom in pure Rust."),
        craft("photocraft", "PhotoCraft", "Image editing", "An open-source, clean-room reimplementation of Adobe Photoshop in pure Rust."),
        // Files were named printcraft until 0.2.1 and pdfcraft from 0.4.0.
        Entry {
            repository: "pdfcraft".into(),
            aliases: vec!["pdfcraft".into()],
            ..craft("printcraft", "PDFCraft", "PDF documents", "An open-source, clean-room reimplementation of Adobe Acrobat built in pure Rust.")
        },
        craft("vectorcraft", "VectorCraft", "Vector graphics", "An open-source, clean-room reimplementation of Adobe Illustrator, built in pure Rust."),
        craft("wordcraft", "WordCraft", "Word processing", "An open-source, clean-room reimplementation of Microsoft Word in pure Rust."),
        craft("gridcraft", "GridCraft", "Spreadsheets", "An open-source, clean-room spreadsheet (Microsoft Excel-style) in pure Rust."),
        craft("deckcraft", "DeckCraft", "Presentations", "Presentations and slide shows: an open-source, clean-room reimplementation of Microsoft PowerPoint in pure Rust."),
        craft("cadcraft", "CADCraft", "CAD and drafting", "Computer-aided design and drafting: an open-source, clean-room AutoCAD-style app in pure Rust."),
        craft("soundcraft", "SoundCraft", "Audio production", "An open-source, clean-room reimplementation of Avid Pro Tools in pure Rust."),
        Entry {
            key: "artcraft".into(),
            repository: "artcraft".into(),
            title: "ArtCraft".into(),
            category: "AI image and video".into(),
            description: "ArtCraft is an intentional crafting engine for artists, designers, and filmmakers.".into(),
            scheme: Scheme::Tauri,
            asset_prefix: "ArtCraft".into(),
            aliases: Vec::new(),
            bundle_ids: vec!["ai.artcraft.app".into()],
            release: true,
            source: false,
        },
        Entry {
            key: "artcraftx".into(),
            repository: "artcraftx".into(),
            title: "ArtCraft X".into(),
            category: "Source only".into(),
            description: "ArtCraft-X".into(),
            scheme: Scheme::None,
            asset_prefix: String::new(),
            aliases: Vec::new(),
            bundle_ids: Vec::new(),
            release: false,
            source: true,
        },
    ]
}

static CURRENT: RwLock<Vec<Entry>> = RwLock::new(Vec::new());

pub fn all() -> Vec<Entry> {
    let current = CURRENT.read().unwrap();
    if current.is_empty() {
        builtin()
    } else {
        current.clone()
    }
}
pub fn get(key: &str) -> Option<Entry> {
    all().into_iter().find(|e| e.key == key)
}
/// Apps with installable releases, in a stable order.
pub fn release_keys() -> Vec<String> {
    all()
        .into_iter()
        .filter(|e| e.release)
        .map(|e| e.key)
        .collect()
}
/// macOS bundle identifiers an installed copy of the app may carry.
pub fn bundle_ids(key: &str) -> Vec<String> {
    let mut ids: Vec<String> = names(key)
        .iter()
        .map(|name| format!("ai.storyteller.{}", name.to_ascii_lowercase()))
        .collect();
    if let Some(entry) = get(key) {
        ids.extend(entry.bundle_ids);
    }
    ids.dedup();
    ids
}
/// Apps whose source can be downloaded and built.
pub fn source_keys() -> Vec<String> {
    all()
        .into_iter()
        .filter(|e| e.source)
        .map(|e| e.key)
        .collect()
}
/// Every name an app's release files and programs may use, preferred first.
pub fn names(key: &str) -> Vec<String> {
    let mut names = vec![key.to_string()];
    if let Some(entry) = get(key) {
        names.push(entry.asset_prefix);
        names.extend(entry.aliases);
        names.push(entry.repository);
    }
    let mut seen = std::collections::BTreeSet::new();
    names.retain(|n| !n.is_empty() && seen.insert(n.to_ascii_lowercase()));
    names
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cache {
    checked_at: i64,
    entries: Vec<Entry>,
    /// File names apps started using after this build, found in their releases.
    #[serde(default)]
    learned: std::collections::BTreeMap<String, Vec<String>>,
}
fn cache_path(root: &Path) -> std::path::PathBuf {
    root.join("runtime/catalog.json")
}
fn valid(entry: &Entry) -> bool {
    let simple = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    };
    simple(&entry.key)
        && simple(&entry.repository)
        && !entry.title.trim().is_empty()
        && entry.aliases.iter().all(|a| simple(a))
        && entry.bundle_ids.iter().all(|id| {
            !id.is_empty()
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        })
        && (entry.scheme == Scheme::None
            || (!entry.asset_prefix.is_empty()
                && entry
                    .asset_prefix
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric())))
}
/// Built-in apps stay authoritative; discovery only adds apps they do not cover.
fn merge(
    discovered: Vec<Entry>,
    learned: &std::collections::BTreeMap<String, Vec<String>>,
) -> Vec<Entry> {
    let mut entries = builtin();
    let mut found: Vec<Entry> = Vec::new();
    for entry in discovered {
        if valid(&entry)
            && !entries
                .iter()
                .chain(&found)
                .any(|e| e.key == entry.key || e.repository == entry.repository)
        {
            found.push(entry);
        }
    }
    // Newly published apps follow the built-in ones.
    found.sort_by(|a, b| a.key.cmp(&b.key));
    entries.extend(found);
    for entry in &mut entries {
        for alias in learned.get(&entry.key).into_iter().flatten() {
            if valid_alias(alias) && !entry.aliases.contains(alias) {
                entry.aliases.push(alias.clone());
            }
        }
    }
    entries
}
fn valid_alias(alias: &str) -> bool {
    !alias.is_empty()
        && alias
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}
/// Remembers a new name an app's release files use, so its program is found
/// under that name from now on.
pub fn learn_alias(root: &Path, key: &str, alias: &str) -> Result<()> {
    if names(key).iter().any(|n| n.eq_ignore_ascii_case(alias)) || !valid_alias(alias) {
        return Ok(());
    }
    let mut cache: Cache = files::read_or_default(&cache_path(root)).unwrap_or_default();
    let aliases = cache.learned.entry(key.to_string()).or_default();
    if !aliases.iter().any(|a| a == alias) {
        aliases.push(alias.to_string());
    }
    files::write_json(&cache_path(root), &cache)?;
    *CURRENT.write().unwrap() = merge(cache.entries, &cache.learned);
    Ok(())
}
/// Uses the saved discovery result without contacting GitHub.
pub fn load(root: &Path) {
    let cache: Cache = files::read_or_default(&cache_path(root)).unwrap_or_default();
    *CURRENT.write().unwrap() = merge(cache.entries, &cache.learned);
}
/// Seconds since the last successful discovery, if any.
pub fn age(root: &Path) -> Option<i64> {
    let cache: Cache = files::read_or_default(&cache_path(root)).ok()?;
    (cache.checked_at > 0).then(|| chrono::Utc::now().timestamp() - cache.checked_at)
}
pub fn refresh_if_older(root: &Path, seconds: i64) -> Result<Vec<Entry>> {
    if age(root).is_some_and(|age| (0..seconds).contains(&age)) {
        load(root);
        return Ok(all());
    }
    refresh(root)
}

#[derive(Deserialize)]
struct Repository {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    fork: bool,
    #[serde(default)]
    disabled: bool,
}
/// Lists Storytold repositories and classifies new Craft apps.
pub fn refresh(root: &Path) -> Result<Vec<Entry>> {
    let network = Network::new(root)?;
    let previous: Cache = files::read_or_default(&cache_path(root)).unwrap_or_default();
    let mut repositories: Vec<Repository> = Vec::new();
    for page in 1..=10 {
        let batch: Vec<Repository> = network.json(&format!(
            "https://api.github.com/orgs/{ORG}/repos?per_page=100&page={page}"
        ))?;
        let last = batch.len() < 100;
        repositories.extend(batch);
        if last {
            break;
        }
    }
    let pattern = regex::Regex::new(r"^[a-z0-9]+craft[a-z0-9]*$")?;
    let known = builtin();
    let mut discovered = Vec::new();
    for repository in repositories {
        if repository.archived
            || repository.fork
            || repository.disabled
            || !pattern.is_match(&repository.name)
            || known.iter().any(|e| e.repository == repository.name)
        {
            continue;
        }
        // Classified apps keep their identity; source-only repositories are
        // probed again so their first release is noticed.
        if let Some(entry) = previous
            .entries
            .iter()
            .find(|e| e.repository == repository.name && e.scheme != Scheme::None)
        {
            discovered.push(entry.clone());
            continue;
        }
        let release: Option<Release> = network.json_optional(&format!(
            "https://api.github.com/repos/{ORG}/{}/releases/latest",
            repository.name
        ))?;
        let mut entry = classify(&repository.name, release.as_ref());
        entry.description = repository
            .description
            .unwrap_or_default()
            .trim()
            .to_string();
        discovered.push(entry);
    }
    files::write_json(
        &cache_path(root),
        &Cache {
            checked_at: chrono::Utc::now().timestamp(),
            entries: discovered.clone(),
            learned: previous.learned.clone(),
        },
    )?;
    let merged = merge(discovered, &previous.learned);
    *CURRENT.write().unwrap() = merged.clone();
    Ok(merged)
}

/// Readable name for a repository without a curated title: `mapcraft` → `MapCraft`.
pub fn pretty(repository: &str) -> String {
    let mut title = String::new();
    for (i, part) in repository.split("craft").enumerate() {
        if i > 0 {
            title.push_str("Craft");
        }
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            title.extend(first.to_uppercase());
            title.push_str(chars.as_str());
        }
    }
    title
}
/// Identifies a new app from its latest release: the file naming it uses and
/// the name its files carry.
pub fn classify(repository: &str, release: Option<&Release>) -> Entry {
    let mut entry = Entry {
        key: repository.into(),
        repository: repository.into(),
        title: pretty(repository),
        category: String::new(),
        description: String::new(),
        scheme: Scheme::None,
        asset_prefix: String::new(),
        aliases: Vec::new(),
        bundle_ids: Vec::new(),
        release: false,
        source: true,
    };
    let Some(release) = release.filter(|r| !r.draft && !r.prerelease) else {
        return entry;
    };
    let Ok(version) = crate::updates::release_version(&release.tag_name) else {
        return entry;
    };
    if let Some(name) = release.name.as_deref() {
        let name = regex::Regex::new(r"\s+v?\d+\.\d+\.\d+.*$")
            .unwrap()
            .replace(name.trim(), "");
        if !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b' ') {
            entry.title = name.into_owned();
        }
    }
    let parsed: Vec<_> = release
        .assets
        .iter()
        .filter_map(|a| parse_asset(&a.name))
        .filter(|a| a.version == version)
        .collect();
    if let Some(a) = parsed.iter().find(|a| a.scheme == Scheme::Craft) {
        entry.key = a.prefix.clone();
        entry.scheme = Scheme::Craft;
        entry.asset_prefix = a.prefix.clone();
        entry.release = true;
    } else if let Some(a) = parsed.iter().find(|a| a.scheme == Scheme::Tauri) {
        entry.scheme = Scheme::Tauri;
        entry.asset_prefix = a.prefix.clone();
        entry.release = true;
        entry.source = false;
    }
    entry
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedAsset {
    pub scheme: Scheme,
    pub prefix: String,
    pub version: String,
    /// `windows`, `macos` or `linux`.
    pub os: &'static str,
}
/// Recognises the desktop release files of both naming schemes.
pub fn parse_asset(name: &str) -> Option<ParsedAsset> {
    static CRAFT: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static TAURI: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let craft = CRAFT.get_or_init(|| {
        regex::Regex::new(r"^([a-z0-9]+)-(\d+\.\d+\.\d+)-(windows|macos|linux)-[a-z0-9_]+[-.]")
            .unwrap()
    });
    if let Some(c) = craft.captures(name) {
        return Some(ParsedAsset {
            scheme: Scheme::Craft,
            prefix: c[1].into(),
            version: c[2].into(),
            os: match &c[3] {
                "windows" => "windows",
                "macos" => "macos",
                _ => "linux",
            },
        });
    }
    let tauri = TAURI.get_or_init(|| {
        regex::Regex::new(
            r"^([A-Za-z0-9]+)_(\d+\.\d+\.\d+)_(?:x64|x86|arm64|aarch64|universal|amd64)(-setup\.exe|_[A-Za-z]{2}-[A-Za-z]{2}\.msi|\.dmg|\.AppImage|\.deb)$",
        )
        .unwrap()
    });
    let c = tauri.captures(name)?;
    Some(ParsedAsset {
        scheme: Scheme::Tauri,
        prefix: c[1].into(),
        version: c[2].into(),
        os: match &c[3] {
            ".dmg" => "macos",
            ".AppImage" | ".deb" => "linux",
            _ => "windows",
        },
    })
}

#[cfg(test)]
mod tests {
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
}
