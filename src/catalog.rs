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
        // Third-party apps hosted outside the Storytold organization. Their
        // releases follow the Craft asset naming, so they update like the rest.
        Entry {
            key: "solvecraft".into(),
            repository: "bherbruck/solvecraft".into(),
            title: "SolveCraft".into(),
            category: "Crossword puzzles".into(),
            description: "Create, solve, and print crosswords with a free and open-source crossword editor built in Rust.".into(),
            scheme: Scheme::Craft,
            asset_prefix: "solvecraft".into(),
            aliases: Vec::new(),
            bundle_ids: Vec::new(),
            release: true,
            source: false,
        },
        Entry {
            key: "concat".into(),
            repository: "jub0t/concat".into(),
            title: "Concat".into(),
            category: "Version control".into(),
            description: "A fast, native desktop Git client written in Rust.".into(),
            scheme: Scheme::Craft,
            asset_prefix: "Concat".into(),
            aliases: Vec::new(),
            bundle_ids: Vec::new(),
            release: true,
            source: false,
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
    let cache: Cache = files::read_or_default(&cache_path(root)).unwrap_or_default();
    reload(root, &cache);
}
/// Rebuilds the process-wide catalog from saved discovery plus user apps.
fn reload(root: &Path, cache: &Cache) {
    *CURRENT.write().unwrap() = merge(cache.entries.clone(), &cache.learned, &custom_apps(root));
}
/// Apps the user added from GitHub links in Settings, newest last.
fn custom_path(root: &Path) -> PathBuf {
    root.join("runtime/custom-apps.json")
}
pub fn custom_apps(root: &Path) -> Vec<Entry> {
    files::read_or_default(&custom_path(root)).unwrap_or_default()
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
/// Readable title from a release name: `MapCraft v1.2.0` → `MapCraft`.
fn release_title(name: Option<&str>, repo: &str) -> String {
    if let Some(name) = name {
        let name = regex::Regex::new(r"\s+v?\d+\.\d+\.\d+.*$")
            .unwrap()
            .replace(name.trim(), "");
        if !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b' ') {
            return name.into_owned();
        }
    }
    let pretty = pretty(repo);
    if !pretty.trim().is_empty() {
        pretty
    } else {
        repo.into()
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
    let repo_lower = repository.name.to_ascii_lowercase();
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
        anyhow::bail!(
            "{slug} {version} has no installable file for this system. The manager tracks repositories whose releases carry Craft-style files ({key}-{version}-…​) or Tauri-style files."
        );
    };
    Ok(Entry {
        key,
        repository: slug.into(),
        title: release_title(release.name.as_deref(), &repository.name),
        category: String::new(),
        description: repository
            .description
            .clone()
            .unwrap_or_default()
            .trim()
            .into(),
        scheme,
        asset_prefix,
        aliases: Vec::new(),
        bundle_ids: Vec::new(),
        release: true,
        source: false,
    })
}
/// Adds a user-supplied GitHub repository to the catalog after verifying it.
pub fn add_custom_app(
    root: &Path,
    link: &str,
    release_format: &str,
    architecture: &str,
) -> Result<Entry> {
    let slug = parse_github_slug(link)?;
    if let Some(known) = all().iter().find(|e| e.repository == slug) {
        anyhow::bail!("{} is already in your app list", known.title);
    }
    let network = Network::new(root)?;
    let repository: Repository = network.json(&format!("https://api.github.com/repos/{slug}"))?;
    if repository.archived || repository.fork || repository.disabled {
        anyhow::bail!("{slug} is archived, disabled or a fork; pick the upstream repository");
    }
    let release: Option<Release> = network.json_optional(&format!(
        "https://api.github.com/repos/{slug}/releases/latest"
    ))?;
    let Some(release) = release else {
        anyhow::bail!("{slug} has no releases yet");
    };
    let entry = classify_custom(&slug, &repository, &release, release_format, architecture)?;
    if let Some(known) = all().iter().find(|e| e.key == entry.key) {
        anyhow::bail!("{} is already tracked as {}", known.title, known.repository);
    }
    let mut apps = custom_apps(root);
    apps.retain(|e| e.key != entry.key && e.repository != entry.repository);
    apps.push(entry.clone());
    apps.sort_by(|a, b| a.key.cmp(&b.key));
    write_custom_apps(root, &apps)?;
    let cache: Cache = files::read_or_default(&cache_path(root)).unwrap_or_default();
    reload(root, &cache);
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
    let cache = Cache {
        checked_at: chrono::Utc::now().timestamp(),
        entries: discovered,
        learned: previous.learned,
    };
    files::write_json(&cache_path(root), &cache)?;
    reload(root, &cache);
    Ok(all())
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
    #[test]
    fn fresh_catalog_uses_cache_without_a_network_request() {
        let root =
            std::env::temp_dir().join(format!("craft-catalog-cache-{}", uuid::Uuid::new_v4()));
        files::write_json(
            &cache_path(&root),
            &Cache {
                checked_at: chrono::Utc::now().timestamp(),
                entries: vec![craft("mapcraft", "MapCraft", "Maps", "")],
                ..Default::default()
            },
        )
        .unwrap();
        let result = refresh_if_older(&root, REFRESH_INTERVAL).unwrap();
        assert!(result.iter().any(|e| e.key == "mapcraft"));
        // Restore the process-wide catalog for the remaining serial tests.
        load(&root.join("unused"));
        std::fs::remove_dir_all(root).unwrap();
    }
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
            &[],
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
    #[test]
    fn parses_github_links_to_owner_slugs() {
        assert_eq!(
            parse_github_slug("https://github.com/Bherbruck/SolveCraft").unwrap(),
            "bherbruck/solvecraft"
        );
        assert_eq!(
            parse_github_slug("github.com/jub0t/concat/").unwrap(),
            "jub0t/concat"
        );
        assert_eq!(parse_github_slug("jub0t/concat").unwrap(), "jub0t/concat");
        assert_eq!(
            parse_github_slug("https://github.com/jub0t/concat.git").unwrap(),
            "jub0t/concat"
        );
        assert_eq!(
            parse_github_slug("https://github.com/jub0t/concat?tab=releases").unwrap(),
            "jub0t/concat"
        );
        for bad in [
            "",
            "not a link",
            "https://gitlab.com/owner/repo",
            "https://github.com/just-an-owner",
            "https://github.com/owner/repo/releases",
            "https://github.com/owner/repo/blob/main/README.md",
            "owner/repo/extra",
            "https://github.com/BAD OWNER/repo",
        ] {
            assert!(parse_github_slug(bad).is_err(), "{bad}");
        }
    }
    #[test]
    fn classifies_custom_apps_from_their_release() {
        fn platform_asset() -> (&'static str, &'static str) {
            if cfg!(target_os = "macos") {
                ("zookraft-1.2.0-macos-universal.dmg", "portable")
            } else if cfg!(target_os = "linux") {
                ("zookraft-1.2.0-linux-x86_64.AppImage", "portable")
            } else {
                ("zookraft-1.2.0-windows-x64.msi", "installer")
            }
        }
        let repo = Repository {
            name: "ZooKraft".into(),
            description: Some(" Puzzles ".into()),
            archived: false,
            fork: false,
            disabled: false,
        };
        let (asset, format) = platform_asset();
        let craft_release = release("v1.2.0", "ZooKraft v1.2.0", &[asset]);
        let entry =
            classify_custom("someone/zookraft", &repo, &craft_release, format, "x64").unwrap();
        assert_eq!(
            (entry.key.as_str(), entry.repository.as_str()),
            ("zookraft", "someone/zookraft")
        );
        assert_eq!(
            (entry.scheme, entry.release, entry.source),
            (Scheme::Craft, true, false)
        );
        assert_eq!(entry.asset_prefix.as_str(), "zookraft");
        assert_eq!(entry.title.as_str(), "ZooKraft");
        assert_eq!(entry.description.as_str(), "Puzzles");
        if cfg!(target_os = "macos") {
            let tauri_release = release("1.2.0", "", &["Zoo_1.2.0_universal.dmg"]);
            let entry =
                classify_custom("someone/zookraft", &repo, &tauri_release, "portable", "x64")
                    .unwrap();
            assert_eq!(
                (entry.scheme, entry.asset_prefix.as_str()),
                (Scheme::Tauri, "Zoo")
            );
        } else if cfg!(target_os = "windows") {
            let tauri_release = release("1.2.0", "", &["Zoo_1.2.0_x64-setup.exe"]);
            let entry = classify_custom(
                "someone/zookraft",
                &repo,
                &tauri_release,
                "installer",
                "x64",
            )
            .unwrap();
            assert_eq!(
                (entry.scheme, entry.asset_prefix.as_str()),
                (Scheme::Tauri, "Zoo")
            );
        }
        let web = release("v1.2.0", "", &["zookraft-1.2.0-web.zip"]);
        assert!(classify_custom("someone/zookraft", &repo, &web, format, "x64").is_err());
        let draft = Release {
            draft: true,
            ..release("v1.2.0", "", &[asset])
        };
        assert!(classify_custom("someone/zookraft", &repo, &draft, format, "x64").is_err());
    }
    #[test]
    fn custom_apps_persist_and_win_over_discovery() {
        let root =
            std::env::temp_dir().join(format!("craft-catalog-custom-{}", uuid::Uuid::new_v4()));
        let mine = Entry {
            key: "zookraft".into(),
            repository: "someone/zookraft".into(),
            title: "Zoo".into(),
            category: String::new(),
            description: String::new(),
            scheme: Scheme::Craft,
            asset_prefix: "zookraft".into(),
            aliases: Vec::new(),
            bundle_ids: Vec::new(),
            release: true,
            source: false,
        };
        files::write_json(&custom_path(&root), &vec![mine.clone()]).unwrap();
        load(&root);
        assert!(all()
            .iter()
            .any(|e| e.key == "zookraft" && e.repository == "someone/zookraft"));
        // A discovered entry for the same repository is dropped, never duplicated.
        let merged = merge(
            vec![mine.clone()],
            &std::collections::BTreeMap::new(),
            &custom_apps(&root),
        );
        assert_eq!(merged.iter().filter(|e| e.key == "zookraft").count(), 1);
        let removed = remove_custom_app(&root, "zookraft").unwrap();
        assert_eq!(removed.repository, "someone/zookraft");
        assert!(remove_custom_app(&root, "zookraft").is_err());
        assert!(remove_custom_app(&root, "designcraft").is_err());
        // Restore the process-wide catalog for the remaining serial tests.
        load(&root.join("unused"));
        assert!(all().iter().all(|e| e.key != "zookraft"));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn third_party_apps_use_an_owner_slug_and_bare_file_names() {
        assert_eq!(crate::model::repository("filmcraft"), "storytold/filmcraft");
        assert_eq!(
            crate::model::repository("solvecraft"),
            "bherbruck/solvecraft"
        );
        assert_eq!(crate::model::repository("concat"), "jub0t/concat");
        assert_eq!(crate::model::repository_name("solvecraft"), "solvecraft");
        assert_eq!(crate::model::repository_name("concat"), "concat");
        assert_eq!(names("concat"), ["concat"]);
        assert!(crate::model::apps().iter().any(|a| a == "solvecraft"));
        assert!(crate::model::apps().iter().any(|a| a == "concat"));
    }
}
