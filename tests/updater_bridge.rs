#![cfg(feature = "service-updater")]
mod common_updater;
use common_updater::*;
use craft_apps_manager::manager_updates::{
    catalog::verify_catalog, model::*, routing::select_update, transaction::production_trust,
};
#[test]
fn restart_rechecks_withdrawal_and_consent() {
    let c = catalog();
    let first = select_update(&verified(&c), &profile(false)).unwrap();
    assert!(matches!(first, CheckOutcome::BridgeUpdate(_)));
    let mut next = c.clone();
    next.sequence = 2;
    next.withdrawn.insert("c-linux".into());
    let mut b = profile(true);
    b.version = "1.1.0".into();
    assert!(matches!(
        select_update(&verified(&next), &b).unwrap(),
        CheckOutcome::Current
    ));
    // A saved first offer is not replayed; a new check and explicit approval own hop two.
}
#[test]
fn test_keys_rejected_by_production() {
    let mut c = catalog();
    let trust = production_trust().unwrap();
    c.repository = trust.repository.clone();
    for p in &mut c.packages {
        p.url = p.url.replace("owner/manager", &trust.repository);
    }
    assert!(verify_catalog(&signed(&c), &trust, 1001, None).is_err());
}
#[test]
fn production_profile_to_c_has_no_bridge() {
    assert!(matches!(
        select_update(&verified(&catalog()), &profile(true)).unwrap(),
        CheckOutcome::DirectUpdate(_)
    ));
}
