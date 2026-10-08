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
    if !bundle.is_dir()
        || crate::files::linked(bundle)?
        || crate::files::linked(&bundle.join("Contents"))?
        || crate::files::linked(&bundle.join("Contents/Info.plist"))?
    {
        bail!("App bundle or metadata is a link");
    }
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
/// Selects the current valid bundle, falling back past unrelated or linked
/// candidates. Detection, default launch and uninstall share this resolver.
pub fn is_owned_bundle(bundle: &Path, app: &str) -> bool {
    verify_identity(bundle, app).is_ok()
}
pub fn resolve_bundle(folder: &Path, app: &str) -> Option<PathBuf> {
    crate::model::executable_names(app)
        .into_iter()
        .map(|name| folder.join(name))
        .find(|bundle| verify_identity(bundle, app).is_ok())
}
fn installed_bundle(bundle: &Path, app: &str) -> Result<Installed> {
    Ok(Installed {
        name: app.into(),
        version: platform::executable_version(bundle).unwrap_or_else(|| "0.0.0".into()),
        path: bundle
            .parent()
            .context("Missing Applications folder")?
            .display()
            .to_string(),
        architecture: crate::model::MANAGER_ARCH.into(),
        install_kind: "installer".into(),
        product_code: verify_identity(bundle, app)?,
    })
}
fn detect_in_folders(app: &str, folders: &[PathBuf]) -> Result<Option<Installed>> {
    if !crate::model::APPS.contains(&app) {
        return Ok(None);
    }
    for folder in folders {
        if let Some(bundle) = resolve_bundle(folder, app) {
            return installed_bundle(&bundle, app).map(Some);
        }
    }
    Ok(None)
}
pub fn detect(app: &str) -> Result<Option<Installed>> {
    detect_in_folders(app, &application_folders())
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
    let result = (|| {
        let staged = extract_app(&file, &stage, app, job)?;
        job.check()?;
        job.stage(
            "Installing",
            None,
            format!("Copying into {}", folder.display()),
        );
        publish_bundle(&staged, &folder, app, &NativeInstall)
    })();
    // Rollback bundles live separately: cleaning an extraction stage can never
    // delete the only surviving previous installation after failed restoration.
    if stage.exists() {
        let _ = fs::remove_dir_all(&stage);
    }
    result
}

trait InstallOps {
    fn running(&self, app: &str) -> Result<bool>;
    fn rename(&self, from: &Path, to: &Path) -> Result<()>;
    fn verify(&self, bundle: &Path) -> Result<()>;
}
struct NativeInstall;
impl InstallOps for NativeInstall {
    fn running(&self, app: &str) -> Result<bool> {
        platform::running_app(app)
    }
    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        fs::rename(from, to)
            .with_context(|| format!("Could not move {} to {}", from.display(), to.display()))
    }
    fn verify(&self, bundle: &Path) -> Result<()> {
        verify_signature(bundle)
    }
}
fn publish_bundle(
    staged: &Path,
    folder: &Path,
    app: &str,
    ops: &impl InstallOps,
) -> Result<Installed> {
    let name = staged
        .file_name()
        .context("Missing app name")?
        .to_string_lossy();
    if !crate::model::executable_names(app).contains(&name.to_string()) {
        bail!("Unexpected staged app name");
    }
    let expected_id = verify_identity(staged, app)?;
    let expected_version =
        platform::executable_version(staged).context("Staged app has no version")?;
    ops.verify(staged)?;
    if crate::files::linked(folder)? {
        bail!("Applications folder is a link; leaving it unchanged");
    }
    let destination = folder.join(name.as_ref());
    if fs::symlink_metadata(&destination).is_ok() && verify_identity(&destination, app).is_err() {
        bail!(
            "Destination is unrelated or a link; leaving it unchanged: {}",
            destination.display()
        );
    }
    let aliases: Vec<_> = crate::model::executable_names(app)
        .into_iter()
        .map(|name| folder.join(name))
        .filter(|bundle| verify_identity(bundle, app).is_ok())
        .collect();
    let recovery = folder.join(format!(".craft-rollback-{app}-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&recovery)?;
    let mut moved = Vec::new();
    let mut published = false;
    let result = (|| {
        // Download/extraction can take minutes. Refuse an app opened since the
        // outer update guard, immediately before changing its installed bundles.
        if ops.running(app)? {
            bail!("Close the app before installing it.");
        }
        for alias in &aliases {
            verify_identity(alias, app)?;
            let saved = recovery.join(alias.file_name().context("Missing alias name")?);
            ops.rename(alias, &saved)?;
            moved.push((alias.clone(), saved));
        }
        if fs::symlink_metadata(&destination).is_ok() {
            bail!("Destination appeared during installation; leaving it unchanged");
        }
        ops.rename(staged, &destination)?;
        published = true;
        ops.verify(&destination)?;
        let installed = installed_bundle(&destination, app)?;
        if installed.product_code != expected_id
            || installed.version != expected_version
            || resolve_bundle(folder, app).as_ref() != Some(&destination)
        {
            bail!("Published app does not match the verified release");
        }
        Ok(installed)
    })();
    match result {
        Ok(installed) => {
            if moved.is_empty() {
                let _ = fs::remove_dir(&recovery);
            } else {
                prune_rollbacks(folder, app, &recovery);
            }
            Ok(installed)
        }
        Err(error) => {
            let mut failures = Vec::new();
            if published {
                if let Err(failure) = ops.rename(&destination, staged) {
                    failures.push(format!("could not withdraw new bundle: {failure:#}"));
                }
            }
            for (original, saved) in moved.iter().rev() {
                // Never overwrite something that appeared during failed recovery.
                if fs::symlink_metadata(original).is_ok() {
                    failures.push(format!(
                        "restore destination exists: {}",
                        original.display()
                    ));
                } else if let Err(failure) = ops.rename(saved, original) {
                    failures.push(format!(
                        "could not restore {}: {failure:#}",
                        original.display()
                    ));
                }
            }
            if failures.is_empty() {
                let _ = fs::remove_dir(&recovery);
                Err(error)
            } else {
                Err(error.context(format!(
                    "Recovery incomplete; previous bundles retained at {}. {}",
                    recovery.display(),
                    failures.join("; ")
                )))
            }
        }
    }
}
// Retain one previous successful set. Never prune unknown contents, links, or
// failed recovery evidence; an incomplete set is deliberately left for recovery.
fn prune_rollbacks(folder: &Path, app: &str, current: &Path) {
    // A marker is written only after the new destination has passed verification.
    let marker = current.join("completed");
    if fs::write(&marker, app).is_err() {
        return;
    }
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry.path();
        let prefix = format!(".craft-rollback-{app}-");
        let name = entry.file_name().to_string_lossy().into_owned();
        if old == current
            || name
                .strip_prefix(&prefix)
                .is_none_or(|s| uuid::Uuid::parse_str(s).is_err())
            || !entry.file_type().is_ok_and(|t| t.is_dir())
        {
            continue;
        }
        let marker = old.join("completed");
        if crate::files::linked(&marker).unwrap_or(true)
            || fs::read_to_string(&marker).ok().as_deref() != Some(app)
        {
            continue;
        }
        let Ok(children) = fs::read_dir(&old) else {
            continue;
        };
        let safe = children
            .collect::<std::io::Result<Vec<_>>>()
            .is_ok_and(|children| {
                children.iter().all(|child| {
                    let name = child.file_name().to_string_lossy().into_owned();
                    name == "completed"
                        || (crate::model::executable_names(app).contains(&name)
                            && verify_identity(&child.path(), app).is_ok())
                })
            });
        if safe {
            let _ = fs::remove_dir_all(old);
        }
    }
}
pub fn uninstall(app: &Installed) -> Result<()> {
    uninstall_in_folders(app, &application_folders())
}
fn uninstall_in_folders(app: &Installed, folders: &[PathBuf]) -> Result<()> {
    crate::model::valid_app(&app.name)?;
    if !identities(&app.name).contains(&app.product_code) {
        bail!("Installed app identity mismatch");
    }
    let folder = PathBuf::from(&app.path);
    if !folders.contains(&folder) {
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
    use std::cell::{Cell, RefCell};
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("craft-bundles-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn folder(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            fs::create_dir_all(&path).unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn bundle(folder: &Path, name: &str, id: &str, version: &str) -> PathBuf {
        let path = folder.join(name);
        fs::create_dir_all(path.join("Contents")).unwrap();
        fs::write(path.join("Contents/Info.plist"), format!(r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>{id}</string><key>CFBundleShortVersionString</key><string>{version}</string></dict></plist>"#)).unwrap();
        fs::write(path.join("payload"), version).unwrap();
        path
    }
    #[derive(Default)]
    struct Ops {
        running: bool,
        rename_count: Cell<usize>,
        fail_renames: Vec<usize>,
        verify_count: Cell<usize>,
        fail_verify: usize,
        corrupt_published: bool,
        calls: RefCell<Vec<&'static str>>,
    }
    impl InstallOps for Ops {
        fn running(&self, _: &str) -> Result<bool> {
            self.calls.borrow_mut().push("running");
            Ok(self.running)
        }
        fn rename(&self, from: &Path, to: &Path) -> Result<()> {
            self.calls.borrow_mut().push("rename");
            let n = self.rename_count.get() + 1;
            self.rename_count.set(n);
            if self.fail_renames.contains(&n) {
                bail!("injected rename {n}");
            }
            fs::rename(from, to)?;
            Ok(())
        }
        fn verify(&self, path: &Path) -> Result<()> {
            self.calls.borrow_mut().push("verify");
            let n = self.verify_count.get() + 1;
            self.verify_count.set(n);
            if self.fail_verify == n {
                bail!("injected verification {n}");
            }
            if self.corrupt_published && n == 2 {
                let plist = path.join("Contents/Info.plist");
                let contents = fs::read_to_string(&plist)?;
                fs::write(plist, contents.replace("0.4.0", "99.0.0"))?;
            }
            Ok(())
        }
    }
    fn rollback_dirs(folder: &Path) -> Vec<PathBuf> {
        fs::read_dir(folder)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".craft-rollback-")
            })
            .collect()
    }
    #[test]
    fn resolver_prefers_valid_current_alias_and_preserves_folder_precedence() {
        let f = Fixture::new();
        let system = f.folder("system");
        let user = f.folder("user");
        let legacy = bundle(
            &system,
            "PrintCraft.app",
            "ai.storyteller.printcraft",
            "0.2.1",
        );
        bundle(&user, "PdfCraft.app", "ai.storyteller.pdfcraft", "0.4.0");
        assert_eq!(
            detect_in_folders("printcraft", &[system.clone(), user])
                .unwrap()
                .unwrap()
                .version,
            "0.2.1"
        );
        let canonical = bundle(&system, "PdfCraft.app", "unrelated.owner", "99");
        assert_eq!(resolve_bundle(&system, "printcraft"), Some(legacy.clone()));
        fs::remove_dir_all(&canonical).unwrap();
        std::os::unix::fs::symlink(&legacy, &canonical).unwrap();
        assert_eq!(resolve_bundle(&system, "printcraft"), Some(legacy));
        fs::remove_file(&canonical).unwrap();
        bundle(&system, "PdfCraft.app", "ai.storyteller.pdfcraft", "0.4.0");
        assert_eq!(
            crate::model::installed_executable(&system, "printcraft"),
            Some(canonical)
        );
    }
    #[test]
    fn migration_moves_both_owned_aliases_and_uninstall_uses_current_bundle() {
        for both in [false, true] {
            let f = Fixture::new();
            let folder = f.folder("Applications");
            let other = f.folder("other-Applications");
            let stage = f.folder("stage");
            bundle(
                &folder,
                "PrintCraft.app",
                "ai.storyteller.printcraft",
                "0.2.1",
            );
            if both {
                bundle(&folder, "PdfCraft.app", "ai.storyteller.pdfcraft", "0.3.0");
            }
            let untouched = bundle(
                &other,
                "PrintCraft.app",
                "ai.storyteller.printcraft",
                "0.2.1",
            );
            let staged = bundle(&stage, "PdfCraft.app", "ai.storyteller.pdfcraft", "0.4.0");
            let installed =
                publish_bundle(&staged, &folder, "printcraft", &Ops::default()).unwrap();
            assert_eq!(installed.version, "0.4.0");
            assert!(!folder.join("PrintCraft.app").exists());
            assert!(untouched.exists());
            assert_eq!(rollback_dirs(&folder).len(), 1);
            assert_eq!(
                fs::read(rollback_dirs(&folder)[0].join("PrintCraft.app/payload")).unwrap(),
                b"0.2.1"
            );
            assert_eq!(
                detect_in_folders("printcraft", std::slice::from_ref(&folder))
                    .unwrap()
                    .unwrap()
                    .version,
                "0.4.0"
            );
            uninstall_in_folders(&installed, std::slice::from_ref(&folder)).unwrap();
            assert!(!folder.join("PdfCraft.app").exists());
            assert!(detect_in_folders("printcraft", &[folder])
                .unwrap()
                .is_none());
        }
    }
    #[test]
    fn ordinary_same_name_update_retains_one_previous_set() {
        let f = Fixture::new();
        let folder = f.folder("Applications");
        let stage = f.folder("stage");
        bundle(
            &folder,
            "PhotoCraft.app",
            "ai.storyteller.photocraft",
            "0.1.0",
        );
        for version in ["0.2.0", "0.3.0"] {
            let staged = bundle(
                &stage,
                "PhotoCraft.app",
                "ai.storyteller.photocraft",
                version,
            );
            publish_bundle(&staged, &folder, "photocraft", &Ops::default()).unwrap();
        }
        assert_eq!(rollback_dirs(&folder).len(), 1);
        assert_eq!(
            fs::read(rollback_dirs(&folder)[0].join("PhotoCraft.app/payload")).unwrap(),
            b"0.2.0"
        );
    }
    #[test]
    fn install_refuses_unrelated_and_link_destinations_without_moving_legacy() {
        for linked in [false, true] {
            let f = Fixture::new();
            let folder = f.folder("Applications");
            let stage = f.folder("stage");
            let legacy = bundle(
                &folder,
                "PrintCraft.app",
                "ai.storyteller.printcraft",
                "0.2.1",
            );
            if linked {
                std::os::unix::fs::symlink(&legacy, folder.join("PdfCraft.app")).unwrap();
            } else {
                bundle(&folder, "PdfCraft.app", "another.owner", "99");
            }
            let staged = bundle(&stage, "PdfCraft.app", "ai.storyteller.pdfcraft", "0.4.0");
            let ops = Ops::default();
            assert!(publish_bundle(&staged, &folder, "printcraft", &ops).is_err());
            assert_eq!(ops.rename_count.get(), 0);
            assert!(legacy.exists());
        }
    }
    #[test]
    fn running_recheck_happens_after_stage_verification_before_old_bundle_moves() {
        let f = Fixture::new();
        let folder = f.folder("Applications");
        let stage = f.folder("stage");
        let old = bundle(
            &folder,
            "PrintCraft.app",
            "ai.storyteller.printcraft",
            "0.2.1",
        );
        let staged = bundle(&stage, "PdfCraft.app", "ai.storyteller.pdfcraft", "0.4.0");
        let ops = Ops {
            running: true,
            ..Default::default()
        };
        assert!(publish_bundle(&staged, &folder, "printcraft", &ops)
            .unwrap_err()
            .to_string()
            .contains("Close the app"));
        assert_eq!(*ops.calls.borrow(), ["verify", "running"]);
        assert!(old.exists());
        assert!(rollback_dirs(&folder).is_empty());
    }
    #[test]
    fn move_publish_and_postvalidation_failures_restore_every_alias() {
        for (fail_renames, fail_verify) in [(vec![2], 0), (vec![3], 0), (vec![], 2)] {
            let f = Fixture::new();
            let folder = f.folder("Applications");
            let stage = f.folder("stage");
            bundle(&folder, "PdfCraft.app", "ai.storyteller.pdfcraft", "0.3.0");
            bundle(
                &folder,
                "PrintCraft.app",
                "ai.storyteller.printcraft",
                "0.2.1",
            );
            let staged = bundle(&stage, "PdfCraft.app", "ai.storyteller.pdfcraft", "0.4.0");
            let ops = Ops {
                fail_renames,
                fail_verify,
                ..Default::default()
            };
            assert!(publish_bundle(&staged, &folder, "printcraft", &ops).is_err());
            assert_eq!(
                fs::read(folder.join("PrintCraft.app/payload")).unwrap(),
                b"0.2.1"
            );
            assert_eq!(
                fs::read(folder.join("PdfCraft.app/payload")).unwrap(),
                b"0.3.0"
            );
            assert!(rollback_dirs(&folder).is_empty());
        }
    }
    #[test]
    fn exact_destination_version_is_checked_before_commit() {
        let f = Fixture::new();
        let folder = f.folder("Applications");
        let stage = f.folder("stage");
        bundle(
            &folder,
            "PrintCraft.app",
            "ai.storyteller.printcraft",
            "0.2.1",
        );
        let staged = bundle(&stage, "PdfCraft.app", "ai.storyteller.pdfcraft", "0.4.0");
        let ops = Ops {
            corrupt_published: true,
            ..Default::default()
        };
        let error = publish_bundle(&staged, &folder, "printcraft", &ops).unwrap_err();
        assert!(error.to_string().contains("does not match"));
        assert_eq!(
            fs::read(folder.join("PrintCraft.app/payload")).unwrap(),
            b"0.2.1"
        );
        assert!(!folder.join("PdfCraft.app").exists());
    }
    #[test]
    fn invalid_stage_is_rejected_before_moving_installed_app() {
        let f = Fixture::new();
        let folder = f.folder("Applications");
        let stage = f.folder("stage");
        let old = bundle(
            &folder,
            "PrintCraft.app",
            "ai.storyteller.printcraft",
            "0.2.1",
        );
        let staged = bundle(&stage, "PdfCraft.app", "ai.storyteller.pdfcraft", "0.4.0");
        let ops = Ops {
            fail_verify: 1,
            ..Default::default()
        };
        assert!(publish_bundle(&staged, &folder, "printcraft", &ops).is_err());
        assert_eq!(ops.rename_count.get(), 0);
        assert!(old.exists());
        assert!(rollback_dirs(&folder).is_empty());
    }
    #[test]
    fn rollback_failure_keeps_old_bytes_outside_stage_and_preserves_original_error() {
        let f = Fixture::new();
        let folder = f.folder("Applications");
        let stage = f.folder("stage");
        bundle(
            &folder,
            "PrintCraft.app",
            "ai.storyteller.printcraft",
            "0.2.1",
        );
        let staged = bundle(&stage, "PdfCraft.app", "ai.storyteller.pdfcraft", "0.4.0");
        let ops = Ops {
            fail_renames: vec![2, 3],
            ..Default::default()
        };
        let error = publish_bundle(&staged, &folder, "printcraft", &ops).unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("injected rename 2"));
        assert!(message.contains("injected rename 3"));
        assert!(message.contains("Recovery incomplete"));
        fs::remove_dir_all(stage).unwrap();
        assert_eq!(
            fs::read(rollback_dirs(&folder)[0].join("PrintCraft.app/payload")).unwrap(),
            b"0.2.1"
        );
        // Later successful installs must not prune failed recovery evidence.
        let next_stage = f.folder("next-stage");
        let staged = bundle(
            &next_stage,
            "PdfCraft.app",
            "ai.storyteller.pdfcraft",
            "0.4.0",
        );
        publish_bundle(&staged, &folder, "printcraft", &Ops::default()).unwrap();
        assert_eq!(rollback_dirs(&folder).len(), 1);
    }
    #[test]
    fn failed_withdrawal_preserves_previous_canonical_bytes_and_restores_legacy() {
        let f = Fixture::new();
        let folder = f.folder("Applications");
        let stage = f.folder("stage");
        bundle(&folder, "PdfCraft.app", "ai.storyteller.pdfcraft", "0.3.0");
        bundle(
            &folder,
            "PrintCraft.app",
            "ai.storyteller.printcraft",
            "0.2.1",
        );
        let staged = bundle(&stage, "PdfCraft.app", "ai.storyteller.pdfcraft", "0.4.0");
        let ops = Ops {
            fail_renames: vec![4],
            fail_verify: 2,
            ..Default::default()
        };
        let error = publish_bundle(&staged, &folder, "printcraft", &ops).unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("injected verification 2"));
        assert!(message.contains("injected rename 4"));
        fs::remove_dir_all(stage).unwrap();
        assert_eq!(
            fs::read(rollback_dirs(&folder)[0].join("PdfCraft.app/payload")).unwrap(),
            b"0.3.0"
        );
        assert_eq!(
            fs::read(folder.join("PrintCraft.app/payload")).unwrap(),
            b"0.2.1"
        );
    }
    /// Run explicitly on macOS with CRAFT_TEST_LEGACY_APP pointing to an
    /// official PrintCraft 0.2.1 bundle and CRAFT_TEST_CURRENT_APP pointing to
    /// an official PdfCraft 0.4.0 bundle. Optionally set CRAFT_TEST_CURRENT_DMG
    /// to its official release DMG to exercise extraction as well.
    /// All mutations use temporary copies;
    /// no installed apps, user preferences, profiles or launch agents are used.
    #[test]
    #[ignore = "requires official signed macOS release bundle fixtures"]
    fn official_renamed_release_migrates_in_isolated_applications_folders() {
        let legacy = PathBuf::from(
            std::env::var_os("CRAFT_TEST_LEGACY_APP").expect("Set CRAFT_TEST_LEGACY_APP"),
        );
        let current = PathBuf::from(
            std::env::var_os("CRAFT_TEST_CURRENT_APP").expect("Set CRAFT_TEST_CURRENT_APP"),
        );
        assert_eq!(platform::executable_version(&legacy).unwrap(), "0.2.1");
        assert_eq!(platform::executable_version(&current).unwrap(), "0.4.0");
        let copy = |from: &Path, to: &Path| {
            assert!(Command::new("/usr/bin/ditto")
                .args(["--noextattr", "--norsrc"])
                .arg(from)
                .arg(to)
                .status()
                .unwrap()
                .success());
        };
        for both in [false, true] {
            let f = Fixture::new();
            let folder = f.folder("Applications");
            let stage = f.folder("stage");
            copy(&legacy, &folder.join("PrintCraft.app"));
            if both {
                copy(&current, &folder.join("PdfCraft.app"));
            }
            let staged = stage.join("PdfCraft.app");
            if let Some(dmg) = std::env::var_os("CRAFT_TEST_CURRENT_DMG") {
                let job = Job::new(f.0.join("install.log"), &Default::default());
                assert_eq!(
                    extract_app(&PathBuf::from(dmg), &stage, "printcraft", &job).unwrap(),
                    staged
                );
            } else {
                copy(&current, &staged);
            }
            let installed = publish_bundle(&staged, &folder, "printcraft", &NativeInstall).unwrap();
            assert_eq!(installed.version, "0.4.0");
            assert_eq!(
                crate::model::installed_executable(&folder, "printcraft"),
                Some(folder.join("PdfCraft.app"))
            );
            assert!(!folder.join("PrintCraft.app").exists());
            let detected = detect_in_folders("printcraft", std::slice::from_ref(&folder))
                .unwrap()
                .unwrap();
            assert_eq!(detected.version, "0.4.0");
            assert!(
                crate::updates::version(&detected.version).unwrap()
                    >= crate::updates::version("0.4.0").unwrap()
            );
            assert_eq!(
                fs::read(folder.join("PdfCraft.app/Contents/MacOS/PdfCraft")).unwrap(),
                fs::read(current.join("Contents/MacOS/PdfCraft")).unwrap()
            );
            assert_eq!(
                fs::read(
                    rollback_dirs(&folder)[0].join("PrintCraft.app/Contents/MacOS/PrintCraft")
                )
                .unwrap(),
                fs::read(legacy.join("Contents/MacOS/PrintCraft")).unwrap()
            );
            uninstall_in_folders(&installed, std::slice::from_ref(&folder)).unwrap();
            assert!(detect_in_folders("printcraft", &[folder])
                .unwrap()
                .is_none());
        }
    }
    #[test]
    fn bundle_identities_cover_renamed_repository() {
        assert!(identities("printcraft").contains(&"ai.storyteller.pdfcraft".to_string()));
        assert!(identities("photocraft").contains(&"ai.storyteller.photocraft".to_string()));
        assert_eq!(
            crate::model::executable_names("printcraft"),
            ["PdfCraft.app", "PrintCraft.app"]
        );
        assert_eq!(
            crate::model::executable_name("photocraft"),
            "PhotoCraft.app"
        );
    }
}
