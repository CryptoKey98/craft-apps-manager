#[cfg(target_os = "macos")]
use crate::platform;
use crate::{jobs::Job, model::Installed};
// Contract-test adapter only: production always uses macOS's plist reader.
// This deliberately reads only the simple string fields written by fixtures;
// it neither validates real plist files nor substitutes for signature checks.
#[cfg(all(test, not(target_os = "macos")))]
mod platform {
    use std::path::Path;
    pub fn bundle_value(bundle: &Path, key: &str) -> Option<String> {
        let contents = std::fs::read_to_string(bundle.join("Contents/Info.plist")).ok()?;
        let expression = regex::Regex::new(&format!(
            r"<key>{}</key>\s*<string>([^<]*)</string>",
            regex::escape(key)
        ))
        .ok()?;
        Some(expression.captures(&contents)?.get(1)?.as_str().to_owned())
    }
    pub fn executable_version(bundle: &Path) -> Option<String> {
        bundle_value(bundle, "CFBundleShortVersionString")
    }
    pub fn running_app(app: &str) -> anyhow::Result<bool> {
        crate::platform::running_app(app)
    }
}
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
/// Upstream bundles use `ai.storyteller.<name>` identifiers; the catalog lists
/// other ones, such as ArtCraft's `ai.artcraft.app`.
fn identities(app: &str) -> Vec<String> {
    let saved: Vec<_> = crate::app_identity::native(app)
        .into_iter()
        .filter(|identity| identity.platform == "macos")
        .map(|identity| identity.package)
        .collect();
    if crate::catalog::builtin()
        .iter()
        .any(|entry| entry.key == app)
    {
        let mut known = crate::catalog::bundle_ids(app);
        for id in saved {
            if !known.contains(&id) {
                known.push(id);
            }
        }
        known
    } else if saved.is_empty() {
        // Older custom registrations can carry explicitly recorded bundle IDs.
        // Do not restore the catalog helper's repository-derived guesses.
        crate::catalog::get(app)
            .map(|entry| entry.bundle_ids)
            .unwrap_or_default()
    } else {
        saved
    }
}
fn bundle_identity(bundle: &Path) -> Result<String> {
    if !bundle.is_dir()
        || crate::files::linked(bundle)?
        || crate::files::linked(&bundle.join("Contents"))?
        || crate::files::linked(&bundle.join("Contents/Info.plist"))?
    {
        bail!("App bundle or metadata is a link");
    }
    let id = platform::bundle_value(bundle, "CFBundleIdentifier")
        .context("App bundle has no identifier")?;
    if id.is_empty() || id.chars().any(char::is_control) {
        bail!("App bundle has an invalid identifier");
    }
    Ok(id)
}
fn verify_identity(bundle: &Path, app: &str) -> Result<String> {
    let id = bundle_identity(bundle)?;
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
fn bundle_paths(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut bundles: Vec<_> = entries
        .flatten()
        .filter(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_dir())
                && entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "app")
        })
        .map(|entry| entry.path())
        .collect();
    bundles.sort();
    bundles
}
#[cfg(target_os = "macos")]
fn bundle_names(app: &str) -> Vec<String> {
    crate::model::executable_names(app)
}
// Match the macOS basename preference in contract tests on other host OSes.
#[cfg(all(test, not(target_os = "macos")))]
fn bundle_names(app: &str) -> Vec<String> {
    let mut names: Vec<String> = crate::catalog::names(app)
        .into_iter()
        .map(|name| {
            if name == "cadcraft" {
                return "CADCraft.app".into();
            }
            format!(
                "{}{}.app",
                name[..1].to_uppercase(),
                name[1..].replace("craft", "Craft")
            )
        })
        .collect();
    names.extend(
        crate::app_identity::native(app)
            .into_iter()
            .filter(|identity| identity.platform == "macos")
            .map(|identity| identity.executable),
    );
    names.dedup();
    names.reverse();
    names
}
fn bundle_candidates(folder: &Path, app: &str) -> Vec<PathBuf> {
    bundle_names(app)
        .into_iter()
        .map(|name| folder.join(name))
        .chain(bundle_paths(folder))
        .collect()
}
pub fn resolve_bundle(folder: &Path, app: &str) -> Option<PathBuf> {
    bundle_candidates(folder, app)
        .into_iter()
        .find(|bundle| verify_identity(bundle, app).is_ok())
}

fn installed_bundle(bundle: &Path, app: &str) -> Result<Installed> {
    Ok(Installed {
        installation_label: String::new(),
        package_type: String::new(),
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
    if !crate::model::apps().iter().any(|a| a == app) {
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
    let bundles: Vec<_> = bundle_paths(&mounted.0)
        .into_iter()
        .filter_map(|path| bundle_identity(&path).ok().map(|id| (path, id)))
        .collect();
    let saved = crate::app_identity::native(app)
        .into_iter()
        .any(|identity| identity.platform == "macos");
    let builtin = crate::catalog::builtin()
        .iter()
        .any(|entry| entry.key == app);
    let known = identities(app);
    let bound = saved || builtin || !known.is_empty();
    let eligible: Vec<_> = bundles
        .into_iter()
        .filter(|(_, id)| !bound || known.contains(id))
        .collect();
    if eligible.is_empty() {
        bail!("Disk image contains no app matching the selected app identity");
    }
    let index = if eligible.len() == 1 && bound {
        0
    } else {
        job.choose_identity(
            "Choose the app bundle to install",
            eligible
                .iter()
                .map(|(path, id)| {
                    format!(
                        "{} ({id})",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    )
                })
                .collect(),
        )?
    };
    let (bundle, expected_id) = eligible
        .get(index)
        .context("Invalid app bundle selection")?;
    let name = bundle
        .file_name()
        .context("Missing app bundle name")?
        .to_str()
        .context("App bundle name is not UTF-8")?
        .to_owned();
    job.check()?;
    fs::create_dir_all(stage)?;
    let staged = stage.join(&name);
    job.stage("Extracting", None, format!("Copying {name}"));
    // Upstream DMGs tag every file with empty Finder info, which strict
    // signature checks reject. App bundles need no extended attributes.
    let status = Command::new("/usr/bin/ditto")
        .args(["--noextattr", "--norsrc"])
        .arg(bundle)
        .arg(&staged)
        .status()?;
    drop(mounted);
    if !status.success() {
        bail!("Could not copy {name} from the disk image");
    }
    job.stage("Verifying", None, "Checking the app signature");
    verify_signature(&staged)?;
    if bundle_identity(&staged)? != *expected_id {
        bail!("Copied app identity differs from the selected bundle");
    }
    // Bind only the explicitly selected, signature-verified package. Updates
    // cannot expand this binding to a new, unrelated identifier.
    let remembered = crate::app_identity::native(app)
        .into_iter()
        .find(|identity| identity.platform == "macos" && identity.package == *expected_id);
    let preferred_name = remembered.map(|identity| identity.executable).or_else(|| {
        // Explicit legacy catalog IDs may predate the identity store. Preserve
        // the resolver's currently selected installed basename in that case too.
        application_folders().into_iter().find_map(|folder| {
            let existing = resolve_bundle(&folder, app)?;
            if verify_identity(&existing, app).ok().as_ref() != Some(expected_id) {
                return None;
            }
            existing.file_name()?.to_str().map(str::to_owned)
        })
    });
    crate::app_identity::remember_current(
        app,
        crate::app_identity::NativeIdentity {
            launch: None,
            family: String::new(),
            platform: "macos".into(),
            package: expected_id.clone(),
            // Keep launch/update pointed at the selected old bundle until the
            // native publish transaction has committed the new basename.
            executable: preferred_name.unwrap_or(name),
        },
    )?;
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
    fn remember(&self, _app: &str, _id: &str, _name: &str) -> Result<()> {
        Ok(())
    }
}
struct NativeInstall;
impl InstallOps for NativeInstall {
    fn running(&self, app: &str) -> Result<bool> {
        if platform::running_app(app)? {
            return Ok(true);
        }
        // A release may rename the bundle before its old process exits. Resolve
        // the process's enclosing bundle by its ID rather than its new basename.
        let out = Command::new("/bin/ps")
            .args(["-axww", "-o", "comm="])
            .output()?;
        if !out.status.success() {
            bail!("Could not list running processes");
        }
        Ok(String::from_utf8_lossy(&out.stdout).lines().any(|line| {
            let line = line.trim_start();
            line.rfind(".app/Contents/MacOS/")
                .is_some_and(|end| verify_identity(Path::new(&line[..end + 4]), app).is_ok())
        }))
    }
    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        fs::rename(from, to)
            .with_context(|| format!("Could not move {} to {}", from.display(), to.display()))
    }
    fn verify(&self, bundle: &Path) -> Result<()> {
        verify_signature(bundle)
    }
    fn remember(&self, app: &str, id: &str, name: &str) -> Result<()> {
        crate::app_identity::remember_current(
            app,
            crate::app_identity::NativeIdentity {
                launch: None,
                family: String::new(),
                platform: "macos".into(),
                package: id.into(),
                executable: name.into(),
            },
        )
    }
}
fn publish_bundle(
    staged: &Path,
    folder: &Path,
    app: &str,
    ops: &impl InstallOps,
) -> Result<Installed> {
    let curated = crate::catalog::builtin()
        .iter()
        .any(|entry| entry.key == app);
    publish_bundle_with_policy(staged, folder, app, ops, curated)
}
fn publish_bundle_with_policy(
    staged: &Path,
    folder: &Path,
    app: &str,
    ops: &impl InstallOps,
    curated: bool,
) -> Result<Installed> {
    let name = staged
        .file_name()
        .context("Missing app name")?
        .to_string_lossy();
    if staged
        .extension()
        .is_none_or(|extension| extension != "app")
    {
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
    let aliases: Vec<_> = if curated {
        // Catalog-owned legacy names intentionally migrate together.
        bundle_paths(folder)
            .into_iter()
            .filter(|bundle| verify_identity(bundle, app).is_ok())
            .collect()
    } else {
        // A manually copied bundle can share the ID. Update only the same copy
        // selected for launch, and never overwrite another copy at the new name.
        let selected = resolve_bundle(folder, app);
        if fs::symlink_metadata(&destination).is_ok() && selected.as_ref() != Some(&destination) {
            bail!("Destination belongs to another app copy; leaving it unchanged");
        }
        selected.into_iter().collect()
    };
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
            || (curated && resolve_bundle(folder, app).as_ref() != Some(&destination))
        {
            bail!("Published app does not match the verified release");
        }
        // Failure to persist the new selection is still inside the rollback
        // transaction, before old bundles are marked as safely recoverable.
        ops.remember(app, &expected_id, name.as_ref())?;
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
                        || (child
                            .path()
                            .extension()
                            .is_some_and(|extension| extension == "app")
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
    let bundle = bundle_candidates(&folder, &app.name)
        .into_iter()
        .find(|bundle| verify_identity(bundle, &app.name).is_ok_and(|id| id == app.product_code))
        .context("Installed app is missing")?;
    if crate::files::linked(&bundle)? || verify_identity(&bundle, &app.name)? != app.product_code {
        bail!("Installed app identity mismatch");
    }
    fs::remove_dir_all(&bundle)
        .with_context(|| format!("Could not remove {}", bundle.display()))?;
    Ok(())
}
#[cfg(test)]
#[path = "macos/tests.rs"]
mod tests;
