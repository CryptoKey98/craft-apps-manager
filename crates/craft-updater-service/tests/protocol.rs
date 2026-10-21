use craft_updater_service::{
    catalog::{verify_catalog, TrustContext},
    model::*,
    routing::select_update,
    trust::TrustRecord,
};
use ed25519_dalek::{Signer, SigningKey};
fn fixture() -> Catalog {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/updater-v2/catalog.json"
    ))
    .unwrap()
}
fn signed(c: &Catalog) -> (Vec<u8>, TrustContext) {
    let key = SigningKey::from_bytes(&[7; 32]);
    let payload = serde_json::to_string(c).unwrap();
    let signature = key
        .sign(payload.as_bytes())
        .to_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
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
fn signed_catalog_and_replay_rules() {
    let (bytes, ctx) = signed(&fixture());
    let v = verify_catalog(&bytes, &ctx, 1001, None).unwrap();
    let mut prior = TrustRecord::from_catalog(&v);
    assert!(verify_catalog(&bytes, &ctx, 1001, Some(&prior)).is_ok());
    prior.sequence += 1;
    assert!(verify_catalog(&bytes, &ctx, 1001, Some(&prior)).is_err());
    prior.sequence -= 1;
    prior.payload_sha256 = "00".repeat(32);
    assert!(verify_catalog(&bytes, &ctx, 1001, Some(&prior)).is_err());
}
#[test]
fn bad_signature_scope_and_expiry_fail() {
    let (mut bytes, mut ctx) = signed(&fixture());
    assert!(verify_catalog(&bytes, &ctx, 3_000_000, None).is_err());
    ctx.repository = "other/repo".into();
    assert!(verify_catalog(&bytes, &ctx, 1001, None).is_err());
    let last = bytes.len() - 3;
    bytes[last] ^= 1;
    assert!(verify_catalog(&bytes, &ctx, 1001, None).is_err());
}
#[test]
fn strict_relative_and_version_parity() {
    for bad in [
        "../a", "/a", "C:/a", r"a\b", "a//b", "a/./b", "CON.exe", "a.",
    ] {
        assert!(relative(bad).is_err(), "{bad}");
    }
    for valid in [
        "a/b",
        "Craft Apps Manager.app/Contents/MacOS/manager",
        "a-b.zip",
    ] {
        relative(valid).unwrap();
    }
    for bad in ["1", "1.2.3-beta", "1.2.3.4.5", "1..2", "9007199254740992.0"] {
        assert!(Position {
            version: bad.into(),
            revision: 1
        }
        .key()
        .is_err());
    }
    assert_eq!(
        Position {
            version: "v1.2.3".into(),
            revision: 2
        }
        .key()
        .unwrap(),
        ([1, 2, 3, 0], 2)
    );
}
#[test]
fn bridge_routes_still_select_verified_compatible_packages() {
    let (bytes, ctx) = signed(&fixture());
    let v = verify_catalog(&bytes, &ctx, 1001, None).unwrap();
    let profile = ClientProfile {
        repository: "owner/manager".into(),
        channel: "stable".into(),
        version: "1.0.0".into(),
        revision: 1,
        platform: "linux".into(),
        architecture: "x64".into(),
        installation: InstallationKind::Portable,
        capabilities: [ZIP.into(), LAYOUT.into()].into_iter().collect(),
    };
    assert!(matches!(
        select_update(&v, &profile).unwrap(),
        CheckOutcome::BridgeUpdate(_)
    ));
}
