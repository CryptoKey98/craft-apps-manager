//! Conservative, journaled migration of allowlisted legacy preference JSON.
use super::{document, ConfigSnapshot, ConfigStore};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

const JOURNAL: &str = "runtime/config-migration.json";
const SOURCES: &[(&str, &str)] = &[
    ("manager-settings.json", "manager"),
    ("updater-settings.json", "manager"),
    ("builder-settings.json", "builder"),
    ("runtime/custom-apps.json", "custom_apps"),
    ("runtime/launch-settings.json", "launch"),
    ("runtime/build-launch-settings.json", "build_launch"),
];

pub struct MigrationOutcome {
    pub snapshot: ConfigSnapshot,
    pub warnings: Vec<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    name: String,
    sha256: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema_version: u32,
    candidate_hash: String,
    verified: bool,
    sources: Vec<Source>,
}

pub fn ensure_migrated(store: &ConfigStore) -> Result<MigrationOutcome> {
    let _lock = crate::platform::Lock::take("Local\\CraftAppsManager")?;
    let _config_lock = super::write_lock()?;
    ensure_migrated_locked(store)
}
/// Caller owns the existing settings mutation mutex; never recursively lock it.
pub(crate) fn ensure_migrated_locked(store: &ConfigStore) -> Result<MigrationOutcome> {
    // Remove only unchanged legacy inputs after durable publication/read-back.
    // Cleanup failures remain warnings; config.yaml stays authoritative.
    migrate_locked(store, true)
}

fn migrate_locked(store: &ConfigStore, cleanup_allowed: bool) -> Result<MigrationOutcome> {
    if exists(&store.path())? {
        let snapshot = store.load_locked()?;
        let warnings = resume_cleanup(store, &snapshot, cleanup_allowed);
        return Ok(MigrationOutcome { snapshot, warnings });
    }
    let (mut candidate, sources) = candidate(store)?;
    candidate = document::parse(&store.document_bytes(&candidate)?)?;
    crate::portable_mode::convert(&store.path(), &mut candidate, false)?;
    document::validate(&candidate)?;
    // Encode before publishing the journal so unsupported data changes no files.
    document::encode(&candidate)?;
    for source in &sources {
        let path = store.root().join(&source.name);
        crate::files::inside(&path, store.root())?;
        ensure!(
            hash(&bounded_read(&path, document::MAX_BYTES)?) == source.sha256,
            "Legacy configuration changed during migration: {}",
            source.name
        );
    }
    let mut journal = Journal {
        schema_version: 1,
        candidate_hash: semantic_hash(&candidate)?,
        verified: false,
        sources,
    };
    write_journal(store, &journal)?;
    let snapshot = store.publish_locked(candidate.clone())?;
    ensure!(
        snapshot.document == candidate,
        "Migration configuration read-back mismatch"
    );
    journal.verified = true;
    // YAML is already durable: later journal/cleanup failures are recoverable warnings.
    if write_journal(store, &journal).is_err() {
        return Ok(MigrationOutcome { snapshot, warnings: vec![
            "Configuration imported; migration verification marker could not be saved. Legacy JSON was retained.".into()
        ] });
    }
    let warnings = cleanup(store, &snapshot, &journal, cleanup_allowed);
    Ok(MigrationOutcome { snapshot, warnings })
}

fn candidate(store: &ConfigStore) -> Result<(Value, Vec<Source>)> {
    let mut candidate = document::defaults();
    let mut sources = Vec::new();
    let manager_present = exists(&store.root().join("manager-settings.json"))?;
    for &(name, section) in SOURCES {
        let path = store.root().join(name);
        if !exists(&path)? {
            continue;
        }
        crate::files::inside(&path, store.root())?;
        let bytes = bounded_read(&path, document::MAX_BYTES)?;
        let input = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
        let mut value: Value = serde_json::from_slice(input)
            .map_err(|_| anyhow::anyhow!("Invalid legacy configuration: {name}"))?;
        crate::portable_mode::convert(&path, &mut value, false)?;
        if section == "custom_apps" {
            ensure!(
                value.is_array(),
                "Invalid legacy configuration shape: {name}"
            );
        } else {
            ensure!(
                value.is_object(),
                "Invalid legacy configuration shape: {name}"
            );
        }
        if name == "updater-settings.json" && manager_present {
            candidate["legacy_extensions"][name] = value;
        } else {
            candidate[section] = value;
        }
        sources.push(Source {
            name: name.into(),
            sha256: hash(&bytes),
        });
    }
    // Existing readers clamp these fields; preserve that compatibility without
    // catalog adoption/pruning, which could erase not-yet-loaded custom app choices.
    for (section, field, minimum, maximum) in [
        ("manager", "appCheckIntervalMinutes", 10, 60),
        ("builder", "logSizeMB", 1, 100),
        ("builder", "logArchives", 0, 5),
    ] {
        if let Some(number) = candidate[section].get(field).and_then(Value::as_u64) {
            candidate[section][field] = Value::from(number.clamp(minimum, maximum));
        }
    }
    document::normalize_manager_alias(&mut candidate);
    Ok((candidate, sources))
}

fn resume_cleanup(store: &ConfigStore, snapshot: &ConfigSnapshot, allowed: bool) -> Vec<String> {
    let path = store.root().join(JOURNAL);
    let result = (|| -> Result<Option<Journal>> {
        if !exists(&path)? {
            return Ok(None);
        }
        crate::files::inside(&path, store.root())?;
        let journal: Journal = serde_json::from_slice(&bounded_read(&path, 64 * 1024)?)?;
        ensure!(
            journal.schema_version == 1 && journal.sources.len() <= SOURCES.len(),
            "Invalid migration journal"
        );
        let mut seen = std::collections::BTreeSet::new();
        for source in &journal.sources {
            ensure!(
                SOURCES.iter().any(|(name, _)| *name == source.name) && seen.insert(&source.name),
                "Unrecognized migration journal source"
            );
            ensure!(
                source.sha256.len() == 64 && source.sha256.bytes().all(|c| c.is_ascii_hexdigit()),
                "Invalid migration journal hash"
            );
        }
        ensure!(
            verified_receipt(&journal),
            "Migration was not durably verified"
        );
        Ok(Some(journal))
    })();
    match result {
        Ok(Some(journal)) => cleanup(store, snapshot, &journal, allowed),
        Ok(None) => {
            if SOURCES
                .iter()
                .any(|(name, _)| store.root().join(name).exists())
            {
                vec!["Legacy JSON was retained; it has no verified migration journal and is not imported over config.yaml.".into()]
            } else {
                Vec::new()
            }
        }
        Err(_) => vec![
            "Legacy JSON was retained because its migration receipt could not be verified.".into(),
        ],
    }
}

fn cleanup(
    store: &ConfigStore,
    snapshot: &ConfigSnapshot,
    journal: &Journal,
    allowed: bool,
) -> Vec<String> {
    if !journal.sources.is_empty() && !allowed {
        return vec!["Configuration imported. Legacy JSON cleanup is postponed because older Manager windows cannot be reliably excluded; close old versions before a future supported cleanup.".into()];
    }
    let mut warnings = Vec::new();
    if !verified_receipt(journal) {
        return vec!["Legacy JSON was retained because configuration verification failed.".into()];
    }
    // The receipt proves publication/read-back completed at migration time.
    // Later legitimate edits must not invalidate that completed step. Reverify
    // against this cleanup attempt's snapshot instead of the original import.
    let expected = match semantic_hash(&snapshot.document) {
        Ok(hash) => hash,
        Err(_) => {
            return vec![
                "Legacy JSON was retained because configuration verification failed.".into(),
            ]
        }
    };
    let verified_on_disk = || -> Result<()> {
        let current = store.load_locked()?;
        ensure!(
            semantic_hash(&current.document)? == expected,
            "Authoritative configuration changed before legacy cleanup"
        );
        Ok(())
    };
    if verified_on_disk().is_err() {
        return vec![
            "Legacy JSON was retained because config.yaml could not be reverified on disk.".into(),
        ];
    }
    for source in &journal.sources {
        let result = (|| -> Result<()> {
            let path = store.root().join(&source.name);
            if !exists(&path)? {
                return Ok(());
            }
            crate::files::inside(&path, store.root())?;
            ensure!(
                SOURCES.iter().any(|(name, _)| *name == source.name),
                "Unrecognized legacy cleanup source"
            );
            ensure!(
                hash(&bounded_read(&path, document::MAX_BYTES)?) == source.sha256,
                "Legacy configuration changed"
            );
            verified_on_disk()?;
            fs::remove_file(path)?;
            Ok(())
        })();
        if result.is_err() {
            warnings.push(format!(
                "Legacy configuration retained or cleanup failed: {}",
                source.name
            ));
        }
    }
    if warnings.is_empty() {
        let path = store.root().join(JOURNAL);
        if crate::files::inside(&path, store.root())
            .and_then(|_| Ok(fs::remove_file(&path)?))
            .is_err()
        {
            warnings.push(
                "Legacy cleanup completed, but its migration journal could not be removed.".into(),
            );
        }
    }
    warnings
}

fn verified_receipt(journal: &Journal) -> bool {
    journal.verified
        && journal.candidate_hash.len() == 64
        && journal
            .candidate_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
}

fn write_journal(store: &ConfigStore, journal: &Journal) -> Result<()> {
    let path = store.root().join(JOURNAL);
    crate::files::inside(&path, store.root())?;
    fs::create_dir_all(path.parent().context("Missing journal directory")?)?;
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(&serde_json::to_vec(journal)?)?;
        file.sync_all()?;
        drop(file);
        crate::platform::atomic_replace(&temp, &path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
fn exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}
fn bounded_read(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        bail!("Legacy configuration exceeds migration size limit");
    }
    Ok(bytes)
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn semantic_hash(value: &Value) -> Result<String> {
    Ok(hash(&serde_json::to_vec(value)?))
}

#[cfg(test)]
#[path = "migration_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod recovery_tests;

pub fn has_legacy_sources(root: &Path) -> Result<bool> {
    for &(name, _) in SOURCES {
        if exists(&root.join(name))? {
            return Ok(true);
        }
    }
    Ok(false)
}
