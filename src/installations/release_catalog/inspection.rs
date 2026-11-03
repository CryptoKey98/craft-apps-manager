use super::*;
use anyhow::{Context, Result};
use std::path::Path;
pub const INSPECTOR_VERSION: u32 = 1;
pub fn policy(all_users: Option<&str>) -> ScopePolicy {
    match all_users.filter(|s| !s.is_empty()) {
        None => ScopePolicy::UserOnly,
        Some("1") => ScopePolicy::MachineOnly,
        Some("2") => ScopePolicy::Dual,
        _ => ScopePolicy::Unknown,
    }
}
pub fn inspect(
    package: &Path,
    repo: &str,
    tag: &str,
    asset: &crate::model::Asset,
    architecture: &str,
) -> Result<ScopeEvidence> {
    let _ = verification::canonical_repo(repo)?;
    anyhow::ensure!(
        asset.name.to_ascii_lowercase().ends_with(".msi"),
        "Scope inspection supports MSI only"
    );
    crate::files::no_links(package)?;
    crate::network::verify_asset(package, asset)?;
    let policy = read_policy(package)?;
    Ok(ScopeEvidence {
        tag: tag.into(),
        asset_name: asset.name.clone(),
        digest: asset.digest.clone().context("Package has no digest")?,
        size: asset.size,
        os: "windows".into(),
        architecture: crate::model::release_arch(architecture).into(),
        policy,
        inspector_version: INSPECTOR_VERSION,
    })
}
pub fn observe(
    root: &Path,
    repo: &str,
    asset: &crate::model::Asset,
    architecture: &str,
    package: &Path,
) -> Result<()> {
    if !asset.name.to_ascii_lowercase().ends_with(".msi") {
        return Ok(());
    }
    let url = reqwest::Url::parse(&asset.browser_download_url)?;
    let parts: Vec<_> = url
        .path_segments()
        .context("Missing package path")?
        .collect();
    let tag = parts
        .windows(3)
        .find(|s| s[0] == "releases" && s[1] == "download")
        .and_then(|s| provider::decode_tag(s[2]))
        .context("Missing package tag")?;
    let repo = verification::canonical_repo(repo)?;
    anyhow::ensure!(
        crate::updates::expected_asset_url(&asset.browser_download_url, &repo),
        "Unexpected package repository"
    );
    let evidence = inspect(package, &repo, &tag, asset, architecture)?;
    let _lock = store::lock(root)?;
    let mut state = store::load(root, provider::source())?;
    state.observations.retain(|o| {
        !(o.repository == repo
            && o.evidence.tag == tag
            && o.evidence.asset_name == asset.name
            && o.evidence.architecture == evidence.architecture)
    });
    state.observations.push(store::Observation {
        repository: repo.clone(),
        evidence,
        used: chrono::Utc::now().timestamp(),
    });
    store::save(root, &mut state, &repo)?;
    store::publish_memory(root, store::snapshot(&state, provider::source())?);
    Ok(())
}
pub fn decision(
    root: &Path,
    repo: &str,
    tag: &str,
    asset: &crate::model::Asset,
    architecture: &str,
    scope: Option<&str>,
) -> CompatibilityDecision {
    if !cfg!(target_os = "windows") || !asset.name.to_ascii_lowercase().ends_with(".msi") {
        return CompatibilityDecision::NotApplicable;
    }
    let unknown = || {
        CompatibilityDecision::Unknown(
            "Installer compatibility will be verified before installation changes.".into(),
        )
    };
    let Some(scope) = scope.filter(|s| matches!(*s, "user" | "machine")) else {
        return unknown();
    };
    let Ok(repo) = verification::canonical_repo(repo) else {
        return unknown();
    };
    let Some(snapshot) = store::peek(root) else {
        return unknown();
    };
    let Some(digest) = asset.digest.as_ref() else {
        return unknown();
    };
    let key = (
        repo,
        tag.into(),
        asset.name.clone(),
        digest.clone(),
        asset.size,
        crate::model::release_arch(architecture).into(),
    );
    match snapshot.scope.get(&key) {
        Some(ScopePolicy::Dual) => CompatibilityDecision::Compatible,
        Some(ScopePolicy::UserOnly) if scope == "user" => CompatibilityDecision::Compatible,
        Some(ScopePolicy::MachineOnly) if scope == "machine" => CompatibilityDecision::Compatible,
        Some(ScopePolicy::UserOnly) => CompatibilityDecision::Incompatible(
            "This version only supports installing for your Windows account. Your copy is installed for everyone on this computer, so this downgrade isn’t supported.".into(),
        ),
        Some(ScopePolicy::MachineOnly) => CompatibilityDecision::Incompatible(
            "This version only supports installing for everyone on this computer. Your copy is installed just for you, so this downgrade isn’t supported.".into(),
        ),
        _ => unknown(),
    }
}

#[cfg(target_os = "windows")]
fn read_policy(package: &Path) -> Result<ScopePolicy> {
    Ok(policy(
        crate::installers::msi_authored_all_users(package)?.as_deref(),
    ))
}
#[cfg(not(target_os = "windows"))]
fn read_policy(_package: &Path) -> Result<ScopePolicy> {
    anyhow::bail!("MSI scope inspection requires Windows")
}
