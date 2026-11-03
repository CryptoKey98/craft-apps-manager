use super::*;

/// Keep different versions of identically named setup assets from overwriting
/// one another while an update and its recovery package are staged together.
pub(crate) fn installer_package_path(paths: &Paths, app: &str, asset: &Asset) -> Result<PathBuf> {
    let [legacy, addressed] = installer_cache_paths(paths, app, asset)?;
    if legacy.is_file() && crate::network::verify_asset(&legacy, asset).is_ok() {
        return Ok(legacy);
    }
    Ok(addressed)
}

pub(crate) fn installer_cache_paths(
    paths: &Paths,
    app: &str,
    asset: &Asset,
) -> Result<[PathBuf; 2]> {
    crate::model::valid_app(app)?;
    files::safe_relative(&asset.name)?;
    anyhow::ensure!(
        Path::new(&asset.name).components().count() == 1,
        "Invalid installer filename"
    );
    let digest = asset
        .digest
        .as_deref()
        .and_then(|d| d.strip_prefix("sha256:"))
        .context("Installer has no published SHA-256 digest")?;
    anyhow::ensure!(
        digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid installer digest"
    );
    let legacy = paths.at(format!("releases/installers/{app}/{}", asset.name));
    let name = asset.name.to_ascii_lowercase();
    let suffix = [".pkg.tar.zst", ".msi", ".exe", ".deb", ".rpm", ".dmg"]
        .into_iter()
        .find(|suffix| name.ends_with(suffix))
        .context("Unsupported installer cache format")?;
    Ok([
        legacy,
        paths.at(format!(
            "releases/installers/{app}/{}{suffix}",
            digest.to_ascii_lowercase()
        )),
    ])
}

pub(crate) fn planned_installer_path(
    paths: &Paths,
    plan: &ReleasePlan,
    app: &str,
    asset: &Asset,
) -> Result<PathBuf> {
    if let Some(id) = &plan.native_rollback {
        return crate::installations::rollbacks::native::target_path(paths, app, id, asset);
    }
    installer_package_path(paths, app, asset)
}

// Commit the filesystem and metadata together. Restore the old copy if metadata cannot be saved.
pub fn replace_transaction(
    staged: &Path,
    destination: &Path,
    backup: Option<&Path>,
    commit: impl FnOnce() -> Result<()>,
) -> Result<()> {
    if destination.exists() {
        let b = backup.context("Missing rollback path")?;
        fs::create_dir_all(b.parent().unwrap())?;
        files::move_verified(destination, b)?;
    }
    fs::create_dir_all(destination.parent().unwrap())?;
    if let Err(e) = files::move_verified(staged, destination) {
        if let Some(b) = backup {
            if b.exists() {
                files::move_verified(b, destination).context("Could not restore rollback copy")?;
            }
        }
        return Err(e);
    }
    if let Err(e) = commit() {
        files::move_verified(destination, staged)
            .context("Could not remove uncommitted replacement")?;
        if let Some(b) = backup {
            if b.exists() {
                files::move_verified(b, destination)
                    .context("Could not restore previous version")?;
            }
        }
        return Err(e);
    }
    Ok(())
}

#[cfg(any(target_os = "windows", test))]
pub(super) fn replace_installer_transaction<T>(
    remove: impl FnOnce() -> Result<()>,
    install: impl FnOnce() -> Result<T>,
    restore: impl FnOnce() -> Result<()>,
) -> Result<T> {
    match remove().and_then(|()| install()) {
        Ok(installed) => Ok(installed),
        Err(error) => match restore() {
            Ok(()) => Err(error.context("Previous installation restored")),
            Err(recovery) => Err(anyhow::anyhow!(
                "Installation failed: {error:#}. Recovery also failed: {recovery:#}"
            )),
        },
    }
}

/// Windows registrations may append a zero fourth component to the release version.
/// Prefer the exact release spelling; numeric fallback must identify one stable release.
pub(crate) fn recovery_release(releases: Vec<Release>, installed: &str) -> Result<Release> {
    let installed_numeric = version(installed).context(
        "Cannot safely downgrade: the installed version cannot be compared for recovery",
    )?;
    let mut candidates: Vec<_> = releases
        .into_iter()
        .filter(|release| {
            !release.draft
                && !release.prerelease
                && version(&release.tag_name).is_ok_and(|value| value == installed_numeric)
        })
        .collect();
    if let Some(index) = candidates.iter().position(|release| {
        release_version(&release.tag_name).is_ok_and(|value| value == installed)
    }) {
        return Ok(candidates.remove(index));
    }
    anyhow::ensure!(
        candidates.len() <= 1,
        "Cannot safely downgrade: multiple releases match the installed version for recovery"
    );
    candidates
        .pop()
        .context("Cannot safely downgrade: the installed version is unavailable for recovery")
}

#[cfg(target_os = "windows")]
pub(crate) fn prepare_downgrade_recovery(
    paths: &Paths,
    app: &crate::model::Installed,
    job: &Job,
) -> Result<(Asset, PathBuf)> {
    let prepared = crate::installations::rollbacks::native::prepare_current(paths, app, job, None)?;
    Ok((prepared.asset, prepared.package))
}

#[cfg(target_os = "windows")]
pub(super) fn downgrade_recovery_path(
    paths: &Paths,
    app: &crate::model::Installed,
    job: &Job,
    approved: Option<&std::collections::BTreeMap<String, Asset>>,
) -> Result<PathBuf> {
    if let Some(approved) = approved {
        let asset = approved
            .get(&app.name)
            .context("Missing approved recovery installer")?;
        crate::model::valid_app(&app.name)?;
        files::safe_relative(&asset.name)?;
        anyhow::ensure!(
            asset.name.to_ascii_lowercase().ends_with(".msi"),
            "Recovery requires an MSI installer"
        );
        let file = installer_package_path(paths, &app.name, asset)?;
        crate::network::verify_asset(&file, asset)?;
        crate::installers::validate_msi_replacement_scope(&file, app)?;
        return Ok(file);
    }
    prepare_downgrade_recovery(paths, app, job).map(|(_, file)| file)
}

#[cfg(test)]
mod version_tests {
    use super::*;
    fn release(tag: &str) -> Release {
        serde_json::from_value(
            serde_json::json!({"tag_name":tag,"draft":false,"prerelease":false,"assets":[]}),
        )
        .unwrap()
    }
    #[test]
    fn recovery_matches_windows_trailing_zero_versions_but_rejects_other_builds() {
        assert_eq!(
            recovery_release(vec![release("4.5.0")], "4.5.0.0")
                .unwrap()
                .tag_name,
            "4.5.0"
        );
        assert!(recovery_release(vec![release("4.5.0")], "4.5.0.1").is_err());
        assert!(recovery_release(vec![release("4.4.0")], "4.5.0.0").is_err());
        assert!(recovery_release(vec![release("4.5.0")], "invalid").is_err());
        let mut preview = release("4.5.0");
        preview.prerelease = true;
        assert!(recovery_release(vec![preview], "4.5.0.0").is_err());
    }
    #[test]
    fn recovery_prefers_exact_versions_and_rejects_ambiguous_numeric_matches() {
        assert_eq!(
            recovery_release(vec![release("4.5.0"), release("4.5.0.0")], "4.5.0.0")
                .unwrap()
                .tag_name,
            "4.5.0.0"
        );
        assert!(recovery_release(vec![release("v4.5.0"), release("4.5")], "4.5.0.0").is_err());
        let mut draft = release("4.5.0");
        draft.draft = true;
        assert!(recovery_release(vec![draft], "4.5.0.0").is_err());
    }
}
