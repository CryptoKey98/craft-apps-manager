pub use super::api::ManagerCheck;
use anyhow::Result;
#[derive(Debug, PartialEq, Eq)]
pub enum UpdateSource {
    Github,
    Service,
}
pub fn selected_source(setting: &str) -> Result<UpdateSource> {
    match setting {
        "github" => Ok(UpdateSource::Github),
        "service" => Ok(UpdateSource::Service),
        _ => anyhow::bail!("Unsupported manager update source"),
    }
}
#[cfg(feature = "service-updater")]
pub use super::service_coordinator::{
    check_status, message, needs_bridge_check, record_bridge, PendingBridge,
};
#[cfg(not(feature = "service-updater"))]
pub fn needs_bridge_check(_: &crate::model::Paths) -> bool {
    false
}
#[cfg(not(feature = "service-updater"))]
pub fn check_status(
    paths: &crate::model::Paths,
    _: super::api::CheckTrigger,
) -> Result<ManagerCheck> {
    let available = crate::self_update::check_legacy(paths)?;
    let (status, message) = if let Some(a) = &available {
        (
            super::api::UpdateStatus::Available,
            format!("Version {} is available.", a.display_version()),
        )
    } else {
        (
            super::api::UpdateStatus::Current,
            "You’re running the latest manager version.".into(),
        )
    };
    Ok(ManagerCheck {
        status,
        message,
        available,
        continuation: None,
        continuation_paused: None,
    })
}
