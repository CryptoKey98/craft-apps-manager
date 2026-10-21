#![cfg(feature = "service-updater")]
mod common_updater;
use common_updater::*;
use craft_apps_manager::manager_updates::transaction::{target_lock, validate_source};
#[test]
fn helper_rechecks_target_identity() {
    let temp = Temp::new();
    let target = temp.0.join("manager");
    std::fs::write(&target, b"old").unwrap();
    let hash = craft_apps_manager::files::hash(&target).unwrap();
    validate_source(&target, &hash).unwrap();
    std::fs::write(&target, b"changed").unwrap();
    assert!(validate_source(&target, &hash).is_err());
}
#[test]
fn target_lock_covers_distinct_data_roots() {
    let temp = Temp::new();
    let target = temp.0.join("manager");
    std::fs::write(&target, b"old").unwrap();
    let held = target_lock(&target).unwrap();
    let other = target.clone();
    let blocked = std::thread::spawn(move || target_lock(&other).is_err())
        .join()
        .unwrap();
    assert!(blocked);
    drop(held);
    assert!(target_lock(&target).is_ok());
}
#[test]
fn expiry_after_staging_prevents_commit() {
    use craft_apps_manager::manager_updates::{
        model::CheckOutcome,
        routing::select_update,
        transaction::{authorize_commit, SignedSelection},
        trust::TrustStore,
    };
    let temp = Temp::new();
    let c = verified(&catalog());
    let p = profile(true);
    let CheckOutcome::DirectUpdate(offer) = select_update(&c, &p).unwrap() else {
        panic!("fixture offer")
    };
    TrustStore::new(&temp.0, "owner/manager", "stable")
        .accept(&c)
        .unwrap();
    let selection = SignedSelection { catalog: c, offer };
    assert!(authorize_commit(&temp.0, &selection, &p, &context(), 2_593_000).is_err());
}
#[test]
fn accepted_withdrawal_after_staging_prevents_commit() {
    use craft_apps_manager::manager_updates::{
        model::CheckOutcome,
        routing::select_update,
        transaction::{authorize_commit, SignedSelection},
        trust::TrustStore,
    };
    let temp = Temp::new();
    let old = verified(&catalog());
    let p = profile(true);
    let CheckOutcome::DirectUpdate(offer) = select_update(&old, &p).unwrap() else {
        panic!("fixture offer")
    };
    let store = TrustStore::new(&temp.0, "owner/manager", "stable");
    store.accept(&old).unwrap();
    let mut next = catalog();
    next.sequence = 2;
    next.withdrawn.insert("c-linux".into());
    store.accept(&verified(&next)).unwrap();
    assert!(authorize_commit(
        &temp.0,
        &SignedSelection {
            catalog: old,
            offer
        },
        &p,
        &context(),
        1002
    )
    .is_err());
}
#[test]
fn accepted_policy_cannot_change_while_commit_guard_is_held() {
    use craft_apps_manager::manager_updates::{
        model::CheckOutcome,
        routing::select_update,
        transaction::{authorize_commit, SignedSelection},
        trust::TrustStore,
    };
    let temp = Temp::new();
    let old = verified(&catalog());
    let p = profile(true);
    let CheckOutcome::DirectUpdate(offer) = select_update(&old, &p).unwrap() else {
        panic!("fixture offer")
    };
    let store = TrustStore::new(&temp.0, "owner/manager", "stable");
    store.accept(&old).unwrap();
    let guard = authorize_commit(
        &temp.0,
        &SignedSelection {
            catalog: old,
            offer,
        },
        &p,
        &context(),
        1002,
    )
    .unwrap();
    let mut next = catalog();
    next.sequence = 2;
    let next = verified(&next);
    let root = temp.0.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let writer = std::thread::spawn(move || {
        let result = TrustStore::new(&root, "owner/manager", "stable").accept(&next);
        tx.send(result.is_ok()).unwrap();
    });
    assert!(matches!(
        rx.recv_timeout(std::time::Duration::from_millis(50)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    drop(guard);
    assert!(rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap());
    writer.join().unwrap();
    assert_eq!(store.load().unwrap().unwrap().sequence, 2);
}
