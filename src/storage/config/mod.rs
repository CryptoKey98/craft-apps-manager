//! Library-scoped YAML configuration. Locked helpers require mutation ownership.
pub(crate) mod adapters;
pub(crate) mod document;
pub mod migration;
mod patch;
#[cfg(test)]
mod tests;

use anyhow::{bail, Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

// Configuration writers take this short-lived lock after the operation lock, if
// one is needed. Cosmetic edits never acquire the long-running operation lock.
pub(crate) fn write_lock() -> Result<crate::platform::Lock> {
    crate::platform::Lock::take("Local\\CraftAppsManagerConfiguration")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigRevision(pub String);
#[derive(Clone, Debug)]
pub struct ConfigSnapshot {
    pub revision: ConfigRevision,
    pub document: Value,
}
impl ConfigSnapshot {
    pub fn manager(&self) -> Result<crate::storage::preferences::Preferences> {
        document::section(&self.document, "manager")
    }
    pub fn builder(&self) -> Result<crate::builds::preferences::BuilderPreferences> {
        document::section(&self.document, "builder")
    }
    pub fn custom_apps(&self) -> Result<Vec<crate::catalog::Entry>> {
        document::section(&self.document, "custom_apps")
    }
}
#[derive(Clone, Debug, Default)]
pub struct ConfigPatch {
    pub fields: Vec<FieldPatch>,
}
#[derive(Clone, Debug)]
pub struct FieldPatch {
    pub path: Vec<String>,
    pub before: Option<Value>,
    pub after: Option<Value>,
}
#[derive(Clone, Debug)]
pub struct CommitOutcome {
    pub snapshot: ConfigSnapshot,
    pub changed: bool,
}
#[derive(Clone, Debug)]
pub struct ConfigStore {
    root: PathBuf,
}
impl ConfigStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn path(&self) -> PathBuf {
        self.root.join("config.yaml")
    }
    pub fn load(&self) -> Result<ConfigSnapshot> {
        if fs::symlink_metadata(self.path()).is_ok() {
            return self.load_locked();
        }
        // A fresh store has nothing to migrate or mutate. Operations may already
        // own the settings lock; reading defaults must not acquire it again.
        if !migration::has_legacy_sources(self.root())? {
            return self.load_locked();
        }
        let _lock = crate::platform::Lock::take("Local\\CraftAppsManager")?;
        let _config_lock = write_lock()?;
        self.load_or_migrate_locked()
    }
    pub fn commit(&self, patch: ConfigPatch) -> Result<CommitOutcome> {
        let _lock = crate::platform::Lock::take("Local\\CraftAppsManager")?;
        let _config_lock = write_lock()?;
        let snapshot = self.load_or_migrate_locked()?;
        let merged = patch::apply(&snapshot.document, patch)?;
        if merged == snapshot.document {
            return Ok(CommitOutcome {
                snapshot,
                changed: false,
            });
        }
        Ok(CommitOutcome {
            snapshot: self.publish_locked(merged)?,
            changed: true,
        })
    }
    pub(crate) fn load_or_migrate_locked(&self) -> Result<ConfigSnapshot> {
        let absent = match fs::symlink_metadata(self.path()) {
            Ok(_) => false,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Err(error) => return Err(error.into()),
        };
        if absent && migration::has_legacy_sources(self.root())? {
            return Ok(migration::ensure_migrated_locked(self)?.snapshot);
        }
        self.load_locked()
    }

    pub(crate) fn set_theme(&self, theme: crate::model::Theme) -> Result<ConfigSnapshot> {
        let _lock = write_lock()?;
        // Migration still needs operation ownership; never race an import.
        anyhow::ensure!(
            self.path().exists() || !migration::has_legacy_sources(self.root())?,
            "Settings must finish migrating before changing the theme"
        );
        let snapshot = self.load_locked()?;
        let mut document = snapshot.document.clone();
        document["manager"]["theme"] = serde_json::to_value(theme)?;
        if document == snapshot.document {
            return Ok(snapshot);
        }
        self.publish_locked(document)
    }
    /// Caller must hold Local\\CraftAppsManager; never recursively take the lock.
    pub(crate) fn load_locked(&self) -> Result<ConfigSnapshot> {
        let path = self.path();
        match fs::symlink_metadata(&path) {
            Ok(_) => ensure_unlinked(&path)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ConfigSnapshot {
                    revision: revision(&[]),
                    document: document::defaults(),
                })
            }
            Err(e) => return Err(e.into()),
        }
        let mut bytes = Vec::new();
        fs::File::open(&path)?
            .take(document::MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        let mut document = document::parse(&bytes)?;
        crate::portable_mode::convert(&path, &mut document, false)?;
        Ok(ConfigSnapshot {
            revision: revision(&bytes),
            document,
        })
    }
    pub(crate) fn document_bytes(&self, document: &Value) -> Result<Vec<u8>> {
        let mut encoded = document.clone();
        crate::portable_mode::convert(&self.path(), &mut encoded, true)?;
        document::encode(&encoded)
    }
    /// Caller owns mutation lock. No external callbacks or inventory scans occur.
    pub(crate) fn publish_locked(&self, document: Value) -> Result<ConfigSnapshot> {
        let mut encoded = document.clone();
        crate::portable_mode::convert(&self.path(), &mut encoded, true)?;
        let bytes = document::encode(&encoded)?;
        let mut expected = encoded;
        crate::portable_mode::convert(&self.path(), &mut expected, false)?;
        fs::create_dir_all(&self.root)?;
        ensure_unlinked(&self.path())?;
        let temp = self
            .root
            .join(format!(".config-{}.tmp", uuid::Uuid::new_v4()));
        let write = (|| -> Result<()> {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            crate::platform::atomic_replace(&temp, &self.path())?;
            Ok(())
        })();
        if write.is_err() {
            let _ = fs::remove_file(&temp);
        }
        write.context("Could not publish library configuration")?;
        let snapshot = self.load_locked()?;
        if snapshot.document != expected {
            bail!("Configuration read-back did not match the saved document");
        }
        Ok(snapshot)
    }
}
pub(crate) fn revision(bytes: &[u8]) -> ConfigRevision {
    ConfigRevision(format!("{:x}", Sha256::digest(bytes)))
}
fn ensure_unlinked(path: &Path) -> Result<()> {
    if fs::symlink_metadata(path).is_ok() && crate::files::linked(path)? {
        bail!("Linked configuration target is not supported");
    }
    Ok(())
}
