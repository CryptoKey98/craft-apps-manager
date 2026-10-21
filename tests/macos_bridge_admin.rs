#![cfg(target_os = "macos")]
use std::{fs, process::Command};

#[test]
fn unprivileged_worker_rejects_before_gui_settings_or_startup_logging() {
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let root = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("craft-mac-admin-cli-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    fs::write(root.join("settings.json"), b"preserve user settings").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_craft-apps-manager"))
        .env("HOME", &root)
        .env("TMPDIR", &root)
        .arg("--native-bridge-admin")
        .arg(root.join("missing-approval.json"))
        .arg("a".repeat(64))
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let expected = if cfg!(feature = "service-updater") {
        "Administrator"
    } else {
        "Update service is not compiled into this manager"
    };
    assert!(
        stderr.contains(expected),
        "Unexpected helper rejection: {stderr}"
    );
    assert_eq!(
        fs::read(root.join("settings.json")).unwrap(),
        b"preserve user settings"
    );
    assert!(!root.join("Library").exists());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    fs::remove_dir_all(root).unwrap();
}
