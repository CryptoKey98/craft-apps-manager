//! The Craft apps the manager knows about.
//!
//! A built-in list keeps the manager usable offline. `refresh` lists the
//! Storytold repositories on GitHub and classifies every new `*craft*`
//! repository from its latest release, so apps published after this build
//! appear without a manager update.
use crate::{files, model::Release, network::Network};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::RwLock,
};

pub const ORG: &str = "storytold";
pub const REFRESH_INTERVAL: i64 = 6 * 3600;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scheme {
    /// `photocraft-0.5.0-windows-x64-portable.zip`, `photocraft-0.5.0-macos-universal.dmg`
    #[default]
    Craft,
    /// `ArtCraft_0.41.0_x64-setup.exe`, `ArtCraft_0.41.0_universal.dmg`
    Tauri,
    /// User-selected download using common system and architecture labels.
    Generic,
    /// Source only: the repository has no installable release.
    None,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// Stable identity used in settings, folders and logs.
    pub key: String,
    pub repository: String,
    /// GitHub repository creation time, independent of release updates or local discovery.
    #[serde(default)]
    pub created_at: Option<i64>,
    pub title: String,
    /// Short label under the app name.
    #[serde(default)]
    pub category: String,
    /// The repository description, shown on the app page.
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub scheme: Scheme,
    #[serde(default)]
    pub download_rule: Option<crate::downloads::Rule>,
    #[serde(default)]
    pub download_rules: Vec<crate::downloads::Rule>,
    #[serde(default)]
    pub registered_formats: Vec<String>,
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
        download_rule: None,
        download_rules: Vec::new(),
        registered_formats: Vec::new(),
        asset_prefix: key.into(),
        aliases: Vec::new(),
        created_at: None,
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
            download_rule: None,
        download_rules: Vec::new(),
        registered_formats: Vec::new(),
            asset_prefix: "ArtCraft".into(),
            aliases: Vec::new(),
            created_at: None,
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
            download_rule: None,
        download_rules: Vec::new(),
        registered_formats: Vec::new(),
            asset_prefix: String::new(),
            aliases: Vec::new(),
            created_at: None,
        bundle_ids: Vec::new(),
            release: false,
            source: true,
        },
    ]
}

static CURRENT: RwLock<Vec<Entry>> = RwLock::new(Vec::new());

pub fn all() -> Vec<Entry> {
    known_entries()
}
pub fn known_entries() -> Vec<Entry> {
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
/// The bare repository name, without any `owner/` prefix.
pub fn repository_name(repository: &str) -> &str {
    repository.rsplit('/').next().unwrap_or(repository)
}
/// Every name an app's release files and programs may use, preferred first.
pub fn names(key: &str) -> Vec<String> {
    let mut names = vec![key.to_string()];
    if let Some(entry) = get(key) {
        names.push(entry.asset_prefix);
        names.extend(entry.aliases);
        names.push(repository_name(&entry.repository).to_string());
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
/// One `owner` or repository name segment: lowercase, digits, `-_.`.
fn repository_part(part: &str) -> bool {
    !part.is_empty()
        && part.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_' || b == b'.'
        })
}
fn valid(entry: &Entry) -> bool {
    let simple = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    };
    // A repository is either a bare Storytold name or an `owner/name` slug
    // for an app hosted under a different account.
    let repository = |s: &str| {
        let (owner, name) = s.split_once('/').unwrap_or(("", s));
        (owner.is_empty() || repository_part(owner)) && repository_part(name)
    };
    simple(&entry.key)
        && repository(&entry.repository)
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
/// User-added apps follow discovery and win over discovered duplicates, so a
/// repository the user tracks explicitly is never shadowed by classification.
fn merge(
    discovered: Vec<Entry>,
    learned: &BTreeMap<String, Vec<String>>,
    custom: &[Entry],
) -> Vec<Entry> {
    let mut entries = builtin();
    let mut found: Vec<Entry> = Vec::new();
    for entry in discovered {
        if valid(&entry)
            && !entries
                .iter()
                .chain(&found)
                .any(|e| e.key == entry.key || e.repository == entry.repository)
            && !custom.iter().any(|e| e.repository == entry.repository)
        {
            found.push(entry);
        }
    }
    // Newly published apps follow the built-in ones.
    found.sort_by(|a, b| a.key.cmp(&b.key));
    entries.extend(found);
    let mut customs: Vec<Entry> = custom
        .iter()
        .filter(|e| {
            valid(e)
                && !entries
                    .iter()
                    .any(|x| x.key == e.key || x.repository == e.repository)
        })
        .cloned()
        .collect();
    customs.sort_by(|a, b| a.key.cmp(&b.key));
    entries.extend(customs);
    for entry in &mut entries {
        if entry.category.trim().is_empty() {
            entry.category = repository_subtitle(&entry.title, &entry.description);
        }
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
    reload(root, &cache);
    Ok(())
}
/// Uses the saved discovery result without contacting GitHub.
pub fn load(root: &Path) {
    crate::app_identity::load(root);
    let cache: Cache = files::read_or_default(&cache_path(root)).unwrap_or_default();
    reload(root, &cache);
}
/// Rebuilds the process-wide catalog from saved discovery plus user apps.
fn reload(root: &Path, cache: &Cache) {
    let value: serde_json::Value =
        files::read_or_default(&root.join("manager-settings.json")).unwrap_or_default();
    let format = value
        .get("releaseFormat")
        .and_then(|v| v.as_str())
        .unwrap_or("portable")
        .to_string();
    *CURRENT.write().unwrap() = merge(
        cache.entries.clone(),
        &cache.learned,
        &custom_apps_for_format_migration(root, &format),
    );
}
/// Apps the user added from GitHub links in Settings, newest last.
fn custom_path(root: &Path) -> PathBuf {
    root.join("runtime/custom-apps.json")
}
pub fn custom_apps(root: &Path) -> Vec<Entry> {
    let value: serde_json::Value =
        files::read_or_default(&root.join("manager-settings.json")).unwrap_or_default();
    custom_apps_for_format_migration(
        root,
        value
            .get("releaseFormat")
            .and_then(|v| v.as_str())
            .unwrap_or("portable"),
    )
}
fn custom_apps_for_format_migration(root: &Path, format: &str) -> Vec<Entry> {
    let mut entries: Vec<Entry> = files::read_or_default(&custom_path(root)).unwrap_or_default();
    for entry in &mut entries {
        let repo = entry
            .repository
            .split('/')
            .next_back()
            .unwrap_or(&entry.key);
        if !same_app_title(&entry.title, repo) {
            let metadata: Option<Repository> = Network::cached_json(
                root,
                &format!("https://api.github.com/repos/{}", entry.repository),
            );
            entry.title = release_title(
                None,
                metadata.as_ref().map_or(repo, |repo| repo.name.as_str()),
            );
        }
        if entry.registered_formats.is_empty() {
            entry.registered_formats = entry
                .download_rules
                .iter()
                .chain(entry.download_rule.iter())
                .map(|r| r.format.clone())
                .collect();
            entry.registered_formats.sort();
            entry.registered_formats.dedup();
            if entry.registered_formats.is_empty() {
                entry.registered_formats.push(format.into());
            }
        }
    }
    entries
}
pub fn merge_custom_registration(entries: &mut Vec<Entry>, entry: Entry) -> Result<()> {
    anyhow::ensure!(
        !known_entries()
            .iter()
            .any(|known| !known.repository.contains('/') && known.key == entry.key),
        "This app is already in the default catalog"
    );
    if let Some(existing) = entries.iter_mut().find(|e| e.key == entry.key) {
        anyhow::ensure!(
            existing.repository.eq_ignore_ascii_case(&entry.repository),
            "Another repository already uses this app name"
        );
        if existing.scheme != entry.scheme || entry.scheme == Scheme::Generic {
            existing.scheme = Scheme::Generic;
        }
        for format in entry.registered_formats {
            if !existing.registered_formats.contains(&format) {
                existing.registered_formats.push(format);
            }
        }
        for rule in entry.download_rules.into_iter().chain(entry.download_rule) {
            existing.download_rules.retain(|r| !r.same_target(&rule));
            existing.download_rules.push(rule);
        }
    } else {
        entries.push(entry);
    }
    Ok(())
}
pub fn selected_custom_download(choice: &DownloadChoice, index: usize) -> Result<Entry> {
    let asset = choice
        .assets
        .get(index)
        .ok_or_else(|| anyhow::anyhow!("Choose a download"))?;
    let mut entry = choice.entry.clone();
    let rule = crate::downloads::Rule::new(asset, &choice.version, &choice.preferences);
    entry.download_rules = vec![rule.clone()];
    entry.download_rule = Some(rule);
    entry.scheme = Scheme::Generic;
    entry.registered_formats = vec![choice.preferences.release_format.clone()];
    Ok(entry)
}
fn write_custom_apps(root: &Path, apps: &Vec<Entry>) -> Result<()> {
    files::write_json(&custom_path(root), apps)
}
/// Accepts `https://github.com/owner/repo`, `github.com/owner/repo` or a bare
/// `owner/repo` slug. Anything deeper (a release page, a file) is rejected so
/// people paste repository links, not subpages.
pub fn parse_github_slug(link: &str) -> Result<String> {
    let rest = link.trim();
    let rest = match rest.split(['?', '#']).next() {
        Some(first) => first.trim_end_matches('/'),
        None => rest,
    };
    let lower = rest.to_ascii_lowercase();
    let mut rest = rest;
    for scheme in ["https://", "http://"] {
        if lower.starts_with(scheme) {
            rest = &rest[scheme.len()..];
            break;
        }
    }
    rest = rest.strip_prefix("www.").unwrap_or(rest);
    let path = match rest.split_once('/') {
        Some((host, path)) if host.contains('.') => {
            if !host.eq_ignore_ascii_case("github.com") {
                anyhow::bail!("Only github.com repository links are supported, got {host}");
            }
            path
        }
        _ => rest,
    };
    let mut segments = path.split('/').filter(|s| !s.is_empty());
    let (Some(owner), Some(repo), None) = (segments.next(), segments.next(), segments.next())
    else {
        anyhow::bail!("Paste a repository link like https://github.com/owner/repo");
    };
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    if !repository_part(&owner.to_ascii_lowercase()) || !repository_part(&repo.to_ascii_lowercase())
    {
        anyhow::bail!("{owner}/{repo} is not a valid owner/repository name");
    }
    Ok(format!(
        "{}/{}",
        owner.to_ascii_lowercase(),
        repo.to_ascii_lowercase()
    ))
}
/// The catalog key for a repository name: lowercase alphanumeric only, so the
/// entry stays valid and usable in folders and settings.
fn key_for_repo(repo: &str) -> Result<String> {
    let key: String = repo
        .to_ascii_lowercase()
        .bytes()
        .filter(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        .map(char::from)
        .collect();
    if key.is_empty() {
        anyhow::bail!("The repository name {repo} has no usable characters");
    }
    Ok(key)
}
/// Accept release titles only when they name the repository's app.
/// Generic labels and release nicknames must not become app identities.
fn release_title(name: Option<&str>, repo: &str) -> String {
    if let Some(name) = name {
        let name = regex::Regex::new(r"\s+v?\d+\.\d+\.\d+.*$")
            .unwrap()
            .replace(name.trim(), "");
        if same_app_title(&name, repo) {
            return name.into_owned();
        }
    }
    repo.split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(pretty)
        .collect::<Vec<_>>()
        .join(" ")
}
fn same_app_title(title: &str, repo: &str) -> bool {
    let normalize = |value: &str| {
        value
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    !title.trim().is_empty() && normalize(title) == normalize(repo)
}
#[cfg(test)]
#[path = "custom_title_tests.rs"]
mod custom_title_tests;
#[derive(Clone, Debug)]
pub struct DownloadChoice {
    pub entry: Entry,
    pub assets: Vec<crate::model::Asset>,
    pub version: String,
    pub preferences: crate::model::Preferences,
}
impl std::fmt::Display for DownloadChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Choose a compatible download for {}", self.entry.title)
    }
}
impl std::error::Error for DownloadChoice {}
pub fn choose_custom_download(root: &Path, choice: &DownloadChoice, index: usize) -> Result<Entry> {
    let asset = choice
        .assets
        .get(index)
        .ok_or_else(|| anyhow::anyhow!("Choose a download first"))?;
    let customs = custom_apps(root);
    let mut entry = customs
        .iter()
        .find(|entry| entry.key == choice.entry.key)
        .cloned()
        .unwrap_or_else(|| choice.entry.clone());
    anyhow::ensure!(
        entry.repository == choice.entry.repository,
        "The repository changed while choosing a download"
    );
    let rule = crate::downloads::Rule::new(asset, &choice.version, &choice.preferences);
    if let Some(previous) = &entry.download_rule {
        if !entry.download_rules.contains(previous) {
            entry.download_rules.push(previous.clone());
        }
    }
    entry
        .download_rules
        .retain(|previous| !previous.same_target(&rule));
    entry.download_rules.push(rule.clone());
    entry.download_rule = Some(rule);
    entry.scheme = Scheme::Generic;
    if customs.iter().any(|existing| existing.key == entry.key) {
        let mut apps = customs;
        if let Some(existing) = apps.iter_mut().find(|existing| existing.key == entry.key) {
            *existing = entry.clone();
        }
        write_custom_apps(root, &apps)?;
        load(root);
        Ok(entry)
    } else {
        save_custom_app(root, entry)
    }
}

/// Resolve a registered custom app for the current package preferences.
pub fn prepare_custom_download(
    root: &Path,
    app: &str,
    prefs: &crate::model::Preferences,
    tag: Option<&str>,
) -> Result<Entry> {
    let entry = custom_apps(root)
        .into_iter()
        .find(|entry| entry.key == app)
        .ok_or_else(|| anyhow::anyhow!("Custom app is no longer registered"))?;
    let mut url = reqwest::Url::parse(&format!(
        "https://api.github.com/repos/{}/releases/",
        entry.repository
    ))?;
    if let Some(tag) = tag {
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("Invalid release endpoint"))?
            .pop_if_empty()
            .push("tags")
            .push(tag);
    } else {
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("Invalid release endpoint"))?
            .pop_if_empty()
            .push("latest");
    }
    let release: Release = Network::new(root)?.json(url.as_str())?;
    anyhow::ensure!(
        !release.draft && !release.prerelease,
        "Choose a stable release"
    );
    if let Some(tag) = tag {
        anyhow::ensure!(release.tag_name == tag, "The requested release changed");
    }
    let version = crate::updates::release_version(&release.tag_name)?;
    if let Ok(asset) = crate::updates::select_asset(&release, app, prefs) {
        let choice = DownloadChoice {
            entry,
            assets: vec![asset.clone()],
            version,
            preferences: prefs.clone(),
        };
        return choose_custom_download(root, &choice, 0);
    }
    let assets: Vec<_> = crate::downloads::candidates(&release, prefs)
        .into_iter()
        .cloned()
        .collect();
    anyhow::ensure!(
        !assets.is_empty(),
        "{} has no compatible {} download for {}. Your existing choices are preserved.",
        entry.title,
        prefs.release_format,
        prefs.architecture
    );
    let choice = DownloadChoice {
        entry,
        assets,
        version,
        preferences: prefs.clone(),
    };
    if choice.assets.len() == 1 && !crate::downloads::needs_confirmation(&choice.assets[0], prefs) {
        choose_custom_download(root, &choice, 0)
    } else {
        Err(choice.into())
    }
}

/// Builds a release-tracking entry for a user-supplied repository, verifying
/// that its latest stable release has an installable file for this machine.
fn classify_custom(
    slug: &str,
    repository: &Repository,
    release: &Release,
    release_format: &str,
    architecture: &str,
) -> Result<Entry> {
    if release.draft || release.prerelease {
        anyhow::bail!("{slug} has no stable release yet");
    }
    let version = crate::updates::release_version(&release.tag_name).map_err(|_| {
        anyhow::anyhow!("{slug} tags its releases in a way the manager cannot read")
    })?;
    let key = key_for_repo(&repository.name)?;
    anyhow::ensure!(
        !known_entries()
            .iter()
            .any(|known| !known.repository.contains('/') && known.key == key),
        "This app is already in the default catalog"
    );
    let repo_lower = repository.name.to_ascii_lowercase();
    let mut download_rule = None;
    let mut choices = Vec::new();
    let prefs = crate::model::Preferences {
        release_format: release_format.into(),
        architecture: architecture.into(),
        ..Default::default()
    };
    let (scheme, asset_prefix) = if crate::updates::find_asset(
        release,
        &[key.clone(), repo_lower],
        release_format,
        architecture,
    )
    .is_some()
    {
        (Scheme::Craft, key.clone())
    } else if let Some(prefix) =
        crate::updates::tauri_prefix(release, &version, release_format, architecture)
    {
        (Scheme::Tauri, prefix)
    } else {
        choices = crate::downloads::candidates(release, &prefs)
            .into_iter()
            .cloned()
            .collect();
        if choices.is_empty() {
            return Err(anyhow::Error::new(NoCompatibleCustomDownload).context(format!(
                "{slug} has no compatible {release_format} download for {} ({}). Try another architecture or release format.",
                crate::model::release_os(), crate::model::architecture_label(architecture)
            )));
        }
        if choices.len() == 1 && !crate::downloads::needs_confirmation(&choices[0], &prefs) {
            download_rule = Some(crate::downloads::Rule::new(&choices[0], &version, &prefs));
        }
        (Scheme::Generic, key.clone())
    };
    let entry = Entry {
        key,
        repository: slug.into(),
        title: release_title(release.name.as_deref(), &repository.name),
        category: repository_subtitle(
            &release_title(release.name.as_deref(), &repository.name),
            repository.description.as_deref().unwrap_or_default(),
        ),
        description: repository
            .description
            .clone()
            .unwrap_or_default()
            .trim()
            .into(),
        scheme,
        registered_formats: vec![release_format.into()],
        download_rules: download_rule.clone().into_iter().collect(),
        download_rule,
        asset_prefix,
        aliases: if scheme == Scheme::Generic {
            let base = repository
                .name
                .split(['-', '_'])
                .next()
                .unwrap_or(&repository.name)
                .to_ascii_lowercase();
            if valid_alias(&base) {
                vec![base.clone(), format!("{base}64"), format!("{base}32")]
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        },
        created_at: None,
        bundle_ids: Vec::new(),
        release: true,
        source: false,
    };
    if choices.len() > 1
        || choices
            .first()
            .is_some_and(|asset| crate::downloads::needs_confirmation(asset, &prefs))
    {
        return Err(DownloadChoice {
            entry,
            assets: choices,
            version,
            preferences: prefs,
        }
        .into());
    }
    Ok(entry)
}
/// A format/architecture miss is retryable; repository and network errors are not.
#[derive(Debug)]
struct NoCompatibleCustomDownload;
impl std::fmt::Display for NoCompatibleCustomDownload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("No compatible desktop release found for this system.")
    }
}
impl std::error::Error for NoCompatibleCustomDownload {}

/// Inspect the formats supported by this system without discarding inspection errors.
pub fn inspect_custom_app_for_system(root: &Path, link: &str) -> Result<(Entry, String, String)> {
    let mut architectures = vec![
        crate::install_options::host_architecture(),
        "x86".into(),
        "x64".into(),
        "arm64".into(),
    ];
    let mut seen = std::collections::HashSet::new();
    architectures
        .retain(|arch| crate::install_options::host_supports(arch) && seen.insert(arch.clone()));
    inspect_custom_app_options_with(architectures, |format, architecture| {
        inspect_custom_app(root, link, format, architecture)
    })
}

fn inspect_custom_app_options_with(
    architectures: impl IntoIterator<Item = String>,
    mut inspect: impl FnMut(&str, &str) -> Result<Entry>,
) -> Result<(Entry, String, String)> {
    for architecture in architectures {
        for format in ["installer", "portable"] {
            match inspect(format, &architecture) {
                Ok(entry) => return Ok((entry, format.into(), architecture)),
                Err(error) => {
                    if let Some(choice) = error.downcast_ref::<DownloadChoice>() {
                        return Ok((choice.entry.clone(), format.into(), architecture));
                    }
                    if error.downcast_ref::<NoCompatibleCustomDownload>().is_none() {
                        return Err(error);
                    }
                }
            }
        }
    }
    anyhow::bail!("No compatible desktop release found for this system.")
}

/// Adds a user-supplied GitHub repository to the catalog after verifying it.
pub fn inspect_custom_app(
    root: &Path,
    link: &str,
    release_format: &str,
    architecture: &str,
) -> Result<Entry> {
    let slug = parse_github_slug(link)?;
    let network = Network::new(root)?;
    let repository: Repository = network.json(&format!("https://api.github.com/repos/{slug}"))?;
    anyhow::ensure!(
        !repository.archived,
        "{slug} is archived. Choose an active repository instead."
    );
    anyhow::ensure!(!repository.disabled, "{slug} is disabled on GitHub.");
    anyhow::ensure!(
        !repository.fork,
        "{slug} is a fork. Choose the upstream repository instead."
    );
    let release: Option<Release> = network.json_optional(&format!(
        "https://api.github.com/repos/{slug}/releases/latest"
    ))?;
    let Some(release) = release else {
        anyhow::bail!("{slug} has no releases yet");
    };
    let description = repository
        .description
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    let has_gui = [
        "gui",
        "desktop",
        "electron",
        "tauri",
        "qt",
        "terminal-emulator",
        "terminal-emulators",
    ]
    .iter()
    .any(|hint| repository.topics.iter().any(|topic| topic == hint))
        || description.contains("graphical")
        || description.contains("terminal emulator");
    let cli = repository
        .topics
        .iter()
        .any(|topic| ["cli", "command-line", "command-line-tool"].contains(&topic.as_str()))
        || [
            "command-line",
            "command line",
            "a command-line",
            "a command line",
        ]
        .iter()
        .any(|hint| description.starts_with(hint));
    anyhow::ensure!(
        !cli || has_gui,
        "This repository describes a command-line tool. The manager supports desktop applications."
    );
    let entry = classify_custom(&slug, &repository, &release, release_format, architecture)?;
    Ok(entry)
}
pub fn add_custom_app(
    root: &Path,
    link: &str,
    release_format: &str,
    architecture: &str,
) -> Result<Entry> {
    let entry = inspect_custom_app(root, link, release_format, architecture)?;
    save_custom_app(root, entry)
}

fn save_custom_app(root: &Path, entry: Entry) -> Result<Entry> {
    let mut apps = custom_apps(root);
    anyhow::ensure!(
        !apps.iter().any(|existing| existing.key == entry.key
            && existing
                .registered_formats
                .iter()
                .any(|format| entry.registered_formats.contains(format))),
        "This app is already added for this release format"
    );
    merge_custom_registration(&mut apps, entry.clone())?;
    apps.sort_by(|a, b| a.key.cmp(&b.key));
    write_custom_apps(root, &apps)?;
    load(root);
    Ok(entry)
}
/// Removes a user-added app. Built-in and discovered apps cannot be removed.
pub fn remove_custom_app(root: &Path, key: &str) -> Result<Entry> {
    let mut apps = custom_apps(root);
    let Some(position) = apps.iter().position(|e| e.key == key) else {
        if all().iter().any(|e| e.key == key) {
            anyhow::bail!("{key} is built in and cannot be removed; hide it instead");
        }
        anyhow::bail!("Unknown custom app: {key}");
    };
    let removed = apps.remove(position);
    write_custom_apps(root, &apps)?;
    let cache: Cache = files::read_or_default(&cache_path(root)).unwrap_or_default();
    reload(root, &cache);
    Ok(removed)
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
    created_at: Option<String>,
    #[serde(default)]
    topics: Vec<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    fork: bool,
    #[serde(default)]
    disabled: bool,
}

/// Use the publisher's description rather than guessing an app's purpose.
/// Keep curated subtitles intact and bound discovery text to one short line.
fn repository_subtitle(title: &str, description: &str) -> String {
    let description = description.trim();
    let description = description
        .split_once(':')
        .filter(|(prefix, _)| prefix.trim().eq_ignore_ascii_case(title))
        .map_or(description, |(_, rest)| rest.trim());
    let clause = description.split([';', '\n']).next().unwrap_or_default();
    let clause = clause.split(". ").next().unwrap_or_default();
    let normalized = clause.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return "Desktop application".into();
    }
    if normalized.chars().count() <= 72 {
        return normalized;
    }
    let mut short: String = normalized.chars().take(69).collect();
    if let Some(space) = short.rfind(' ') {
        short.truncate(space);
    }
    format!("{}…", short.trim_end())
}

fn apply_repository_metadata(entry: &mut Entry, repository: &Repository) {
    if let Some(created_at) = repository.created_at.as_deref().and_then(|date| {
        chrono::DateTime::parse_from_rfc3339(date)
            .ok()
            .map(|date| date.timestamp())
    }) {
        entry.created_at = Some(created_at);
    }
    entry.description = repository
        .description
        .as_deref()
        .unwrap_or_default()
        .trim()
        .into();
    entry.category = repository_subtitle(&entry.title, &entry.description);
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
    // A catalog refresh may discover more apps, but is not an uninstall or
    // removal request. Keep existing identities even if a repository vanishes.
    let mut discovered = previous.entries.clone();
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
            let mut entry = entry.clone();
            apply_repository_metadata(&mut entry, &repository);
            retain_discovery(&mut discovered, entry);
            continue;
        }
        let release: Option<Release> = network.json_optional(&format!(
            "https://api.github.com/repos/{ORG}/{}/releases/latest",
            repository.name
        ))?;
        let mut entry = classify(&repository.name, release.as_ref());
        apply_repository_metadata(&mut entry, &repository);
        // Do not admit source-only candidates or infer a new app's identity
        // from an unrelated release asset. Existing curated aliases still work.
        if verified_discovery(&entry) {
            retain_discovery(&mut discovered, entry);
        }
    }
    let cache = Cache {
        checked_at: chrono::Utc::now().timestamp(),
        entries: discovered,
        learned: previous.learned,
    };
    files::write_json(&cache_path(root), &cache)?;
    reload(root, &cache);
    Ok(all())
}

fn verified_discovery(entry: &Entry) -> bool {
    entry.release && entry.asset_prefix.eq_ignore_ascii_case(&entry.repository)
}

fn retain_discovery(entries: &mut Vec<Entry>, entry: Entry) {
    if let Some(existing) = entries
        .iter_mut()
        .find(|e| e.repository == entry.repository)
    {
        // A formerly source-only candidate may have gained its first release.
        // Keep its stable key so saved build/installation state remains attached.
        let key = existing.key.clone();
        *existing = entry;
        existing.key = key;
    } else {
        entries.push(entry);
    }
}

/// Readable name for a repository without a curated title: `mapcraft` â†’ `MapCraft`.
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
        download_rule: None,
        download_rules: Vec::new(),
        registered_formats: Vec::new(),
        asset_prefix: String::new(),
        aliases: Vec::new(),
        created_at: None,
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
    // Multiple application prefixes are ambiguous; never choose the first
    // download and associate another app's settings or installation with it.
    let prefixes: std::collections::BTreeSet<_> = parsed
        .iter()
        .map(|asset| asset.prefix.to_ascii_lowercase())
        .collect();
    if prefixes.len() != 1 {
        return entry;
    }
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
        regex::Regex::new(r"^([a-z0-9]+)-(\d+\.\d+\.\d+)-(windows|macos|linux)-(?:x64|x86|x86_64|amd64|i686|i386|arm64|aarch64|universal)(-portable\.zip|\.msi|\.exe|\.dmg|\.AppImage|\.deb|\.rpm|\.pkg\.tar\.zst)$")
            .unwrap()
    });
    if let Some(c) = craft.captures(name) {
        let compatible = match &c[3] {
            "windows" => matches!(&c[4], "-portable.zip" | ".msi" | ".exe"),
            "macos" => &c[4] == ".dmg",
            "linux" => matches!(&c[4], ".AppImage" | ".deb" | ".rpm" | ".pkg.tar.zst"),
            _ => false,
        };
        if !compatible {
            return None;
        }
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
#[path = "tests.rs"]
mod tests;
