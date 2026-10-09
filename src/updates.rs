use crate::{
    backups, files,
    jobs::Job,
    model::{Asset, Paths, Preferences, Release, Source},
    network::Network,
    platform,
};
use anyhow::{bail, Context, Result};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
/// Parses `1.2.3`, `v1.2.3` and app-prefixed tags such as `artcraft-v0.41.0`.
pub fn version(v: &str) -> Result<(u64, u64, u64)> {
    let nums: Vec<_> = v
        .rsplit_once("-v")
        .map_or(v, |(_, version)| version)
        .trim_start_matches('v')
        .split('.')
        .map(str::parse::<u64>)
        .collect::<std::result::Result<_, _>>()?;
    if nums.len() != 3 {
        bail!("Unsupported version: {v}");
    }
    Ok((nums[0], nums[1], nums[2]))
}
/// The plain `x.y.z` version of a release tag.
pub fn release_version(tag: &str) -> Result<String> {
    let (a, b, c) = version(tag)?;
    Ok(format!("{a}.{b}.{c}"))
}
pub fn check_app(paths: &Paths, app: &str) -> Result<Option<String>> {
    crate::model::valid_app(app)?;
    let installed = crate::apps::installed(paths, app)?;
    let release: Release = Network::new(&paths.root)?.json(&format!(
        "https://api.github.com/repos/storytold/{}/releases/latest",
        crate::model::repository(app)
    ))?;
    if release.draft || release.prerelease {
        bail!("No stable release available");
    }
    if version(&release.tag_name)? > version(&installed.version)? {
        select_asset(&release, app, &paths.preferences()?)?;
        Ok(Some(release_version(&release.tag_name)?))
    } else {
        Ok(None)
    }
}
pub fn installed_check_targets(config: &crate::model::Config) -> Vec<crate::model::Installed> {
    let apps = crate::model::apps();
    config
        .apps
        .iter()
        .filter(|app| apps.contains(&app.name) && !app.path.is_empty() && !app.version.is_empty())
        .cloned()
        .collect()
}
/// Chooses the release file for this system, architecture and release format.
pub fn select_asset<'a>(r: &'a Release, name: &str, p: &Preferences) -> Result<&'a Asset> {
    let release = release_version(&r.tag_name)?;
    let entry = crate::catalog::get(name).context("Unknown app")?;
    if entry.scheme == crate::catalog::Scheme::Tauri {
        return tauri_asset(r, &entry, &release, p);
    }
    // Newest names first, like the repository name the release now uses.
    let mut known = crate::catalog::names(name);
    known.reverse();
    if let Some(asset) = asset_named(r, &known, p)? {
        return Ok(asset);
    }
    // A release from the app's own repository whose files use one new name is
    // the same app renamed (printcraft → pdfcraft).
    let renamed: std::collections::BTreeSet<String> = r
        .assets
        .iter()
        .filter_map(|a| crate::catalog::parse_asset(&a.name))
        .filter(|a| {
            a.scheme == crate::catalog::Scheme::Craft
                && a.version == release
                && a.os == crate::model::release_os()
                && !known.iter().any(|k| k.eq_ignore_ascii_case(&a.prefix))
        })
        .map(|a| a.prefix)
        .collect();
    if renamed.len() == 1 {
        if let Some(asset) = asset_named(r, &renamed.into_iter().collect::<Vec<_>>(), p)? {
            return Ok(asset);
        }
    }
    bail!(
        "This release has no {} {}. Choose another option in Settings.",
        p.architecture,
        p.release_format
    )
}
/// Release file whose name starts with one of `prefixes`.
fn asset_named<'a>(
    r: &'a Release,
    prefixes: &[String],
    p: &Preferences,
) -> Result<Option<&'a Asset>> {
    let release = release_version(&r.tag_name)?;
    let mut names = Vec::new();
    for asset_name in prefixes {
        let prefix = format!(
            "{asset_name}-{}-{}-{}",
            release,
            crate::model::release_os(),
            crate::model::release_arch(&p.architecture)
        );
        let package_names = if cfg!(target_os = "macos") {
            // The same DMG serves both formats: portable copies its app bundle
            // into the library, installer copies it into Applications.
            vec![format!("{prefix}.dmg")]
        } else if cfg!(target_os = "linux") {
            vec![format!(
                "{prefix}{}",
                if p.release_format == "installer" {
                    crate::installers::installer_extension()?
                } else {
                    ".AppImage"
                }
            )]
        } else if p.release_format == "installer" {
            vec![format!("{prefix}.msi"), format!("{prefix}.exe")]
        } else {
            vec![format!("{prefix}-portable.zip")]
        };
        names.extend(package_names);
        #[cfg(target_os = "linux")]
        if p.release_format == "installer"
            && crate::installers::package_kind()? == crate::installers::PackageKind::Arch
        {
            let start = format!("{asset_name}-{release}-");
            let end = format!(
                "-{}.pkg.tar.zst",
                crate::model::release_arch(&p.architecture)
            );
            let mut candidates = Vec::new();
            for asset in &r.assets {
                if let Some(release) = asset
                    .name
                    .strip_prefix(&start)
                    .and_then(|value| value.strip_suffix(&end))
                {
                    if release.split('.').all(|part| {
                        !part.is_empty() && part.bytes().all(|value| value.is_ascii_digit())
                    }) {
                        candidates.push(asset.name.clone());
                    }
                }
            }
            if candidates.len() > 1 {
                bail!("Ambiguous Arch release assets");
            }
            names.extend(candidates);
        }
    }
    names.dedup();
    for n in names {
        let assets: Vec<_> = r.assets.iter().filter(|a| a.name == n).collect();
        if assets.len() > 1 {
            bail!("Ambiguous release assets");
        }
        if let Some(a) = assets.first() {
            return Ok(Some(a));
        }
    }
    Ok(None)
}
/// ArtCraft-style releases: `ArtCraft_0.41.0_x64_en-US.msi`, `ArtCraft_0.41.0_universal.dmg`.
fn tauri_asset<'a>(
    r: &'a Release,
    entry: &crate::catalog::Entry,
    release: &str,
    p: &Preferences,
) -> Result<&'a Asset> {
    let prefix = regex::escape(&entry.asset_prefix);
    let version = regex::escape(release);
    let arch = match p.architecture.as_str() {
        "x86" => "x86",
        "arm64" => "(?:arm64|aarch64)",
        _ => "x64",
    };
    let patterns: Vec<String> = if cfg!(target_os = "macos") {
        vec![
            format!("^{prefix}_{version}_universal\\.dmg$"),
            format!("^{prefix}_{version}_{arch}\\.dmg$"),
        ]
    } else if cfg!(target_os = "windows") {
        if p.release_format != "installer" {
            bail!(
                "{} is only published as an installer. Choose Installer in Settings.",
                entry.title
            );
        }
        vec![
            format!("^{prefix}_{version}_{arch}_[A-Za-z]{{2}}-[A-Za-z]{{2}}\\.msi$"),
            format!("^{prefix}_{version}_{arch}-setup\\.exe$"),
        ]
    } else if p.release_format == "installer" {
        vec![format!("(?i)^{prefix}_{version}_amd64\\.deb$")]
    } else {
        vec![format!("^{prefix}_{version}_amd64\\.AppImage$")]
    };
    for pattern in patterns {
        let pattern = regex::Regex::new(&pattern)?;
        let assets: Vec<_> = r
            .assets
            .iter()
            .filter(|a| pattern.is_match(&a.name))
            .collect();
        if assets.len() > 1 {
            bail!("Ambiguous release assets");
        }
        if let Some(asset) = assets.first() {
            return Ok(asset);
        }
    }
    bail!(
        "This {} release has no {} package for this system.",
        entry.title,
        p.release_format
    )
}
fn backup_path(paths: &Paths, name: &str, v: &str, source: bool) -> PathBuf {
    paths
        .at(if source {
            "backups/sources"
        } else {
            "backups/releases"
        })
        .join(if source {
            format!(
                "{name}-source-{}-{}.zip",
                &v[..7],
                uuid::Uuid::new_v4().simple()
            )
        } else {
            format!("{name}-{v}-{}", uuid::Uuid::new_v4().simple())
        })
}
// Commit the filesystem and metadata together. Restore the old copy if metadata cannot be saved.
pub fn replace_transaction(
    staged: &Path,
    destination: &Path,
    backup: Option<&Path>,
    commit: impl FnOnce() -> Result<()>,
) -> Result<()> {
    if destination.exists() {
        let b = backup.context("Missing rollback path")?;
        fs::create_dir_all(b.parent().unwrap())?;
        fs::rename(destination, b)?;
    }
    fs::create_dir_all(destination.parent().unwrap())?;
    if let Err(e) = fs::rename(staged, destination) {
        if let Some(b) = backup {
            if b.exists() {
                fs::rename(b, destination).context("Could not restore rollback copy")?;
            }
        }
        return Err(e.into());
    }
    if let Err(e) = commit() {
        fs::rename(destination, staged).context("Could not remove uncommitted replacement")?;
        if let Some(b) = backup {
            if b.exists() {
                fs::rename(b, destination).context("Could not restore previous version")?;
            }
        }
        return Err(e);
    }
    Ok(())
}
pub fn releases(paths: &Paths, job: &Job, background: bool) -> Result<()> {
    if background {
        return crate::hourly::run(paths, job);
    }
    releases_for(paths, job, None)
}
pub fn install_app(paths: &Paths, app: &str, job: &Job) -> Result<()> {
    crate::model::valid_app(app)?;
    if !crate::model::apps().iter().any(|a| a == app) {
        bail!("This app has no managed release");
    }
    releases_for(paths, job, Some(app))
}
#[derive(Clone, Debug)]
pub struct ReleaseEntry {
    pub app: String,
    pub action: String,
    pub version: String,
    pub asset: Option<Asset>,
    pub destination: Option<PathBuf>,
    pub error: Option<String>,
}
#[derive(Clone, Debug)]
pub struct ReleasePlan {
    pub entries: Vec<ReleaseEntry>,
    pub preferences: Preferences,
    snapshot: Vec<u8>,
}
impl ReleasePlan {
    pub fn executable_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.action == "Install" || e.action == "Update")
            .count()
    }
}
fn state_snapshot(
    paths: &Paths,
    prefs: &Preferences,
    config: &crate::model::Config,
) -> Result<Vec<u8>> {
    let destinations: Vec<_> = config
        .apps
        .iter()
        .filter(|a| prefs.selected_apps.contains(&a.name))
        .map(|a| {
            let target = if prefs.release_format == "installer" && !a.path.is_empty() {
                PathBuf::from(&a.path)
            } else if a.path.is_empty() || a.install_kind == "installer" {
                paths.at(format!("releases/{}", a.name))
            } else {
                PathBuf::from(&a.path)
            };
            let metadata = fs::symlink_metadata(&target).ok();
            let executable = crate::model::installed_executable(&target, &a.name);
            let executable_state = executable.as_ref().map(|path| {
                let metadata = fs::symlink_metadata(path).ok();
                (
                    path.clone(),
                    metadata.map(|m| (m.len(), m.modified().ok())),
                    crate::platform::executable_version(path),
                )
            });
            (
                target,
                executable_state,
                metadata
                    .as_ref()
                    .map(|m| (m.len(), m.modified().ok(), m.file_type().is_symlink())),
            )
        })
        .collect();
    Ok(serde_json::to_vec(&(prefs, config, destinations))?)
}
pub fn plan_releases(paths: &Paths, job: &Job) -> Result<ReleasePlan> {
    let network = Network::new(&paths.root)?;
    plan_with(paths, job, None, |name| {
        network.json(&format!(
            "https://api.github.com/repos/storytold/{}/releases/latest",
            crate::model::repository(name)
        ))
    })
}
fn plan_with(
    paths: &Paths,
    job: &Job,
    selected: Option<&str>,
    mut fetch: impl FnMut(&str) -> Result<Release>,
) -> Result<ReleasePlan> {
    let prefs = paths.read_preferences()?;
    let config = paths.config()?;
    let snapshot = state_snapshot(paths, &prefs, &config)?;
    let mut entries = Vec::new();
    for app in config.apps.iter().filter(|a| {
        selected
            .map(|name| a.name == name)
            .unwrap_or_else(|| prefs.selected_apps.contains(&a.name))
    }) {
        if let Err(error) = job.check() {
            let previous: Vec<_> = entries
                .iter()
                .filter_map(|entry: &ReleaseEntry| entry.error.as_ref())
                .collect();
            if !previous.is_empty() {
                bail!(
                    "Planning failed before cancellation: {}",
                    previous
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join("; ")
                );
            }
            return Err(error);
        }
        job.stage("Planning releases", None, crate::model::title(&app.name));
        let mut entry = ReleaseEntry {
            app: app.name.clone(),
            action: "Error".into(),
            version: String::new(),
            asset: None,
            destination: None,
            error: None,
        };
        let result = (|| -> Result<()> {
            let release = fetch(&app.name)?;
            if release.draft || release.prerelease {
                bail!("Not a stable release");
            }
            let asset = select_asset(&release, &app.name, &prefs)?;
            // Remember a new file name so extraction and detection find the program.
            if let Some(parsed) = crate::catalog::parse_asset(&asset.name) {
                crate::catalog::learn_alias(&paths.root, &app.name, &parsed.prefix)?;
            }
            if !asset.browser_download_url.starts_with(&format!(
                "https://github.com/storytold/{}/releases/download/",
                crate::model::repository(&app.name)
            )) {
                bail!("Unexpected asset URL");
            }
            files::safe_relative(&asset.name)?;
            let digest = asset
                .digest
                .as_deref()
                .and_then(|digest| digest.strip_prefix("sha256:"))
                .context("Release asset has no published SHA-256 digest")?;
            if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                bail!("Release asset has an invalid SHA-256 digest");
            }

            entry.version = release_version(&release.tag_name)?;
            entry.asset = Some(asset.clone());
            let installer = prefs.release_format == "installer";
            let target = if installer {
                PathBuf::from(&app.path)
            } else if app.path.is_empty() || app.install_kind == "installer" {
                paths.at(format!("releases/{}", app.name))
            } else {
                PathBuf::from(&app.path)
            };
            if !installer {
                files::inside(&target, &paths.at("releases"))?;
                entry.destination = Some(target.clone());
            }
            let exists = !app.path.is_empty()
                && (app.install_kind == "installer") == installer
                && crate::model::installed_executable(&target, &app.name).is_some();
            entry.action = if exists
                && !app.version.is_empty()
                && version(&app.version)? >= version(&entry.version)?
                && (installer
                    || crate::model::release_arch(&app.architecture)
                        == crate::model::release_arch(&prefs.architecture))
            {
                "Skip"
            } else if exists {
                "Update"
            } else {
                "Install"
            }
            .into();
            Ok(())
        })();
        if let Err(error) = result {
            if crate::jobs::is_cancelled(&error) {
                let previous: Vec<_> = entries
                    .iter()
                    .filter_map(|entry: &ReleaseEntry| entry.error.as_deref())
                    .collect();
                if !previous.is_empty() {
                    bail!(
                        "Planning failed before cancellation: {}",
                        previous.join("; ")
                    );
                }
                return Err(error);
            }
            entry.error = Some(format!("{error:#}"));
        }
        entries.push(entry);
    }
    if let Err(error) = job.check() {
        let previous: Vec<_> = entries
            .iter()
            .filter_map(|entry| entry.error.as_deref())
            .collect();
        if !previous.is_empty() {
            bail!(
                "Planning failed before cancellation: {}",
                previous.join("; ")
            );
        }
        return Err(error);
    }
    Ok(ReleasePlan {
        entries,
        preferences: prefs,
        snapshot,
    })
}
pub fn execute_plan(paths: &Paths, plan: &ReleasePlan, job: &Job) -> Result<()> {
    execute_validated(paths, plan, |config| {
        execute_entries(paths, plan, config, job)
    })
}
fn execute_validated(
    paths: &Paths,
    plan: &ReleasePlan,
    execute: impl FnOnce(&mut crate::model::Config) -> Result<()>,
) -> Result<()> {
    let _lock = platform::Lock::take("Local\\CraftAppsManager")?;
    let prefs = paths.read_preferences()?;
    let mut config = paths.config()?;
    if state_snapshot(paths, &prefs, &config)? != plan.snapshot {
        bail!("Release preferences, installations, or destinations changed. Review a new plan before installing.");
    }
    execute(&mut config)
}
fn execute_entries(
    paths: &Paths,
    plan: &ReleasePlan,
    config: &mut crate::model::Config,
    job: &Job,
) -> Result<()> {
    let prefs = &plan.preferences;
    let network = Network::new(&paths.root)?;
    execute_with(plan, job, |entry| {
        let i = config
            .apps
            .iter()
            .position(|a| a.name == entry.app)
            .context("Planned app missing")?;
        let app = config.apps[i].clone();
        let asset = entry.asset.as_ref().context("Planned asset missing")?;
        let release = Release {
            tag_name: entry.version.clone(),
            name: None,
            draft: false,
            prerelease: false,
            assets: vec![],
        };
        let result = (|| -> Result<bool> {
            if prefs.release_format == "installer" {
                if platform::running_app(&app.name)? {
                    bail!("Close {} before installing", app.name);
                }
                let dest = paths.at(format!("releases/installers/{}/{}", app.name, asset.name));
                if dest.exists() {
                    crate::network::verify_asset(&dest, asset)?;
                } else {
                    network.asset(asset, &dest, job)?;
                }
                job.stage(
                    "Installing",
                    None,
                    if cfg!(target_os = "windows") {
                        "Complete the Windows installer wizard; Windows may ask for administrator permission."
                    } else {
                        "Installing the app."
                    },
                );
                let mut installed = crate::installers::run_with_job(&dest, &app.name, Some(job))?;
                installed.architecture = if cfg!(target_os = "macos") {
                    "universal".into()
                } else {
                    prefs.architecture.clone()
                };
                config.apps[i] = installed;
                paths.save_config(config)?;
                job.log(&format!("{}: installer completed", app.name));
                return Ok(true);
            }
            let v = release.tag_name.trim_start_matches('v');
            let target = entry
                .destination
                .clone()
                .context("Missing portable destination")?;
            if platform::running_app(&app.name)? {
                job.log(&format!("{}: app is open; skipped", app.name));
                return Ok(false);
            }
            let cache = paths.at("runtime/downloads");
            let archive = cache.join(&asset.name);
            let stage = cache.join(uuid::Uuid::new_v4().simple().to_string());
            let attempt = (|| -> Result<()> {
                network.asset(asset, &archive, job)?;
                if cfg!(target_os = "macos") {
                    #[cfg(target_os = "macos")]
                    crate::installers::extract_app(&archive, &stage, &app.name, job)?;
                } else if cfg!(target_os = "linux") {
                    fs::create_dir_all(&stage)?;
                    let executable = stage.join(crate::model::executable_name(&app.name));
                    fs::copy(&archive, &executable)?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))?;
                    }
                } else {
                    files::extract_zip(&archive, &stage, job)?;
                }
                let executables: Vec<_> = walkdir::WalkDir::new(&stage)
                    .into_iter()
                    .collect::<std::result::Result<Vec<_>, _>>()?
                    .into_iter()
                    .filter(|e| {
                        !e.path_is_symlink()
                            && crate::model::is_executable(e.path())
                            && crate::model::executable_names(&app.name)
                                .iter()
                                .any(|name| e.file_name().to_string_lossy() == *name)
                    })
                    .collect();
                if executables.len() != 1 {
                    bail!("Release must contain exactly one app executable");
                }
                job.check()?;
                if platform::running_app(&app.name)? {
                    bail!("App opened during download; update deferred")
                }
                let backup = if target.exists() {
                    version(&app.version)?;
                    Some(backup_path(paths, &app.name, &app.version, false))
                } else {
                    None
                };
                let old = config.apps[i].clone();
                config.apps[i].path = target.to_string_lossy().into_owned();
                config.apps[i].version = v.into();
                config.apps[i].architecture = if cfg!(target_os = "macos") {
                    "universal".into()
                } else {
                    prefs.architecture.clone()
                };
                config.apps[i].install_kind = "portable".into();
                config.apps[i].product_code.clear();
                if let Err(e) = replace_transaction(
                    executables[0].path().parent().unwrap(),
                    &target,
                    backup.as_deref(),
                    || paths.save_config(config),
                ) {
                    config.apps[i] = old;
                    return Err(e);
                }
                if let Err(e) =
                    backups::finish(paths, prefs, backup.as_deref(), &app.name, false, job)
                {
                    job.log(&format!("Backup cleanup warning: {e:#}"))
                }
                if let Err(error) = crate::profiles::restore_portable(paths, &app.name, &target) {
                    job.log(&format!(
                        "Retained profile was kept for recovery: {error:#}"
                    ));
                }
                let shortcut = paths.at(format!(
                    "releases/{}.{}",
                    app.name,
                    crate::platform::shortcut_extension()
                ));
                if let Err(e) = platform::shortcut(
                    &shortcut,
                    &crate::model::installed_executable(&target, &app.name)
                        .context("Installed executable is missing")?,
                    "",
                    &target,
                ) {
                    job.log(&format!("Shortcut warning: {e}"))
                }
                job.log(&format!("{}: updated to {v}", app.name));
                Ok(())
            })();
            for p in [&stage, &archive] {
                if p.exists() {
                    let _ = files::remove_managed(p, &cache);
                }
            }
            attempt.map(|()| true)
        })();
        result
    })
}
#[derive(Debug, Default)]
pub struct ReleaseSummary {
    pub completed: usize,
    pub skipped: usize,
    pub failed: usize,
    pub remaining: usize,
    pub current: Option<String>,
}
impl std::fmt::Display for ReleaseSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} completed, {} skipped, {} failed, {} remaining{}",
            self.completed,
            self.skipped,
            self.failed,
            self.remaining,
            self.current
                .as_ref()
                .map(|app| format!("; interrupted: {app}"))
                .unwrap_or_default()
        )
    }
}
fn execute_with(
    plan: &ReleasePlan,
    job: &Job,
    mut execute: impl FnMut(&ReleaseEntry) -> Result<bool>,
) -> Result<()> {
    let mut summary = ReleaseSummary {
        skipped: plan
            .entries
            .iter()
            .filter(|e| e.action == "Skip" || e.error.is_some())
            .count(),
        ..Default::default()
    };
    let executable: Vec<_> = plan
        .entries
        .iter()
        .filter(|e| (e.action == "Install" || e.action == "Update") && e.error.is_none())
        .collect();
    let mut cancelled = false;
    let mut handled = 0;
    for entry in &executable {
        if job.check().is_err() {
            cancelled = true;
            break;
        }
        summary.current = Some(entry.app.clone());
        let result = execute(entry);
        match result {
            Ok(true) => summary.completed += 1,
            Ok(false) => summary.skipped += 1,
            Err(error) if crate::jobs::is_cancelled(&error) => {
                cancelled = true;
                break;
            }
            Err(error) => {
                summary.failed += 1;
                job.log(&format!("{}: {error:#}", entry.app));
                if error.to_string().contains("API limit") {
                    handled += 1;
                    summary.current = None;
                    break;
                }
            }
        }
        handled += 1;
        summary.current = None;
    }
    summary.remaining = executable.len().saturating_sub(handled);
    job.log(&summary.to_string());
    if summary.failed > 0 {
        bail!("Release operations: {summary}");
    }
    if cancelled {
        return Err(anyhow::Error::new(crate::jobs::Cancelled).context(summary));
    }
    Ok(())
}
fn releases_for(paths: &Paths, job: &Job, selected: Option<&str>) -> Result<()> {
    // CLI retains its deliberate install/update scope; GUI uses plan + confirmation.
    let _lock = platform::Lock::take("Local\\CraftAppsManager")?;
    let network = Network::new(&paths.root)?;
    let plan = plan_with(paths, job, selected, |name| {
        network.json(&format!(
            "https://api.github.com/repos/storytold/{}/releases/latest",
            crate::model::repository(name)
        ))
    })?;
    let errors: Vec<_> = plan
        .entries
        .iter()
        .filter_map(|e| e.error.as_ref().map(|error| format!("{}: {error}", e.app)))
        .collect();
    for error in &errors {
        job.log(error);
    }
    let mut config = paths.config()?;
    let result = execute_entries(paths, &plan, &mut config, job);
    if !errors.is_empty() {
        bail!("{} release planning error(s); see the log", errors.len());
    }
    result
}
pub fn sources(paths: &Paths, names: &[String], job: &Job) -> Result<()> {
    let _lock = platform::Lock::take("Local\\CraftAppsManager")?;
    job.log(&format!(
        "\nSource updates — {}",
        chrono::Utc::now().to_rfc3339()
    ));
    let prefs = paths.preferences()?;
    let network = Network::new(&paths.root)?;
    let mut index: BTreeMap<String, Source> =
        files::read_or_default(&paths.at("sources/source-index.json"))?;
    let mut errors = 0;
    for name in names {
        crate::model::valid_app(name)?;
        if let Err(error) = job.check() {
            if errors > 0 {
                bail!("{errors} source update(s) failed before cancellation; see the log");
            }
            return Err(error);
        }
        job.stage("Checking source", None, crate::model::title(name));
        let result = (|| -> Result<()> {
            let repository = crate::model::repository(name);
            let repo: serde_json::Value = network.json(&format!(
                "https://api.github.com/repos/storytold/{repository}"
            ))?;
            let branch = repo["default_branch"]
                .as_str()
                .context("No default branch")?;
            let mut endpoint = reqwest::Url::parse(&format!(
                "https://api.github.com/repos/storytold/{repository}/commits/"
            ))?;
            endpoint
                .path_segments_mut()
                .map_err(|_| anyhow::anyhow!("Invalid API endpoint"))?
                .pop_if_empty()
                .push(branch);
            let commit: serde_json::Value = network.json(endpoint.as_str())?;
            let sha = commit["sha"].as_str().context("No commit")?;
            if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
                bail!("Invalid source commit")
            }
            let destination = paths.at(format!("sources/{name}-source.zip"));
            files::inside(&destination, &paths.at("sources"))?;
            let old = index.get(name).cloned();
            if destination.exists() {
                if files::linked(&destination)? {
                    bail!("Linked source archive")
                };
                let old = old
                    .as_ref()
                    .context("Existing ZIP is unmanaged; leaving it intact")?;
                if old.sha.len() != 40 || !old.sha.bytes().all(|b| b.is_ascii_hexdigit()) {
                    bail!("Invalid recorded source commit; leaving the existing ZIP intact");
                }
                if old.sha == sha
                    && files::hash(&destination)?.eq_ignore_ascii_case(&old.archive_sha256)
                {
                    job.log(&format!("{name}: source up to date"));
                    return Ok(());
                }
            }
            let archive = paths.at(format!("runtime/downloads/{name}-{sha}.zip"));
            network.download(
                &format!("https://codeload.github.com/storytold/{repository}/zip/{sha}"),
                &archive,
                job,
            )?;
            files::verify_source(&archive, name, sha)?;
            let hash = files::hash(&archive)?;
            let backup = old
                .as_ref()
                .filter(|_| destination.exists())
                .map(|o| backup_path(paths, name, &o.sha, true));
            index.insert(
                name.clone(),
                Source {
                    sha: sha.into(),
                    branch: branch.into(),
                    repository: format!("storytold/{repository}"),
                    archive_sha256: hash,
                    downloaded_at: chrono::Utc::now().to_rfc3339(),
                },
            );
            if let Err(e) = replace_transaction(&archive, &destination, backup.as_deref(), || {
                files::write_json(&paths.at("sources/source-index.json"), &index)
            }) {
                if let Some(old) = old {
                    index.insert(name.clone(), old);
                } else {
                    index.remove(name);
                }
                return Err(e);
            }
            if let Err(e) = backups::finish(paths, &prefs, backup.as_deref(), name, true, job) {
                job.log(&format!("Source backup warning: {e}"))
            }
            job.log(&format!("{name}: source updated ({})", &sha[..7]));
            Ok(())
        })();
        if let Err(e) = result {
            if crate::jobs::is_cancelled(&e) {
                if errors == 0 {
                    return Err(e);
                }
                job.log(&format!(
                    "{name}: cancelled; {errors} earlier source failure(s) retained"
                ));
                break;
            }
            errors += 1;
            job.log(&format!("{name}: {e:#}"));
            if e.to_string().contains("API limit")
                || job.cancel.load(std::sync::atomic::Ordering::Relaxed)
            {
                break;
            }
        }
    }
    if errors > 0 {
        bail!("{errors} source update(s) failed; see the log");
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_apps_select_matching_release_formats_and_reject_missing_architectures() {
        for app in [
            "wordcraft",
            "gridcraft",
            "deckcraft",
            "cadcraft",
            "soundcraft",
        ] {
            for format in ["portable", "installer"] {
                let preferences = Preferences {
                    architecture: "x64".into(),
                    release_format: format.into(),
                    ..Default::default()
                };
                let suffix = if cfg!(target_os = "macos") {
                    ".dmg"
                } else if format == "installer" {
                    crate::installers::installer_extension().unwrap()
                } else if cfg!(target_os = "windows") {
                    "-portable.zip"
                } else {
                    ".AppImage"
                };
                let name = format!(
                    "{app}-0.3.0-{}-{}{suffix}",
                    crate::model::release_os(),
                    crate::model::release_arch("x64")
                );
                let release = Release {
                    tag_name: "v0.3.0".into(),
                    name: None,
                    draft: false,
                    prerelease: false,
                    assets: vec![crate::model::Asset {
                        name: name.clone(),
                        size: 1,
                        digest: None,
                        browser_download_url: format!(
                            "https://github.com/storytold/{app}/releases/download/v0.3.0/{name}"
                        ),
                    }],
                };
                assert_eq!(
                    select_asset(&release, app, &preferences).unwrap().name,
                    name
                );
                if !cfg!(target_os = "macos") {
                    let other = Preferences {
                        architecture: "x86".into(),
                        ..preferences
                    };
                    assert!(select_asset(&release, app, &other).is_err());
                }
            }
        }
    }
    #[test]
    fn rollback() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let dest = root.join("app");
        let stage = root.join("new");
        let backup = root.join("backup");
        fs::write(&dest, "old").unwrap();
        fs::write(&stage, "new").unwrap();
        assert!(
            replace_transaction(&stage, &dest, Some(&backup), || bail!("metadata failure"))
                .is_err()
        );
        assert_eq!(fs::read_to_string(&dest).unwrap(), "old");
        assert_eq!(fs::read_to_string(&stage).unwrap(), "new");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn versions() {
        assert!(version("v0.10.0").unwrap() > version("0.9.0").unwrap());
        assert!(version("1.2.3-beta").is_err());
    }
}
#[cfg(test)]
mod planning_tests {
    use super::*;
    use std::sync::atomic::Ordering;
    fn fixture() -> (Paths, Job) {
        let root = std::env::temp_dir().join(format!("craft-plan-{}", uuid::Uuid::new_v4()));
        let paths = Paths::new(root, None);
        files::write_json(
            &paths.at("updater-settings.json"),
            &Preferences {
                release_format: "portable".into(),
                selected_apps: vec!["photocraft".into(), "filmcraft".into()],
                ..Default::default()
            },
        )
        .unwrap();
        let job = Job::new(paths.at("log"), &Default::default());
        (paths, job)
    }
    fn release(app: &str, tag: &str) -> Release {
        let suffix = if cfg!(target_os = "macos") {
            ".dmg"
        } else if cfg!(target_os = "linux") {
            ".AppImage"
        } else {
            "-portable.zip"
        };
        let name = format!(
            "{app}-{tag}-{}-{}{suffix}",
            crate::model::release_os(),
            crate::model::release_arch(crate::model::MANAGER_ARCH)
        );
        Release {
            tag_name: tag.into(),
            name: None,
            draft: false,
            prerelease: false,
            assets: vec![Asset {
                name: name.clone(),
                size: 1234,
                digest: Some(format!("sha256:{}", "a".repeat(64))),
                browser_download_url: format!(
                    "https://github.com/storytold/{app}/releases/download/v{tag}/{name}"
                ),
            }],
        }
    }
    #[test]
    fn invalid_digest_metadata_is_excluded_before_package_download() {
        let (paths, job) = fixture();
        for digest in [None, Some("sha256:invalid".into())] {
            let plan = plan_with(&paths, &job, None, |app| {
                let mut release = release(app, "0.4.0");
                release.assets[0].digest = digest.clone();
                Ok(release)
            })
            .unwrap();
            assert_eq!(plan.executable_count(), 0);
            assert!(plan.entries.iter().all(|entry| entry.error.is_some()));
            execute_validated(&paths, &plan, |_| {
                execute_with(&plan, &job, |_| {
                    panic!("invalid metadata must never download")
                })
            })
            .unwrap();
        }
        assert!(!paths.at("runtime/downloads").exists());
        fs::remove_dir_all(paths.root).unwrap();
    }
    #[test]
    fn legacy_mac_architecture_is_current_for_same_universal_release() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let (paths, job) = fixture();
        let folder = paths.at("releases/photocraft");
        let contents = folder
            .join(crate::model::executable_name("photocraft"))
            .join("Contents");
        fs::create_dir_all(&contents).unwrap();
        fs::write(contents.join("Info.plist"), "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>ai.storyteller.photocraft</string><key>CFBundleShortVersionString</key><string>0.4.0</string></dict></plist>").unwrap();
        let mut prefs = paths.read_preferences().unwrap();
        prefs.architecture = "arm64".into();
        prefs.selected_apps = vec!["photocraft".into()];
        files::write_json(&paths.at("manager-settings.json"), &prefs).unwrap();
        let record = crate::model::Installed {
            name: "photocraft".into(),
            version: "0.4.0".into(),
            path: folder.display().to_string(),
            install_kind: "portable".into(),
            architecture: "x86".into(),
            ..Default::default()
        };
        let config = crate::model::Config {
            apps_root: paths.root.display().to_string(),
            apps: vec![record.clone()],
            installations: vec![record],
        };
        files::write_json(&paths.at("settings.json"), &config).unwrap();
        let plan = plan_with(&paths, &job, None, |app| Ok(release(app, "0.4.0"))).unwrap();
        assert_eq!(plan.entries[0].action, "Skip");
        assert_eq!(plan.executable_count(), 0);
        execute_validated(&paths, &plan, |_| {
            execute_with(&plan, &job, |_| {
                panic!("must not reinstall universal release")
            })
        })
        .unwrap();
        let plist = contents.join("Info.plist");
        let changed = fs::read_to_string(&plist)
            .unwrap()
            .replace("0.4.0", "0.5.0");
        fs::write(&plist, changed).unwrap();
        assert!(execute_validated(&paths, &plan, |_| panic!(
            "actual installed version changed without inventory write"
        ))
        .is_err());
        fs::remove_dir_all(paths.root).unwrap();
    }
    #[test]
    fn planning_is_read_only_pins_assets_and_excludes_per_app_errors() {
        let (paths, job) = fixture();
        let legacy = fs::read(paths.at("updater-settings.json")).unwrap();
        let plan = plan_with(&paths, &job, None, |app| {
            if app == "filmcraft" {
                bail!("Metadata unavailable");
            } else {
                Ok(release(app, "0.4.0"))
            }
        })
        .unwrap();
        assert_eq!(plan.executable_count(), 1);
        assert!(plan
            .entries
            .iter()
            .any(|e| e.app == "filmcraft" && e.error.is_some()));
        execute_validated(&paths, &plan, |_| {
            execute_with(&plan, &job, |entry| {
                assert_eq!(entry.version, "0.4.0");
                assert_eq!(entry.asset.as_ref().unwrap().size, 1234);
                assert_eq!(
                    entry.asset.as_ref().unwrap().digest.as_deref(),
                    Some("sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                );
                Ok(true)
            })
        })
        .unwrap();
        assert_eq!(fs::read(paths.at("updater-settings.json")).unwrap(), legacy);
        for name in [
            "manager-settings.json",
            "settings.json",
            "releases",
            "runtime/downloads",
        ] {
            assert!(!paths.at(name).exists(), "{name}");
        }
        fs::remove_dir_all(paths.root).unwrap();
    }
    #[test]
    fn changed_preferences_destination_and_lock_reject_before_execution() {
        let (paths, job) = fixture();
        let plan = plan_with(&paths, &job, None, |app| Ok(release(app, "0.4.0"))).unwrap();
        let lock = platform::Lock::take("Local\\CraftAppsManager").unwrap();
        // Windows mutexes are recursive on their owning thread. A competing
        // worker must acquire on another thread, as real GUI operations do.
        let blocked = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    execute_validated(&paths, &plan, |_| panic!("must not execute while locked"))
                })
                .join()
                .unwrap()
        });
        assert!(blocked.is_err());
        drop(lock);
        fs::create_dir_all(paths.at("releases/photocraft")).unwrap();
        assert!(execute_validated(&paths, &plan, |_| panic!(
            "must not execute after destination change"
        ))
        .is_err());
        fs::remove_dir_all(paths.at("releases")).unwrap();
        let mut prefs = paths.read_preferences().unwrap();
        prefs.selected_apps.clear();
        files::write_json(&paths.at("manager-settings.json"), &prefs).unwrap();
        assert!(execute_validated(&paths, &plan, |_| panic!(
            "must not execute after selection change"
        ))
        .is_err());
        fs::remove_dir_all(paths.root).unwrap();
    }
    #[test]
    fn cancellation_counts_completed_work_and_keeps_real_errors() {
        let (paths, job) = fixture();
        let plan = plan_with(&paths, &job, None, |app| Ok(release(app, "0.4.0"))).unwrap();
        let result = execute_with(&plan, &job, |_| {
            job.cancel.store(true, Ordering::Relaxed);
            Ok(true)
        })
        .unwrap_err();
        assert!(crate::jobs::is_cancelled(&result));
        assert!(result.to_string().contains("1 completed"));
        assert!(result.to_string().contains("1 remaining"));
        job.cancel.store(false, Ordering::Relaxed);
        let error = execute_with(&plan, &job, |_| {
            job.cancel.store(true, Ordering::Relaxed);
            bail!("Actual installer failure");
        })
        .unwrap_err();
        assert!(!crate::jobs::is_cancelled(&error));
        assert!(error.to_string().contains("1 failed"));
        job.cancel.store(false, Ordering::Relaxed);
        let mut single = plan.clone();
        single.entries.truncate(1);
        assert!(execute_with(&single, &job, |_| {
            job.cancel.store(true, Ordering::Relaxed);
            Ok(true)
        })
        .is_ok());
        job.cancel.store(false, Ordering::Relaxed);
        execute_with(&single, &job, |_| Ok(false)).unwrap();
        assert!(job
            .state
            .lock()
            .unwrap()
            .log
            .contains("0 completed, 1 skipped"));
        fs::remove_dir_all(paths.root).unwrap();
    }
    #[test]
    fn cancelled_planning_never_fetches_packages_and_preserves_prior_metadata_failure() {
        let (paths, job) = fixture();
        job.cancel.store(true, Ordering::Relaxed);
        let error =
            plan_with(&paths, &job, None, |_| panic!("must not fetch metadata")).unwrap_err();
        assert!(crate::jobs::is_cancelled(&error));
        job.cancel.store(false, Ordering::Relaxed);
        let error = plan_with(&paths, &job, None, |_| {
            job.cancel.store(true, Ordering::Relaxed);
            bail!("Actual metadata error");
        })
        .unwrap_err();
        assert!(!crate::jobs::is_cancelled(&error));
        assert!(error.to_string().contains("Actual metadata error"));
        assert!(!paths.at("runtime/downloads").exists());
        fs::remove_dir_all(paths.root).unwrap();
    }
}
