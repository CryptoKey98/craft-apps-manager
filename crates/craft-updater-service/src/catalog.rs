use super::{model::*, trust::TrustRecord};
use anyhow::{ensure, Context, Result};
use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct TrustContext {
    pub repository: String,
    pub channel: String,
    pub keys: Vec<[u8; 32]>,
}
#[derive(Debug, Clone)]
pub struct VerifiedCatalog {
    catalog: Catalog,
    hash: String,
    envelope: Vec<u8>,
}
impl VerifiedCatalog {
    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }
    pub fn payload_hash(&self) -> &str {
        &self.hash
    }
    pub fn envelope(&self) -> &[u8] {
        &self.envelope
    }
}
pub fn verify_catalog(
    bytes: &[u8],
    context: &TrustContext,
    now: i64,
    prior: Option<&TrustRecord>,
) -> Result<VerifiedCatalog> {
    ensure!(bytes.len() <= MAX_ENVELOPE, "Catalog envelope too large");
    let envelope: Envelope = serde_json::from_slice(bytes)?;
    ensure!(
        envelope.payload.len() <= MAX_PAYLOAD,
        "Catalog payload too large"
    );
    let signature = Signature::from_slice(&decode_hex(&envelope.signature, 64)?)?;
    ensure!(
        context
            .keys
            .iter()
            .any(|key| VerifyingKey::from_bytes(key).is_ok_and(|key| key
                .verify_strict(envelope.payload.as_bytes(), &signature)
                .is_ok())),
        "Catalog signature did not verify"
    );
    let catalog: Catalog =
        serde_json::from_str(&envelope.payload).context("Invalid signed catalog")?;
    catalog.validate()?;
    ensure!(
        catalog.repository == context.repository && catalog.channel == context.channel,
        "Catalog belongs to another repository/channel"
    );
    ensure!(
        catalog.issued <= now.saturating_add(300)
            && catalog.expires > now
            && catalog.expires > catalog.issued
            && catalog.expires.saturating_sub(catalog.issued) <= VALIDITY,
        "Catalog expired or clock is incorrect"
    );
    let hash = format!("{:x}", Sha256::digest(envelope.payload.as_bytes()));
    if let Some(record) = prior {
        ensure!(
            record.repository == context.repository && record.channel == context.channel,
            "Trust state scope mismatch"
        );
        ensure!(
            catalog.sequence >= record.sequence
                && (catalog.sequence != record.sequence || hash == record.payload_sha256),
            "Catalog replay or sequence equivocation"
        );
    }
    Ok(VerifiedCatalog {
        catalog,
        hash,
        envelope: bytes.to_vec(),
    })
}
pub fn decode_hex(text: &str, size: usize) -> Result<Vec<u8>> {
    ensure!(
        text.len() == size * 2 && text.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid signature/hash encoding"
    );
    (0..size)
        .map(|i| Ok(u8::from_str_radix(&text[i * 2..i * 2 + 2], 16)?))
        .collect()
}

#[derive(serde::Deserialize)]
struct Envelope {
    payload: String,
    signature: String,
}
