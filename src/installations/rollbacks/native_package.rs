use super::native::PackageIdentity;
use super::*;

pub(super) fn inspect(
    paths: &Paths,
    installed: &Installed,
    file: &Path,
    job: &Job,
) -> Result<PackageIdentity> {
    files::no_links(file)?;
    #[cfg(target_os = "windows")]
    {
        let _ = (paths, installed, job);
        crate::installers::rollback_package_identity(file)
    }
    #[cfg(target_os = "linux")]
    {
        let _ = (paths, job);
        crate::installers::rollback_package_identity(file, &installed.architecture)
    }
    #[cfg(target_os = "macos")]
    {
        ensure!(
            file.extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("dmg")),
            "Rollback supports DMG app bundles only"
        );
        let transaction = staging_path(paths).parent().unwrap().to_path_buf();
        let stage = transaction.join("inspection");
        let result = (|| {
            let bundle = crate::installers::extract_app(file, &stage, &installed.name, job)?;
            let identity = crate::platform::bundle_value(&bundle, "CFBundleIdentifier")
                .context("Package has no bundle identity")?;
            let version = crate::platform::bundle_value(&bundle, "CFBundleShortVersionString")
                .context("Package has no bundle version")?;
            Ok(PackageIdentity {
                format: "dmg".into(),
                identity: identity.clone(),
                product: identity,
                version,
                architecture: "universal".into(),
            })
        })();
        if transaction.exists() {
            if let Err(error) =
                files::remove_managed(&transaction, &paths.at("runtime/rollback-transactions"))
            {
                job.log(&format!("Package inspection cleanup warning: {error:#}"));
            }
        }
        cleanup_empty(paths, job);
        result
    }
}
