//! Local, verified previous versions, keyed by the portable ownership identity.
mod cleanup;
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

/// Metadata authorization for explicit deletion, independent of file integrity.
/// Restoration continues to require the complete verified manifest.
pub fn removal_candidate(paths: &Paths, installed: &Installed) -> Result<Option<RollbackPoint>> {
    selected(paths, installed)?;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    native: Option<native::NativePoint>,
}
#[derive(Clone)]
pub struct RollbackStatus {
    pub installed: Installed,
    pub previous: Option<RollbackPoint>,
    pub preference: Option<bool>,
    pub global_enabled: bool,
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
        preference: id.and_then(|id| prefs.rollback_overrides.get(&id).copied()),
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
