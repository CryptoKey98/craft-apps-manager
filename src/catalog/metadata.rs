use anyhow::{bail, Result};

/// Apps with installable releases, from the current catalog.
pub fn apps() -> Vec<String> {
    crate::catalog::release_keys()
}

/// Keeps saved ordering across formats, displaying only apps registered for this format.
pub fn ordered_release_apps(order: &[String], format: &str) -> Vec<String> {
    let _ = format;
    let eligible: std::collections::BTreeSet<_> = crate::catalog::known_entries()
        .into_iter()
        .filter(|entry| entry.release)
        .map(|entry| entry.key)
        .collect();
    order
        .iter()
        .filter(|name| eligible.contains(*name))
        .cloned()
        .collect()
}

/// Apps whose source can be downloaded and built, from the current catalog.
pub fn sources() -> Vec<String> {
    crate::catalog::source_keys()
}

pub fn title(name: &str) -> String {
    crate::catalog::get(name)
        .map(|e| e.title)
        .unwrap_or_else(|| crate::catalog::pretty(name))
}

/// The upstream repository as an `owner/name` slug. Storytold apps store a
/// bare name in the catalog; third-party apps store a full slug.
pub fn repository(name: &str) -> String {
    crate::catalog::get(name)
        .map(|e| {
            if e.repository.contains('/') {
                e.repository
            } else {
                format!("{}/{}", crate::catalog::ORG, e.repository)
            }
        })
        .unwrap_or_else(|| format!("{}/{}", crate::catalog::ORG, name))
}

/// The bare repository name, without any `owner/` prefix. Used where a single
/// path segment or file name is required, such as source ZIP folder names.
pub fn repository_name(name: &str) -> String {
    crate::catalog::get(name)
        .map(|e| crate::catalog::repository_name(&e.repository).to_string())
        .unwrap_or_else(|| name.into())
}

/// Short category shown under each app, taken from the upstream repository description.
pub fn category(name: &str) -> String {
    crate::catalog::get(name)
        .map(|e| e.category)
        .unwrap_or_default()
}

/// The upstream repository description, shown on each app page.
pub fn description(name: &str) -> String {
    crate::catalog::get(name)
        .map(|e| e.description)
        .unwrap_or_default()
}

pub fn valid_app(name: &str) -> Result<()> {
    if crate::catalog::get(name).is_none() {
        bail!("Unknown app: {name}");
    }
    Ok(())
}
