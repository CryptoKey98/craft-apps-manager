//! Worker-only disk storage. Callers hold the key stripe across read/HTTP/write.
use anyhow::{ensure, Result};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
const MAX_BODY: usize = 16 * 1024 * 1024;
const MAX_META: usize = 64 * 1024;
#[derive(Clone, Debug)]
pub(super) struct Record {
    pub at: i64,
    pub accessed: i64,
    pub etag: Option<String>,
    pub value: Value,
    pub generation: String,
    pub bytes: usize,
}
#[derive(Serialize, Deserialize)]
struct Metadata {
    version: u8,
    codec: String,
    body_hash: String,
    decoded_len: usize,
    at: i64,
    accessed: i64,
    etag: Option<String>,
    generation: String,
}
pub(super) struct DiskCache {
    root: PathBuf,
}
pub(super) struct Guard {
    _file: File,
}
#[derive(Default, Debug)]
pub(super) struct PruneStats {
    pub bytes: u64,
    pub files: usize,
}
fn valid_key(key: &str) -> bool {
    key.len() == 64
        && key
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
impl DiskCache {
    /// Worker-only explicit cleanup. Keep lock files stable for other processes.
    pub(super) fn clear(&self) -> Result<()> {
        self.safe(&self.root)?;
        if !self.root.exists() {
            return Ok(());
        }
        let _maintenance = self.lock_file("maintenance.lock", true)?;
        // Acquire in a fixed order, excluding writers and legacy migrations
        // throughout the scan. Never recurse into unrecognized directories.
        let mut stripes = Vec::with_capacity(64);
        for index in 0..64 {
            stripes.push(self.lock_file(&format!("{index:02x}.lock"), true)?);
        }
        for paths in self.groups()?.into_values() {
            for path in paths {
                let Some(metadata) = self.inspect_path(&path)? else {
                    continue;
                };
                if metadata.is_file() {
                    fs::remove_file(path)?;
                }
            }
        }
        Ok(())
    }
    pub(super) fn generation(&self, key: &str) -> Result<String> {
        Ok(self.metadata(key)?.generation)
    }
    pub(super) fn new(root: &Path) -> Self {
        Self {
            root: root.to_owned(),
        }
    }
    // Inspect each ancestor once. Reuse that metadata to detect Windows reparse
    // points too, rather than issuing a second filesystem query through linked().
    fn inspect_path(&self, path: &Path) -> Result<Option<fs::Metadata>> {
        ensure!(path.starts_with(&self.root), "Cache path escaped root");
        let mut target = None;
        for (index, ancestor) in path.ancestors().enumerate() {
            match fs::symlink_metadata(ancestor) {
                Ok(metadata) => {
                    #[cfg(windows)]
                    let linked = {
                        use std::os::windows::fs::MetadataExt;
                        metadata.file_attributes() & 0x400 != 0
                    };
                    #[cfg(not(windows))]
                    let linked = metadata.file_type().is_symlink();
                    ensure!(!linked, "Linked cache path");
                    if index == 0 {
                        target = Some(metadata);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
        }
        Ok(target)
    }
    fn safe(&self, path: &Path) -> Result<()> {
        self.inspect_path(path).map(|_| ())
    }
    fn ensure_dir(&self, dir: &Path) -> Result<()> {
        if let Some(metadata) = self.inspect_path(dir)? {
            ensure!(metadata.is_dir(), "Cache directory is not a directory");
        } else {
            fs::create_dir_all(dir)?;
            self.safe(dir)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }
    pub(super) fn remove(&self, key: &str) -> Result<()> {
        for suffix in ["body", "meta.json", "json"] {
            let path = self.path(key, suffix)?;
            self.safe(&path)?;
            match fs::remove_file(&path) {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
    fn path(&self, key: &str, suffix: &str) -> Result<PathBuf> {
        ensure!(valid_key(key), "Invalid cache key");
        // Construction performs no I/O. Each operation validates immediately
        // before accessing the path, including both sides of open().
        Ok(self.root.join(format!("{key}.{suffix}")))
    }
    fn open(&self, path: &Path, create: bool) -> Result<File> {
        self.safe(path)?;
        let mut options = OpenOptions::new();
        options.read(true);
        if create {
            options.write(true).create(true).truncate(false);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW).mode(0o600);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x00200000);
        }
        let file = options.open(path)?;
        ensure!(
            file.metadata()?.is_file(),
            "Cache path is not a regular file"
        );
        self.safe(path)?;
        Ok(file)
    }
    fn bounded(&self, path: &Path, limit: usize) -> Result<Vec<u8>> {
        let file = self.open(path, false)?;
        ensure!(
            file.metadata()?.len() <= limit as u64,
            "Cache entry exceeds limit"
        );
        let mut bytes = Vec::new();
        file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= limit, "Cache entry exceeds limit");
        Ok(bytes)
    }
    fn atomic(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        self.safe(path)?;
        self.ensure_dir(&self.root)?;
        self.safe(path)?;
        let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
        self.safe(&temp)?;
        let result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut f = options.open(&temp)?;
            f.write_all(bytes)?;
            f.sync_all()?;
            drop(f);
            self.safe(path)?;
            crate::platform::atomic_replace(&temp, path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }
    fn lock_file(&self, name: &str, blocking: bool) -> Result<Option<Guard>> {
        let dir = self.root.join(".locks");
        self.ensure_dir(&self.root)?;
        self.ensure_dir(&dir)?;
        let f = self.open(&dir.join(name), true)?;
        if blocking {
            f.lock()?;
        } else if f.try_lock().is_err() {
            return Ok(None);
        }
        Ok(Some(Guard { _file: f }))
    }
    fn stripe(key: &str) -> Result<String> {
        ensure!(valid_key(key), "Invalid cache key");
        Ok(format!(
            "{:02x}.lock",
            u8::from_str_radix(&key[62..], 16)? % 64
        ))
    }
    pub(super) fn lock(&self, key: &str) -> Result<Guard> {
        Ok(self
            .lock_file(&Self::stripe(key)?, true)?
            .expect("blocking lock"))
    }
    pub(super) fn try_lock(&self, key: &str) -> Result<Option<Guard>> {
        self.lock_file(&Self::stripe(key)?, false)
    }
    fn metadata(&self, key: &str) -> Result<Metadata> {
        let m: Metadata =
            serde_json::from_slice(&self.bounded(&self.path(key, "meta.json")?, MAX_META)?)?;
        ensure!(
            m.version == 1
                && valid_key(&m.body_hash)
                && m.decoded_len <= MAX_BODY
                && matches!(m.codec.as_str(), "gzip" | "identity"),
            "Invalid cache metadata"
        );
        Ok(m)
    }
    fn write_metadata(&self, key: &str, m: &Metadata) -> Result<()> {
        let bytes = serde_json::to_vec(m)?;
        ensure!(bytes.len() <= MAX_META, "Cache metadata exceeds limit");
        self.atomic(&self.path(key, "meta.json")?, &bytes)
    }
    fn read_current(&self, key: &str) -> Result<Record> {
        let m = self.metadata(key)?;
        let body = self.bounded(&self.path(key, "body")?, MAX_BODY)?;
        ensure!(hash(&body) == m.body_hash, "Cache checksum mismatch");
        let decoded = if m.codec == "gzip" {
            let mut bytes = Vec::new();
            GzDecoder::new(body.as_slice())
                .take(MAX_BODY as u64 + 1)
                .read_to_end(&mut bytes)?;
            bytes
        } else {
            body
        };
        ensure!(
            decoded.len() == m.decoded_len && decoded.len() <= MAX_BODY,
            "Cache decoded size mismatch"
        );
        Ok(Record {
            at: m.at,
            accessed: m.accessed,
            etag: m.etag,
            value: serde_json::from_slice(&decoded)?,
            generation: m.generation,
            bytes: decoded.len(),
        })
    }
    pub(super) fn read(&self, key: &str, legacy_key: Option<&str>) -> Result<Option<Record>> {
        self.path(key, "body")?;
        self.safe(&self.root)?;
        if let Ok(record) = self.read_current(key) {
            return Ok(Some(record));
        }
        let Some(legacy) = legacy_key else {
            return Ok(None);
        };
        let path = self.path(legacy, "json")?;
        let Ok(bytes) = self.bounded(&path, MAX_BODY) else {
            return Ok(None);
        };
        #[derive(Deserialize)]
        struct Legacy {
            at: i64,
            etag: Option<String>,
            value: Value,
        }
        let Ok(old) = serde_json::from_slice::<Legacy>(&bytes) else {
            return Ok(None);
        };
        // Migration is optional; a failure must leave the old entry intact.
        if let Ok(record) = self.write(key, old.at, old.etag.clone(), &old.value) {
            if self
                .read_current(key)
                .is_ok_and(|r| r.value == old.value && r.at == old.at && r.etag == old.etag)
                && self
                    .bounded(&path, MAX_BODY)
                    .is_ok_and(|current| current == bytes)
            {
                self.safe(&path)?;
                let _ = fs::remove_file(path);
            }
            return Ok(Some(record));
        }
        let decoded = serde_json::to_vec(&old.value)?;
        Ok(Some(Record {
            at: old.at,
            accessed: old.at,
            etag: old.etag,
            value: old.value,
            generation: String::new(),
            bytes: decoded.len(),
        }))
    }
    pub(super) fn write(
        &self,
        key: &str,
        at: i64,
        etag: Option<String>,
        value: &Value,
    ) -> Result<Record> {
        let path = self.path(key, "body")?;
        let decoded = serde_json::to_vec(value)?;
        ensure!(decoded.len() <= MAX_BODY, "Cache payload exceeds limit");
        let mut body = decoded.clone();
        let mut codec = "identity";
        if decoded.len() >= 4096 {
            let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
            encoder.write_all(&decoded)?;
            let compressed = encoder.finish()?;
            if compressed.len() < body.len() {
                body = compressed;
                codec = "gzip";
            }
        }
        let m = Metadata {
            version: 1,
            codec: codec.into(),
            body_hash: hash(&body),
            decoded_len: decoded.len(),
            at,
            accessed: at,
            etag: etag.clone(),
            generation: uuid::Uuid::new_v4().simple().to_string(),
        };
        // Serialize/validate metadata before replacing either half.
        ensure!(
            serde_json::to_vec(&m)?.len() <= MAX_META,
            "Cache metadata exceeds limit"
        );
        self.atomic(&path, &body)?;
        self.write_metadata(key, &m)?;
        Ok(Record {
            at,
            accessed: at,
            etag,
            value: value.clone(),
            generation: m.generation,
            bytes: decoded.len(),
        })
    }
    pub(super) fn refresh(&self, key: &str, record: &Record, at: i64) -> Result<Record> {
        let mut m = self.metadata(key)?;
        ensure!(m.generation == record.generation, "Cache entry changed");
        m.at = at;
        m.accessed = at;
        m.generation = uuid::Uuid::new_v4().simple().to_string();
        self.write_metadata(key, &m)?;
        Ok(Record {
            at,
            accessed: at,
            etag: m.etag,
            value: record.value.clone(),
            generation: m.generation,
            bytes: record.bytes,
        })
    }
    pub(super) fn touch(&self, key: &str, now: i64) -> Result<()> {
        let mut m = self.metadata(key)?;
        if now.saturating_sub(m.accessed) >= 300 {
            m.accessed = now;
            m.generation = uuid::Uuid::new_v4().simple().to_string();
            self.write_metadata(key, &m)?;
        }
        Ok(())
    }
    fn owned_name(name: &str) -> Option<(&str, bool)> {
        let (key, suffix) = name.split_once('.')?;
        if !valid_key(key) {
            return None;
        }
        if matches!(suffix, "json" | "body" | "meta.json") {
            return Some((key, false));
        }
        let temp = suffix
            .strip_prefix("meta.")
            .unwrap_or(suffix)
            .strip_suffix(".tmp")?;
        (temp.len() == 32 && temp.bytes().all(|b| b.is_ascii_hexdigit())).then_some((key, true))
    }
    fn groups(&self) -> Result<BTreeMap<String, Vec<PathBuf>>> {
        let mut groups = BTreeMap::<String, Vec<PathBuf>>::new();
        self.safe(&self.root)?;
        if !self.root.exists() {
            return Ok(groups);
        }
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if let Some((key, _)) = Self::owned_name(name) {
                if self.safe(&entry.path()).is_ok() && entry.file_type()?.is_file() {
                    groups.entry(key.into()).or_default().push(entry.path());
                }
            }
        }
        Ok(groups)
    }
    fn size(paths: &[PathBuf]) -> u64 {
        paths
            .iter()
            .filter_map(|p| fs::symlink_metadata(p).ok())
            .map(|m| m.len())
            .sum()
    }
    fn accessed(&self, key: &str, paths: &[PathBuf]) -> i64 {
        self.metadata(key).map(|m| m.accessed).unwrap_or_else(|_| {
            // Legacy JSON and interrupted bodies use their last filesystem activity.
            paths
                .iter()
                .filter_map(|p| {
                    fs::symlink_metadata(p)
                        .ok()?
                        .modified()
                        .ok()?
                        .duration_since(UNIX_EPOCH)
                        .ok()
                })
                .map(|d| d.as_secs().min(i64::MAX as u64) as i64)
                .max()
                .unwrap_or(0)
        })
    }
    fn prune_candidate(
        &self,
        candidate: &PruneCandidate,
        now: i64,
        budget: u64,
        total: &mut u64,
        max_age: i64,
    ) -> Result<()> {
        let Some(_guard) = self.try_lock(&candidate.key)? else {
            return Ok(());
        };
        let used = self.accessed(&candidate.key, &candidate.paths);
        // A refresh/replacement may occur in the same timestamp second. Its
        // generation (and orphan file identity) must still invalidate the scan.
        if used > candidate.used
            || self.metadata(&candidate.key).ok().map(|m| m.generation) != candidate.generation
            || Self::fingerprints(&candidate.paths) != candidate.files
        {
            return Ok(());
        }
        let expired = now.saturating_sub(used) > max_age;
        let evict = expired || *total > budget;
        for path in &candidate.paths {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let temp = Self::owned_name(name).is_some_and(|(_, t)| t);
            let old_temp = fs::symlink_metadata(path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .is_some_and(|t| now.saturating_sub(t.as_secs() as i64) > 86400);
            if temp && !old_temp {
                continue;
            }
            if evict || (temp && old_temp) {
                self.safe(path)?;
                let Ok(metadata) = fs::symlink_metadata(path) else {
                    continue;
                };
                if !metadata.is_file() {
                    continue;
                }
                let size = metadata.len();
                fs::remove_file(path)?;
                *total = total.saturating_sub(size);
            }
        }
        Ok(())
    }
    fn fingerprints(paths: &[PathBuf]) -> Vec<Option<FileFingerprint>> {
        paths
            .iter()
            .map(|path| {
                fs::symlink_metadata(path).ok().map(|m| FileFingerprint {
                    len: m.len(),
                    modified: m.modified().ok(),
                    regular: m.is_file(),
                })
            })
            .collect()
    }
    fn candidate(&self, key: String, paths: Vec<PathBuf>) -> PruneCandidate {
        // Include absent pair/legacy paths so an interrupted write that creates a
        // body after the scan cannot be mistaken for an unchanged orphan.
        let mut paths = paths;
        for suffix in ["body", "meta.json", "json"] {
            let path = self.root.join(format!("{key}.{suffix}"));
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
        paths.sort();
        PruneCandidate {
            used: self.accessed(&key, &paths),
            generation: self.metadata(&key).ok().map(|m| m.generation),
            files: Self::fingerprints(&paths),
            key,
            paths,
        }
    }
    pub(super) fn prune(&self, now: i64, budget: u64, max_age: i64) -> Result<PruneStats> {
        self.safe(&self.root)?;
        if !self.root.exists() {
            return Ok(PruneStats::default());
        }
        let Some(_maintenance) = self.lock_file("maintenance.lock", false)? else {
            return Ok(PruneStats::default());
        };
        let groups = self.groups()?;
        let mut total: u64 = groups.values().map(|p| Self::size(p)).sum();
        let mut entries: Vec<_> = groups
            .into_iter()
            .map(|(key, paths)| self.candidate(key, paths))
            .collect();
        entries.sort_by_key(|e| e.used);
        for candidate in entries {
            self.prune_candidate(&candidate, now, budget, &mut total, max_age)?;
        }
        let groups = self.groups()?;
        Ok(PruneStats {
            bytes: groups.values().map(|p| Self::size(p)).sum(),
            files: groups.values().map(Vec::len).sum(),
        })
    }
}
#[derive(PartialEq)]
struct FileFingerprint {
    len: u64,
    modified: Option<SystemTime>,
    regular: bool,
}
struct PruneCandidate {
    generation: Option<String>,
    files: Vec<Option<FileFingerprint>>,
    used: i64,
    key: String,
    paths: Vec<PathBuf>,
}
#[cfg(test)]
#[path = "disk_cache_tests.rs"]
mod tests;
