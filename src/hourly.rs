use crate::{
    files,
    jobs::Job,
    model::{Config, Paths, Preferences},
    platform, updates,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Availability {
    pub installed: String,
    pub latest: Option<String>,
    pub notified: Option<String>,
}
pub type Checks = BTreeMap<String, Availability>;
pub fn key(prefs: &Preferences, app: &str) -> String {
    format!("{}:{}:{app}", prefs.release_format, prefs.architecture)
}
pub fn read(paths: &Paths) -> Result<Checks> {
    files::read_or_default(&paths.at("runtime/app-update-checks.json"))
}
pub fn run(paths: &Paths, job: &Job) -> Result<()> {
    let _lock = platform::Lock::take("Local\\CraftAppsManagerHourlyChecks")?;
    let before = paths.read_preferences()?;
    match crate::catalog::refresh(&paths.root) {
        Ok(entries) => {
            for entry in entries
                .iter()
                .filter(|e| !before.known_apps.contains(&e.key))
            {
                job.log(&format!("{}: new Craft app found", entry.key));
                if before.notify_updates {
                    let message = format!(
                        "{} is a new Craft app. Open Craft Apps Manager to install it.",
                        entry.title
                    );
                    if let Err(error) = platform::notify(&std::env::current_exe()?, &message) {
                        job.log(&format!("Notification warning: {error:#}"));
                    }
                }
            }
        }
        Err(error) => job.log(&format!("App list refresh failed: {error:#}")),
    }
    // Reading the settings again adopts and remembers newly found apps.
    let prefs = paths.preferences()?;
    let config = paths.config()?;
    let mut checks = read(paths)?;
    job.log("Hourly app availability check (no downloads or installation)");
    scan(
        &config,
        &prefs,
        &mut checks,
        job,
        |name| updates::check_app(paths, name),
        |message| platform::notify(&std::env::current_exe()?, message),
    )?;
    files::write_json(&paths.at("runtime/app-update-checks.json"), &checks)
}
pub fn sources(paths: &Paths, job: &Job) -> Result<()> {
    source_checks(paths, job, |message| {
        platform::notify(&std::env::current_exe()?, message)
    })
}
fn source_checks(
    paths: &Paths,
    job: &Job,
    mut notify: impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let _lock = platform::Lock::take("Local\\CraftAppsManagerHourlySources")?;
    let prefs = paths.preferences()?;
    let index: BTreeMap<String, crate::model::Source> =
        files::read_or_default(&paths.at("sources/source-index.json"))?;
    let network = crate::network::Network::new(&paths.root)?;
    let file = paths.at("runtime/source-update-checks.json");
    let mut checks: Checks = files::read_or_default(&file)?;
    job.log("Hourly source availability check (no downloads)");
    for name in &prefs.selected_sources {
        crate::model::valid_app(name)?;
        let Some(old) = index
            .get(name)
            .filter(|_| paths.at(format!("sources/{name}-source.zip")).is_file())
        else {
            continue;
        };
        job.check()?;
        let result = (|| -> Result<String> {
            let repo = crate::model::repository(name);
            let metadata: serde_json::Value =
                network.json(&format!("https://api.github.com/repos/storytold/{repo}"))?;
            let branch = metadata["default_branch"]
                .as_str()
                .context("No default branch")?;
            let mut url = reqwest::Url::parse(&format!(
                "https://api.github.com/repos/storytold/{repo}/commits/"
            ))?;
            url.path_segments_mut()
                .map_err(|_| anyhow::anyhow!("Invalid API endpoint"))?
                .pop_if_empty()
                .push(branch);
            let commit: serde_json::Value = network.json(url.as_str())?;
            let sha = commit["sha"].as_str().context("No commit")?;
            anyhow::ensure!(
                sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()),
                "Invalid source commit"
            );
            Ok(sha.into())
        })();
        match result {
            Ok(sha) => {
                let state = checks.entry(name.clone()).or_default();
                state.installed = old.sha.clone();
                state.latest = (sha != old.sha).then_some(sha.clone());
                if state.latest.is_some()
                    && prefs.notify_updates
                    && state.notified.as_deref() != Some(&sha)
                {
                    let message = format!(
                        "{} has newer source files ({}). Open Craft Apps Manager to download them.",
                        crate::model::title(name),
                        &sha[..7]
                    );
                    match notify(&message) {
                        Ok(()) => state.notified = Some(sha),
                        Err(error) => job.log(&format!("Notification warning: {error:#}")),
                    }
                }
                job.log(&format!(
                    "{name}: {}",
                    if state.latest.is_some() {
                        "source update available"
                    } else {
                        "source up to date"
                    }
                ));
            }
            Err(error) => job.log(&format!("{name}: source check failed: {error:#}")),
        }
    }
    files::write_json(&file, &checks)
}
fn scan(
    config: &Config,
    prefs: &Preferences,
    checks: &mut Checks,
    job: &Job,
    mut check: impl FnMut(&str) -> Result<Option<String>>,
    mut notify: impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    for app in updates::installed_check_targets(config) {
        if !prefs.selected_apps.contains(&app.name) {
            continue;
        }
        job.check()?;
        match check(&app.name) {
            Ok(latest) => {
                let state = checks.entry(key(prefs, &app.name)).or_default();
                state.installed = app.version;
                state.latest = latest.clone();
                if let Some(version) = latest {
                    job.log(&format!("{}: update available ({version})", app.name));
                    if prefs.notify_updates && state.notified.as_deref() != Some(&version) {
                        let message = format!(
                            "{} {version} is available. Open Craft Apps Manager to install it.",
                            crate::model::title(&app.name)
                        );
                        match notify(&message) {
                            Ok(()) => state.notified = Some(version),
                            Err(error) => job.log(&format!("Notification warning: {error:#}")),
                        }
                    }
                } else {
                    job.log(&format!("{}: up to date", app.name));
                }
            }
            Err(error) => job.log(&format!("{}: check failed: {error:#}", app.name)),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_checks_notify_once_per_commit_without_changing_archives_or_index() {
        use sha2::{Digest, Sha256};
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let paths = Paths::new(root.clone(), None);
        let job = Job::new(root.join("checks.log"), &Default::default());
        files::write_json(
            &paths.at("manager-settings.json"),
            &Preferences {
                selected_sources: vec!["filmcraft".into(), "artcraftx".into()],
                ..Default::default()
            },
        )
        .unwrap();
        let old_sha = "a".repeat(40);
        let index = BTreeMap::from([(
            "filmcraft",
            crate::model::Source {
                sha: old_sha,
                branch: "main".into(),
                repository: "storytold/filmcraft".into(),
                archive_sha256: String::new(),
                downloaded_at: String::new(),
            },
        )]);
        files::write_json(&paths.at("sources/source-index.json"), &index).unwrap();
        std::fs::write(
            paths.at("sources/filmcraft-source.zip"),
            b"original source bytes",
        )
        .unwrap();
        let index_before = std::fs::read(paths.at("sources/source-index.json")).unwrap();
        let cache = |url: &str, value: serde_json::Value| {
            files::write_json(
                &paths.at(format!(
                    "runtime/api-cache/{:x}.json",
                    Sha256::digest(url.as_bytes())
                )),
                &serde_json::json!({"at":chrono::Utc::now().timestamp(),"etag":null,"value":value}),
            )
            .unwrap();
        };
        cache(
            "https://api.github.com/repos/storytold/filmcraft",
            serde_json::json!({"default_branch":"main"}),
        );
        let mut notifications = 0;
        for sha in ["b".repeat(40), "b".repeat(40), "c".repeat(40)] {
            cache(
                "https://api.github.com/repos/storytold/filmcraft/commits/main",
                serde_json::json!({"sha":sha}),
            );
            source_checks(&paths, &job, |_| {
                notifications += 1;
                Ok(())
            })
            .unwrap();
        }
        assert_eq!(notifications, 2);
        assert_eq!(
            std::fs::read(paths.at("sources/source-index.json")).unwrap(),
            index_before
        );
        assert_eq!(
            std::fs::read(paths.at("sources/filmcraft-source.zip")).unwrap(),
            b"original source bytes"
        );
        assert!(!paths.at("sources/artcraftx-source.zip").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn checks_selected_installed_apps_and_notifies_once_per_version_in_both_formats() {
        for format in ["portable", "installer"] {
            let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
            let job = Job::new(root.join("checks.log"), &Default::default());
            let prefs = Preferences {
                release_format: format.into(),
                selected_apps: vec!["filmcraft".into()],
                ..Default::default()
            };
            let config = Config {
                apps: vec![
                    crate::model::Installed {
                        name: "filmcraft".into(),
                        path: "installed".into(),
                        version: "0.1.0".into(),
                        ..Default::default()
                    },
                    crate::model::Installed {
                        name: "photocraft".into(),
                        path: "installed".into(),
                        version: "0.1.0".into(),
                        ..Default::default()
                    },
                    crate::model::Installed {
                        name: "designcraft".into(),
                        ..Default::default()
                    },
                ],
                apps_root: String::new(),
                installations: vec![],
            };
            let mut checks = Checks::new();
            let mut notifications = 0;
            for latest in ["0.2.0", "0.2.0", "0.3.0"] {
                scan(
                    &config,
                    &prefs,
                    &mut checks,
                    &job,
                    |app| {
                        assert_eq!(app, "filmcraft");
                        Ok(Some(latest.into()))
                    },
                    |_| {
                        notifications += 1;
                        Ok(())
                    },
                )
                .unwrap();
            }
            assert_eq!(notifications, 2);
            assert_eq!(
                checks[&key(&prefs, "filmcraft")].latest.as_deref(),
                Some("0.3.0")
            );
            scan(
                &config,
                &prefs,
                &mut checks,
                &job,
                |_| Ok(None),
                |_| panic!("No notification when current"),
            )
            .unwrap();
            assert!(checks[&key(&prefs, "filmcraft")].latest.is_none());
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}
