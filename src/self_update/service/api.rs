//! Host update API: available even when service support is not compiled.
use crate::self_update::Available;
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy)]
pub enum CheckTrigger {
    Manual,
    Startup,
    BridgeRestart,
    Periodic(u64),
}
#[derive(Debug, PartialEq, Eq)]
pub enum UpdateStatus {
    Current,
    Available,
    Blocked(String),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedIdentity {
    pub version: String,
    pub revision: u32,
    pub architecture: String,
    pub platform: String,
}
#[cfg(feature = "service-updater")]
pub type ApprovedContinuation = super::bridge::Journey;
#[cfg(not(feature = "service-updater"))]
pub enum ApprovedContinuation {}
pub struct ManagerCheck {
    pub status: UpdateStatus,
    pub message: String,
    pub available: Option<Available>,
    pub continuation: Option<ApprovedContinuation>,
    pub continuation_paused: Option<String>,
    #[cfg(feature = "service-updater")]
    pub outcome: super::model::CheckOutcome,
}
#[cfg(feature = "service-updater")]
impl ManagerCheck {
    pub(crate) fn service(
        outcome: super::model::CheckOutcome,
        available: Option<Available>,
        continuation: Option<ApprovedContinuation>,
        continuation_paused: Option<String>,
    ) -> Self {
        let status = match &outcome {
            super::model::CheckOutcome::Current => UpdateStatus::Current,
            super::model::CheckOutcome::DirectUpdate(_)
            | super::model::CheckOutcome::BridgeUpdate(_) => UpdateStatus::Available,
            other => UpdateStatus::Blocked(super::service_coordinator::message(other)),
        };
        let message = if matches!(&outcome, super::model::CheckOutcome::DirectUpdate(_)) {
            available
                .as_ref()
                .map(|a| format!("Version {} is available.", a.display_version()))
                .unwrap_or_else(|| super::service_coordinator::message(&outcome))
        } else {
            super::service_coordinator::message(&outcome)
        };
        Self {
            status,
            message,
            available,
            continuation,
            continuation_paused,
            outcome,
        }
    }
}
