//! Installation inventory filename compatibility. Operational state remains JSON.
use crate::model::{Config, Paths};
use anyhow::{ensure, Context, Result};
use std::{fs, io::Write, path::PathBuf};

impl Paths {
    /// Resolve the authoritative inventory, importing old-manager data only when absent.
    pub fn inventory_path(&self) -> Result<PathBuf> {
        let target = self.at("installations.json");
        if exists(&target)? {
            validate(self, &target)?;
            return Ok(target);
        }
        // Separate from the settings/operation mutex: callers may already hold it.
        let _lock = crate::platform::Lock::take("Local\\CraftAppsManagerInventoryMigration")?;
        if exists(&target)? {
            validate(self, &target)?;
            return Ok(target);
        }
        let legacy = self.at("settings.json");
        if !exists(&legacy)? {
            return Ok(target);
        }
        crate::files::inside(&legacy, &self.root)?;
        let original = fs::read(&legacy)?;
        let value = validate(self, &legacy)?;
        // Publish only while the inventory migration mutex excludes other importers.
        let temp = self.at(format!(".inventory-{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| -> Result<()> {
            let bytes = crate::files::json_bytes(&target, &value)?;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            if exists(&target)? {
                validate(self, &target)?;
                return Ok(());
            }
            crate::platform::atomic_replace(&temp, &target)?;
            let published = validate(self, &target)?;
            ensure!(
                published == value,
                "Inventory read-back changed during import"
            );
            // Never remove a source that changed while publication was in progress.
            crate::files::inside(&legacy, &self.root)?;
            if fs::read(&legacy)? == original {
                fs::remove_file(&legacy)
                    .context("Imported inventory, but could not remove legacy settings.json")?;
            }
            Ok(())
        })();
        let _ = fs::remove_file(temp);
        result?;
        validate(self, &target)?;
        Ok(target)
    }

    /// Read inventory facts without executable detection or default reconstruction.
    pub fn read_inventory(&self) -> Result<Option<Config>> {
        let file = self.inventory_path()?;
        if !file.exists() {
            return Ok(None);
        }
        Ok(Some(crate::files::read_json(&file)?))
    }
}

fn validate(paths: &Paths, file: &std::path::Path) -> Result<serde_json::Value> {
    crate::files::inside(file, &paths.root)?;
    let value: serde_json::Value = crate::files::read_json(file)?;
    let config: Config = serde_json::from_value(value.clone())
        .with_context(|| format!("Invalid installation inventory: {}", file.display()))?;
    ensure!(
        std::path::Path::new(&config.apps_root) == paths.root,
        "{} points to a different data folder. Select that folder in Settings.",
        file.display()
    );
    Ok(value)
}

fn exists(file: &std::path::Path) -> Result<bool> {
    match fs::symlink_metadata(file) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}
