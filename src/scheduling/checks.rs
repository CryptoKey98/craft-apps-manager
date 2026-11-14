use crate::{
    files,
    jobs::Job,
    model::{Paths, Preferences},
    platform, updates,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Availability {
    pub installed: String,
    #[serde(default)]
    pub path: String,
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
/// Persist availability for the new installed version using already-cached release metadata.
pub fn refresh_after_install(
    paths: &Paths,
    installed: &crate::model::Installed,
    prefs: &Preferences,
) -> Result<()> {
    let Some(result) = updates::cached_installation_check(paths, installed, prefs) else {
        return Ok(());
    };
    let latest = result?;
    let _lock = platform::Lock::take("Local\\CraftAppsManagerHourlyChecks")?;
    let effective = crate::install_options::for_installed(prefs, installed)?;
    let mut checks = read(paths)?;
    let state = checks.entry(key(&effective, &installed.name)).or_default();
    if state.installed != installed.version {
        state.notified = None;
    }
    state.installed = installed.version.clone();
    state.path.clone_from(&installed.path);
    state.latest = latest;
    files::write_json(&paths.at("runtime/app-update-checks.json"), &checks)
}
#[derive(Default, Serialize, Deserialize)]
struct LastAutomaticCheck {
    at: i64,
    format: String,
    architecture: String,
    #[serde(default)]
    options: BTreeMap<String, crate::install_options::InstallOptions>,
    #[serde(default)]
    targets: Vec<crate::model::Installed>,
}
fn recently_checked(last: &LastAutomaticCheck, prefs: &Preferences, now: i64) -> bool {
    let elapsed = now - last.at;
    last.options == prefs.app_install_options
        && elapsed >= 0
        && elapsed < (prefs.app_check_interval_minutes.clamp(10, 60) * 60) as i64
}
pub fn run(paths: &Paths, job: &Job) -> Result<()> {
    job.tracked("Background update check", "Application library", |job| {
        let prefs = paths.read_preferences()?;
        discover(paths, &prefs, job, false);
        let prefs = paths.preferences()?;
        let apps = updates::automatic_check_targets(&paths.config()?, &prefs);
        run_batch(paths, job, &prefs, &apps, false, |_, _| {})
    })
}

pub fn run_window_check(
    paths: &Paths,
    job: &Job,
    prefs: &Preferences,
    apps: &[crate::model::Installed],
    periodic: bool,
    receive: impl FnMut(crate::model::Installed, Result<Option<String>>),
) -> Result<()> {
    job.tracked("Background update check", "Application library", |job| {
        run_batch(paths, job, prefs, apps, periodic, receive)?;
        discover(paths, prefs, job, periodic);
        Ok(())
    })
}

fn run_batch(
    paths: &Paths,
    job: &Job,
    prefs: &Preferences,
    apps: &[crate::model::Installed],
    periodic: bool,
    mut receive: impl FnMut(crate::model::Installed, Result<Option<String>>),
) -> Result<()> {
    let _lock = platform::Lock::take(r"Local\CraftAppsManagerHourlyChecks")?;
    let last_path = paths.at("runtime/last-automatic-app-check.json");
    if periodic {
        let last: LastAutomaticCheck = files::read_or_default(&last_path)?;
        if recently_checked(&last, prefs, chrono::Utc::now().timestamp()) && last.targets == *apps {
            job.log("A background check ran recently; using its saved results.");
            let checks = read(paths)?;
            for app in apps {
                if let Ok(effective) = crate::install_options::for_installed(prefs, app) {
                    if let Some(check) = checks.get(&key(&effective, &app.name)) {
                        if check.installed == app.version {
                            receive(app.clone(), Ok(check.latest.clone()));
                        }
                    }
                }
            }
            return Ok(());
        }
    }
    let mut checks = read(paths)?;
    job.check()?;
    job.log("App availability check (no downloads or installation)");
    let mut all_succeeded = true;
    updates::check_installation_batch(paths, apps, prefs, |app, result| {
        if job.check().is_ok() {
            all_succeeded &= result.is_ok();
            record_check(&app, prefs, &mut checks, job, &result, |message| {
                platform::notify(&std::env::current_exe()?, message)
            });
            receive(app, result);
        }
    });
    job.check()?;
    files::write_json(&paths.at("runtime/app-update-checks.json"), &checks)?;
    if !all_succeeded {
        return Ok(());
    }
    files::write_json(
        &last_path,
        &LastAutomaticCheck {
            at: chrono::Utc::now().timestamp(),
            format: prefs.release_format.clone(),
            architecture: prefs.architecture.clone(),
            options: prefs.app_install_options.clone(),
            targets: apps.to_vec(),
        },
    )
}

fn discover(paths: &Paths, prefs: &Preferences, job: &Job, periodic: bool) {
    if prefs.check_catalog_with_app_updates {
        match crate::catalog::refresh_for(
            &paths.root,
            if periodic {
                crate::catalog::DiscoveryTrigger::Periodic
            } else {
                crate::catalog::DiscoveryTrigger::Startup
            },
        ) {
            Ok(entries) => {
                for entry in entries
                    .iter()
                    .filter(|e| !prefs.known_apps.contains(&e.key))
                {
                    job.log(&format!("{}: new Craft app found", entry.key));
                    if prefs.notify_updates {
                        let message = format!(
                            "{} is a new Craft app. Open Craft Apps Manager to install it.",
                            entry.title
                        );
                        if let Err(error) = std::env::current_exe()
                            .map_err(anyhow::Error::from)
                            .and_then(|exe| platform::notify(&exe, &message))
                        {
                            job.log(&format!("Notification warning: {error:#}"));
                        }
                    }
                }
            }
            Err(error) => job.log(&format!("App list refresh failed: {error:#}")),
        }
    }
}
pub fn sources(_paths: &Paths, job: &Job) -> Result<()> {
    job.log(
        "Automatic source checks are disabled. Download sources from the app tools when needed.",
    );
    Ok(())
}
fn record_check(
    app: &crate::model::Installed,
    prefs: &Preferences,
    checks: &mut Checks,
    job: &Job,
    result: &Result<Option<String>>,
    notify: impl FnOnce(&str) -> Result<()>,
) {
    match result {
        Ok(latest) => {
            let Ok(effective) = crate::install_options::for_installed(prefs, app) else {
                return;
            };
            let state = checks.entry(key(&effective, &app.name)).or_default();
            state.installed = app.version.clone();
            state.path.clone_from(&app.path);
            state.latest.clone_from(latest);
            if let Some(version) = latest {
                job.log(&format!("{}: update available ({version})", app.name));
                if prefs.notify_updates && state.notified.as_ref() != Some(version) {
                    let message = format!(
                        "{} {version} is available. Open Craft Apps Manager to install it.",
                        crate::model::title(&app.name)
                    );
                    match notify(&message) {
                        Ok(()) => state.notified = Some(version.clone()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Config;

    #[test]
    fn automatic_checks_include_apps_excluded_from_update_all() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let prefs = Preferences {
            app_order: vec![
                "designcraft".into(),
                "photocraft".into(),
                "filmcraft".into(),
            ],
            home_app_order: vec!["filmcraft".into(), "designcraft".into()],
            selected_apps: vec!["filmcraft".into(), "designcraft".into()],
            notify_updates: false,
            ..Default::default()
        };
        let config = Config {
            apps: ["filmcraft", "photocraft", "designcraft"]
                .into_iter()
                .map(|name| crate::model::Installed {
                    name: name.into(),
                    version: "1.0.0".into(),
                    path: format!("installed/{name}"),
                    install_kind: "portable".into(),
                    architecture: crate::model::MANAGER_ARCH.into(),
                    ..Default::default()
                })
                .collect(),
            apps_root: root.display().to_string(),
            installations: Vec::new(),
            portable_label_counters: Default::default(),
        };
        let visited: Vec<_> = updates::automatic_check_targets(&config, &prefs)
            .into_iter()
            .map(|app| app.name)
            .collect();
        assert_eq!(visited, ["designcraft", "photocraft", "filmcraft"]);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn periodic_interval_respects_recent_background_checks_and_package_changes() {
        let mut prefs = Preferences::default();
        let last = LastAutomaticCheck {
            at: 1000,
            format: prefs.release_format.clone(),
            architecture: prefs.architecture.clone(),
            options: prefs.app_install_options.clone(),
            targets: Vec::new(),
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
        assert!(recently_checked(&last, &prefs, 1001));
        prefs.app_install_options.insert(
            crate::install_options::identity("photocraft"),
            crate::install_options::InstallOptions {
                release_format: "portable".into(),
                architecture: "x64".into(),
            },
        );
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
    fn notifies_once_per_version_in_both_formats() {
        for format in ["portable", "installer"] {
            let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
            let job = Job::new(root.join("checks.log"), &Default::default());
            let prefs = Preferences {
                release_format: format.into(),
                selected_apps: vec!["filmcraft".into()],
                ..Default::default()
            };
            let config = Config {
                portable_label_counters: Default::default(),
                apps: vec![
                    crate::model::Installed {
                        name: "filmcraft".into(),
                        path: "installed".into(),
                        version: "0.1.0".into(),
                        install_kind: format.into(),
                        architecture: crate::model::MANAGER_ARCH.into(),
                        ..Default::default()
                    },
                    crate::model::Installed {
                        name: "photocraft".into(),
                        path: "installed".into(),
                        version: "0.1.0".into(),
                        install_kind: format.into(),
                        architecture: crate::model::MANAGER_ARCH.into(),
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
            let app = &config.apps[0];
            for latest in ["0.2.0", "0.2.0", "0.3.0"] {
                record_check(
                    app,
                    &prefs,
                    &mut checks,
                    &job,
                    &Ok(Some(latest.into())),
                    |_| {
                        notifications += 1;
                        Ok(())
                    },
                );
            }
            assert_eq!(notifications, 2);
            assert_eq!(
                checks[&key(&prefs, "filmcraft")].latest.as_deref(),
                Some("0.3.0")
            );
            record_check(app, &prefs, &mut checks, &job, &Ok(None), |_| {
                panic!("No notification when current")
            });
            assert!(checks[&key(&prefs, "filmcraft")].latest.is_none());
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}
