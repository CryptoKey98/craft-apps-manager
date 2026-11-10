//! Local, verified previous versions, keyed by the portable ownership identity.
mod cleanup;
mod compression;
pub(crate) mod native;
mod native_package;
mod restore;
mod store;
mod uninstall;
use crate::{
    files,
    jobs::Job,
    model::{Installed, Paths},
    portable,
};
use anyhow::{ensure, Context, Result};
#[cfg(all(test, not(target_os = "macos")))]
use restore::restore_with;
pub use restore::rollback;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use store::{ensure_identity, identity, manifest, owner, read_point, root, selected, verify_point};
pub use store::{record_update, remove_previous};
pub use uninstall::discard_uninstalled;

pub fn remove_all(paths: &Paths) -> Result<usize> {
    let _lock = crate::platform::Lock::take("Local\\CraftAppsManager")?;
    ensure!(
        !crate::operations::Manager::new(&paths.root)
            .list()
            .iter()
            .any(|op| !op.lifecycle.terminal()),
        "Wait for active operations to finish"
    );
    let Some(config) = paths.read_inventory()? else {
        return Ok(0);
    };
    ensure!(
        Path::new(&config.apps_root) == paths.root,
        "Installation inventory belongs to another library"
    );
    let mut seen = std::collections::BTreeSet::new();
    let mut removed = 0;
    let mut preserved = 0;
    for installed in config.apps.iter().chain(&config.installations) {
        let id = if installed.install_kind == "installer" {
            native::key(installed)
        } else {
            identity(paths, installed)
        };
        let Ok(id) = id else {
            continue;
        };
        if !seen.insert(id) {
            continue;
        }
        match removal_candidate_for(paths, installed) {
            Ok(Some(point)) => match store::remove_checked(paths, installed, &point) {
                Ok(()) => removed += 1,
                Err(_) => preserved += 1,
            },
            Ok(None) => {}
            Err(_) => preserved += 1,
        }
    }
    ensure!(preserved == 0, "Removed {removed} rollback versions; {preserved} could not be safely removed. Active apps and recovery files were preserved.");
    Ok(removed)
}

/// Metadata authorization for explicit deletion, independent of file integrity.
/// Restoration continues to require the complete verified manifest.
pub fn removal_candidate(paths: &Paths, installed: &Installed) -> Result<Option<RollbackPoint>> {
    selected(paths, installed)?;
    removal_candidate_for(paths, installed)
}
fn removal_candidate_for(paths: &Paths, installed: &Installed) -> Result<Option<RollbackPoint>> {
    let id = if installed.install_kind == "installer" {
        native::key(installed)?
    } else {
        identity(paths, installed)?
    };
    let Some(point) = read_point(paths, &id)? else {
        return Ok(None);
    };
    let folder = if installed.install_kind == "installer" {
        native::bound_folder(paths, installed, &point)?
    } else {
        store::bound_folder(paths, installed, &point)?
    };
    if let Some(config) = paths.read_inventory()? {
        for copy in config
            .apps
            .iter()
            .chain(&config.installations)
            .filter(|copy| !copy.path.is_empty())
        {
            let copy_path = std::path::absolute(&copy.path)?;
            ensure!(
                !copy_path
                    .ancestors()
                    .any(|parent| crate::portable_mode::same_library(parent, &folder)),
                "Rollback folder contains an active installation"
            );
        }
    }
    Ok(Some(point))
}

/// Caller holds the existing installation operation lock.
pub(crate) fn cleanup_empty(paths: &Paths, job: &Job) {
    if let Err(error) = cleanup::prune_empty(paths) {
        job.log(&format!("Empty rollback folder cleanup warning: {error:#}"));
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RollbackPoint {
    pub id: String,
    pub version: String,
    pub bytes: u64,
    pub architecture: String,
    pub created_at: String,
    installation_id: String,
    app: String,
    package_type: String,
    executable: PathBuf,
    files: BTreeMap<PathBuf, (u64, String)>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    directories: BTreeMap<PathBuf, u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    native: Option<native::NativePoint>,
    #[serde(default)]
    compressed: bool,
}
#[derive(Clone)]
pub struct RollbackStatus {
    pub installed: Installed,
    pub previous: Option<RollbackPoint>,
    pub preference: Option<bool>,
    pub global_enabled: bool,
    pub compression_preference: Option<bool>,
    pub global_compression_enabled: bool,
    pub unsupported: Option<String>,
}
pub fn status(paths: &Paths, installed: &Installed) -> Result<RollbackStatus> {
    let prefs = crate::storage::config::ConfigStore::new(paths.root.clone())
        .load_locked()?
        .manager()?;
    if installed.install_kind == "installer" {
        return native::status(paths, installed, &prefs);
    }
    let unsupported=(installed.install_kind!="portable").then(||"Native installations use verified installer version selection; folder rollback is unavailable.".into());
    let id = if unsupported.is_none() {
        let id = owner(paths, installed)?.installation_id;
        if id.is_some() {
            Some(identity(paths, installed)?)
        } else {
            None
        }
    } else {
        None
    };
    Ok(RollbackStatus {
        installed: installed.clone(),
        previous: id
            .as_deref()
            .map(|id| read_point(paths, id))
            .transpose()?
            .flatten(),
        preference: id
            .as_ref()
            .and_then(|id| prefs.rollback_overrides.get(id).copied()),
        compression_preference: id
            .as_ref()
            .and_then(|id| prefs.rollback_compression_overrides.get(id).copied()),
        global_compression_enabled: prefs.compress_rollbacks,
        global_enabled: prefs.rollback_enabled(),
        unsupported,
    })
}
pub fn staging_path(paths: &Paths) -> PathBuf {
    paths
        .at("runtime/rollback-transactions")
        .join(uuid::Uuid::new_v4().to_string())
        .join("previous")
}
pub fn prepare_update(paths: &Paths, previous: &Installed, staged: &Path) -> Result<()> {
    if previous.path.is_empty() || previous.install_kind != "portable" {
        return Ok(());
    }
    let id = ensure_identity(paths, previous)?;
    files::inside(&staged.join(portable::MARKER), staged)?;
    let mut marker: portable::Ownership = files::read_json(&staged.join(portable::MARKER))?;
    let old = owner(paths, previous)?;
    ensure!(
        marker.app == previous.name
            && crate::portable_mode::same_library(
                &marker.library,
                &std::path::absolute(&paths.root)?
            ),
        "Staged application identity mismatch"
    );
    marker.installation_id = Some(id);
    marker.shortcut = old.shortcut;
    files::write_json(&staged.join(portable::MARKER), &marker)
}
pub fn save_override(
    paths: &Paths,
    expected: &Installed,
    expected_preference: Option<bool>,
    preference: Option<bool>,
) -> Result<()> {
    save_options(paths, expected, expected_preference, preference, None)
}
pub fn save_options(
    paths: &Paths,
    expected: &Installed,
    expected_preference: Option<bool>,
    preference: Option<bool>,
    compression: Option<(Option<bool>, Option<bool>)>,
) -> Result<()> {
    let _lock = crate::platform::Lock::take("Local\\CraftAppsManager")?;
    selected(paths, expected)?;
    let id = if expected.install_kind == "installer" {
        native::supported(expected)?;
        native::key(expected)?
    } else {
        ensure_identity(paths, expected)?
    };
    let base = paths.read_preferences_locked()?;
    ensure!(
        base.rollback_overrides.get(&id).copied() == expected_preference,
        "Rollback preference changed. Reopen the dialog."
    );
    let mut edited = base.clone();
    if let Some((original, value)) = compression {
        ensure!(
            expected.install_kind == "portable",
            "Installer packages retain their existing format"
        );
        ensure!(
            base.rollback_compression_overrides.get(&id).copied() == original,
            "Compression preference changed. Reopen the dialog."
        );
        if let Some(value) = value {
            edited
                .rollback_compression_overrides
                .insert(id.clone(), value);
        } else {
            edited.rollback_compression_overrides.remove(&id);
        }
    }
    if let Some(value) = preference {
        edited.rollback_overrides.insert(id, value);
    } else {
        edited.rollback_overrides.remove(&id);
    }
    crate::storage::settings::save_preferences_patch_locked(paths, &base, &edited)?;
    Ok(())
}
#[cfg(test)]
mod native_tests;
#[cfg(test)]
mod tests;
