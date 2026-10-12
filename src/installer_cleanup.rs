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
    Ok("Downloaded installer removed.".into())
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
            let message = format!("Installation succeeded; setup file was kept because cleanup could not finish: {error:#}");
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
