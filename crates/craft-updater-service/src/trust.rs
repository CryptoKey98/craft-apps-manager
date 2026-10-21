use super::catalog::VerifiedCatalog;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
pub fn lock_name(scope: &str, value: impl AsRef<[u8]>) -> String {
    format!(
        "Local\\CraftUpdate{scope}{:x}",
        Sha256::digest(value.as_ref())
    )
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustRecord {
    pub repository: String,
    pub channel: String,
    pub sequence: u64,
    pub payload_sha256: String,
    #[serde(default)]
    pub metadata_origins: Vec<String>,
}
impl TrustRecord {
    pub fn from_catalog(catalog: &VerifiedCatalog) -> Self {
        let c = catalog.catalog();
        Self {
            repository: c.repository.clone(),
            channel: c.channel.clone(),
            sequence: c.sequence,
            payload_sha256: catalog.payload_hash().into(),
            metadata_origins: c.metadata_origins.clone(),
        }
    }
}
