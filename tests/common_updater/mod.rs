#![allow(dead_code)]
use craft_apps_manager::manager_updates::{
    catalog::{verify_catalog, TrustContext, VerifiedCatalog},
    model::*,
};
use ed25519_dalek::{Signer, SigningKey};
pub fn catalog() -> Catalog {
    serde_json::from_str(include_str!("../fixtures/updater-v2/catalog.json")).unwrap()
}
pub fn context() -> TrustContext {
    TrustContext {
        repository: "owner/manager".into(),
        channel: "stable".into(),
        keys: vec![SigningKey::from_bytes(&[7; 32]).verifying_key().to_bytes()],
    }
}
pub fn signed(c: &Catalog) -> Vec<u8> {
    let payload = serde_json::to_string(c).unwrap();
    let signature: String = SigningKey::from_bytes(&[7; 32])
        .sign(payload.as_bytes())
        .to_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    serde_json::to_vec(&serde_json::json!({"payload":payload,"signature":signature})).unwrap()
}
pub fn verified(c: &Catalog) -> VerifiedCatalog {
    verify_catalog(&signed(c), &context(), 1001, None).unwrap()
}
pub fn profile(tar: bool) -> ClientProfile {
    let mut capabilities = [ZIP.into(), LAYOUT.into()]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    if tar {
        capabilities.insert(TAR.into());
    }
    ClientProfile {
        repository: "owner/manager".into(),
        channel: "stable".into(),
        version: "1.0.0".into(),
        revision: 1,
        platform: "linux".into(),
        architecture: "x64".into(),
        installation: InstallationKind::Portable,
        capabilities,
    }
}
pub struct Temp(pub std::path::PathBuf);
impl Temp {
    pub fn new() -> Self {
        let p = std::env::temp_dir().join(format!("craft-v2-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
