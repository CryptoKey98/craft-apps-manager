use super::{catalog::VerifiedCatalog, model::*};
use anyhow::{ensure, Context, Result};
use std::collections::{BTreeSet, VecDeque};

pub fn known_handler(handler: &str, platform: &str, kind: &InstallationKind) -> bool {
    matches!(
        (handler, platform, kind),
        (ZIP, "windows" | "linux", InstallationKind::Portable)
            | (ZIP | TAR | DMG, "macos", InstallationKind::Bundle)
            | (TAR, "linux", InstallationKind::Portable)
            | ("native.msi.v1", "windows", InstallationKind::Msi)
            | ("native.deb.v1", "linux", InstallationKind::Deb)
            | ("native.rpm.v1", "linux", InstallationKind::Rpm)
            | ("native.arch.v1", "linux", InstallationKind::Arch)
    )
}
fn target_matches(p: &PackageRecord, client: &ClientProfile) -> bool {
    p.platform == client.platform
        && p.architecture == client.architecture
        && p.installation == client.installation
}
fn eligible(c: &Catalog, route: &RouteRecord, client: &ClientProfile) -> Result<bool> {
    let p = c
        .packages
        .iter()
        .find(|p| p.id == route.target)
        .expect("validated route");
    let current = client.position().key()?;
    Ok(target_matches(p, client)
        && !c.withdrawn.contains(&p.id)
        && p.layout.schema == 1
        && known_handler(&p.handler, &p.platform, &p.installation)
        && client.capabilities.contains(&p.handler)
        && client.capabilities.contains(LAYOUT)
        && p.required_capabilities.is_subset(&client.capabilities)
        && route.required_capabilities.is_subset(&client.capabilities)
        && route.min_source.key()? <= current
        && route
            .max_source
            .as_ref()
            .map(|s| s.key().map(|key| current <= key))
            .transpose()?
            .unwrap_or(true)
        && c.position(p).key()? > current)
}
pub fn select_update(verified: &VerifiedCatalog, client: &ClientProfile) -> Result<CheckOutcome> {
    select_catalog(verified.catalog(), client)
}
fn select_catalog(c: &Catalog, client: &ClientProfile) -> Result<CheckOutcome> {
    ensure!(
        c.repository == client.repository && c.channel == client.channel,
        "Client catalog scope mismatch"
    );
    let current = client.position().key()?;
    let targets: Vec<_> = c
        .packages
        .iter()
        .filter(|p| target_matches(p, client) && !c.withdrawn.contains(&p.id))
        .collect();
    if targets.is_empty() {
        return Ok(CheckOutcome::UnsupportedPlatform(
            "No supported package for this installation".into(),
        ));
    }
    if !targets
        .iter()
        .any(|p| c.position(p).key().is_ok_and(|k| k > current))
    {
        return Ok(CheckOutcome::Current);
    }
    let mut queue = VecDeque::from([(client.clone(), Vec::<String>::new())]);
    let mut visited = BTreeSet::new();
    let mut best: Option<(([u64; 4], u32), UpdateOffer)> = None;
    while let Some((state, path)) = queue.pop_front() {
        if path.len() >= 8 {
            continue;
        }
        let mut routes: Vec<_> = c.routes.iter().collect();
        routes.sort_by(|a, b| a.id.cmp(&b.id));
        for route in routes {
            if !eligible(c, route, &state)? {
                continue;
            }
            let p = c
                .packages
                .iter()
                .find(|p| p.id == route.target)
                .expect("validated route");
            let mut next_path = path.clone();
            next_path.push(route.id.clone());
            let first = c
                .routes
                .iter()
                .find(|r| r.id == next_path[0])
                .expect("validated route");
            let offer = UpdateOffer {
                immediate_package_id: first.target.clone(),
                final_package_id: p.id.clone(),
                route_ids: next_path.clone(),
            };
            let key = c.position(p).key()?;
            let replace = best.as_ref().is_none_or(|(prior_key, prior)| {
                key > *prior_key
                    || key == *prior_key
                        && (
                            offer.route_ids.len(),
                            &offer.final_package_id,
                            &offer.route_ids,
                        ) < (
                            prior.route_ids.len(),
                            &prior.final_package_id,
                            &prior.route_ids,
                        )
            });
            if replace {
                best = Some((key, offer));
            }
            if route.bridge
                && visited.insert((p.id.clone(), route.destination_capabilities.clone()))
            {
                let mut next = state.clone();
                next.version = c.release(p).version.clone();
                next.revision = p.revision;
                next.capabilities = route.destination_capabilities.clone();
                queue.push_back((next, next_path));
            }
        }
    }
    Ok(match best {
        Some((_, offer)) if offer.route_ids.len() > 1 => CheckOutcome::BridgeUpdate(offer),
        Some((_, offer)) => CheckOutcome::DirectUpdate(offer),
        None if client.installation == InstallationKind::External => CheckOutcome::ExternalUpdate(
            "Update through the software manager that installed this app".into(),
        ),
        None => CheckOutcome::ManualRequired("A required compatible update is unavailable".into()),
    })
}
pub fn validate_selection(
    catalog: &VerifiedCatalog,
    client: &ClientProfile,
    offer: &UpdateOffer,
) -> Result<()> {
    match select_update(catalog, client)? {
        CheckOutcome::DirectUpdate(expected) | CheckOutcome::BridgeUpdate(expected) => ensure!(
            &expected == offer,
            "Server selection is not the eligible signed route"
        ),
        _ => anyhow::bail!("No authorized update route"),
    }
    Ok(())
}
/// Publishing validation only. Clients must use select_update with a VerifiedCatalog.
pub fn validate_publishing_routes(catalog: &Catalog, profiles: &[ClientProfile]) -> Result<()> {
    catalog.validate()?;
    for profile in profiles {
        let latest = catalog
            .packages
            .iter()
            .filter(|p| target_matches(p, profile) && !catalog.withdrawn.contains(&p.id))
            .map(|p| catalog.position(p).key())
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .max()
            .context("No target for supported profile")?;
        let outcome = select_catalog(catalog, profile)?;
        match &outcome {
            CheckOutcome::Current => ensure!(
                profile.position().key()? >= latest,
                "Supported profile cannot reach latest target"
            ),
            CheckOutcome::DirectUpdate(offer) | CheckOutcome::BridgeUpdate(offer) => {
                let target = catalog
                    .packages
                    .iter()
                    .find(|p| p.id == offer.final_package_id)
                    .expect("selected target");
                ensure!(
                    catalog.position(target).key()? == latest,
                    "Supported profile stops before latest target"
                );
            }
            _ => anyhow::bail!("No compatible route for supported profile"),
        }
        ensure!(
            matches!(
                outcome,
                CheckOutcome::Current
                    | CheckOutcome::DirectUpdate(_)
                    | CheckOutcome::BridgeUpdate(_)
            ),
            "No safe route for supported profile {} {} {}",
            profile.platform,
            profile.architecture,
            profile.version
        );
    }
    Ok(())
}
