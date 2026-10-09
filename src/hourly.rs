use crate::{
    files,
    jobs::Job,
    model::{Config, Paths, Preferences},
    platform, updates,
};
use anyhow::Result;
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
    if before.check_catalog_with_app_updates {
        match crate::catalog::refresh_if_older(&paths.root, crate::catalog::REFRESH_INTERVAL) {
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
    }
    // Reading the settings again adopts and remembers newly found apps.
    let prefs = paths.preferences()?;
    let config = paths.config()?;
    let mut checks = read(paths)?;
    job.log("App availability check (no downloads or installation)");
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
pub fn sources(_paths: &Paths, job: &Job) -> Result<()> {
    job.log(
        "Automatic source checks are disabled. Download sources from the app tools when needed.",
    );
    Ok(())
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
    fn legacy_source_check_does_not_create_files_or_fetch_updates() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let paths = Paths::new(root.clone(), None);
        let job = Job::new(root.join("checks.log"), &Default::default());
        sources(&paths, &job).unwrap();
        assert!(!paths.at("runtime/source-update-checks.json").exists());
        assert!(!paths.at("sources").exists());
        let prefs = Preferences::default();
        assert!(prefs.check_catalog_on_startup);
        assert!(prefs.check_installed_apps_periodically);
        assert!(prefs.notify_updates);
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
