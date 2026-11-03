use super::*;
use std::cell::Cell;

struct Fixture {
    root: PathBuf,
    authorization: PathBuf,
    source: PathBuf,
    helper: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("craft-launch-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let target = root.join(if cfg!(target_os = "macos") {
            "Manager.app"
        } else {
            "manager"
        });
        let source = self_update::staged_executable(&target);
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, b"source manager fixture; must never execute").unwrap();
        let helper = root.join(self_update::helper_name());
        fs::copy(&source, &helper).unwrap();
        let authorization = root.join("authorization.json");
        let plan = TransactionPlanV2 {
            schema: 2,
            source_hash: files::hash(&source).unwrap(),
            profile: profile_for(
                crate::model::release_os(),
                crate::model::MANAGER_ARCH,
                if cfg!(target_os = "macos") {
                    InstallationKind::Bundle
                } else {
                    InstallationKind::Portable
                },
            ),
            offer: UpdateOffer {
                immediate_package_id: "fixture".into(),
                final_package_id: "fixture".into(),
                route_ids: Vec::new(),
            },
            // These tests exercise only the launch boundary. The helper's apply
            // path independently verifies the real signed catalog and package.
            envelope: Vec::new(),
            legacy: self_update::Plan {
                expected_identity: None,
                layout: 0,
                msi: false,
                linux_package: false,
                parent_pid: std::process::id(),
                target,
                staged: root.join("incoming"),
                hash: "a".repeat(64),
                root: root.clone(),
                tools: root.join("tools"),
            },
        };
        files::write_json(&authorization, &plan).unwrap();
        Self {
            root,
            authorization,
            source,
            helper,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn handoff_guard_can_cancel_after_hash_checks_without_starting_helper() {
    let fixture = Fixture::new();
    let job = Job::new(fixture.root.join("logs/update.log"), &Default::default());
    let checked = Cell::new(false);
    let result = self_update::launch_with_guard(&fixture.authorization, || {
        checked.set(true);
        job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        job.check()
    });
    assert!(checked.get());
    assert!(crate::jobs::is_cancelled(&result.unwrap_err()));
}

#[test]
fn changed_source_still_fails_before_handoff_guard() {
    let fixture = Fixture::new();
    fs::write(&fixture.source, b"changed installed manager").unwrap();
    let result = self_update::launch_with_guard(&fixture.authorization, || {
        panic!("Guard must not run before installed manager verification")
    });
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Installed manager changed after preparation"));
}

#[test]
fn changed_helper_still_fails_before_handoff_guard() {
    let fixture = Fixture::new();
    fs::write(&fixture.helper, b"changed helper").unwrap();
    let result = self_update::launch_with_guard(&fixture.authorization, || {
        panic!("Guard must not run before helper verification")
    });
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Update helper mismatch"));
}

#[test]
fn missing_helper_still_fails_before_handoff_guard() {
    let fixture = Fixture::new();
    fs::remove_file(&fixture.helper).unwrap();
    assert!(self_update::launch_with_guard(&fixture.authorization, || {
        panic!("Guard must not run with missing helper")
    })
    .is_err());
}

#[test]
fn unsupported_authorization_still_fails_before_handoff_guard() {
    let fixture = Fixture::new();
    let mut plan = read_plan(&fixture.authorization).unwrap();
    plan.schema = 3;
    files::write_json(&fixture.authorization, &plan).unwrap();
    let result = self_update::launch_with_guard(&fixture.authorization, || {
        panic!("Guard must not accept unsupported authorization")
    });
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Unsupported update plan schema"));
}
