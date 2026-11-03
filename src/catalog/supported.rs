//! Explicitly supported Craft sources outside the main organization.
//! Stable bare keys retain the managed-app behavior; only repository routing changes.
pub(crate) struct ExternalApp {
    pub key: &'static str,
    pub repository: &'static str,
    pub category: &'static str,
    pub aliases: &'static [&'static str],
    pub bundle_ids: &'static [&'static str],
    pub icon_path: &'static str,
}

pub(crate) const EXTERNAL_APPS: &[ExternalApp] = &[ExternalApp {
    key: "solidcraft",
    repository: "tangency-labs/solidcraft",
    category: "Parametric 3D CAD",
    // SolidCraft's former executable name; its upstream MSI retains that identity.
    aliases: &["solvecraft"],
    bundle_ids: &["io.tangencylabs.solidcraft"],
    icon_path: "assets/icons/app/linux/exports/hicolor/64x64/apps/io.tangencylabs.solidcraft.png",
}];

/// Resolve managed keys without changing explicit user-added repository slugs.
pub fn repository_slug(value: &str) -> String {
    if value.contains('/') {
        value.into()
    } else if let Some(app) = EXTERNAL_APPS
        .iter()
        .find(|app| app.key.eq_ignore_ascii_case(value))
    {
        app.repository.into()
    } else {
        format!("{}/{value}", super::ORG)
    }
}
