#![cfg(feature = "service-updater")]
use craft_apps_manager::manager_updates::{coordinator::message, model::CheckOutcome};
#[test]
fn unsupported_is_not_current() {
    let text = message(&CheckOutcome::UnsupportedPlatform("No package".into()));
    assert!(!text.contains("latest"));
    assert!(text.contains("No package"));
    assert!(message(&CheckOutcome::CheckFailed("offline".into())).contains("offline"));
}
#[test]
fn bridge_restart_evidence_does_not_authorize_installation() {
    let root = std::env::temp_dir().join(format!("craft-pending-{}", uuid::Uuid::new_v4()));
    let paths = craft_apps_manager::model::Paths::new(root.clone(), None);
    let version = env!("CARGO_PKG_VERSION");
    let revision = craft_apps_manager::update_revisions::BUILD_REVISION;
    let record = serde_json::json!({"schema":1,"immediate":{"version":version,"revision":revision},"final_target":{"version":"99.0.0","revision":1},"final_package_id":"future","originating_sequence":1});
    craft_apps_manager::files::write_json(
        &root.join("runtime/manager-updates/pending-bridge.json"),
        &record,
    )
    .unwrap();
    assert!(craft_apps_manager::manager_updates::coordinator::needs_bridge_check(&paths));
    // Only a check is scheduled. The record contains neither executable instructions nor approval.
    std::fs::remove_dir_all(root).unwrap();
}
