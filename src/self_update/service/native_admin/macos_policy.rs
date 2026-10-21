//! Exact signed scope for bundle replacement, separate from native installers.
use super::*;

pub(super) const CAPABILITY: &str = "admin.bundle-replace.v1";

pub(super) fn packages<'a>(
    catalog: &'a VerifiedCatalog,
    profile: &ClientProfile,
    offer: &UpdateOffer,
) -> Result<Vec<&'a PackageRecord>> {
    use super::super::model::{DMG, TAR, ZIP};
    ensure!(
        profile.platform == "macos"
            && profile.installation == InstallationKind::Bundle
            && profile.capabilities.contains(CAPABILITY)
            && (1..=8).contains(&offer.route_ids.len()),
        "Not a supported macOS bundle approval"
    );
    // Validate the complete server selection before applying any single-step limit.
    super::super::transaction::validate_offer(catalog, profile, offer)?;
    offer
        .route_ids
        .iter()
        .map(|id| {
            let route = catalog
                .catalog()
                .routes
                .iter()
                .find(|r| &r.id == id)
                .context("Missing macOS route")?;
            let package = catalog
                .catalog()
                .packages
                .iter()
                .find(|p| p.id == route.target)
                .context("Missing macOS package")?;
            ensure!(
                package.platform == "macos"
                    && package.installation == InstallationKind::Bundle
                    && package.architecture == profile.architecture
                    && matches!(package.handler.as_str(), ZIP | TAR | DMG)
                    && profile.capabilities.contains(&package.handler)
                    && package
                        .required_capabilities
                        .is_subset(&profile.capabilities),
                "macOS bundle needs an unsupported capability; separate approval is required"
            );
            Ok(package)
        })
        .collect()
}
