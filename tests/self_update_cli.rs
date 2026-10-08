#![cfg(target_os = "macos")]

use craft_apps_manager::{files, self_update::Plan};
use std::{fs, process::Command};

#[test]
fn rejected_helper_plan_exits_nonzero_and_reports_outside_bundle() {
    let directory = std::env::temp_dir().join(format!("craft-helper-cli-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&directory).unwrap();
    let target = directory.join("Craft Apps Manager.app");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("original"), "unchanged").unwrap();
    let plan_path = directory.join("plan.json");
    files::write_json(
        &plan_path,
        &Plan {
            layout: 0,
            msi: false,
            linux_package: false,
            parent_pid: 0,
            target: target.clone(),
            staged: directory.join("untrusted.app"),
            hash: String::new(),
            root: directory.join("library"),
            tools: directory.join("tools"),
        },
    )
    .unwrap();
    let home = directory.join("home");
    let status = Command::new(env!("CARGO_BIN_EXE_craft-apps-manager"))
        .arg("--apply-self-update")
        .arg(&plan_path)
        .env("HOME", &home)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(1));
    let data = home.join("Library/Application Support/Craft Apps Manager");
    let result: serde_json::Value =
        files::read_json(&data.join("self-update-result.json")).unwrap();
    assert_eq!(result["status"], "failed");
    assert!(result["message"]
        .as_str()
        .unwrap()
        .contains("legacy layout"));
    assert!(fs::read_to_string(data.join("logs/startup.log"))
        .unwrap()
        .contains("legacy layout"));
    assert_eq!(fs::read(target.join("original")).unwrap(), b"unchanged");
    assert!(!target.join("logs").exists());
    assert!(!directory.join("library").exists());
    fs::remove_dir_all(directory).unwrap();
}
