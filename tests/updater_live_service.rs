#![cfg(feature = "service-updater")]
//! Opt-in HTTPS checks against the isolated Worker; never downloads or installs apps.
mod common_updater;
use common_updater::*;
use craft_apps_manager::manager_updates::{
    catalog::verify_catalog,
    check_state::{check, CheckContext, CheckTrigger},
    model::CheckOutcome,
    transaction::production_trust,
    transport::{FetchMetadata, FetchResponse, MetadataTransport, ValidatedEndpoint},
};

#[test]
#[ignore = "requires an explicitly selected isolated test Worker"]
fn isolated_worker_signed_catalog_smoke() {
    let base = std::env::var("CRAFT_TEST_UPDATE_URL").expect("Set isolated test Worker URL");
    let url = reqwest::Url::parse(&base).unwrap();
    assert!(url
        .host_str()
        .is_some_and(|host| host.starts_with("craft-apps-manager-updates-test.")
            && host.ends_with(".workers.dev")));
    assert_eq!(url.path(), "/");
    let static_endpoint =
        ValidatedEndpoint::new(url.join("v2/catalog.json").unwrap().as_str()).unwrap();
    let dynamic_endpoint = ValidatedEndpoint::new(url.join("v2/check").unwrap().as_str()).unwrap();
    let transport = MetadataTransport::new().unwrap();
    let FetchResponse::Body { bytes, .. } = transport.fetch(&static_endpoint, None).unwrap() else {
        panic!("Expected signed static catalog")
    };
    let now = chrono::Utc::now().timestamp();
    let mut trust = context();
    trust.channel = "test".into();
    let verified = verify_catalog(&bytes, &trust, now, None).unwrap();
    assert_eq!(verified.catalog().channel, "test");
    // Also prove key rejection with matching scope, rather than relying only on
    // the deliberately different repository/channel to reject this fixture.
    let mut production = production_trust().unwrap();
    production.repository = trust.repository.clone();
    production.channel = trust.channel.clone();
    assert!(verify_catalog(&bytes, &production, now, None).is_err());
    for endpoint in [static_endpoint, dynamic_endpoint] {
        for (tar, version, outcome) in [
            (false, "1.0.0", "bridge"),
            (true, "1.0.0", "direct"),
            (true, "1.2.0", "current"),
        ] {
            let temp = Temp::new();
            let mut client = profile(tar);
            client.channel = "test".into();
            client.version = version.into();
            let result = check(
                &client,
                &CheckContext {
                    root: &temp.0,
                    trust: &trust,
                    endpoints: std::slice::from_ref(&endpoint),
                    transport: &transport,
                    now,
                    trigger: CheckTrigger::Manual,
                },
            );
            match (outcome, result) {
                ("bridge", CheckOutcome::BridgeUpdate(offer)) => {
                    assert_eq!(offer.immediate_package_id, "b-linux");
                    assert_eq!(offer.final_package_id, "c-linux");
                }
                ("direct", CheckOutcome::DirectUpdate(offer)) => {
                    assert_eq!(offer.immediate_package_id, "c-linux");
                }
                ("current", CheckOutcome::Current) => {}
                (_, result) => panic!("{} expected {outcome}, got {result:?}", endpoint.as_str()),
            }
        }
    }
}
