//! Preview decisions use metadata only. Actual installers are independently checked.
use super::*;
pub fn downgrade_scope_decision(
    root: &Path,
    installed: &crate::model::Installed,
    release: &Release,
    preferences: &Preferences,
) -> crate::installations::release_catalog::CompatibilityDecision {
    use crate::installations::release_catalog::{inspection, CompatibilityDecision as Decision};
    if installed.install_kind != "installer"
        || installed.path.is_empty()
        || !version(&release.tag_name)
            .ok()
            .zip(version(&installed.version).ok())
            .is_some_and(|(target, current)| target < current)
    {
        return Decision::NotApplicable;
    }
    let Ok(prefs) = crate::install_options::for_installed(preferences, installed) else {
        return Decision::Unknown("Installation package choices are unavailable.".into());
    };
    let Ok(asset) = select_asset(release, &installed.name, &prefs) else {
        return Decision::Unknown("Package information is unavailable.".into());
    };
    inspection::decision(
        root,
        &crate::model::repository(&installed.name),
        &release.tag_name,
        asset,
        &prefs.architecture,
        installed.windows_msi_scope.as_deref(),
    )
}
pub fn downgrade_scope_error(
    root: &Path,
    installed: &crate::model::Installed,
    release: &Release,
    preferences: &Preferences,
) -> Option<String> {
    match downgrade_scope_decision(root, installed, release, preferences) {
        crate::installations::release_catalog::CompatibilityDecision::Incompatible(reason) => {
            Some(reason)
        }
        _ => None,
    }
}
pub fn check_installer_scope_preview(
    paths: &Paths,
    plan: &mut ReleasePlan,
    _job: &Job,
) -> Result<()> {
    if !plan.installer_downgrade() {
        return Ok(());
    }
    let (_, config, _) =
        serde_json::from_slice::<(Preferences, crate::model::Config, serde_json::Value)>(
            &plan.snapshot,
        )?;
    let entry = &mut plan.entries[0];
    let installed = config
        .apps
        .iter()
        .find(|a| a.name == entry.app)
        .context("Selected installation missing")?;
    let asset = entry.asset.as_ref().context("Missing installer")?;
    if let crate::installations::release_catalog::CompatibilityDecision::Incompatible(error) =
        crate::installations::release_catalog::inspection::decision(
            &paths.root,
            &crate::model::repository(&entry.app),
            plan.requested_tag.as_deref().unwrap_or(&entry.version),
            asset,
            &entry.preferences.architecture,
            installed.windows_msi_scope.as_deref(),
        )
    {
        entry.action = "Error".into();
        entry.error = Some(error);
    }
    Ok(())
}
