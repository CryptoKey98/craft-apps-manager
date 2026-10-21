#![cfg(feature = "service-updater")]
use craft_apps_manager::manager_updates::model::Catalog;
use craft_apps_manager::manager_updates::{
    catalog::{verify_catalog, TrustContext},
    trust::TrustRecord,
};
use ed25519_dalek::{Signer, SigningKey};

fn signed(catalog: &Catalog) -> (Vec<u8>, TrustContext) {
    let key = SigningKey::from_bytes(&[7; 32]);
    let payload = serde_json::to_string(catalog).unwrap();
    let signature: String = key
        .sign(payload.as_bytes())
        .to_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    (
        serde_json::to_vec(&serde_json::json!({"payload":payload,"signature":signature})).unwrap(),
        TrustContext {
            repository: "owner/manager".into(),
            channel: "stable".into(),
            keys: vec![key.verifying_key().to_bytes()],
        },
    )
}

#[test]
fn signature_precedes_action_parsing() {
    let (bytes, context) = signed(&fixture());
    let mut envelope: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    envelope["payload"] = "not JSON".into();
    let error = verify_catalog(
        &serde_json::to_vec(&envelope).unwrap(),
        &context,
        1001,
        None,
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("signature"));
}

#[test]
fn wrong_repository_rejected() {
    let (bytes, mut context) = signed(&fixture());
    context.repository = "other/repo".into();
    assert!(verify_catalog(&bytes, &context, 1001, None).is_err());
}

#[test]
fn sequence_rollback_and_equivocation_rejected() {
    let (bytes, context) = signed(&fixture());
    let verified = verify_catalog(&bytes, &context, 1001, None).unwrap();
    let mut record = TrustRecord::from_catalog(&verified);
    assert!(verify_catalog(&bytes, &context, 1001, Some(&record)).is_ok());
    record.sequence = 2;
    assert!(verify_catalog(&bytes, &context, 1001, Some(&record)).is_err());
    record.sequence = 1;
    record.payload_sha256 = "00".repeat(32);
    assert!(verify_catalog(&bytes, &context, 1001, Some(&record)).is_err());
}

#[test]
fn expired_and_future_issued_rejected() {
    let (bytes, context) = signed(&fixture());
    assert!(verify_catalog(&bytes, &context, 2_593_000, None).is_err());
    assert!(verify_catalog(&bytes, &context, 699, None).is_err());
    assert!(verify_catalog(&vec![0; 2 * 1024 * 1024 + 1], &context, 1001, None).is_err());
}
#[test]
fn interrupted_trust_write_preserves_valid_record() {
    use craft_apps_manager::manager_updates::trust::TrustStore;
    let root = std::env::temp_dir().join(format!("craft-trust-{}", uuid::Uuid::new_v4()));
    let (bytes, context) = signed(&fixture());
    let verified = verify_catalog(&bytes, &context, 1001, None).unwrap();
    let store = TrustStore::new(&root, "owner/manager", "stable");
    store.accept(&verified).unwrap();
    std::fs::write(
        root.join("runtime/manager-updates/interrupted.tmp"),
        b"partial",
    )
    .unwrap();
    assert_eq!(store.load().unwrap().unwrap().sequence, 1);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn escaped_asset_filename_remains_valid() {
    let mut c = fixture();
    c.packages[0].asset = "Bridge App.zip".into();
    c.packages[0].url =
        "https://github.com/owner/manager/releases/download/v1.1.0/Bridge%20App.zip".into();
    c.validate().unwrap();
}
#[test]
fn signed_origin_migration_is_bounded() {
    let mut c = fixture();
    c.metadata_origins = vec!["https://new-updates.example/v2/catalog.json".into()];
    c.validate().unwrap();
    c.metadata_origins
        .push("https://user:secret@example.com/catalog".into());
    assert!(c.validate().is_err());
}
#[test]
fn maximum_origin_trust_record_can_be_reloaded() {
    use craft_apps_manager::manager_updates::trust::TrustStore;
    let root = std::env::temp_dir().join(format!("craft-trust-limit-{}", uuid::Uuid::new_v4()));
    let mut c = fixture();
    c.metadata_origins = (0..4)
        .map(|i| format!("https://updates{i}.example/{}", "a".repeat(2000)))
        .collect();
    let (bytes, context) = signed(&c);
    let verified = verify_catalog(&bytes, &context, 1001, None).unwrap();
    let store = TrustStore::new(&root, "owner/manager", "stable");
    store.accept(&verified).unwrap();
    let loaded = store.load();
    std::fs::remove_dir_all(root).unwrap();
    assert_eq!(loaded.unwrap().unwrap().metadata_origins.len(), 4);
}

fn fixture() -> Catalog {
    serde_json::from_str(include_str!("fixtures/updater-v2/catalog.json")).unwrap()
}

#[test]
fn model_rejects_duplicate_ids() {
    let mut catalog = fixture();
    catalog.packages.push(catalog.packages[0].clone());
    assert!(catalog.validate().is_err());
}

#[test]
fn model_bounds_collections() {
    for (releases, packages, routes) in [(129, 0, 0), (0, 513, 0), (0, 0, 513)] {
        let mut catalog = fixture();
        if releases > 0 {
            catalog
                .releases
                .resize(releases, catalog.releases[0].clone());
        }
        if packages > 0 {
            catalog
                .packages
                .resize(packages, catalog.packages[0].clone());
        }
        if routes > 0 {
            catalog.routes.resize(routes, catalog.routes[0].clone());
        }
        assert!(catalog.validate().is_err());
    }
    let mut catalog = fixture();
    catalog.sequence = 9_007_199_254_740_992;
    assert!(catalog.validate().is_err());
    catalog.sequence = 1;
    catalog.schema = 3;
    assert!(catalog.validate().is_err());
}

#[test]
fn model_accepts_unknown_optional_fields() {
    let mut json: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/updater-v2/catalog.json")).unwrap();
    json["future_optional"] = true.into();
    let catalog: Catalog = serde_json::from_value(json).unwrap();
    catalog.validate().unwrap();
}
