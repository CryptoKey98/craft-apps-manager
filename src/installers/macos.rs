use crate::{jobs::Job, model::Installed, platform};
use anyhow::{bail, Context, Result};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
pub fn installer_extension() -> Result<&'static str> {
    Ok(".dmg")
}
pub fn installer_label() -> &'static str {
    "macOS"
}
/// Upstream bundles use `ai.storyteller.<repository>` identifiers.
fn identities(app: &str) -> [String; 2] {
    [
        format!("ai.storyteller.{}", crate::model::repository(app)),
        format!("ai.storyteller.{app}"),
    ]
}
fn verify_identity(bundle: &Path, app: &str) -> Result<String> {
    let id = platform::bundle_value(bundle, "CFBundleIdentifier")
        .context("App bundle has no identifier")?;
    if !identities(app).contains(&id) {
        bail!("App bundle identity does not match the selected app");
    }
    Ok(id)
}
fn verify_signature(bundle: &Path) -> Result<()> {
    let out = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(bundle)
        .output()?;
    if !out.status.success() {
        bail!(
            "App signature is invalid: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let out = Command::new("/usr/sbin/spctl")
        .args(["--assess", "--type", "execute"])
        .arg(bundle)
        .output()?;
    if !out.status.success() {
        bail!(
            "Gatekeeper rejected the app: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}
fn application_folders() -> Vec<PathBuf> {
    let mut folders = vec![PathBuf::from("/Applications")];
    if let Some(home) = std::env::var_os("HOME") {
        folders.push(PathBuf::from(home).join("Applications"));
    }
    folders
}
pub fn detect(app: &str) -> Result<Option<Installed>> {
    if !crate::model::APPS.contains(&app) {
        return Ok(None);
    }
    for folder in application_folders() {
        let Some(bundle) = crate::model::installed_executable(&folder, app) else {
            continue;
        };
        if !bundle.is_dir() || crate::files::linked(&bundle)? {
            continue;
        }
        let Ok(id) = verify_identity(&bundle, app) else {
            continue;
        };
        return Ok(Some(Installed {
            name: app.into(),
            version: platform::executable_version(&bundle).unwrap_or_else(|| "0.0.0".into()),
            path: folder.display().to_string(),
            architecture: crate::model::MANAGER_ARCH.into(),
            install_kind: "installer".into(),
            product_code: id,
        }));
    }
    Ok(None)
}
/// Keeps a DMG attached only as long as it is needed.
struct Mounted(PathBuf);
impl Mounted {
    fn attach(image: &Path) -> Result<Self> {
        let point =
            std::env::temp_dir().join(format!("craft-dmg-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&point)?;
        let out = Command::new("/usr/bin/hdiutil")
            .args([
                "attach",
                "-nobrowse",
                "-readonly",
                "-noautoopen",
                "-mountpoint",
            ])
            .arg(&point)
            .arg(image)
            .stdin(Stdio::null())
            .output()?;
        if !out.status.success() {
            let _ = fs::remove_dir(&point);
            bail!(
                "Could not open the disk image: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(Self(point))
    }
}
impl Drop for Mounted {
    fn drop(&mut self) {
        let detached = Command::new("/usr/bin/hdiutil")
            .args(["detach", "-quiet"])
            .arg(&self.0)
            .status()
            .is_ok_and(|s| s.success());
        if !detached {
            let _ = Command::new("/usr/bin/hdiutil")
                .args(["detach", "-force", "-quiet"])
                .arg(&self.0)
                .status();
        }
        let _ = fs::remove_dir(&self.0);
    }
}
/// Copies the app bundle from a release DMG into `stage` and verifies it.
pub fn extract_app(image: &Path, stage: &Path, app: &str, job: &Job) -> Result<PathBuf> {
    crate::model::valid_app(app)?;
    job.stage("Extracting", None, "Opening the disk image");
    let mounted = Mounted::attach(image)?;
    let bundles: Vec<_> = fs::read_dir(&mounted.0)?
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .filter(|e| {
            e.file_type().is_ok_and(|t| t.is_dir())
                && e.file_name().to_string_lossy().ends_with(".app")
        })
        .collect();
    let [bundle] = bundles.as_slice() else {
        bail!("Disk image must contain exactly one app");
    };
    let name = bundle.file_name().to_string_lossy().into_owned();
    if !crate::model::executable_names(app).contains(&name) {
        bail!("Disk image contains an unexpected app: {name}");
    }
    verify_identity(&bundle.path(), app)?;
    job.check()?;
    fs::create_dir_all(stage)?;
    let staged = stage.join(&name);
    job.stage("Extracting", None, format!("Copying {name}"));
    // Upstream DMGs tag every file with empty Finder info, which strict
    // signature checks reject. App bundles need no extended attributes.
    let status = Command::new("/usr/bin/ditto")
        .args(["--noextattr", "--norsrc"])
        .arg(bundle.path())
        .arg(&staged)
        .status()?;
    drop(mounted);
    if !status.success() {
        bail!("Could not copy {name} from the disk image");
    }
    job.stage("Verifying", None, "Checking the app signature");
    verify_signature(&staged)?;
    Ok(staged)
}
fn writable(folder: &Path) -> bool {
    let probe = folder.join(format!(".craft-manager-{}", uuid::Uuid::new_v4().simple()));
    fs::create_dir(&probe).is_ok() && fs::remove_dir(&probe).is_ok()
}
pub fn run(file: &Path, app: &str) -> Result<Installed> {
    run_with_job(file, app, None)
}
pub fn run_with_job(file: &Path, app: &str, job: Option<&Job>) -> Result<Installed> {
    crate::model::valid_app(app)?;
    let file = file.canonicalize()?;
    if file.extension().is_none_or(|s| s != "dmg") {
        bail!("Package format does not match macOS");
    }
    let owned;
    let job = match job {
        Some(job) => job,
        None => {
            owned = Job::new(
                std::env::temp_dir().join("craft-apps-manager-install.log"),
                &Default::default(),
            );
            &owned
        }
    };
    let folder = match detect(app)? {
        Some(existing) => PathBuf::from(existing.path),
        None => {
            let system = PathBuf::from("/Applications");
            if writable(&system) {
                system
            } else {
                let user = application_folders().pop().context("No home directory")?;
                fs::create_dir_all(&user)?;
                user
            }
        }
    };
    if !writable(&folder) {
        bail!(
            "{} is not writable. Use the portable release format instead.",
            folder.display()
        );
    }
    let stage = folder.join(format!(".craft-install-{}", uuid::Uuid::new_v4().simple()));
    let result = (|| -> Result<()> {
        let staged = extract_app(&file, &stage, app, job)?;
        job.check()?;
        job.stage(
            "Installing",
            None,
            format!("Copying into {}", folder.display()),
        );
        let destination = folder.join(staged.file_name().context("Missing app name")?);
        let previous = stage.join("previous.app");
        if destination.exists() {
            if crate::files::linked(&destination)? {
                bail!("Installed app is a link; leaving it unchanged");
            }
            fs::rename(&destination, &previous)?;
        }
        if let Err(error) = fs::rename(&staged, &destination) {
            if previous.exists() {
                fs::rename(&previous, &destination)
                    .context("Could not restore the previous app")?;
            }
            return Err(error.into());
        }
        Ok(())
    })();
    if stage.exists() {
        let _ = fs::remove_dir_all(&stage);
    }
    result?;
    detect(app)?.context("App was copied, but it could not be detected")
}
pub fn uninstall(app: &Installed) -> Result<()> {
    crate::model::valid_app(&app.name)?;
    if !identities(&app.name).contains(&app.product_code) {
        bail!("Installed app identity mismatch");
    }
    let folder = PathBuf::from(&app.path);
    if !application_folders().contains(&folder) {
        bail!("Only apps in an Applications folder can be uninstalled here");
    }
    let bundle = crate::model::installed_executable(&folder, &app.name)
        .context("Installed app is missing")?;
    if crate::files::linked(&bundle)? || verify_identity(&bundle, &app.name)? != app.product_code {
        bail!("Installed app identity mismatch");
    }
    fs::remove_dir_all(&bundle)
        .with_context(|| format!("Could not remove {}", bundle.display()))?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundle_identities_cover_renamed_repository() {
        assert!(identities("printcraft").contains(&"ai.storyteller.pdfcraft".to_string()));
        assert!(identities("photocraft").contains(&"ai.storyteller.photocraft".to_string()));
        assert_eq!(
            crate::model::executable_names("printcraft"),
            ["PrintCraft.app", "PdfCraft.app"]
        );
        assert_eq!(
            crate::model::executable_name("photocraft"),
            "PhotoCraft.app"
        );
    }
}
