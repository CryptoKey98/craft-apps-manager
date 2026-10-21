use super::launch::installed;
use crate::{
    files,
    model::{Installed, Paths},
    platform,
};
use anyhow::{bail, Context, Result};
use std::path::PathBuf;

pub fn uninstall(paths: &Paths, app: &str) -> Result<()> {
    crate::model::valid_app(app)?;
    let _lock = platform::Lock::take("Local\\CraftAppsManager")?;
    uninstall_locked(paths, app)
}

fn uninstall_locked(paths: &Paths, app: &str) -> Result<()> {
    if platform::running_app(app)? {
        bail!("Close the app before uninstalling it.");
    }
    let mut config = paths.config()?;
    let installed = installed(paths, app)?;
    if installed.install_kind == "installer" {
        crate::installers::uninstall(&installed)?;
        if let Err(error) = crate::portable::remove_installer_shortcut(paths, app) {
            eprintln!("Desktop shortcut cleanup: {error:#}");
        }
        forget_installation(&mut config, &installed);
        return paths.save_config(&config);
    }
    let root = PathBuf::from(&installed.path);
    crate::portable::validate(paths, app, &root)?;
    let releases = root
        .parent()
        .context("Missing app parent folder")?
        .to_path_buf();
    let temporary = releases.join(format!(".uninstall-{app}-{}", uuid::Uuid::new_v4()));
    std::fs::rename(&root, &temporary)?;
    forget_installation(&mut config, &installed);
    if let Err(error) = paths.save_config(&config) {
        std::fs::rename(&temporary, &root)?;
        return Err(error);
    }
    if let Err(error) = crate::portable::remove_shortcut(&temporary) {
        eprintln!("Shortcut cleanup: {error:#}");
    }
    files::remove_managed(&temporary, &releases)?;
    if let Err(error) = crate::portable::cleanup_empty_installation_parent(paths, app, &root) {
        eprintln!("Empty installation folder cleanup: {error:#}");
    }
    Ok(())
}

/// Remove only the completed uninstall target; keep other verified copies
/// available immediately and preserve the format of an empty selected record.
pub(crate) fn forget_installation(config: &mut crate::model::Config, removed: &Installed) {
    config.installations.retain(|record| {
        !(record.name == removed.name
            && record.path == removed.path
            && (record.install_kind == "installer") == (removed.install_kind == "installer"))
    });
    let remaining = config
        .installations
        .iter()
        .find(|record| {
            record.name == removed.name
                && !record.path.is_empty()
                && crate::model::installed_executable(
                    std::path::Path::new(&record.path),
                    &record.name,
                )
                .is_some()
        })
        .cloned();
    for entry in config.apps.iter_mut().filter(|a| a.name == removed.name) {
        if let Some(record) = &remaining {
            *entry = record.clone();
        } else {
            entry.path.clear();
            entry.version.clear();
            entry.product_code.clear();
            entry.install_kind = removed.install_kind.clone();
            entry.architecture = removed.architecture.clone();
        }
    }
}

pub fn uninstall_with_profile(paths: &Paths, app: &str, delete_profile: bool) -> Result<()> {
    crate::model::valid_app(app)?;
    let _lock = platform::Lock::take("Local\\CraftAppsManager")?;

    if platform::running_app(app)? {
        bail!("Close the app before uninstalling it.");
    }
    let targets = if delete_profile {
        crate::profiles::targets(paths, app)?
    } else {
        Vec::new()
    };
    if delete_profile {
        crate::profiles::validate(&targets)?;
    }
    let preserved = if delete_profile {
        None
    } else {
        crate::profiles::preserve_portable(paths, app)?
    };
    if let Err(error) = uninstall_locked(paths, app) {
        if let Some((original, kept)) = preserved {
            files::move_verified(&kept, &original)
                .context("Uninstall failed; could not restore the retained profile")?;
        }
        return Err(error);
    }
    if delete_profile {
        crate::profiles::remove(&targets)
            .context("App was uninstalled, but profile cleanup failed")?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn repair_linux_shortcuts(paths: &Paths) -> Result<()> {
    for app in crate::model::apps() {
        let old = paths.at(format!("releases/{app}.lnk"));
        if !old.is_file() {
            continue;
        }
        files::inside(&old, &paths.at("releases"))?;
        let target = paths.at(format!("releases/{app}/{app}"));
        if !target.is_file() {
            continue;
        }
        files::inside(&target, &paths.at("releases"))?;
        let expected = format!("Exec=\"{}\" ", target.display());
        if std::fs::metadata(&old)?.len() > 65536 {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(&old) else {
            continue;
        };
        if !contents.starts_with("[Desktop Entry]\n")
            || !contents.lines().any(|line| line == expected)
        {
            continue;
        }
        platform::shortcut(
            &paths.at(format!("releases/{app}.desktop")),
            &target,
            "",
            target.parent().unwrap(),
        )?;
        std::fs::remove_file(old)?;
    }
    Ok(())
}
