use super::*;

pub fn rollback(
    paths: &Paths,
    expected: &Installed,
    point: &RollbackPoint,
    job: &Job,
) -> Result<()> {
    if expected.install_kind == "installer" {
        return native::restore(paths, expected, point, job);
    }
    let _lock = crate::platform::Lock::take("Local\\CraftAppsManager")?;
    restore_with(paths, expected, point, job, |config| {
        paths.save_config(config)
    })
}
pub(super) fn restore_with(
    paths: &Paths,
    expected: &Installed,
    point: &RollbackPoint,
    job: &Job,
    commit: impl FnOnce(&crate::model::Config) -> Result<()>,
) -> Result<()> {
    let current = selected(paths, expected)?;
    let retained = verify_point(paths, &current, point)?;
    files::no_links(Path::new(&current.path))?;
    ensure!(
        !crate::platform::running_app(&current.name)?,
        "Close the app before rolling back"
    );
    let recovery = staging_path(paths);
    let stage = recovery
        .parent()
        .context("Missing transaction folder")?
        .join("staged");
    job.stage(
        "Preparing rollback",
        None,
        "Copying and verifying the retained version locally",
    );
    let prepared = if point.compressed {
        compression::verify(&retained, point, Some(&stage), Some(job))
    } else {
        files::copy_directory_verified(&retained, &stage)
    }
    .and_then(|()| job.check());
    if let Err(error) = prepared {
        let _ = files::remove_managed(&stage, &paths.at("runtime/rollback-transactions"));
        cleanup_empty(paths, job);
        return Err(error);
    }
    let result = (|| -> Result<()> {
        // Keep current ownership/shortcut metadata and the current portable profile.
        fs::copy(
            Path::new(&current.path).join(portable::MARKER),
            stage.join(portable::MARKER),
        )?;
        if current.name == "photocraft" {
            let profile = Path::new(&current.path).join("PhotoCraftData");
            let dest = stage.join("PhotoCraftData");
            if dest.exists() {
                files::remove_managed(&dest, &stage)?;
            }
            if profile.exists() {
                files::no_links(&profile)?;
                files::copy_directory_verified(&profile, &dest)?;
            }
        }
        let exe = stage.join(&point.executable);
        if exe.is_file() {
            ensure!(
                files::hash(&exe)?
                    == point
                        .files
                        .get(&point.executable)
                        .context("Missing executable verification")?
                        .1,
                "Staged executable verification failed"
            );
        } else {
            ensure!(
                exe.is_dir() && point.files.keys().any(|p| p.starts_with(&point.executable)),
                "Staged executable is missing"
            );
        }
        let mut restored = current.clone();
        restored.version = point.version.clone();
        restored.architecture = point.architecture.clone();
        restored.package_type = point.package_type.clone();
        let mut config = paths
            .read_inventory()?
            .context("Installation inventory is missing")?;
        let record = config
            .apps
            .iter_mut()
            .find(|a| a.name == current.name)
            .context("Missing installation")?;
        *record = restored.clone();
        ensure!(
            !crate::platform::running_app(&current.name)?,
            "Close the app before rolling back"
        );
        job.check()?;
        crate::updates::replace_transaction(
            &stage,
            Path::new(&current.path),
            Some(&recovery),
            || commit(&config),
        )?;
        // Consuming the point leaves the just-replaced current version as recovery until cleanup succeeds.
        let directory = root(paths, &point.installation_id)?;
        if let Err(error) = fs::remove_file(directory.join("point.json")) {
            job.log(&format!("Rollback metadata cleanup warning: {error}"));
            return Ok(());
        }
        for (path, base) in [
            (&retained, &directory),
            (&recovery, &paths.at("runtime/rollback-transactions")),
        ] {
            if let Err(error) = files::remove_managed(path, base) {
                job.log(&format!("Rollback cleanup warning: {error:#}"));
            }
        }
        job.log(&format!("Restored {} {}", current.name, point.version));
        Ok(())
    })();
    if result.is_err() {
        if recovery.exists() {
            job.log(&format!(
                "Rollback recovery retained at {}",
                recovery.display()
            ));
        } else {
            let _ = files::remove_managed(&stage, &paths.at("runtime/rollback-transactions"));
        }
    }
    if let Err(error) = cleanup::prune_empty(paths) {
        job.log(&format!("Empty rollback folder cleanup warning: {error:#}"));
    }
    result
}
