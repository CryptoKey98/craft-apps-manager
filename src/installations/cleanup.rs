//! Cleanup is part of a verified installation, never a separate destructive job.
use crate::{
    files,
    jobs::Job,
    model::{Installed, Paths},
};
use anyhow::{Context, Result};
use std::path::Path;

fn remove_download(
    paths: &Paths,
    installed: &Installed,
    package: &Path,
    retention: impl FnOnce() -> Result<Option<String>>,
) -> Result<String> {
    crate::model::valid_app(&installed.name)?;
    anyhow::ensure!(
        installed.install_kind == "installer" && !installed.path.is_empty(),
        "Installation must be verified before cleanup"
    );
    let cache = paths.at(format!("releases/installers/{}", installed.name));
    files::inside(package, &cache)?;
    anyhow::ensure!(
        package.parent() == Some(cache.as_path()),
        "Cleanup only accepts a downloaded package in this app's cache"
    );
    anyhow::ensure!(
        !Path::new(&installed.path).starts_with(&cache),
        "Installed application overlaps the download cache"
    );
    if let Some(reason) = retention()? {
        return Ok(format!("Installer retained: {reason}"));
    }
    if package.exists() {
        anyhow::ensure!(
            package.is_file(),
            "Downloaded package is not a regular file"
        );
        std::fs::remove_file(package).context("Could not remove the downloaded installer")?;
    }
    // Remove only this app's empty download directory. A non-recursive removal
    // preserves retained packages, recovery files and concurrent downloads.
    match std::fs::remove_dir(&cache) {
        Ok(()) => {}
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
            ) => {}
        Err(error) => {
            return Err(error).context(
                "Setup file was removed, but its empty download folder could not be removed",
            )
        }
    }
    Ok("Downloaded installer removed.".into())
}

/// Legacy downloads are eligible only when cached GitHub metadata proves their
/// exact name, repository, size and SHA-256. Unknown files remain untouched.
fn cached_package_assets(paths: &Paths, app: &str) -> Vec<crate::model::Asset> {
    let repository = crate::model::repository(app);
    let base = format!("https://api.github.com/repos/{repository}/releases");
    let mut releases = crate::network::Network::cached_json::<Vec<crate::model::Release>>(
        &paths.root,
        &format!("{base}?per_page=100"),
    )
    .unwrap_or_default();
    if let Some(latest) = crate::network::Network::cached_json::<crate::model::Release>(
        &paths.root,
        &format!("{base}/latest"),
    ) {
        releases.push(latest);
    }
    releases
        .into_iter()
        .filter(|release| !release.draft && !release.prerelease)
        .flat_map(|release| release.assets)
        .filter(|asset| {
            crate::updates::expected_asset_url(&asset.browser_download_url, &repository)
        })
        .collect()
}

fn prune_verified_downloads(
    paths: &Paths,
    installed: &Installed,
    assets: &[crate::model::Asset],
    mut retention: impl FnMut(&Path) -> Result<Option<String>>,
) -> Result<()> {
    for asset in assets {
        let name = Path::new(&asset.name);
        if name.components().count() != 1 || files::safe_relative(&asset.name).is_err() {
            continue;
        }
        let Ok(candidates) = crate::updates::installer_cache_paths(paths, &installed.name, asset)
        else {
            continue;
        };
        for package in candidates {
            if !package.exists() {
                continue;
            }
            files::inside(
                &package,
                &paths.at(format!("releases/installers/{}", installed.name)),
            )?;
            if !package.is_file() || crate::network::verify_asset(&package, asset).is_err() {
                continue;
            }
            remove_download(paths, installed, &package, || retention(&package))?;
        }
    }
    Ok(())
}

fn prune_cached_downloads(paths: &Paths, installed: &Installed) -> Result<()> {
    let assets = cached_package_assets(paths, &installed.name);
    prune_verified_downloads(paths, installed, &assets, |package| {
        #[cfg(target_os = "windows")]
        {
            // A historical EXE might maintain another native copy. Only the exact
            // successful EXE is handled by finish's existing registration check.
            if !package
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("msi"))
            {
                return Ok(Some(
                    "Historical installer maintenance requirements are unknown.".into(),
                ));
            }
            crate::installers::download_retention_reason(package, installed)
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = package;
            Ok(None)
        }
    })
}

/// Called only after native uninstall and the updated inventory have both succeeded,
/// under the existing installation operation lock.
pub(crate) fn after_uninstall(
    paths: &Paths,
    removed: &Installed,
    config: &crate::model::Config,
) -> Result<()> {
    if config.apps.iter().chain(&config.installations).any(|copy| {
        copy.name == removed.name && copy.install_kind == "installer" && !copy.path.is_empty()
    }) || crate::installers::detect(&removed.name)?.is_some()
    {
        return Ok(());
    }
    prune_cached_downloads(paths, removed)
}

/// A cleanup warning must not turn a verified, persisted installation into a failure.
pub(crate) fn finish(paths: &Paths, installed: &Installed, package: &Path, job: &Job) {
    finish_with_retention(paths, installed, package, job, || {
        #[cfg(target_os = "windows")]
        {
            crate::installers::download_retention_reason(package, installed)
        }
        #[cfg(not(target_os = "windows"))]
        {
            Ok(None)
        }
    });
    if let Err(error) = prune_cached_downloads(paths, installed) {
        job.log(&format!("Older setup-file cleanup warning: {error:#}"));
    }
}

fn finish_with_retention(
    paths: &Paths,
    installed: &Installed,
    package: &Path,
    job: &Job,
    retention: impl FnOnce() -> Result<Option<String>>,
) {
    job.stage("Cleaning up", None, "Checking downloaded setup files");
    let result = remove_download(paths, installed, package, retention);
    match result {
        Ok(message) => {
            job.report("Cleanup", &message);
            job.log(&message);
        }
        Err(error) => {
            let message =
                format!("Installation succeeded; setup-file cleanup could not finish: {error:#}");
            job.report("Cleanup", &message);
            job.log(&message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (std::path::PathBuf, Paths, Installed, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("craft-cleanup-{}", uuid::Uuid::new_v4()));
        let paths = Paths::new(root.clone(), None);
        let app = Installed {
            name: "filmcraft".into(),
            path: root.join("installed").display().to_string(),
            install_kind: "installer".into(),
            ..Default::default()
        };
        let package = paths.at("releases/installers/filmcraft/setup.exe");
        std::fs::create_dir_all(package.parent().unwrap()).unwrap();
        std::fs::write(&package, b"test package").unwrap();
        (root, paths, app, package)
    }
    #[test]
    fn removes_only_verified_package_and_preserves_other_versions_and_apps() {
        let (root, paths, app, package) = fixture();
        let recovery = package.with_file_name("recovery.msi");
        std::fs::write(&recovery, b"recovery").unwrap();
        let other = paths.at("releases/installers/designcraft/setup.exe");
        std::fs::create_dir_all(other.parent().unwrap()).unwrap();
        std::fs::write(&other, b"other app").unwrap();
        assert!(remove_download(&paths, &app, &other, || Ok(None)).is_err());
        remove_download(&paths, &app, &package, || Ok(None)).unwrap();
        assert!(!package.exists());
        assert!(recovery.exists() && other.exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn empty_app_download_folder_is_removed_and_can_be_recreated() {
        let (root, paths, app, package) = fixture();
        let cache = package.parent().unwrap();
        let installers = paths.at("releases/installers");
        remove_download(&paths, &app, &package, || Ok(None)).unwrap();
        assert!(!cache.exists());
        assert!(installers.is_dir());
        // Repeated cleanup is harmless; the next download recreates its folder.
        remove_download(&paths, &app, &package, || Ok(None)).unwrap();
        std::fs::create_dir_all(cache).unwrap();
        std::fs::write(&package, b"next installer").unwrap();
        assert_eq!(std::fs::read(&package).unwrap(), b"next installer");
        remove_download(&paths, &app, &package, || Ok(None)).unwrap();
        assert!(!cache.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_packages_and_uncertain_dependencies_are_not_deleted() {
        let (root, paths, app, package) = fixture();
        assert!(remove_download(&paths, &app, &package, || Ok(Some(
            "required for repair".into()
        )))
        .unwrap()
        .contains("retained"));
        assert!(remove_download(&paths, &app, &package, || anyhow::bail!(
            "unknown dependencies"
        ))
        .is_err());
        assert!(package.exists());
        let mut unknown = app.clone();
        unknown.path.clear();
        assert!(remove_download(&paths, &unknown, &package, || Ok(None)).is_err());
        assert!(package.exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cleanup_cannot_delete_application_files_or_directories() {
        let (root, paths, mut app, package) = fixture();
        app.path = package.parent().unwrap().display().to_string();
        assert!(remove_download(&paths, &app, &package, || Ok(None)).is_err());
        app.path = root.join("installed").display().to_string();
        assert!(remove_download(&paths, &app, package.parent().unwrap(), || Ok(None)).is_err());
        let nested = package.parent().unwrap().join("nested/setup.exe");
        assert!(remove_download(&paths, &app, &nested, || Ok(None)).is_err());
        assert!(package.exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cleanup_is_in_install_activity_and_warnings_preserve_success() {
        for warning in [false, true] {
            let (root, paths, app, package) = fixture();
            let job = Job::new(root.join("logs/install.log"), &Default::default());
            job.tracked("Install", "filmcraft", |job| {
                finish_with_retention(&paths, &app, &package, job, || {
                    if warning {
                        anyhow::bail!("maintenance registration unavailable");
                    }
                    Ok(None)
                });
                Ok(())
            })
            .unwrap();
            let records = crate::operations::Manager::new(&root).list();
            assert_eq!(records.len(), 1);
            assert_eq!(
                records[0].lifecycle,
                crate::operations::Lifecycle::Completed
            );
            assert_eq!(package.exists(), warning);
            assert!(records[0].facts["Cleanup"].contains(if warning {
                "Installation succeeded"
            } else {
                "removed"
            }));
            std::fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn stale_download_cleanup_requires_exact_published_content_and_preserves_unknown_files() {
        use sha2::{Digest, Sha256};
        let (root, paths, app, _) = fixture();
        let cache = paths.at("releases/installers/filmcraft");
        let package = cache.join("old.msi");
        std::fs::write(&package, b"owned").unwrap();
        let unknown = cache.join("personal.msi");
        std::fs::write(&unknown, b"personal").unwrap();
        let asset = crate::model::Asset {
            name: "old.msi".into(),
            size: 5,
            browser_download_url: "https://github.com/owner/repo/releases/download/1.0.0/old.msi"
                .into(),
            digest: Some(format!("sha256:{:x}", Sha256::digest(b"owned"))),
        };
        let mut changed = asset.clone();
        changed.digest = Some(format!("sha256:{}", "0".repeat(64)));
        prune_verified_downloads(&paths, &app, &[changed], |_| Ok(None)).unwrap();
        assert!(package.exists());
        let escaped = crate::model::Asset {
            name: "../old.msi".into(),
            ..asset.clone()
        };
        prune_verified_downloads(&paths, &app, &[escaped], |_| Ok(None)).unwrap();
        assert!(package.exists());
        prune_verified_downloads(&paths, &app, std::slice::from_ref(&asset), |_| {
            Ok(Some("Required for repair".into()))
        })
        .unwrap();
        assert!(package.exists());
        prune_verified_downloads(&paths, &app, &[asset], |_| Ok(None)).unwrap();
        assert!(!package.exists());
        assert!(unknown.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn uninstall_cleanup_preserves_downloads_when_another_native_copy_remains() {
        let (root, paths, app, package) = fixture();
        let config = crate::model::Config {
            apps_root: root.display().to_string(),
            apps: Vec::new(),
            installations: vec![app.clone()],
            portable_label_counters: Default::default(),
        };
        after_uninstall(&paths, &app, &config).unwrap();
        assert!(package.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_or_cancelled_install_never_enters_cleanup() {
        for cancelled in [false, true] {
            let (root, _, _, package) = fixture();
            let job = Job::new(root.join("logs/install.log"), &Default::default());
            let result = job.tracked::<()>("Install", "filmcraft", |_| {
                if cancelled {
                    return Err(crate::jobs::Cancelled.into());
                }
                anyhow::bail!("Installation could not be verified")
            });
            assert!(result.is_err() && package.exists());
            let records = crate::operations::Manager::new(&root).list();
            assert!(!records[0].facts.contains_key("Cleanup"));
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}
