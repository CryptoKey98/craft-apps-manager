#![cfg(feature = "service-updater")]
mod common_updater;
use common_updater::*;
use craft_apps_manager::manager_updates::{
    model::*,
    routing::{select_update, validate_selection},
};
#[test]
fn shared_routing_vectors() {
    let vectors: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/updater-v2/routes.json")).unwrap();
    for v in vectors {
        let mut p = profile(v["tar"].as_bool().unwrap());
        p.version = v["version"].as_str().unwrap().into();
        let outcome = select_update(&verified(&catalog()), &p).unwrap();
        match outcome {
            CheckOutcome::DirectUpdate(o) | CheckOutcome::BridgeUpdate(o) => {
                assert_eq!(o.immediate_package_id, v["immediate"].as_str().unwrap());
                assert_eq!(o.final_package_id, v["final"].as_str().unwrap());
                assert_eq!(serde_json::to_value(o.route_ids).unwrap(), v["routes"]);
            }
            CheckOutcome::Current => assert_eq!(v["outcome"], "current"),
            other => panic!("{}: {other:?}", v["name"]),
        }
    }
}
#[test]
fn unsigned_selection_cannot_skip_bridge() {
    let v = verified(&catalog());
    assert!(validate_selection(
        &v,
        &profile(false),
        &UpdateOffer {
            immediate_package_id: "c-linux".into(),
            final_package_id: "c-linux".into(),
            route_ids: vec!["to-c".into()]
        }
    )
    .is_err());
}
#[test]
fn withdrawn_or_missing_bridge_is_manual() {
    let mut c = catalog();
    c.withdrawn.insert("b-linux".into());
    assert!(matches!(
        select_update(&verified(&c), &profile(false)).unwrap(),
        CheckOutcome::ManualRequired(_)
    ));
}
#[test]
fn same_version_revision_advances() {
    let mut c = catalog();
    c.packages[1].revision = 2;
    let mut p = profile(true);
    p.version = "1.2.0".into();
    assert!(matches!(
        select_update(&verified(&c), &p).unwrap(),
        CheckOutcome::DirectUpdate(_)
    ));
}
#[test]
fn unknown_handler_does_not_hide_valid_target() {
    let mut c = catalog();
    c.packages[1].handler = "future.unknown".into();
    assert!(matches!(
        select_update(&verified(&c), &profile(true)).unwrap(),
        CheckOutcome::DirectUpdate(_)
    ));
}
#[test]
fn installation_type_never_changes() {
    let mut p = profile(true);
    p.installation = InstallationKind::Deb;
    assert!(matches!(
        select_update(&verified(&catalog()), &p).unwrap(),
        CheckOutcome::UnsupportedPlatform(_)
    ));
}
#[test]
fn all_supported_profiles_reach_target() {
    let mut c = catalog();
    c.routes[0].destination_capabilities.remove(TAR);
    assert!(
        craft_apps_manager::manager_updates::routing::validate_publishing_routes(
            &c,
            &[profile(false)]
        )
        .is_err()
    );
}
#[test]
fn omitted_optional_arrays_match_worker_defaults() {
    let mut value = serde_json::to_value(catalog()).unwrap();
    value.as_object_mut().unwrap().remove("withdrawn");
    for p in value["packages"].as_array_mut().unwrap() {
        p.as_object_mut().unwrap().remove("required_capabilities");
    }
    for r in value["routes"].as_array_mut().unwrap() {
        let r = r.as_object_mut().unwrap();
        r.remove("required_capabilities");
        r.remove("destination_capabilities");
    }
    let c: Catalog = serde_json::from_value(value).unwrap();
    assert!(matches!(
        select_update(&verified(&c), &profile(true)).unwrap(),
        CheckOutcome::DirectUpdate(_)
    ));
}
