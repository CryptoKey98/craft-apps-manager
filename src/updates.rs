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
pub fn version(v: &str) -> Result<(u64, u64, u64)> {
    let nums: Vec<_> = v
        .trim_start_matches('v')
        .split('.')
        .map(str::parse::<u64>)
        .collect::<std::result::Result<_, _>>()?;
    if nums.len() != 3 {
        bail!("Unsupported version: {v}");
    }
    Ok((nums[0], nums[1], nums[2]))
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
        Ok(Some(release.tag_name.trim_start_matches('v').into()))
    } else {
        Ok(None)
    }
}
pub fn installed_check_targets(config: &crate::model::Config) -> Vec<crate::model::Installed> {
    config
        .apps
        .iter()
        .filter(|app| {
            crate::model::APPS.contains(&app.name.as_str())
                && !app.path.is_empty()
                && !app.version.is_empty()
        })
        .cloned()
        .collect()
}
pub fn select_asset<'a>(r: &'a Release, name: &str, p: &Preferences) -> Result<&'a Asset> {
    version(&r.tag_name)?;
    let mut names = Vec::new();
    for asset_name in [crate::model::repository(name), name] {
        let prefix = format!(
            "{asset_name}-{}-{}-{}",
            r.tag_name.trim_start_matches('v'),
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
    }
    names.dedup();
    for n in names {
        let assets: Vec<_> = r.assets.iter().filter(|a| a.name == n).collect();
        if assets.len() > 1 {
            bail!("Ambiguous release assets");
        }
        if let Some(a) = assets.first() {
            return Ok(a);
        }
    }
    bail!(
        "This release has no {} {}. Choose another option in Settings.",
        p.architecture,
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
    if !crate::model::APPS.contains(&app) {
        bail!("This app has no managed release");
    }
    releases_for(paths, job, Some(app))
}
fn releases_for(paths: &Paths, job: &Job, selected: Option<&str>) -> Result<()> {
    let _lock = platform::Lock::take("Local\\CraftAppsManager")?;
    job.log(&format!(
        "\nRelease updates — {}",
        chrono::Utc::now().to_rfc3339()
    ));
    let mut config = paths.config()?;
    let prefs = paths.preferences()?;
    let network = Network::new(&paths.root)?;
    let mut errors = 0;
    for i in 0..config.apps.len() {
        let app = config.apps[i].clone();
        if !selected
            .map(|name| name == app.name)
            .unwrap_or_else(|| prefs.selected_apps.contains(&app.name))
        {
            continue;
        }
        crate::model::valid_app(&app.name)?;
        job.check()?;
        job.stage("Checking releases", None, crate::model::title(&app.name));
        let result = (|| -> Result<()> {
            let release: Release = network.json(&format!(
                "https://api.github.com/repos/storytold/{}/releases/latest",
                crate::model::repository(&app.name)
            ))?;
            if release.draft || release.prerelease {
                bail!("Not a stable release")
            }
            let asset = select_asset(&release, &app.name, &prefs)?;
            if !asset.browser_download_url.starts_with(&format!(
                "https://github.com/storytold/{}/releases/download/",
                crate::model::repository(&app.name)
            )) {
                bail!("Unexpected asset URL");
            }
            files::safe_relative(&asset.name)?;
            if prefs.release_format == "installer" {
                let latest = release.tag_name.trim_start_matches('v');
                if app.install_kind == "installer"
                    && !app.version.is_empty()
                    && version(&app.version)? >= version(latest)?
                    && crate::model::installed_executable(Path::new(&app.path), &app.name).is_some()
                {
                    job.log(&format!("{}: installed version is current", app.name));
                    return Ok(());
                }
                if platform::running_app(&app.name)? {
                    bail!("Close {} before installing", app.name);
                }
                let dest = paths.at(format!("releases/installers/{}/{}", app.name, asset.name));
                if dest.exists() {
                    crate::network::verify_asset(&dest, asset)?;
                } else {
                    network.asset(asset, &dest, job)?;
                }
                job.stage("Installing", None, "Complete the Windows installer wizard; Windows may ask for administrator permission.");
                let mut installed = crate::installers::run_with_job(&dest, &app.name, Some(job))?;
                installed.architecture = prefs.architecture.clone();
                config.apps[i] = installed;
                paths.save_config(&config)?;
                job.log(&format!("{}: installer completed", app.name));
                return Ok(());
            }
            let v = release.tag_name.trim_start_matches('v');
            let target = if app.path.is_empty() || app.install_kind == "installer" {
                paths.at(format!("releases/{}", app.name))
            } else {
                PathBuf::from(&app.path)
            };
            files::inside(&target, &paths.at("releases"))?;
            if !app.version.is_empty()
                && app.install_kind != "installer"
                && version(v)? <= version(&app.version)?
                && app.architecture == prefs.architecture
                && crate::model::installed_executable(&target, &app.name).is_some()
            {
                job.log(&format!("{}: up to date ({})", app.name, app.version));
                return Ok(());
            }
            if platform::running_app(&app.name)? {
                job.log(&format!("{}: app is open; skipped", app.name));
                return Ok(());
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
                config.apps[i].architecture = prefs.architecture.clone();
                config.apps[i].install_kind = "portable".into();
                config.apps[i].product_code.clear();
                if let Err(e) = replace_transaction(
                    executables[0].path().parent().unwrap(),
                    &target,
                    backup.as_deref(),
                    || paths.save_config(&config),
                ) {
                    config.apps[i] = old;
                    return Err(e);
                }
                if let Err(e) =
                    backups::finish(paths, &prefs, backup.as_deref(), &app.name, false, job)
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
            attempt
        })();
        if let Err(e) = result {
            errors += 1;
            job.log(&format!("{}: {e:#}", app.name));
            if e.to_string().contains("API limit")
                || job.cancel.load(std::sync::atomic::Ordering::Relaxed)
            {
                break;
            }
        }
    }
    if errors > 0 {
        bail!("{errors} release update(s) failed; see logs/updates.log");
    }
    Ok(())
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
        job.check()?;
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
