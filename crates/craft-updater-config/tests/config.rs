use craft_updater_config::{resolve, ConfigOverrides, View};
use std::{collections::BTreeMap, fs, path::PathBuf};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "updater-config-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(p.join("assets")).unwrap();
        fs::write(p.join("assets/update-public-key.hex"), "07".repeat(32)).unwrap();
        Self(p.canonicalize().unwrap())
    }
    fn set(&self, s: &str) {
        fs::write(self.0.join("updater.toml"), s).unwrap();
    }
    fn get(&self) -> anyhow::Result<craft_updater_config::ResolvedConfig> {
        resolve(
            &self.0,
            None,
            &BTreeMap::new(),
            "linux",
            &ConfigOverrides::default(),
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn source_defaults_and_views() {
    let f = Fixture::new();
    let c = f.get().unwrap();
    assert_eq!(c.config.source, "github");
    assert_eq!(c.revision, 1);
    assert!(c.required_features.is_empty());
    for view in [View::Build, View::Packaging, View::Publisher, View::Worker] {
        assert_eq!(
            c.view(view).unwrap()["repository"],
            "CryptoKey98/craft-apps-manager"
        );
    }
}
#[test]
fn service_opt_in_and_target_revision() {
    let f = Fixture::new();
    f.set("source='service'\n[service]\ncatalog_url='https://updates.example/v2/catalog.json'\n[revisions]\nlinux=3\nwindows=2");
    let c = f.get().unwrap();
    assert_eq!(c.revision, 3);
    assert_eq!(c.required_features, vec!["service-updater"]);
}
#[test]
fn environment_then_cli_precedence_and_explicit_empty() {
    let f = Fixture::new();
    f.set("repository='file/repo'\n[service]\nmirror_url='https://updates.example/mirror'");
    let env = BTreeMap::from([
        ("CRAFT_MANAGER_REPOSITORY".into(), "env/repo".into()),
        ("CRAFT_UPDATE_CATALOG_MIRROR".into(), String::new()),
    ]);
    let overrides = ConfigOverrides {
        repository: Some("cli/repo".into()),
        ..Default::default()
    };
    let c = resolve(&f.0, None, &env, "linux", &overrides).unwrap();
    assert_eq!(c.config.repository, "cli/repo");
    assert!(c.config.service.mirror_url.is_empty());
    assert_eq!(c.provenance["repository"], "cli");
    let env = BTreeMap::from([("CRAFT_MANAGER_REPOSITORY".into(), String::new())]);
    assert!(resolve(&f.0, None, &env, "linux", &Default::default()).is_err());
}
#[test]
fn rejects_invalid_config() {
    let f = Fixture::new();
    for bad in [
        "schema_version=2",
        "secret='x'",
        "source='SERVICE'",
        "repository='../repo'",
        "channel='beta'",
        "[service]\ncatalog_url='http://example.com'",
        "[service]\ncatalog_url='https://user@example.com'",
        "[service]\ncatalog_url='https://example.com/?q=x'",
        "[service]\ncatalog_url='https://example.com/#x'",
        "[revisions]\nlinux=0",
        "[revisions]\nlinux=65536",
        "[network]\nconnect_timeout_seconds=0",
        "[network]\nmetadata_timeout_seconds=121",
        "[network]\ndownload_timeout_seconds=1801",
        "[packages]\nlinux=['msi']",
        "[packages]\nlinux=['zip','zip']",
        "[worker]\nrate_limit_period_seconds=30",
        "[worker]\nrate_limit_requests=1001",
        "[worker]\nrate_limit_namespace='abc'",
        "[publishing]\nasset_name_pattern='{stem}{suffix}'",
        "[publishing]\nrenew_before_days=30",
        "public_key_file='../key'",
        "public_key_file='assets/missing'",
        "source='service'",
        "[publishing]\nenabled=true",
    ] {
        f.set(bad);
        assert!(f.get().is_err(), "accepted {bad}");
    }
}
#[test]
fn explicit_absent_config_and_key_validation() {
    let f = Fixture::new();
    assert!(resolve(
        &f.0,
        Some(std::path::Path::new("missing.toml")),
        &BTreeMap::new(),
        "linux",
        &Default::default()
    )
    .is_err());
    fs::write(f.0.join("assets/update-public-key.hex"), "abcd").unwrap();
    assert!(f.get().is_err());
}
#[test]
fn config_relative_paths() {
    let f = Fixture::new();
    fs::create_dir(f.0.join("config")).unwrap();
    fs::write(f.0.join("config/key.hex"), "08".repeat(32)).unwrap();
    fs::write(f.0.join("config/custom.toml"), "public_key_file='key.hex'").unwrap();
    let c = resolve(
        &f.0,
        Some(std::path::Path::new("config/custom.toml")),
        &BTreeMap::new(),
        "macos",
        &Default::default(),
    )
    .unwrap();
    assert_eq!(c.public_key_hex, "08".repeat(32));
}

#[test]
fn compiled_feature_is_a_separate_explicit_choice() {
    let f = Fixture::new();
    let c = f.get().unwrap();
    c.require_build_support(false).unwrap();
    c.require_build_support(true).unwrap();
    f.set("source='service'\n[service]\ncatalog_url='https://updates.example/v2/catalog.json'");
    let c = f.get().unwrap();
    assert!(c
        .require_build_support(false)
        .unwrap_err()
        .to_string()
        .contains("--features service-updater"));
    c.require_build_support(true).unwrap();
}

#[test]
fn rejected_secret_values_are_never_echoed_in_errors() {
    let f = Fixture::new();
    for input in [
        "private_key='sensitive-test-marker'",
        "[worker]\ndynamic_enabled='sensitive-test-marker'",
        "source='sensitive-test-marker",
    ] {
        f.set(input);
        let error = f.get().err().unwrap();
        assert!(!format!("{error:#}").contains("sensitive-test-marker"));
    }
}

#[cfg(unix)]
#[test]
fn linked_public_key_is_rejected() {
    let f = Fixture::new();
    std::os::unix::fs::symlink(
        f.0.join("assets/update-public-key.hex"),
        f.0.join("linked.hex"),
    )
    .unwrap();
    f.set("public_key_file='linked.hex'");
    assert!(f.get().is_err());
}

#[test]
fn runtime_fingerprint_tracks_only_embedded_configuration() {
    let f = Fixture::new();
    let before = f.get().unwrap().view(View::Build).unwrap();
    assert_eq!(
        before["runtime_config_fingerprint"].as_str().unwrap().len(),
        64
    );
    f.set("[service]\ncatalog_url='https://new.example/catalog'\n");
    let endpoint = f.get().unwrap().view(View::Build).unwrap();
    assert_ne!(
        before["runtime_config_fingerprint"],
        endpoint["runtime_config_fingerprint"]
    );
    f.set("[network]\nmetadata_timeout_seconds=45\n");
    let timeout = f.get().unwrap().view(View::Build).unwrap();
    assert_ne!(
        before["runtime_config_fingerprint"],
        timeout["runtime_config_fingerprint"]
    );
    f.set("[worker]\nrate_limit_requests=100\n");
    let publishing = f.get().unwrap().view(View::Build).unwrap();
    assert_eq!(
        before["runtime_config_fingerprint"],
        publishing["runtime_config_fingerprint"]
    );
}
