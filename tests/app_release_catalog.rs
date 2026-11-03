#![cfg(feature = "app-catalog-tools")]
use craft_apps_manager::{
    files,
    installations::release_catalog::{verification, Source},
};
use std::{collections::BTreeMap, path::PathBuf, process::Command};
struct Root(PathBuf);
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn tool(args: &[&std::ffi::OsStr]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_app-release-catalog-tool"))
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn local_publisher_cli_aggregate_sign_verify_and_tamper_rejection() {
    use std::ffi::OsStr;
    let root =
        Root(std::env::temp_dir().join(format!("craft catalog cli {}", uuid::Uuid::new_v4())));
    std::fs::create_dir_all(&root.0).unwrap();
    let discovery = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/app-release-catalog/discovery.json");
    let bundle = root.0.join("bundle.json");
    let output = root.0.join("published");
    let result = tool(&[
        OsStr::new("aggregate"),
        OsStr::new("--discovery"),
        discovery.as_os_str(),
        OsStr::new("--out"),
        bundle.as_os_str(),
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let key = ed25519_dalek::SigningKey::from_bytes(&[87; 32]); // test-only key, never trusted by official source
    let key_path = root.0.join("fixture-private.hex");
    files::write_bytes(&key_path, verification::hex(&key.to_bytes()).as_bytes()).unwrap();
    let source = Source {
        schema: 1,
        enabled: true,
        index_url: None,
        keys: BTreeMap::from([(
            "fixture".into(),
            verification::hex(key.verifying_key().as_bytes()),
        )]),
    };
    let source_file = root.0.join("source.json");
    files::write_json(&source_file, &source).unwrap();
    let result = tool(&[
        OsStr::new("sign"),
        OsStr::new("--input"),
        bundle.as_os_str(),
        OsStr::new("--key-file"),
        key_path.as_os_str(),
        OsStr::new("--key-id"),
        OsStr::new("fixture"),
        OsStr::new("--out"),
        output.as_os_str(),
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let index = output.join("index.json");
    let objects = output.join("apps");
    let verify = || {
        tool(&[
            OsStr::new("verify"),
            OsStr::new("--source"),
            source_file.as_os_str(),
            OsStr::new("--index"),
            index.as_os_str(),
            OsStr::new("--objects"),
            objects.as_os_str(),
        ])
    };
    let result = verify();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!String::from_utf8_lossy(&result.stdout).contains(&verification::hex(&key.to_bytes())));
    let object = std::fs::read_dir(&objects)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    std::fs::write(object, b"tampered").unwrap();
    assert!(!verify().status.success());
}

#[test]
fn pending_count_is_metadata_only_and_can_resume_a_completed_attempt() {
    use std::ffi::OsStr;
    let root =
        Root(std::env::temp_dir().join(format!("craft catalog pending {}", uuid::Uuid::new_v4())));
    std::fs::create_dir_all(&root.0).unwrap();
    let discovery = root.0.join("discovery.json");
    let asset = serde_json::json!({"name":"fixture-windows-x64.msi","size":42,"digest":format!("sha256:{}","a".repeat(64)),"browser_download_url":"https://github.com/fixture/app/releases/download/v1.0.0/fixture-windows-x64.msi"});
    files::write_json(&discovery,&serde_json::json!({"schema":1,"repositories":{"fixture/app":{"checked_at":0,"latest_tag":"v1.0.0","releases":[{"tag_name":"v1.0.0","name":null,"draft":false,"prerelease":false,"assets":[asset]}],"error":null}},"pages":{}})).unwrap();
    let args = [
        OsStr::new("pending-count"),
        OsStr::new("--discovery"),
        discovery.as_os_str(),
    ];
    let first = tool(&args);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&first.stdout).trim(), "1");
    let checkpoint = root.0.join("inspection.json");
    let identity = format!(
        "fixture/app:v1.0.0:fixture-windows-x64.msi:sha256:{}",
        "a".repeat(64)
    );
    files::write_json(
        &checkpoint,
        &serde_json::json!({"schema":1,"observations":[],"attempts":{identity:200},"errors":{}}),
    )
    .unwrap();
    for (since, expected) in [("200", "0"), ("201", "1")] {
        let mut args = args.to_vec();
        args.extend([
            OsStr::new("--checkpoint"),
            checkpoint.as_os_str(),
            OsStr::new("--since"),
            OsStr::new(since),
        ]);
        let result = tool(&args);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), expected);
    }
}
