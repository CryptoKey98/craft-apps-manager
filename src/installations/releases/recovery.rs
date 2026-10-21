use super::*;

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

#[cfg(target_os = "windows")]
pub(crate) fn prepare_downgrade_recovery(
    paths: &Paths,
    app: &crate::model::Installed,
    job: &Job,
) -> Result<(Asset, PathBuf)> {
    let network = Network::new(&paths.root)?;
    // Stage and verify recovery before removing the current version.
    let releases: Vec<Release> = network.json(&format!(
        "https://api.github.com/repos/{}/releases?per_page=100",
        crate::model::repository(&app.name)
    ))?;
    let current_release = releases
        .into_iter()
        .find(|r| {
            !r.draft
                && !r.prerelease
                && release_version(&r.tag_name).is_ok_and(|v| v == app.version)
        })
        .context("Cannot safely downgrade: the installed version is unavailable for recovery")?;
    let recovery_plan = plan_selected_version(paths, job, &app.name, current_release)?;
    let recovery_asset = recovery_plan
        .entries
        .first()
        .and_then(|e| {
            if e.error.is_none() {
                e.asset.as_ref()
            } else {
                None
            }
        })
        .context("Cannot safely downgrade: no verified recovery installer")?;
    anyhow::ensure!(
        recovery_asset.name.ends_with(".msi"),
        "Recovery requires an MSI installer"
    );
    let recovery = paths.at(format!(
        "releases/installers/{}/{}",
        app.name, recovery_asset.name
    ));
    if recovery.exists() {
        crate::network::verify_asset(&recovery, recovery_asset)?;
    } else {
        network.asset(recovery_asset, &recovery, job)?;
    }
    Ok((recovery_asset.clone(), recovery))
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
        let file = paths.at(format!("releases/installers/{}/{}", app.name, asset.name));
        crate::network::verify_asset(&file, asset)?;
        return Ok(file);
    }
    prepare_downgrade_recovery(paths, app, job).map(|(_, file)| file)
}
