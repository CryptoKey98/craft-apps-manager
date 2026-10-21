//! No service runtime. Only legacy hooks that are not applicable may return None.
use crate::self_update::Plan;
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};
pub mod bridge {
    use super::*;
    pub use crate::manager_updates::api::ApprovedContinuation as Journey;
    pub fn restart_requested() -> bool {
        false
    }
    pub fn foreground_busy(records: &[crate::operations::Operation], own: Option<&str>) -> bool {
        records.iter().any(|op| {
            !op.lifecycle.terminal()
                && Some(op.id.as_str()) != own
                && !(own.is_some() && op.parent.as_deref() == own)
                && !op.kind.starts_with("Background ")
        })
    }
    pub(crate) fn revoke(_: &Path) -> Result<()> {
        Ok(())
    }
    pub(crate) fn append_restart(_: &mut std::process::Command, _: &Plan) {}
    pub(crate) fn execution_gate() -> Result<(crate::platform::Lock, crate::platform::Lock)> {
        bail!("Update service is not compiled into this manager")
    }
}
pub mod transaction {
    use super::*;
    pub use crate::manager_updates::api::ExpectedIdentity;
    #[derive(Clone, Debug)]
    pub enum SignedSelection {}
    pub(crate) fn reject_file(path: &Path) -> Result<()> {
        crate::files::no_links(path)?;
        anyhow::ensure!(
            std::fs::metadata(path)?.len() <= 1024 * 1024,
            "Update plan too large"
        );
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
        anyhow::ensure!(
            ![
                "schema",
                "envelope",
                "offer",
                "profile",
                "legacy",
                "authorization"
            ]
            .iter()
            .any(|key| value.get(key).is_some()),
            "Update service authorization requires a service-enabled manager"
        );
        let plan: Plan = serde_json::from_value(value)?;
        reject(&plan, None)
    }
    pub(crate) fn launch_v2(_: &Path) -> Result<()> {
        bail!("Update service is not compiled into this manager")
    }
    pub(crate) fn apply_v2(_: &Path) -> Result<()> {
        bail!("Update service is not compiled into this manager")
    }
    pub(crate) fn authorize_plan(_: &Path, _: &Plan) -> Result<()> {
        bail!("Update service is not compiled into this manager")
    }
    pub(crate) fn verify_executable(_: &Path, _: &ExpectedIdentity, _: &Path) -> Result<()> {
        bail!("Update service is not compiled into this manager")
    }
    #[cfg(target_os = "windows")]
    pub(crate) fn install_native_batch(
        plan: &Plan,
        authorization: Option<&Path>,
    ) -> Result<Option<(i32, String)>> {
        reject(plan, authorization)?;
        Ok(None)
    }
    #[cfg(target_os = "windows")]
    pub(crate) fn acknowledge_native_batch(
        plan: &Plan,
        authorization: Option<&Path>,
    ) -> Result<()> {
        reject(plan, authorization)
    }
    pub(crate) fn reject(plan: &Plan, authorization: Option<&Path>) -> Result<()> {
        anyhow::ensure!(
            plan.expected_identity.is_none() && plan.layout < 2 && authorization.is_none(),
            "Update service authorization requires a service-enabled manager"
        );
        Ok(())
    }
    #[cfg(target_os = "macos")]
    pub(crate) fn verify_macos_admin_plan(_: &Plan, _: &Path) -> Result<()> {
        bail!("Update service is not compiled into this manager")
    }
    #[cfg(target_os = "macos")]
    pub(crate) fn install_macos_batch(_: &Plan, _: &Path) -> Result<()> {
        bail!("Update service is not compiled into this manager")
    }
    #[cfg(target_os = "macos")]
    pub(crate) fn acknowledge_macos_batch(_: &Plan, _: &Path) -> Result<()> {
        bail!("Update service is not compiled into this manager")
    }
}
pub mod native_admin {
    use super::*;
    pub fn worker(_: &Path, _: &str) -> Result<()> {
        bail!("Update service is not compiled into this manager")
    }
    pub fn cancel(_: PathBuf) {}
    #[cfg(target_os = "macos")]
    pub(crate) fn revoke(_: &Path) {}
}
pub mod check_state {
    pub use crate::manager_updates::api::CheckTrigger;
}
