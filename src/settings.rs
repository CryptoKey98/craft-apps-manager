//! Bounded, rollback-protected settings saves. This is not a crash-atomic transaction.
use crate::{
    files,
    model::{BuilderPreferences, Paths, Preferences},
    platform,
};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub fn save<T: Serialize>(
    paths: &Paths,
    locations_path: &Path,
    locations: &T,
    root: &Path,
    tools: &Path,
    preferences: Option<&Preferences>,
    builder: Option<&BuilderPreferences>,
) -> Result<()> {
    // Validate every draft before inventory reads, serialization, or filesystem changes.
    if !root.is_absolute() || !tools.is_absolute() {
        bail!("Folder paths must be absolute");
    }
    for path in [root, tools] {
        if path.exists() && !path.is_dir() {
            bail!("Folder path is not a directory: {}", path.display());
        }
    }
    let mut preferences = preferences.cloned();
    if let Some(prefs) = &mut preferences {
        prefs.validate()?;
    }
    if let Some(prefs) = builder {
        if !(1..=100).contains(&prefs.log_size_mb) || prefs.log_archives > 5 {
            bail!("Unsupported log size or archive count");
        }
    }
    let _lock = platform::Lock::take("Local\\CraftAppsManager")?;
    let mut writes = Vec::new();
    if let Some(prefs) = preferences {
        let mut config = paths.config()?;
        let installer = paths.read_preferences()?.release_format == "installer";
        for app in &config.apps {
            config
                .installations
                .retain(|a| !(a.name == app.name && (a.install_kind == "installer") == installer));
            if !app.path.is_empty() {
                config.installations.push(app.clone());
            }
        }
        writes.push((
            paths.at("settings.json"),
            serde_json::to_vec_pretty(&config)?,
        ));
        writes.push((
            paths.at("manager-settings.json"),
            serde_json::to_vec_pretty(&prefs)?,
        ));
    }
    if let Some(prefs) = builder {
        writes.push((
            paths.at("builder-settings.json"),
            serde_json::to_vec_pretty(prefs)?,
        ));
    }
    writes.push((
        locations_path.to_owned(),
        serde_json::to_vec_pretty(locations)?,
    ));
    commit(&writes, atomic_bytes)
}
/// Reordering changes only the order in the latest preferences, under the mutation lock.
/// Save only the bulk release selection, keeping other preferences and inventory intact.
pub fn select_release_apps(paths: &Paths, selected: &[String]) -> Result<Preferences> {
    let _lock = platform::Lock::take("Local\\CraftAppsManager")?;
    let choices = crate::model::apps();
    if selected.iter().any(|app| !choices.contains(app)) {
        bail!("Unknown release app in selection");
    }
    let mut preferences = paths.read_preferences()?;
    preferences.selected_apps = selected.to_vec();
    preferences.validate()?;
    commit(
        &[(
            paths.at("manager-settings.json"),
            serde_json::to_vec_pretty(&preferences)?,
        )],
        atomic_bytes,
    )?;
    Ok(preferences)
}
pub fn reorder(paths: &Paths, dragged: &str, target: &str, before: bool) -> Result<Preferences> {
    let _lock = platform::Lock::take("Local\\CraftAppsManager")?;
    let mut preferences = paths.read_preferences()?;
    crate::model::valid_app(dragged)?;
    crate::model::valid_app(target)?;
    preferences.app_order.retain(|name| name != dragged);
    let index = preferences
        .app_order
        .iter()
        .position(|name| name == target)
        .context("Reorder target missing")?;
    preferences
        .app_order
        .insert(index + usize::from(!before), dragged.into());
    preferences.validate()?;
    commit(
        &[(
            paths.at("manager-settings.json"),
            serde_json::to_vec_pretty(&preferences)?,
        )],
        atomic_bytes,
    )?;
    Ok(preferences)
}
pub fn reorder_home(
    paths: &Paths,
    dragged: &str,
    target: &str,
    before: bool,
) -> Result<Preferences> {
    let _lock = platform::Lock::take("Local\\CraftAppsManager")?;
    let mut preferences = paths.read_preferences()?;
    crate::model::valid_app(dragged)?;
    crate::model::valid_app(target)?;
    preferences.home_app_order.retain(|name| name != dragged);
    let index = preferences
        .home_app_order
        .iter()
        .position(|name| name == target)
        .context("Reorder target missing")?;
    preferences
        .home_app_order
        .insert(index + usize::from(!before), dragged.into());
    preferences.validate()?;
    commit(
        &[(
            paths.at("manager-settings.json"),
            serde_json::to_vec_pretty(&preferences)?,
        )],
        atomic_bytes,
    )?;
    Ok(preferences)
}
fn atomic_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::create_dir_all(path.parent().context("Missing settings parent")?)?;
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        platform::atomic_replace(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
fn commit(
    writes: &[(PathBuf, Vec<u8>)],
    mut write: impl FnMut(&Path, &[u8]) -> Result<()>,
) -> Result<()> {
    let old: Vec<_> = writes
        .iter()
        .map(|(path, _)| {
            if fs::symlink_metadata(path).is_ok() && files::linked(path)? {
                bail!("Linked settings file: {}", path.display());
            }
            Ok(match fs::read(path) {
                Ok(bytes) => Some(bytes),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.into()),
            })
        })
        .collect::<Result<_>>()?;
    for (index, (path, bytes)) in writes.iter().enumerate() {
        if let Err(error) = write(path, bytes) {
            // Include the failing path: atomic_replace may fail after replacing it.
            let mut recovery = Vec::new();
            for ((path, _), original) in writes[..=index].iter().zip(&old).rev() {
                let restored = match original {
                    Some(bytes) => atomic_bytes(path, bytes),
                    None => match fs::remove_file(path) {
                        Ok(()) => Ok(()),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                        Err(e) => Err(e.into()),
                    },
                };
                if let Err(rollback) = restored {
                    let mut backup =
                        path.with_extension(format!("{}.recovery", uuid::Uuid::new_v4()));
                    let bytes = original.as_deref().unwrap_or(
                        b"Original file did not exist; remove the settings file to restore.",
                    );
                    let mut evidence = fs::write(&backup, bytes);
                    if evidence.is_err() {
                        backup = std::env::temp_dir()
                            .join(format!("craft-settings-{}.recovery", uuid::Uuid::new_v4()));
                        evidence = fs::write(&backup, bytes);
                    }
                    recovery.push(format!(
                        "{}: {rollback:#}; recovery {} ({evidence:?})",
                        path.display(),
                        backup.display()
                    ));
                }
            }
            if !recovery.is_empty() {
                return Err(error.context(format!(
                    "Settings rollback incomplete: {}",
                    recovery.join("; ")
                )));
            }
            return Err(error.context("Settings save failed; previous files restored"));
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn review_selection_preserves_other_preferences_and_inventory() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let paths = Paths::new(root.clone(), None);
        let preferences = Preferences {
            theme: crate::model::Theme::Light,
            selected_apps: vec!["filmcraft".into(), "photocraft".into()],
            selected_sources: vec!["soundcraft".into()],
            ..Default::default()
        };
        files::write_json(&paths.at("manager-settings.json"), &preferences).unwrap();
        fs::write(
            paths.at("settings.json"),
            b"inventory must remain untouched",
        )
        .unwrap();
        let saved = select_release_apps(&paths, &["filmcraft".into()]).unwrap();
        assert_eq!(saved.selected_apps, ["filmcraft"]);
        assert_eq!(saved.selected_sources, ["soundcraft"]);
        assert_eq!(saved.theme, crate::model::Theme::Light);
        assert_eq!(
            fs::read(paths.at("settings.json")).unwrap(),
            b"inventory must remain untouched"
        );
        let bytes = fs::read(paths.at("manager-settings.json")).unwrap();
        assert!(select_release_apps(&paths, &["unknown".into()]).is_err());
        assert_eq!(fs::read(paths.at("manager-settings.json")).unwrap(), bytes);
        assert!(select_release_apps(&paths, &[])
            .unwrap()
            .selected_apps
            .is_empty());
        fs::remove_dir_all(root).unwrap();
    }
    use super::*;
    #[test]
    fn home_reorder_is_independent_and_persists() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let paths = Paths::new(root.clone(), None);
        let current = Preferences::default();
        files::write_json(&paths.at("manager-settings.json"), &current).unwrap();
        let home = reorder_home(&paths, "photocraft", "designcraft", true).unwrap();
        assert_eq!(home.app_order, current.app_order);
        assert_eq!(home.home_app_order[0], "photocraft");
        let sidebar = reorder(&paths, "filmcraft", "designcraft", true).unwrap();
        assert_eq!(sidebar.home_app_order, home.home_app_order);
        assert_eq!(
            paths.read_preferences().unwrap().home_app_order,
            home.home_app_order
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn reorder_preserves_other_window_preferences_and_obeys_operation_lock() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let paths = Paths::new(root.clone(), None);
        let current = Preferences {
            release_format: "portable".into(),
            architecture: "x86".into(),
            selected_apps: vec!["photocraft".into()],
            ..Default::default()
        };
        files::write_json(&paths.at("manager-settings.json"), &current).unwrap();
        let bytes = fs::read(paths.at("manager-settings.json")).unwrap();
        let lock = platform::Lock::take("Local\\CraftAppsManager").unwrap();
        // Hold ownership here while a separate worker attempts the save.
        // A Windows mutex allows repeated acquisition by its owning thread.
        let blocked = std::thread::scope(|scope| {
            scope
                .spawn(|| reorder(&paths, "photocraft", "designcraft", true))
                .join()
                .unwrap()
        });
        assert!(blocked.is_err());
        assert_eq!(fs::read(paths.at("manager-settings.json")).unwrap(), bytes);
        drop(lock);
        let saved = reorder(&paths, "photocraft", "designcraft", true).unwrap();
        assert_eq!(saved.release_format, current.release_format);
        assert_eq!(saved.architecture, current.architecture);
        assert_eq!(saved.selected_apps, current.selected_apps);
        assert_eq!(saved.app_order[0], "photocraft");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn invalid_builder_draft_or_relative_folder_leaves_new_files_absent() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let paths = Paths::new(root.clone(), None);
        let draft = BuilderPreferences {
            log_size_mb: 0,
            ..Default::default()
        };
        assert!(save(
            &paths,
            &root.join("locations"),
            &(),
            &root,
            &paths.tools,
            None,
            Some(&draft)
        )
        .is_err());
        assert!(save(
            &paths,
            &root.join("locations"),
            &(),
            Path::new("relative"),
            &paths.tools,
            None,
            Some(&Default::default())
        )
        .is_err());
        assert!(!root.exists());
    }
    #[test]
    fn rollback_failure_retains_exact_original_bytes_in_recovery_file() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let target = root.join("settings.json");
        fs::write(&target, b"original bytes").unwrap();
        let error = commit(&[(target.clone(), b"new".to_vec())], |path, _| {
            fs::remove_file(path)?;
            fs::create_dir(path)?;
            bail!("injected write failure");
        })
        .unwrap_err();
        assert!(format!("{error:#}").contains("rollback incomplete"));
        let recovery = fs::read_dir(&root)
            .unwrap()
            .flatten()
            .find(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|ext| ext == "recovery")
            })
            .unwrap();
        assert_eq!(fs::read(recovery.path()).unwrap(), b"original bytes");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn successful_save_survives_restart_and_keeps_restart_required_locations() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let paths = Paths::new(root.clone(), None);
        let prefs = Preferences {
            release_format: "portable".into(),
            architecture: "x86".into(),
            ..Default::default()
        };
        files::write_json(&paths.at("updater-settings.json"), &prefs).unwrap();
        let locations = root.join("locations.json");
        let future_root = root.join("next-library");
        save(
            &paths,
            &locations,
            &(future_root.clone(), paths.tools.clone()),
            &future_root,
            &paths.tools,
            Some(&prefs),
            None,
        )
        .unwrap();
        let reopened = Paths::new(root.clone(), None);
        assert_eq!(reopened.read_preferences().unwrap().architecture, "x86");
        assert_eq!(
            reopened.config().unwrap().apps_root,
            root.display().to_string()
        );
        let saved: (PathBuf, PathBuf) = files::read_json(&locations).unwrap();
        assert_eq!(saved.0, future_root);
        assert!(!future_root.exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn later_failure_restores_exact_bytes_and_nonexistence_even_after_replace() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let first = root.join("first");
        let second = root.join("second");
        fs::write(&first, b" original\nbytes ").unwrap();
        let mut count = 0;
        assert!(commit(
            &[
                (first.clone(), b"new".to_vec()),
                (second.clone(), b"new".to_vec())
            ],
            |p, b| {
                atomic_bytes(p, b)?;
                count += 1;
                if count == 2 {
                    bail!("post-replace sync failure");
                }
                Ok(())
            }
        )
        .is_err());
        assert_eq!(count, 2);
        assert_eq!(fs::read(&first).unwrap(), b" original\nbytes ");
        assert!(!second.exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn invalid_save_does_not_migrate_legacy_or_create_inventory() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let paths = Paths::new(root.clone(), None);
        files::write_json(&paths.at("updater-settings.json"), &Preferences::default()).unwrap();
        let bytes = fs::read(paths.at("updater-settings.json")).unwrap();
        let prefs = Preferences {
            release_format: "portable".into(),
            ..Default::default()
        };
        assert!(save(
            &paths,
            &root.join("locations"),
            &(),
            Path::new("relative"),
            &root,
            Some(&prefs),
            None
        )
        .is_err());
        assert_eq!(fs::read(paths.at("updater-settings.json")).unwrap(), bytes);
        assert!(!paths.at("manager-settings.json").exists());
        assert!(!paths.at("settings.json").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
