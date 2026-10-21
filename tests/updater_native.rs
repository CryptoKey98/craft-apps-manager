#![cfg(feature = "service-updater")]
use craft_apps_manager::manager_updates::handlers::native::validate_identity;
#[test]
fn msi_identity_and_native_version_match() {
    validate_identity("Craft Apps Manager (x64)", "0.9.0", "x64", "0.9.0", "x64").unwrap();
    for (name, version, arch) in [
        ("Unrelated app", "0.9.0", "x64"),
        ("Craft Apps Manager (x64)", "0.8.0", "x64"),
        ("Craft Apps Manager (x64)", "0.9.0", "x86"),
    ] {
        assert!(validate_identity(name, version, arch, "0.9.0", "x64").is_err());
    }
}
#[test]
fn package_success_with_wrong_executable_revision_is_not_success() {
    use craft_apps_manager::manager_updates::transaction::{
        validate_build_identity, ExpectedIdentity,
    };
    let expected = ExpectedIdentity {
        version: "1.2.0".into(),
        revision: 2,
        platform: "linux".into(),
        architecture: "x64".into(),
    };
    for (version, revision, target) in [
        ("1.2.0", 1, "x86_64-unknown-linux-gnu"),
        ("1.1.0", 2, "x86_64-unknown-linux-gnu"),
        ("1.2.0", 2, "i686-unknown-linux-gnu"),
    ] {
        let bytes = serde_json::to_vec(
            &serde_json::json!({"version":version,"revision":revision,"target":target}),
        )
        .unwrap();
        assert!(validate_build_identity(&bytes, &expected).is_err());
    }
    validate_build_identity(
        br#"{"version":"1.2.0","revision":2,"target":"x86_64-unknown-linux-gnu"}"#,
        &expected,
    )
    .unwrap();
}
