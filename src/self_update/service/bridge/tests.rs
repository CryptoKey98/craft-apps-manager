use super::*;
use crate::{files, self_update::Plan};
use ed25519_dalek::{Signer, SigningKey};
use std::{fs, path::PathBuf};
#[test]
fn changed_remaining_route_cannot_reuse_approved_journey() {
    let f = Fixture::new();
    let mut value: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/updater-v2/catalog.json"
    )))
    .unwrap();
    value["sequence"] = 2.into();
    value["routes"][1]["id"] = "replacement-route".into();
    let payload = serde_json::to_string(&value).unwrap();
    let signature = SigningKey::from_bytes(&[7; 32])
        .sign(payload.as_bytes())
        .to_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let envelope =
        serde_json::to_vec(&serde_json::json!({"payload":payload,"signature":signature})).unwrap();
    let catalog = verify_catalog(&envelope, &f.trust, 1003, None).unwrap();
    let mut source = f.journey.source.clone();
    source.version = "1.1.0".into();
    source.capabilities.insert(TAR.into());
    let CheckOutcome::DirectUpdate(offer) =
        super::super::routing::select_update(&catalog, &source).unwrap()
    else {
        panic!("Expected direct final hop")
    };
    assert!(
        f.journey
            .next(&SignedSelection { catalog, offer }, &source, &f.trust, 1003)
            .is_err(),
        "A different signed route requires new consent even when its payload is unchanged"
    );
}
#[test]
fn persisted_builder_and_work_started_during_download_pause_continuation() {
    let f = Fixture::new();
    let manager = crate::operations::Manager::new(&f.root);
    assert!(ensure_idle(&f.root, None).is_ok());
    let own = manager.register("Manager update download", "Manager", None);
    let own_id = own.id();
    assert!(ensure_idle(&f.root, Some(&own_id)).is_ok());
    let builder = manager.register("Build", "filmcraft", None);
    assert!(ensure_idle(&f.root, Some(&own_id)).is_err());
    assert!(foreground_busy(&manager.list(), Some(&own_id)));
    builder.finish(crate::operations::Lifecycle::Completed, "done".into());
    assert!(ensure_idle(&f.root, Some(&own_id)).is_ok());
    let late = manager.register("Install", "designcraft", None);
    assert!(ensure_idle(&f.root, Some(&own_id)).is_err());
    late.finish(crate::operations::Lifecycle::Cancelled, "cancelled".into());
    assert!(ensure_idle(&f.root, Some(&own_id)).is_ok());
}
#[test]
fn final_commit_gate_rejects_work_started_after_preparation_and_excludes_new_work() {
    for name in ["Local\\CraftAppsSourceBuilder", "Local\\CraftAppsManager"] {
        let held = crate::platform::Lock::take(name).unwrap();
        assert!(std::thread::spawn(|| execution_gate().is_err())
            .join()
            .unwrap());
        drop(held);
    }
    let held = execution_gate().unwrap();
    for name in ["Local\\CraftAppsSourceBuilder", "Local\\CraftAppsManager"] {
        assert!(
            std::thread::spawn(move || crate::platform::Lock::take(name).is_err())
                .join()
                .unwrap()
        );
    }
    drop(held);
    assert!(execution_gate().is_ok());
}
struct Fixture {
    root: PathBuf,
    plan: Plan,
    path: PathBuf,
    helper: PathBuf,
    journey: Journey,
    trust: TrustContext,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("craft-bridge-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(root.join("stage")).unwrap();
        let target = root.join(if cfg!(target_os = "macos") {
            "Manager.app"
        } else {
            "manager"
        });
        let executable = crate::self_update::staged_executable(&target);
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(&executable, b"new").unwrap();
        let helper = root.join("stage").join(crate::self_update::helper_name());
        fs::write(&helper, b"old").unwrap();
        let key = SigningKey::from_bytes(&[7; 32]);
        let trust = TrustContext {
            repository: "owner/manager".into(),
            channel: "stable".into(),
            keys: vec![key.verifying_key().to_bytes()],
        };
        let payload = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/updater-v2/catalog.json"
        ));
        let signature: String = key
            .sign(payload.as_bytes())
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let envelope =
            serde_json::to_vec(&serde_json::json!({"payload":payload,"signature":signature}))
                .unwrap();
        let catalog = verify_catalog(&envelope, &trust, 1001, None).unwrap();
        let source = ClientProfile {
            repository: trust.repository.clone(),
            channel: trust.channel.clone(),
            version: "1.0.0".into(),
            revision: 1,
            platform: "linux".into(),
            architecture: "x64".into(),
            installation: InstallationKind::Portable,
            capabilities: [ZIP.into(), LAYOUT.into()].into_iter().collect(),
        };
        let CheckOutcome::BridgeUpdate(offer) =
            super::super::routing::select_update(&catalog, &source).unwrap()
        else {
            panic!("bridge")
        };
        let p = catalog
            .catalog()
            .packages
            .iter()
            .find(|p| p.id == offer.immediate_package_id)
            .unwrap();
        let plan = Plan {
            layout: 0,
            msi: false,
            linux_package: false,
            parent_pid: 123,
            target: target.clone(),
            staged: root.join("stage/package"),
            hash: files::hash(&executable).unwrap(),
            root: root.clone(),
            tools: root.join("tools"),
            expected_identity: Some(super::super::transaction::expected(&catalog, p)),
        };
        let journey =
            Journey::approve(&SignedSelection { catalog, offer }, &source, &trust, 1001).unwrap();
        let path = root.join("stage/authorization.json");
        Self {
            root,
            plan,
            path,
            helper,
            journey,
            trust,
        }
    }
    fn prepared(&self) {
        record_prepared(&self.plan, &self.path, Some(&self.journey)).unwrap();
    }
    fn issued(&self) -> String {
        self.prepared();
        issue_at(
            &self.plan,
            &self.helper,
            &self.trust,
            1002,
            |_, _, _| Ok(()),
        )
        .unwrap()
        .unwrap()
    }
    fn take(&self, token: Option<&str>, now: i64) -> Result<Option<ResumePermit>> {
        take_at(
            &self.root,
            &self.plan.target,
            &crate::self_update::staged_executable(&self.plan.target),
            token,
            now,
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
fn handoff_is_one_use_and_old_hints_never_authorize() {
    let f = Fixture::new();
    files::write_json(
        &f.root.join("runtime/manager-updates/pending-bridge.json"),
        &serde_json::json!({"schema":1}),
    )
    .unwrap();
    assert!(f.take(None, 1002).unwrap().is_none());
    f.prepared();
    assert!(f.take(Some("unverified"), 1002).is_err());
    let token = f.issued();
    assert!(f.take(Some(&token), 1003).unwrap().is_some());
    assert!(f.take(Some(&token), 1003).is_err());
}
#[test]
fn ordinary_restart_wrong_token_and_expiry_revoke_consent() {
    for condition in ["normal", "token", "expiry"] {
        let f = Fixture::new();
        let token = f.issued();
        match condition {
            "normal" => assert!(f.take(None, 1003).unwrap().is_none()),
            "token" => assert!(f.take(Some("wrong"), 1003).is_err()),
            _ => assert!(f.take(Some(&token), 4602).is_err()),
        }
        assert!(f.take(Some(&token), 1003).is_err());
    }
}
#[test]
fn wrong_library_target_and_modified_binary_stop_automatic_resume() {
    for condition in ["root", "target", "binary"] {
        let f = Fixture::new();
        let token = f.issued();
        let other = f.root.join("other");
        fs::create_dir(&other).unwrap();
        let alternate = f.root.join("alternate");
        fs::write(&alternate, b"new").unwrap();
        let (root, target) = match condition {
            "root" => {
                let dest = state_path(&other);
                fs::create_dir_all(dest.parent().unwrap()).unwrap();
                fs::copy(state_path(&f.root), dest).unwrap();
                (&other, &f.plan.target)
            }
            "target" => (&f.root, &alternate),
            _ => {
                fs::write(
                    crate::self_update::staged_executable(&f.plan.target),
                    b"tampered",
                )
                .unwrap();
                (&f.root, &f.plan.target)
            }
        };
        assert!(
            take_at(root, target, target, Some(&token), 1003).is_err(),
            "{condition}"
        );
    }
}
#[test]
fn failed_or_changed_commit_cannot_issue_restart_authority() {
    for condition in ["helper", "target", "plan"] {
        let mut f = Fixture::new();
        f.prepared();
        match condition {
            "helper" => fs::write(&f.helper, b"changed").unwrap(),
            "target" => fs::write(
                crate::self_update::staged_executable(&f.plan.target),
                b"old",
            )
            .unwrap(),
            _ => f.plan.hash = "0".repeat(64),
        }
        assert!(
            issue_at(&f.plan, &f.helper, &f.trust, 1002, |_, _, _| Ok(())).is_err(),
            "{condition}"
        );
    }
}
#[test]
fn direct_or_finished_update_revokes_an_older_journey() {
    let f = Fixture::new();
    let token = f.issued();
    record_prepared(&f.plan, &f.path, None).unwrap();
    assert!(f.take(Some(&token), 1003).is_err());
}
#[test]
fn failed_identity_verification_cannot_issue_a_handoff() {
    let f = Fixture::new();
    f.prepared();
    assert!(
        issue_at(&f.plan, &f.helper, &f.trust, 1002, |_, _, _| anyhow::bail!(
            "wrong build identity"
        ))
        .is_err()
    );
    assert!(!state_path(&f.root).exists());
}
#[test]
fn restart_child_consumes_and_revalidates() {
    // Runs as a separate process in the test below, never from production startup.
    let Some(root) = std::env::var_os("CRAFT_BRIDGE_CONSENT_TEST_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let token = std::env::var("CRAFT_BRIDGE_CONSENT_TEST_PROOF").unwrap();
    let record = read_state(&root).unwrap().unwrap();
    let target = record.target.clone();
    let key = SigningKey::from_bytes(&[7; 32]);
    let trust = TrustContext {
        repository: "owner/manager".into(),
        channel: "stable".into(),
        keys: vec![key.verifying_key().to_bytes()],
    };
    let catalog = record.journey.verified(&trust, 1003).unwrap();
    let mut current = record.journey.source.clone();
    current.version = "1.1.0".into();
    current.capabilities.insert(TAR.into());
    let CheckOutcome::DirectUpdate(offer) =
        super::super::routing::select_update(&catalog, &current).unwrap()
    else {
        panic!("final offer")
    };
    let permit = take_at(
        &root,
        &target,
        &crate::self_update::staged_executable(&target),
        Some(&token),
        1003,
    )
    .unwrap()
    .unwrap();
    let next = permit
        .authorize(&SignedSelection { catalog, offer }, &current, &trust, 1003)
        .unwrap();
    assert!(!next.has_more_hops());
    assert!(take_at(
        &root,
        &target,
        &crate::self_update::staged_executable(&target),
        Some(&token),
        1003
    )
    .is_err());
    fs::write(root.join("continued"), b"verified").unwrap();
}
#[test]
fn one_use_authority_survives_a_real_process_restart_without_touching_data() {
    let f = Fixture::new();
    let token = f.issued();
    fs::write(f.root.join("settings.json"), b"preserve settings").unwrap();
    fs::create_dir(f.root.join("apps")).unwrap();
    fs::write(f.root.join("apps/user-file"), b"preserve app").unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            concat!(module_path!(), "::restart_child_consumes_and_revalidates")
                .strip_prefix(concat!(env!("CARGO_CRATE_NAME"), "::"))
                .unwrap(),
            "--test-threads=1",
        ])
        .env("CRAFT_BRIDGE_CONSENT_TEST_ROOT", &f.root)
        .env("CRAFT_BRIDGE_CONSENT_TEST_PROOF", &token)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Restart test failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(f.root.join("continued")).unwrap(), b"verified");
    assert_eq!(
        fs::read(f.root.join("settings.json")).unwrap(),
        b"preserve settings"
    );
    assert_eq!(
        fs::read(f.root.join("apps/user-file")).unwrap(),
        b"preserve app"
    );
    assert!(f.take(Some(&token), 1004).is_err());
}
#[test]
fn invalid_schema_or_restart_arguments_never_authorize() {
    let f = Fixture::new();
    f.prepared();
    let path = state_path(&f.root);
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["schema"] = 2.into();
    files::write_json(&path, &value).unwrap();
    assert!(f.take(Some("a"), 1003).is_err());
    for args in [
        vec![RESTART_FLAG],
        vec![RESTART_FLAG, "bad"],
        vec![
            RESTART_FLAG,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            RESTART_FLAG,
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        ],
    ] {
        let args: Vec<_> = args.into_iter().map(str::to_owned).collect();
        assert!(restart_token(&args).is_err());
    }
}
#[cfg(unix)]
#[test]
fn linked_state_is_rejected_without_touching_its_destination() {
    let f = Fixture::new();
    let external = f.root.join("external");
    fs::write(&external, b"keep").unwrap();
    let path = state_path(&f.root);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&external, &path).unwrap();
    assert!(f.take(Some("wrong"), 1002).is_err());
    assert_eq!(fs::read(external).unwrap(), b"keep");
}
