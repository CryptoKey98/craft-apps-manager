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
#[derive(Default, Serialize, Deserialize)]
struct LastAutomaticCheck {
    at: i64,
    format: String,
    architecture: String,
}
fn recently_checked(last: &LastAutomaticCheck, prefs: &Preferences, now: i64) -> bool {
    let elapsed = now - last.at;
    last.format == prefs.release_format
        && last.architecture == prefs.architecture
        && elapsed >= 0
        && elapsed < (prefs.app_check_interval_minutes.clamp(10, 60) * 60) as i64
}
pub fn run(paths: &Paths, job: &Job) -> Result<()> {
    run_automatic(paths, job, false)
}
pub fn run_periodic(paths: &Paths, job: &Job) -> Result<()> {
    run_automatic(paths, job, true)
}
fn run_automatic(paths: &Paths, job: &Job, periodic: bool) -> Result<()> {
    let _lock = platform::Lock::take("Local\\CraftAppsManagerHourlyChecks")?;
    let before = paths.read_preferences()?;
    let last_path = paths.at("runtime/last-automatic-app-check.json");
    if periodic {
        let last: LastAutomaticCheck = files::read_or_default(&last_path)?;
        if recently_checked(&last, &before, chrono::Utc::now().timestamp()) {
            job.log("A background check ran recently; using its saved results.");
            return Ok(());
        }
    }
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
    files::write_json(&paths.at("runtime/app-update-checks.json"), &checks)?;
    files::write_json(
        &last_path,
        &LastAutomaticCheck {
            at: chrono::Utc::now().timestamp(),
            format: prefs.release_format,
            architecture: prefs.architecture,
        },
    )
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
    fn periodic_interval_respects_recent_background_checks_and_package_changes() {
        let mut prefs = Preferences::default();
        let last = LastAutomaticCheck {
            at: 1000,
            format: prefs.release_format.clone(),
            architecture: prefs.architecture.clone(),
        };
        assert!(recently_checked(&last, &prefs, 2199));
        assert!(!recently_checked(&last, &prefs, 2200));
        assert!(!recently_checked(&last, &prefs, 999));
        prefs.app_check_interval_minutes = 10;
        assert!(!recently_checked(&last, &prefs, 1600));
        prefs.release_format = if last.format == "installer" {
            "portable"
        } else {
            "installer"
        }
        .into();
        assert!(!recently_checked(&last, &prefs, 1001));
    }
    #[test]
    fn interval_preferences_migrate_and_clamp_without_changing_other_options() {
        let mut prefs: Preferences =
            serde_json::from_str(r#"{"checkInstalledAppsPeriodically":false}"#).unwrap();
        assert_eq!(prefs.app_check_interval_minutes, 20);
        assert!(!prefs.check_installed_apps_periodically);
        for (input, expected) in [(0, 10), (35, 35), (100, 60)] {
            prefs.app_check_interval_minutes = input;
            prefs.validate().unwrap();
            assert_eq!(prefs.app_check_interval_minutes, expected);
            assert!(!prefs.check_installed_apps_periodically);
        }
    }
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
