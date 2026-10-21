use super::catalog::VerifiedCatalog;
use anyhow::{ensure, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
const MAX_TRUST_RECORD: usize = 32 * 1024;

pub use craft_updater_service::trust::{lock_name, TrustRecord};
pub struct TrustStore {
    path: PathBuf,
}
pub struct TrustGuard {
    _lock: crate::platform::Lock,
}
impl TrustStore {
    pub fn new(root: &Path, repository: &str, channel: &str) -> Self {
        let scope = format!("{repository}\n{channel}");
        Self {
            path: root
                .join("runtime/manager-updates")
                .join(format!("trust-{:x}.json", Sha256::digest(scope.as_bytes()))),
        }
    }
    pub fn load(&self) -> Result<Option<TrustRecord>> {
        if !self.path.exists() {
            return Ok(None);
        }
        crate::files::no_links(&self.path)?;
        ensure!(
            std::fs::metadata(&self.path)?.len() <= MAX_TRUST_RECORD as u64,
            "Trust record too large"
        );
        let bytes = std::fs::read(&self.path)?;
        ensure!(bytes.len() <= MAX_TRUST_RECORD, "Trust record too large");
        let record: TrustRecord = serde_json::from_slice(&bytes)?;
        ensure!(
            (1..=9_007_199_254_740_991).contains(&record.sequence)
                && record.payload_sha256.len() == 64
                && record.payload_sha256.bytes().all(|c| c.is_ascii_hexdigit())
                && record.metadata_origins.len() <= 4,
            "Corrupt updater trust record"
        );
        for origin in &record.metadata_origins {
            super::transport::ValidatedEndpoint::new(origin)?;
        }
        Ok(Some(record))
    }
    pub fn accept(&self, catalog: &VerifiedCatalog) -> Result<()> {
        let _lock = self.guard()?;
        let record = TrustRecord::from_catalog(catalog);
        if let Some(prior) = self.load()? {
            ensure!(
                record.repository == prior.repository
                    && record.channel == prior.channel
                    && (record.sequence > prior.sequence
                        || record.sequence == prior.sequence
                            && record.payload_sha256 == prior.payload_sha256),
                "Catalog trust changed during check"
            );
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
            crate::files::no_links(parent)?;
        }
        let bytes = serde_json::to_vec(&record)?;
        ensure!(bytes.len() <= MAX_TRUST_RECORD, "Trust record too large");
        crate::files::write_bytes(&self.path, &bytes)
    }
    pub fn guard(&self) -> Result<TrustGuard> {
        let parent = self.path.parent().expect("trust file parent");
        std::fs::create_dir_all(parent)?;
        crate::files::no_links(parent)?;
        let path = parent
            .canonicalize()?
            .join(self.path.file_name().expect("trust filename"));
        #[cfg(target_os = "windows")]
        let name = path.to_string_lossy().to_lowercase();
        #[cfg(not(target_os = "windows"))]
        let name = path.to_string_lossy().into_owned();
        Ok(TrustGuard {
            _lock: crate::platform::Lock::wait(&lock_name("Trust", name.as_bytes()))?,
        })
    }
}
