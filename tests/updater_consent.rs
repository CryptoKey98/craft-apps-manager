#![cfg(feature = "service-updater")]
mod common_updater;
use common_updater::*;
use craft_apps_manager::manager_updates::{
    bridge::Journey, model::*, routing::select_update, transaction::SignedSelection,
};
fn selection(c: &Catalog, p: &ClientProfile) -> SignedSelection {
    let catalog = verified(c);
    let offer = match select_update(&catalog, p).unwrap() {
        CheckOutcome::BridgeUpdate(o) | CheckOutcome::DirectUpdate(o) => o,
        other => panic!("No update: {other:?}"),
    };
    SignedSelection { catalog, offer }
}
fn intermediate() -> ClientProfile {
    let mut p = profile(true);
    p.version = "1.1.0".into();
    p
}
#[test]
fn one_consent_covers_only_remaining_approved_packages() {
    let c = catalog();
    let approved = Journey::approve(
        &selection(&c, &profile(false)),
        &profile(false),
        &context(),
        1001,
    )
    .unwrap();
    let mut renewed = c.clone();
    renewed.sequence = 2;
    renewed.issued = 1002;
    let next = selection(&renewed, &intermediate());
    let continuing = approved
        .next(&next, &intermediate(), &context(), 1003)
        .unwrap();
    assert!(!continuing.has_more_hops());
}
#[test]
fn changed_destination_or_package_requires_new_consent() {
    for change in ["hash", "size", "layout", "version", "url", "handler"] {
        let c = catalog();
        let approved = Journey::approve(
            &selection(&c, &profile(false)),
            &profile(false),
            &context(),
            1001,
        )
        .unwrap();
        let mut updated = c.clone();
        updated.sequence = 2;
        match change {
            "hash" => updated.packages[1].sha256 = "c".repeat(64),
            "size" => updated.packages[1].size += 1,
            "layout" => updated.packages[1].layout.root = "Changed".into(),
            "version" => updated.releases[1].version = "1.3.0".into(),
            "url" => {
                updated.packages[1].url =
                    "https://github.com/owner/manager/releases/download/v1.2.0/different.tar.gz"
                        .into();
                updated.packages[1].asset = "different.tar.gz".into();
            }
            "handler" => updated.packages[1].handler = ZIP.into(),
            _ => unreachable!(),
        }
        assert!(
            approved
                .next(
                    &selection(&updated, &intermediate()),
                    &intermediate(),
                    &context(),
                    1002
                )
                .is_err(),
            "{change}"
        );
    }
}
#[test]
fn wrong_profile_expired_consent_and_withdrawal_stop_continuation() {
    let c = catalog();
    let approved = Journey::approve(
        &selection(&c, &profile(false)),
        &profile(false),
        &context(),
        1001,
    )
    .unwrap();
    let next = selection(&c, &intermediate());
    for change in ["version", "repository", "architecture", "installation"] {
        let mut wrong = intermediate();
        match change {
            "version" => wrong.version = "1.0.0".into(),
            "repository" => wrong.repository = "other/manager".into(),
            "architecture" => wrong.architecture = "x86".into(),
            "installation" => wrong.installation = InstallationKind::Deb,
            _ => unreachable!(),
        }
        assert!(
            approved.next(&next, &wrong, &context(), 1002).is_err(),
            "{change}"
        );
    }
    assert!(approved
        .next(&next, &intermediate(), &context(), 4602)
        .is_err());
    let mut withdrawn = c.clone();
    withdrawn.sequence = 2;
    withdrawn.withdrawn.insert("c-linux".into());
    let rejected = SignedSelection {
        catalog: verified(&withdrawn),
        offer: next.offer,
    };
    assert!(approved
        .next(&rejected, &intermediate(), &context(), 1002)
        .is_err());
}
#[test]
fn a_direct_offer_cannot_create_an_automatic_update_journey() {
    let c = catalog();
    assert!(Journey::approve(
        &selection(&c, &profile(true)),
        &profile(true),
        &context(),
        1001
    )
    .is_err());
}

#[test]
fn multiple_bridge_steps_never_extend_the_original_approval_deadline() {
    let mut c = catalog();
    let mut release = c.releases[1].clone();
    release.id = "d".into();
    release.version = "1.3.0".into();
    release.tag = "v1.3.0".into();
    c.releases.push(release);
    let mut package = c.packages[1].clone();
    package.id = "d-linux".into();
    package.release = "d".into();
    package.required_capabilities.insert("test.future".into());
    package.asset = "final.tar.gz".into();
    package.url = "https://github.com/owner/manager/releases/download/v1.3.0/final.tar.gz".into();
    c.packages.push(package);
    c.routes[1].bridge = true;
    c.routes[1].max_source = Some(Position {
        version: "1.1.0".into(),
        revision: 65535,
    });
    c.routes[1]
        .destination_capabilities
        .insert("test.future".into());
    let mut route = c.routes[1].clone();
    route.id = "to-d".into();
    route.target = "d-linux".into();
    route.bridge = false;
    route.max_source = None;
    route.required_capabilities.insert("test.future".into());
    c.routes.push(route);
    let first = selection(&c, &profile(false));
    assert_eq!(first.offer.route_ids.len(), 3);
    let approved = Journey::approve(&first, &profile(false), &context(), 1001).unwrap();
    let second = approved
        .next(
            &selection(&c, &intermediate()),
            &intermediate(),
            &context(),
            4500,
        )
        .unwrap();
    assert!(second.has_more_hops());
    let mut third = intermediate();
    third.version = "1.2.0".into();
    third.capabilities.insert("test.future".into());
    let last = selection(&c, &third);
    assert!(second.next(&last, &third, &context(), 4600).is_ok());
    assert!(second.next(&last, &third, &context(), 4601).is_err());
}

#[test]
fn new_routes_cannot_add_an_unapproved_extra_hop() {
    let c = catalog();
    let approved = Journey::approve(
        &selection(&c, &profile(false)),
        &profile(false),
        &context(),
        1001,
    )
    .unwrap();
    let mut updated = c.clone();
    updated.sequence += 1;
    let mut release = updated.releases[1].clone();
    release.id = "later".into();
    release.version = "1.3.0".into();
    release.tag = "v1.3.0".into();
    updated.releases.push(release);
    let mut package = updated.packages[1].clone();
    package.id = "later-linux".into();
    package.release = "later".into();
    package.asset = "later.tar.gz".into();
    package.url = "https://github.com/owner/manager/releases/download/v1.3.0/later.tar.gz".into();
    updated.packages.push(package);
    let mut route = updated.routes[1].clone();
    route.id = "to-later".into();
    route.target = "later-linux".into();
    updated.routes.push(route);
    assert!(approved
        .next(
            &selection(&updated, &intermediate()),
            &intermediate(),
            &context(),
            1002
        )
        .is_err());
}
