use super::*;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("craft-legacy-launch-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
#[cfg(not(target_os = "macos"))]
fn legacy_handoff_guard_stops_before_executing_helper() {
    let fixture = Fixture::new();
    let path = fixture.0.join("plan.json");
    files::write_json(
        &path,
        &Plan {
            expected_identity: None,
            layout: 0,
            msi: false,
            linux_package: false,
            parent_pid: std::process::id(),
            target: fixture.0.join("manager"),
            staged: fixture.0.join("incoming"),
            hash: "a".repeat(64),
            root: fixture.0.clone(),
            tools: fixture.0.join("tools"),
        },
    )
    .unwrap();
    let job = Job::new(fixture.0.join("logs/update.log"), &Default::default());
    job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    let error = launch_with_guard(&path, || job.check()).unwrap_err();
    assert!(crate::jobs::is_cancelled(&error));
}

#[test]
#[cfg(not(feature = "service-updater"))]
fn github_only_handoff_still_rejects_service_authorization() {
    let fixture = Fixture::new();
    for name in ["authorization.json", "plan.json"] {
        let path = fixture.0.join(name);
        files::write_json(&path, &serde_json::json!({"schema": 2, "envelope": []})).unwrap();
        let error = launch_with_guard(&path, || {
            panic!("Service authorization must fail closed before guard")
        })
        .unwrap_err();
        assert!(error.to_string().contains(if name == "authorization.json" {
            "Update service is not compiled"
        } else {
            "requires a service-enabled manager"
        }));
    }
}

#[test]
#[cfg(target_os = "macos")]
fn invalid_legacy_bundle_still_fails_before_handoff_guard() {
    let fixture = Fixture::new();
    let path = fixture.0.join("plan.json");
    files::write_json(
        &path,
        &Plan {
            expected_identity: None,
            layout: 0,
            msi: false,
            linux_package: false,
            parent_pid: std::process::id(),
            target: fixture.0.join("Manager.app"),
            staged: fixture.0.join("incoming.app"),
            hash: "a".repeat(64),
            root: fixture.0.clone(),
            tools: fixture.0.join("tools"),
        },
    )
    .unwrap();
    let error = launch_with_guard(&path, || panic!("Invalid bundle reached guard")).unwrap_err();
    assert!(error.to_string().contains("unsupported legacy layout"));
}
