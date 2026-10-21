use super::{check_state::*, model::*, transaction::*, transport::*};
use crate::{
    model::Paths,
    self_update::{self, Available},
};
use anyhow::{bail, Result};

use super::api::ManagerCheck;
#[derive(serde::Serialize, serde::Deserialize)]
pub struct PendingBridge {
    pub schema: u32,
    pub immediate: Position,
    pub final_target: Position,
    pub final_package_id: String,
    pub originating_sequence: u64,
}
pub fn record_bridge(
    paths: &Paths,
    catalog: &super::catalog::VerifiedCatalog,
    offer: &UpdateOffer,
) -> Result<()> {
    if offer.route_ids.len() <= 1 {
        return Ok(());
    }
    let c = catalog.catalog();
    let immediate = c
        .packages
        .iter()
        .find(|p| p.id == offer.immediate_package_id)
        .expect("validated immediate target");
    let final_target = c
        .packages
        .iter()
        .find(|p| p.id == offer.final_package_id)
        .expect("validated final target");
    let record = PendingBridge {
        schema: 1,
        immediate: c.position(immediate),
        final_target: c.position(final_target),
        final_package_id: final_target.id.clone(),
        originating_sequence: c.sequence,
    };
    crate::files::write_json(
        &paths.at("runtime/manager-updates/pending-bridge.json"),
        &record,
    )
}
pub fn needs_bridge_check(paths: &Paths) -> bool {
    let result = (|| -> Result<bool> {
        let path = paths.at("runtime/manager-updates/pending-bridge.json");
        if !path.exists() {
            return Ok(false);
        }
        crate::files::no_links(&path)?;
        anyhow::ensure!(
            std::fs::metadata(&path)?.len() <= 4096,
            "Invalid bridge check record"
        );
        let record: PendingBridge = serde_json::from_slice(&std::fs::read(&path)?)?;
        anyhow::ensure!(record.schema == 1, "Unsupported bridge record");
        let current = Position {
            version: env!("CARGO_PKG_VERSION").into(),
            revision: crate::update_revisions::BUILD_REVISION,
        }
        .key()?;
        if current >= record.final_target.key()? {
            std::fs::remove_file(path)?;
            return Ok(false);
        }
        Ok(current == record.immediate.key()?)
    })();
    // This is only a hint to schedule a fresh authenticated check, never update authority.
    result.unwrap_or(false)
}
pub fn message(outcome: &CheckOutcome) -> String {
    match outcome {
        CheckOutcome::Current => "You’re running the latest manager version.".into(),
        CheckOutcome::DirectUpdate(_) => "A manager update is available.".into(),
        CheckOutcome::BridgeUpdate(_) => {
            "A compatibility update is needed before the latest version.".into()
        }
        CheckOutcome::ExternalUpdate(reason)
        | CheckOutcome::ManualRequired(reason)
        | CheckOutcome::UnsupportedPlatform(reason) => reason.clone(),
        CheckOutcome::CheckFailed(reason) => format!("Could not check for updates: {reason}"),
    }
}
pub fn check_status(paths: &Paths, trigger: CheckTrigger) -> Result<ManagerCheck> {
    if super::coordinator::selected_source(env!("CRAFT_UPDATE_SOURCE"))?
        == super::coordinator::UpdateSource::Github
    {
        // This is an intentional build choice, not fallback after a service
        // failure. Keep the original GitHub protocol and its checks unchanged;
        // retain v2 trust records for a later service-enabled build.
        let available = self_update::check_legacy(paths)?;
        let outcome = if available.is_some() {
            CheckOutcome::DirectUpdate(UpdateOffer {
                immediate_package_id: "legacy".into(),
                final_package_id: "legacy".into(),
                route_ids: vec![],
            })
        } else {
            CheckOutcome::Current
        };
        if super::bridge::restart_requested() {
            super::bridge::revoke(&paths.root)?;
        }
        return Ok(ManagerCheck::service(outcome, available, None, None));
    }
    // Consume before any fallible/network work. A failed check never leaves authority to retry automatically.
    let (permit, mut continuation_paused) = if matches!(trigger, CheckTrigger::BridgeRestart) {
        match super::bridge::take_restart(&paths.root) {
            Ok(permit) => (permit, None),
            Err(error) => (
                None,
                Some(format!(
                    "Automatic update paused: {error:#}. Check again to continue manually."
                )),
            ),
        }
    } else {
        (None, None)
    };
    let url = option_env!("CRAFT_UPDATE_CATALOG_URL").unwrap_or("");
    anyhow::ensure!(
        !url.is_empty(),
        "Service updater endpoint is not configured"
    );
    let trust = production_trust()?;
    let transport = MetadataTransport::with_timeouts(
        std::time::Duration::from_secs(env!("CRAFT_UPDATE_CONNECT_TIMEOUT").parse()?),
        std::time::Duration::from_secs(env!("CRAFT_UPDATE_METADATA_TIMEOUT").parse()?),
    )?;
    let mut endpoints = vec![ValidatedEndpoint::new(url)?];
    if let Some(check) = option_env!("CRAFT_UPDATE_CHECK_URL").filter(|s| !s.is_empty()) {
        endpoints.insert(0, ValidatedEndpoint::new(check)?);
    }
    if let Some(prior) =
        super::trust::TrustStore::new(&paths.root, &trust.repository, &trust.channel).load()?
    {
        for origin in prior.metadata_origins.into_iter().rev() {
            if !endpoints.iter().any(|e| e.as_str() == origin) {
                endpoints.insert(0, ValidatedEndpoint::new(&origin)?);
            }
        }
    }
    if let Some(mirror) = option_env!("CRAFT_UPDATE_CATALOG_MIRROR").filter(|s| !s.is_empty()) {
        endpoints.push(ValidatedEndpoint::new(mirror)?);
    }
    let now = chrono::Utc::now().timestamp();
    let profile = installed_profile(paths)?;
    let outcome = check(
        &profile,
        &CheckContext {
            root: &paths.root,
            trust: &trust,
            endpoints: &endpoints,
            transport: &transport,
            now,
            trigger,
        },
    );
    if let CheckOutcome::CheckFailed(reason) = &outcome {
        bail!("{reason}");
    }
    let available = match &outcome {
        CheckOutcome::DirectUpdate(offer) | CheckOutcome::BridgeUpdate(offer) => {
            let catalog = verified_cached(&paths.root, &trust, now)?;
            validate_offer(&catalog, &profile, offer)?;
            let p = catalog
                .catalog()
                .packages
                .iter()
                .find(|p| p.id == offer.immediate_package_id)
                .expect("validated offer");
            Some(Available {
                version: catalog.catalog().release(p).version.clone(),
                revision: p.revision,
                package_version: p.native_version.clone(),
                asset: asset(p),
                selection: Some(SignedSelection {
                    catalog,
                    offer: offer.clone(),
                }),
            })
        }
        _ => None,
    };
    let continuation = if let Some(permit) = permit {
        match available.as_ref().and_then(|a| a.selection.as_ref()) {
            Some(selection) => {
                match permit.authorize(selection, &profile, &trust, chrono::Utc::now().timestamp())
                {
                    Ok(journey) => Some(journey),
                    Err(error) => {
                        continuation_paused = Some(format!(
                            "Automatic update paused: {error:#}. Review the update to continue."
                        ));
                        None
                    }
                }
            }
            None => {
                continuation_paused = Some(
                    "Automatic update paused: the approved update is no longer offered.".into(),
                );
                None
            }
        }
    } else {
        None
    };
    Ok(ManagerCheck::service(
        outcome,
        available,
        continuation,
        continuation_paused,
    ))
}
