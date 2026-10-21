#![cfg(feature = "service-updater")]
mod common_updater;
use common_updater::*;
use craft_apps_manager::manager_updates::{check_state::*, model::*, transport::*};
use std::sync::atomic::{AtomicUsize, Ordering};
struct Mock {
    calls: AtomicUsize,
    body: Vec<u8>,
    status: u16,
}
impl FetchMetadata for Mock {
    fn fetch(&self, _: &ValidatedEndpoint, _: Option<&str>) -> anyhow::Result<FetchResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(match self.status {
            200 => FetchResponse::Body {
                bytes: self.body.clone(),
                etag: Some("a".into()),
            },
            304 => FetchResponse::NotModified,
            429 => FetchResponse::RateLimited(99999),
            _ => anyhow::bail!("offline"),
        })
    }
}
fn run(temp: &Temp, transport: &Mock, now: i64) -> CheckOutcome {
    let endpoints = [ValidatedEndpoint::new("https://updates.example/v2/catalog.json").unwrap()];
    check(
        &profile(true),
        &CheckContext {
            root: &temp.0,
            trust: &context(),
            endpoints: &endpoints,
            transport,
            now,
            trigger: CheckTrigger::Manual,
        },
    )
}
#[test]
fn cached_catalog_reverified() {
    let temp = Temp::new();
    let mock = Mock {
        calls: AtomicUsize::new(0),
        body: signed(&catalog()),
        status: 200,
    };
    assert!(matches!(
        run(&temp, &mock, 1001),
        CheckOutcome::DirectUpdate(_)
    ));
    assert!(matches!(
        run(&temp, &mock, 1002),
        CheckOutcome::DirectUpdate(_)
    ));
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
    let offline = Mock {
        calls: AtomicUsize::new(0),
        body: vec![],
        status: 304,
    };
    assert!(matches!(
        run(&temp, &offline, 2_593_001),
        CheckOutcome::CheckFailed(_)
    ));
}
#[test]
fn retry_deadline_survives_restart() {
    let temp = Temp::new();
    let mock = Mock {
        calls: AtomicUsize::new(0),
        body: vec![],
        status: 429,
    };
    assert!(matches!(
        run(&temp, &mock, 1001),
        CheckOutcome::CheckFailed(_)
    ));
    run(&temp, &mock, 1010);
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
    run(&temp, &mock, 4602);
    assert_eq!(mock.calls.load(Ordering::SeqCst), 2);
}
#[test]
fn trust_failure_never_uses_legacy() {
    let temp = Temp::new();
    let mock = Mock {
        calls: AtomicUsize::new(0),
        body: b"{}".to_vec(),
        status: 200,
    };
    assert!(matches!(
        run(&temp, &mock, 1001),
        CheckOutcome::CheckFailed(_)
    ));
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
}
#[test]
fn endpoints_reject_credentials_and_unsafe_transport() {
    for url in [
        "http://localhost/a",
        "https://user:pass@example.com/a",
        "https://example.com/a?token=secret",
        "https://example.com/a#x",
    ] {
        assert!(ValidatedEndpoint::new(url).is_err());
    }
}
#[test]
fn asset_redirects_cannot_leave_github() {
    for url in [
        "http://github.com/file",
        "https://github.com.evil.example/file",
        "https://user:secret@github.com/file",
        "https://example.com/file",
    ] {
        assert!(
            !craft_apps_manager::manager_updates::transport::asset_host_allowed(
                &reqwest::Url::parse(url).unwrap()
            )
        );
    }
    assert!(
        craft_apps_manager::manager_updates::transport::asset_host_allowed(
            &reqwest::Url::parse("https://release-assets.githubusercontent.com/file").unwrap()
        )
    );
}
#[test]
fn dynamic_selection_cannot_claim_current() {
    let temp = Temp::new();
    let envelope: serde_json::Value = serde_json::from_slice(&signed(&catalog())).unwrap();
    let body = serde_json::to_vec(
        &serde_json::json!({"envelope":envelope,"selection":{"outcome":"current"}}),
    )
    .unwrap();
    let mock = Mock {
        calls: AtomicUsize::new(0),
        body,
        status: 200,
    };
    let endpoints = [ValidatedEndpoint::new("https://updates.example/v2/check").unwrap()];
    let result = check(
        &profile(true),
        &CheckContext {
            root: &temp.0,
            trust: &context(),
            endpoints: &endpoints,
            transport: &mock,
            now: 1001,
            trigger: CheckTrigger::Manual,
        },
    );
    assert!(matches!(result, CheckOutcome::CheckFailed(_)));
}
#[test]
fn concurrent_checks_share_result() {
    let temp = Temp::new();
    let held = CheckStore::new(&temp.0)
        .begin(1001, CheckTrigger::Manual)
        .unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "check_lock_child", "--test-threads=1"])
        .env("CRAFT_TEST_CHECK_ROOT", &temp.0)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    drop(held);
}
#[test]
fn check_lock_child() {
    if let Some(root) = std::env::var_os("CRAFT_TEST_CHECK_ROOT") {
        assert!(CheckStore::new(std::path::Path::new(&root))
            .begin(1001, CheckTrigger::Manual)
            .is_err());
    }
}
#[test]
fn dynamic_outage_uses_identical_static_policy() {
    struct Outage;
    impl FetchMetadata for Outage {
        fn fetch(
            &self,
            endpoint: &ValidatedEndpoint,
            _: Option<&str>,
        ) -> anyhow::Result<FetchResponse> {
            if endpoint.is_dynamic() {
                anyhow::bail!("offline");
            }
            Ok(FetchResponse::Body {
                bytes: signed(&catalog()),
                etag: None,
            })
        }
    }
    let temp = Temp::new();
    let endpoints = [
        ValidatedEndpoint::new("https://updates.example/v2/check").unwrap(),
        ValidatedEndpoint::new("https://updates.example/v2/catalog.json").unwrap(),
    ];
    assert!(matches!(
        check(
            &profile(true),
            &CheckContext {
                root: &temp.0,
                trust: &context(),
                endpoints: &endpoints,
                transport: &Outage,
                now: 1001,
                trigger: CheckTrigger::Manual
            }
        ),
        CheckOutcome::DirectUpdate(_)
    ));
}
#[test]
fn bridge_restart_fetches_withdrawal_instead_of_cached_offer() {
    let temp = Temp::new();
    let old = Mock {
        calls: AtomicUsize::new(0),
        body: signed(&catalog()),
        status: 200,
    };
    run(&temp, &old, 1001);
    let mut changed = catalog();
    changed.sequence = 2;
    changed.withdrawn.insert("c-linux".into());
    let new = Mock {
        calls: AtomicUsize::new(0),
        body: signed(&changed),
        status: 200,
    };
    let mut installed = profile(true);
    installed.version = "1.1.0".into();
    let endpoints = [ValidatedEndpoint::new("https://updates.example/v2/catalog.json").unwrap()];
    let result = check(
        &installed,
        &CheckContext {
            root: &temp.0,
            trust: &context(),
            endpoints: &endpoints,
            transport: &new,
            now: 1002,
            trigger: CheckTrigger::BridgeRestart,
        },
    );
    assert_eq!(
        new.calls.load(Ordering::SeqCst),
        1,
        "bridge restart must fetch fresh policy"
    );
    assert!(matches!(result, CheckOutcome::Current));
}
