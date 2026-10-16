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
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    /// Opt-in real DMG transactions; never uses /Applications or changes Gatekeeper.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires CRAFT_TEST_MAC_IDENTITY_MANIFEST pointing to approved real DMGs"]
    fn real_custom_dmg_install_reload_reinstall_and_uninstall() {
        #[derive(serde::Deserialize)]
        struct Input {
            repository: String,
            image: PathBuf,
            bundle: String,
        }
        let manifest = std::env::var_os("CRAFT_TEST_MAC_IDENTITY_MANIFEST")
            .expect("missing approved DMG manifest");
        let inputs: Vec<Input> = crate::files::read_json(Path::new(&manifest)).unwrap();
        assert!(
            inputs.len() >= 2,
            "exercise independent repository identities"
        );
        let fixture = Fixture::new();
        let applications = fixture.folder("Applications");
        let mut entries = Vec::new();
        for (i, input) in inputs.iter().enumerate() {
            let mut entry = crate::catalog::builtin().remove(0);
            entry.key = format!("customnative{i}");
            entry.repository = input.repository.to_ascii_lowercase();
            entry.title = input.repository.split('/').next_back().unwrap().into();
            entry.aliases.clear();
            entry.bundle_ids.clear();
            entry.asset_prefix = "NativeFixture".into();
            entry.scheme = crate::catalog::Scheme::Generic;
            entry.download_rule = None;
            entry.download_rules.clear();
            entries.push(entry);
        }
        crate::files::write_json(&fixture.0.join("runtime/custom-apps.json"), &entries).unwrap();
        crate::catalog::load(&fixture.0);
        let extract = |image: &Path, stage: &Path, key: &str, bundle: &str, cancel: bool| {
            let job = Job::interactive(fixture.0.join("install.log"), &Default::default());
            std::thread::scope(|scope| {
                let worker = scope.spawn(|| extract_app(image, stage, key, &job));
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
                while !worker.is_finished() {
                    if let Some(choice) = job.identity_choice() {
                        let index = choice
                            .candidates
                            .iter()
                            .position(|candidate| candidate.starts_with(&format!("{bundle} (")))
                            .expect("expected explicit bundle choice");
                        choice.answer((!cancel).then_some(index));
                        break;
                    }
                    if std::time::Instant::now() > deadline {
                        job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                worker.join().unwrap()
            })
        };
        let mut installed = Vec::new();
        for (i, input) in inputs.iter().enumerate() {
            let key = &entries[i].key;
            assert!(crate::app_identity::native(key).is_empty());
            let cancelled = extract(
                &input.image,
                &fixture.folder(&format!("cancel-{i}")),
                key,
                &input.bundle,
                true,
            )
            .unwrap_err();
            assert!(
                crate::jobs::is_cancelled(&cancelled),
                "expected choice cancellation, got: {cancelled:#}"
            );
            assert!(crate::app_identity::native(key).is_empty());
            let staged = extract(
                &input.image,
                &fixture.folder(&format!("extract-{i}")),
                key,
                &input.bundle,
                false,
            )
            .unwrap();
            let current = publish_bundle(&staged, &applications, key, &NativeInstall).unwrap();
            assert_eq!(
                resolve_bundle(&applications, key),
                Some(applications.join(&input.bundle))
            );
            assert!(!current.product_code.is_empty());
            crate::catalog::load(&fixture.0);
            let detected = detect_in_folders(key, std::slice::from_ref(&applications))
                .unwrap()
                .unwrap();
            assert_eq!(detected.product_code, current.product_code);
            let staged = extract(
                &input.image,
                &fixture.folder(&format!("reinstall-{i}")),
                key,
                &input.bundle,
                false,
            )
            .unwrap();
            let current = publish_bundle(&staged, &applications, key, &NativeInstall).unwrap();
            let mut wrong = current.clone();
            wrong.product_code = "unrelated.identifier".into();
            assert!(uninstall_in_folders(&wrong, std::slice::from_ref(&applications)).is_err());
            assert!(applications.join(&input.bundle).is_dir());
            println!(
                "Verified real DMG install, reload, reinstall, cancellation and identity: {} ({})",
                input.repository, current.product_code
            );
            installed.push(current);
        }
        // Another signed app cannot replace an already-bound repository.
        assert!(extract(
            &inputs[1].image,
            &fixture.folder("wrong-repository"),
            &entries[0].key,
            &inputs[1].bundle,
            false
        )
        .is_err());
        for (i, current) in installed.iter().enumerate() {
            uninstall_in_folders(current, std::slice::from_ref(&applications)).unwrap();
            assert!(!applications.join(&inputs[i].bundle).exists());
            for other in inputs.iter().skip(i + 1) {
                assert!(applications.join(&other.bundle).is_dir());
            }
            println!("Verified selected-app removal: {}", inputs[i].repository);
        }
        crate::catalog::load(&fixture.0.join("empty"));
    }

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
        fail_remember: bool,
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
        fn remember(&self, _: &str, _: &str, _: &str) -> Result<()> {
            if self.fail_remember {
                bail!("injected identity persistence failure");
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
    fn custom_renamed_update_refuses_destination_copy_and_rolls_back_identity_save_failure() {
        for collision in [false, true] {
            let f = Fixture::new();
            let folder = f.folder("Applications");
            let stage = f.folder("stage");
            bundle(
                &folder,
                "PhotoCraft.app",
                "ai.storyteller.photocraft",
                "0.2.0",
            );
            if collision {
                bundle(&folder, "Renamed.app", "ai.storyteller.photocraft", "0.1.0");
            }
            let staged = bundle(&stage, "Renamed.app", "ai.storyteller.photocraft", "0.4.0");
            let ops = Ops {
                fail_remember: true,
                ..Default::default()
            };
            assert!(
                publish_bundle_with_policy(&staged, &folder, "photocraft", &ops, false).is_err()
            );
            assert_eq!(
                fs::read(folder.join("PhotoCraft.app/payload")).unwrap(),
                b"0.2.0"
            );
            if collision {
                assert_eq!(ops.rename_count.get(), 0);
                assert_eq!(
                    fs::read(folder.join("Renamed.app/payload")).unwrap(),
                    b"0.1.0"
                );
            } else {
                assert!(!folder.join("Renamed.app").exists());
            }
        }
    }
    #[test]
    fn custom_update_preserves_duplicate_id_manual_copy() {
        let f = Fixture::new();
        let folder = f.folder("Applications");
        let stage = f.folder("stage");
        let copy = bundle(&folder, "AaaCopy.app", "ai.storyteller.photocraft", "0.1.0");
        bundle(
            &folder,
            "PhotoCraft.app",
            "ai.storyteller.photocraft",
            "0.2.0",
        );
        let staged = bundle(&stage, "Renamed.app", "ai.storyteller.photocraft", "0.4.0");
        let installed =
            publish_bundle_with_policy(&staged, &folder, "photocraft", &Ops::default(), false)
                .unwrap();
        assert_eq!(installed.version, "0.4.0");
        assert_eq!(fs::read(copy.join("payload")).unwrap(), b"0.1.0");
        assert!(!folder.join("PhotoCraft.app").exists());
        assert!(folder.join("Renamed.app").exists());
        assert_eq!(
            fs::read(rollback_dirs(&folder)[0].join("PhotoCraft.app/payload")).unwrap(),
            b"0.2.0"
        );
        assert!(!rollback_dirs(&folder)[0].join("AaaCopy.app").exists());
    }
    #[test]
    fn uninstall_removes_selected_bundle_when_duplicate_ids_exist() {
        let f = Fixture::new();
        let folder = f.folder("Applications");
        let copy = bundle(&folder, "AaaCopy.app", "ai.storyteller.photocraft", "0.1.0");
        let selected = bundle(
            &folder,
            "PhotoCraft.app",
            "ai.storyteller.photocraft",
            "0.4.0",
        );
        let installed = installed_bundle(&selected, "photocraft").unwrap();
        assert_eq!(
            resolve_bundle(&folder, "photocraft"),
            Some(selected.clone())
        );
        uninstall_in_folders(&installed, std::slice::from_ref(&folder)).unwrap();
        assert!(!selected.exists());
        assert!(copy.exists());
    }
    #[test]
    fn unknown_custom_bundle_is_not_adopted_from_its_guessed_name_or_id() {
        let f = Fixture::new();
        let folder = f.folder("Applications");
        let candidate = bundle(
            &folder,
            "UnregisteredFixture.app",
            "ai.storyteller.unregisteredfixture",
            "1.0.0",
        );
        assert_eq!(
            bundle_identity(&candidate).unwrap(),
            "ai.storyteller.unregisteredfixture"
        );
        assert!(verify_identity(&candidate, "unregisteredfixture").is_err());
        assert!(resolve_bundle(&folder, "unregisteredfixture").is_none());
    }
    #[test]
    #[cfg(unix)]
    fn resolver_does_not_follow_renamed_bundle_or_metadata_links() {
        let f = Fixture::new();
        let folder = f.folder("Applications");
        let outside = f.folder("outside");
        let target = bundle(&outside, "Real.app", "ai.storyteller.photocraft", "1.0.0");
        std::os::unix::fs::symlink(&target, folder.join("Linked.app")).unwrap();
        let candidate = folder.join("Metadata.app");
        fs::create_dir(&candidate).unwrap();
        std::os::unix::fs::symlink(target.join("Contents"), candidate.join("Contents")).unwrap();
        assert!(resolve_bundle(&folder, "photocraft").is_none());
    }
    #[test]
    fn resolver_finds_renamed_bundle_only_by_exact_identity() {
        let f = Fixture::new();
        let folder = f.folder("Applications");
        bundle(&folder, "Unrelated.app", "another.owner", "99");
        let renamed = bundle(
            &folder,
            "Different.app",
            "ai.storyteller.photocraft",
            "0.4.0",
        );
        assert_eq!(resolve_bundle(&folder, "photocraft"), Some(renamed));
    }
    #[test]
    fn renamed_bundle_update_preserves_identity_and_previous_bytes() {
        let f = Fixture::new();
        let folder = f.folder("Applications");
        let stage = f.folder("stage");
        bundle(&folder, "OldName.app", "ai.storyteller.photocraft", "0.1.0");
        let staged = bundle(&stage, "NewName.app", "ai.storyteller.photocraft", "0.4.0");
        let installed = publish_bundle(&staged, &folder, "photocraft", &Ops::default()).unwrap();
        assert_eq!(installed.product_code, "ai.storyteller.photocraft");
        assert!(!folder.join("OldName.app").exists());
        assert_eq!(
            fs::read(rollback_dirs(&folder)[0].join("OldName.app/payload")).unwrap(),
            b"0.1.0"
        );
    }
    #[test]
    #[cfg(unix)]
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
        assert_eq!(resolve_bundle(&system, "printcraft"), Some(canonical));
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
    #[cfg(unix)]
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
                resolve_bundle(&folder, "printcraft"),
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
        #[cfg(target_os = "macos")]
        assert_eq!(
            crate::model::executable_names("printcraft"),
            ["PdfCraft.app", "PrintCraft.app"]
        );
        #[cfg(target_os = "macos")]
        assert_eq!(
            crate::model::executable_name("photocraft"),
            "PhotoCraft.app"
        );
    }
}
