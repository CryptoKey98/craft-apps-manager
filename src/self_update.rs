use crate::{
    files,
    jobs::Job,
    model::{Asset, Paths, Release},
    network::Network,
    updates,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

/// The GitHub repository (owner/name) manager updates come from; set at build time.
pub const REPOSITORY_NAME: &str = env!("CRAFT_MANAGER_REPOSITORY");
pub const REPOSITORY: &str = concat!("https://github.com/", env!("CRAFT_MANAGER_REPOSITORY"));
/// Bundle identifier of the macOS app, as set by scripts/package-macos.sh.
#[cfg(target_os = "macos")]
const BUNDLE_ID: &str = "io.github.craft-apps-manager";
/// MSI installs keep their data outside the directory owned by Windows Installer.
pub fn installed_with_msi() -> bool {
    #[cfg(target_os = "windows")]
    {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join("installation-msi.json")))
            .is_some_and(|p| p.is_file())
    }
    #[cfg(not(target_os = "windows"))]
    false
}
#[derive(Clone)]
pub struct Available {
    pub version: String,
    pub asset: Asset,
}
pub fn select(release: Release, current: &str) -> Result<Option<Available>> {
    select_package(release, current, installed_with_msi())
}
fn select_package(release: Release, current: &str, msi: bool) -> Result<Option<Available>> {
    if release.draft
        || release.prerelease
        || updates::version(&release.tag_name)? <= updates::version(current)?
    {
        return Ok(None);
    }
    let version = release.tag_name.trim_start_matches('v').to_owned();
    let arch = crate::model::MANAGER_ARCH;
    let names = if cfg!(target_os = "macos") {
        vec![format!("Craft-Apps-Manager-{version}-macos-universal.zip")]
    } else if cfg!(target_os = "linux") {
        vec![format!("Craft-Apps-Manager-{version}-linux-{arch}.zip")]
    } else if msi {
        vec![format!("Craft-Apps-Manager-{version}-windows-{arch}.msi")]
    } else {
        vec![
            format!("Craft-Apps-Manager-{version}-windows-{arch}.zip"),
            format!("Craft-Apps-Updater-{version}-windows-{arch}.zip"),
        ]
    };
    let mut assets = release.assets;
    let index = names
        .iter()
        .find_map(|name| assets.iter().position(|a| &a.name == name))
        .with_context(|| {
            format!(
                "This release has no supported {} {arch} package",
                crate::model::release_os()
            )
        })?;
    let asset = assets.swap_remove(index);
    if !asset
        .browser_download_url
        .starts_with(&format!("{REPOSITORY}/releases/download/"))
    {
        bail!("Unexpected manager download location");
    }
    Ok(Some(Available { version, asset }))
}
pub fn check(paths: &Paths) -> Result<Option<Available>> {
    let release = Network::new(&paths.root)?.json(&format!(
        "https://api.github.com/repos/{REPOSITORY_NAME}/releases/latest"
    ))?;
    select(release, env!("CARGO_PKG_VERSION"))
}
#[derive(Serialize, Deserialize)]
pub struct Plan {
    /// macOS layout 1 stages beside the target; older library plans are rejected.
    #[serde(default)]
    pub layout: u32,
    #[serde(default)]
    pub msi: bool,
    pub parent_pid: u32,
    pub target: PathBuf,
    pub staged: PathBuf,
    pub hash: String,
    pub root: PathBuf,
    pub tools: PathBuf,
}
pub fn prepare(paths: &Paths, available: &Available, job: &Job) -> Result<PathBuf> {
    let executable = std::env::current_exe()?.canonicalize()?;
    let target = executable.clone();
    // On macOS the whole app bundle is replaced, not just the executable.
    #[cfg(target_os = "macos")]
    let target = app_bundle(&target)?;
    let home = target.parent().context("No executable folder")?;
    #[cfg(unix)]
    check_update_directory(home)?;
    #[cfg(target_os = "macos")]
    {
        files::no_links(&target)?;
        verify_bundle(&target)?;
    }
    let msi = installed_with_msi();
    if available.asset.name.ends_with(".msi") != msi {
        bail!("Update package does not match this installation type");
    }
    // Windows MSI stages in its data directory; other executable updates stage nearby.
    #[cfg(not(target_os = "macos"))]
    let stage_home = if msi { paths.root.as_path() } else { home };
    #[cfg(not(target_os = "macos"))]
    let stage = stage_home
        .join("runtime/self-update")
        .join(uuid::Uuid::new_v4().simple().to_string());
    #[cfg(not(target_os = "macos"))]
    fs::create_dir_all(&stage)?;
    #[cfg(target_os = "macos")]
    let stage = create_macos_stage(home)?;
    let result = (|| {
        let zip = stage.join(if msi { "release.msi" } else { "release.zip" });
        Network::new(&paths.root)?.asset(&available.asset, &zip, job)?;
        if msi {
            let plan = stage.join("plan.json");
            files::write_json(
                &plan,
                &Plan {
                    layout: 0,
                    msi: true,
                    parent_pid: std::process::id(),
                    target: target.clone(),
                    staged: zip.clone(),
                    hash: files::hash(&zip)?,
                    root: paths.root.clone(),
                    tools: paths.tools.clone(),
                },
            )?;
            fs::copy(&executable, stage.join(helper_name()))?;
            return Ok(plan);
        }
        let extracted = stage.join("package");
        files::extract_zip(&zip, &extracted, job)?;
        let manager = extracted.join(if cfg!(target_os = "macos") {
            "Craft Apps Manager.app"
        } else if cfg!(target_os = "linux") {
            "Craft Apps Manager Linux/craft-apps-manager"
        } else {
            "Craft Apps Manager/CraftApps-Manager.exe"
        });
        let staged = if crate::model::is_executable(&manager) {
            manager
        } else {
            extracted.join("Craft Apps Updater/CraftApps-Updater.exe")
        };
        files::no_links(&staged)?;
        if !staged_executable(&staged).is_file() {
            bail!("Release does not contain the manager executable");
        }
        #[cfg(target_os = "macos")]
        verify_bundle(&staged)?;
        let hash = files::hash(&staged_executable(&staged))?;
        let plan = stage.join("plan.json");
        files::write_json(
            &plan,
            &Plan {
                layout: u32::from(cfg!(target_os = "macos")),
                msi: false,
                parent_pid: std::process::id(),
                target: target.clone(),
                staged,
                hash,
                root: paths.root.clone(),
                tools: paths.tools.clone(),
            },
        )?;
        fs::copy(&executable, stage.join(helper_name()))?;
        #[cfg(target_os = "macos")]
        clear_quarantine(&stage.join(helper_name()))?;
        fs::remove_file(zip)?;
        Ok(plan)
    })();
    if result.is_err() {
        #[cfg(not(target_os = "macos"))]
        let _ = files::remove_managed(&stage, &stage_home.join("runtime/self-update"));
        #[cfg(target_os = "macos")]
        let _ = files::remove_managed(&stage, home);
    }
    result
}
#[cfg(unix)]
fn check_update_directory(home: &Path) -> Result<()> {
    let probe = home.join(format!(".craft-manager-update-{}", uuid::Uuid::new_v4()));
    let file = fs::OpenOptions::new().write(true).create_new(true).open(&probe)
        .context(if cfg!(target_os = "macos") {
            "Craft Apps Manager.app is in a folder you cannot write to. Move it to Applications or download the new version manually"
        } else {
            "This installation is managed by the system package manager. Install a newer DEB or RPM package to update Craft Apps Manager"
        })?;
    drop(file);
    fs::remove_file(probe)?;
    Ok(())
}
pub fn launch(plan: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let prepared: Plan = files::read_json(plan)?;
        let helper = plan
            .parent()
            .context("Missing update folder")?
            .join(helper_name());
        validate_macos_layout(&prepared, plan, &helper)?;
        check_update_directory(
            prepared
                .target
                .parent()
                .context("Missing application folder")?,
        )?;
        validate_macos_replacement(&prepared)?;
    }
    Command::new(
        plan.parent()
            .context("Missing update folder")?
            .join(helper_name()),
    )
    .arg("--apply-self-update")
    .arg(plan)
    .spawn()?;
    Ok(())
}
pub fn apply(plan_path: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        apply_macos(plan_path)
    }
    #[cfg(not(target_os = "macos"))]
    apply_executable(plan_path)
}
#[cfg(not(target_os = "macos"))]
fn apply_executable(plan_path: &Path) -> Result<()> {
    let mut plan: Plan = files::read_json(plan_path)?;
    files::no_links(&plan.target)?;
    plan.target = plan.target.canonicalize()?;
    plan.staged = plan.staged.canonicalize()?;
    let stage = plan_path
        .parent()
        .context("Missing update folder")?
        .canonicalize()?;
    let home = plan.target.parent().context("Missing application folder")?;
    let stage_home = if plan.msi { plan.root.as_path() } else { home };
    let stage_root = stage_home.join("runtime/self-update");
    files::no_links(&stage_root)?;
    files::inside(&stage, &stage_root.canonicalize()?)?;
    files::inside(&plan.staged, &stage)?;
    files::no_links(&plan.target)?;
    if std::env::current_exe()?.canonicalize()? != stage.join(helper_name()).canonicalize()? {
        bail!("Update helper location mismatch");
    }
    if files::hash(&staged_executable(&plan.staged))? != plan.hash {
        bail!("Staged manager checksum mismatch");
    }
    #[cfg(target_os = "windows")]
    unsafe {
        use windows::Win32::{
            Foundation::{CloseHandle, WAIT_OBJECT_0},
            System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_ACCESS_RIGHTS},
        };
        if let Ok(handle) = OpenProcess(PROCESS_ACCESS_RIGHTS(0x00100000), false, plan.parent_pid) {
            let status = WaitForSingleObject(handle, 60_000);
            let _ = CloseHandle(handle);
            if status != WAIT_OBJECT_0 {
                bail!("Manager did not close; update was not applied");
            }
        }
    }
    #[cfg(unix)]
    {
        let deadline = Instant::now() + Duration::from_secs(60);
        while unsafe { libc::kill(i32::try_from(plan.parent_pid)?, 0) } == 0 {
            if Instant::now() >= deadline {
                bail!("Manager did not close; update was not applied");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    if plan.msi {
        #[cfg(target_os = "windows")]
        {
            if plan.staged.extension().and_then(|s| s.to_str()) != Some("msi")
                || !home.join("installation-msi.json").is_file()
            {
                bail!("Invalid MSI update plan");
            }
            let package_path = installer_path(&plan.staged);
            let log_path = installer_path(&stage.join("install.log"));
            let status = crate::platform::hidden(
                Command::new("msiexec.exe")
                    .arg("/i")
                    .arg(package_path)
                    .arg("/passive")
                    .arg("/norestart")
                    .arg("/L*v")
                    .arg(log_path),
            )
            .status()?;
            match status.code() {
                Some(0 | 3010) => {}
                Some(1602) => {
                    bail!("Manager update was canceled; the previous installation was kept")
                }
                code => bail!(
                    "Windows Installer could not update the manager (exit {code:?}). See {}",
                    stage.join("install.log").display()
                ),
            }
            Command::new(&plan.target)
                .arg("--root")
                .arg(&plan.root)
                .arg("--tools")
                .arg(&plan.tools)
                .spawn()?;
            return Ok(());
        }
        #[cfg(not(target_os = "windows"))]
        bail!("MSI updates are only supported on Windows");
    }
    let backup = stage.join("previous.exe");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match fs::rename(&plan.target, &backup) {
            Ok(()) => break,
            Err(e) if Instant::now() >= deadline => {
                return Err(e)
                    .context("Close all manager and builder windows, then retry the update")
            }
            Err(_) => std::thread::sleep(Duration::from_millis(250)),
        }
    }
    let result = (|| -> Result<()> {
        fs::rename(&plan.staged, &plan.target)?;
        let mut command = Command::new(&plan.target);
        command
            .arg("--root")
            .arg(&plan.root)
            .arg("--tools")
            .arg(&plan.tools)
            .spawn()?;
        Ok(())
    })();
    if result.is_err() {
        if plan.target.is_dir() {
            fs::remove_dir_all(&plan.target)?;
        } else if plan.target.exists() {
            fs::remove_file(&plan.target)?;
        }
        fs::rename(&backup, &plan.target).context("Could not restore previous manager")?;
    }
    result
}

/// macOS startup errors belong to user data, never the signed bundle. Other
/// platforms retain their existing executable-relative startup log location.
pub fn startup_log_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        macos_data_home().map(|home| home.join("logs/startup.log"))
    }
    #[cfg(not(target_os = "macos"))]
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join("logs/startup.log")))
}

/// A failed helper leaves a durable diagnostic for the next manager window.
pub fn startup_message() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let path = macos_data_home()?.join("self-update-result.json");
        match files::read_json::<UpdateResult>(&path) {
            Ok(result) if matches!(result.status, UpdateStatus::Failed) => Some(result.message),
            Ok(_) => None,
            Err(_) if path.exists() => Some(format!(
                "Could not read the previous manager update result. See {}",
                path.display()
            )),
            Err(_) => None,
        }
    }
    #[cfg(not(target_os = "macos"))]
    None
}

#[cfg(target_os = "macos")]
fn macos_data_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support/Craft Apps Manager"))
}
#[cfg(target_os = "macos")]
#[derive(Serialize, Deserialize)]
struct UpdateResult {
    status: UpdateStatus,
    message: String,
}
#[cfg(target_os = "macos")]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum UpdateStatus {
    LaunchPending,
    LaunchRequested,
    Failed,
}
#[cfg(target_os = "macos")]
const STAGE_PREFIX: &str = ".craft-manager-update-";

#[cfg(target_os = "macos")]
fn create_macos_stage(home: &Path) -> Result<PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    let stage = home.join(format!("{STAGE_PREFIX}{}", uuid::Uuid::new_v4().simple()));
    fs::DirBuilder::new().mode(0o700).create(&stage).with_context(|| {
        format!("Cannot stage a manager update beside the application in {}. Move the manager to a writable folder or update manually", home.display())
    })?;
    if fs::metadata(home)?.dev() != fs::metadata(&stage)?.dev() {
        bail!("Manager update stage is on a different filesystem");
    }
    Ok(stage)
}

/// Accept one exact private sibling layout, rather than arbitrary paths from a
/// plan. Check lexical paths before canonicalizing so links/escapes cannot be
/// laundered into trusted paths.
#[cfg(target_os = "macos")]
fn validate_macos_layout(plan: &Plan, plan_path: &Path, helper: &Path) -> Result<PathBuf> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    if plan.layout != 1 || plan.msi {
        bail!("This manager update plan uses an unsupported legacy layout. Reopen the manager and download the update again; no application was changed");
    }
    let target = &plan.target;
    if !target.is_absolute() || target.canonicalize()? != *target {
        bail!("Manager target must be an absolute, unlinked canonical path");
    }
    files::no_links(target)?;
    if target.extension().is_none_or(|ext| ext != "app") {
        bail!("Manager target is not an app bundle");
    }
    let home = target.parent().context("Missing application folder")?;
    let stage = plan_path.parent().context("Missing update folder")?;
    files::inside(stage, home)?;
    if stage.parent() != Some(home) || stage.canonicalize()? != stage {
        bail!("Manager update stage must be a direct unlinked sibling of the target");
    }
    let name = stage
        .file_name()
        .and_then(|name| name.to_str())
        .context("Invalid update stage name")?;
    let id = name
        .strip_prefix(STAGE_PREFIX)
        .context("Invalid manager update stage name")?;
    if uuid::Uuid::parse_str(id)?.simple().to_string() != id {
        bail!("Invalid manager update stage identifier");
    }
    let metadata = fs::metadata(stage)?;
    if metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o777 != 0o700
        || metadata.dev() != fs::metadata(target)?.dev()
    {
        bail!("Manager update stage must be private, owned by this user, and on the target filesystem");
    }
    if plan_path != stage.join("plan.json")
        || plan.staged != stage.join("package/Craft Apps Manager.app")
        || helper != stage.join(helper_name())
    {
        bail!("Manager update plan, package, or helper location mismatch");
    }
    files::no_links(stage)?;
    // The helper is the already-approved running version, not downloaded code.
    if files::hash(helper)? != files::hash(&staged_executable(target))? {
        bail!("Manager update helper does not match the previous manager");
    }
    verify_bundle(target)?;
    Ok(stage.to_path_buf())
}

#[cfg(target_os = "macos")]
fn validate_macos_replacement(plan: &Plan) -> Result<()> {
    files::no_links(&plan.staged)?;
    if files::hash(&staged_executable(&plan.staged))? != plan.hash {
        bail!("Staged manager checksum mismatch; download the update again");
    }
    verify_bundle(&plan.staged)
}

#[cfg(target_os = "macos")]
fn launch_macos(plan: &Plan) -> Result<()> {
    let output = Command::new("/usr/bin/open")
        .arg("-n")
        .arg(&plan.target)
        .arg("--args")
        .arg("--root")
        .arg(&plan.root)
        .arg("--tools")
        .arg(&plan.tools)
        .output()
        .context("Could not request a manager restart")?;
    check_open_status(output.status, &output.stderr)
}
#[cfg(target_os = "macos")]
fn check_open_status(status: std::process::ExitStatus, stderr: &[u8]) -> Result<()> {
    if !status.success() {
        bail!(
            "macOS rejected the manager launch request ({status}): {}",
            String::from_utf8_lossy(stderr).trim()
        );
    }
    Ok(())
}

/// On failure never delete either version: move the failed replacement aside
/// before restoring. If any rollback move fails, the original survives at the
/// documented previous.app location.
#[cfg(target_os = "macos")]
fn swap_macos(
    plan: &Plan,
    stage: &Path,
    mut rename: impl FnMut(&Path, &Path) -> std::io::Result<()>,
    mut verify: impl FnMut(&Path) -> Result<()>,
    mut launch: impl FnMut() -> Result<()>,
) -> Result<()> {
    let backup = stage.join("previous.app");
    if backup.exists() || stage.join("failed.app").exists() {
        bail!(
            "This manager update was already attempted. Recovery files are in {}",
            stage.display()
        );
    }
    // macOS rename errors here are permanent permission/path errors, not an
    // executable lock. Report them promptly instead of retrying for a minute.
    rename(&plan.target, &backup).with_context(|| {
        format!(
            "Could not move the previous manager into {}. Check the application folder permissions",
            backup.display()
        )
    })?;
    let result: Result<()> = (|| {
        rename(&plan.staged, &plan.target).context("Could not publish the staged manager")?;
        verify(&plan.target)?;
        launch()?;
        Ok(())
    })();
    if let Err(original) = result {
        let rollback = (|| -> Result<()> {
            if plan.target.exists() {
                rename(&plan.target, &stage.join("failed.app"))
                    .context("Could not preserve the failed replacement before restoring")?;
            }
            rename(&backup, &plan.target).context("Could not restore the previous manager")?;
            Ok(())
        })();
        return match rollback {
            Ok(()) => Err(original.context("Update failed; the previous manager was restored")),
            Err(rollback) => Err(anyhow::anyhow!("{original:#}. Recovery failed: {rollback:#}. The previous manager is preserved at {}", backup.display())),
        };
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn report_macos_result(stage: &Path, destination: &Path, result: &UpdateResult) -> Result<()> {
    // Keep a second copy beside recovery files. A reporting failure must not
    // remove the only surviving bundle or obscure the original apply error.
    let local = files::write_json(&stage.join("result.json"), result);
    let persistent = files::write_json(destination, result);
    match (local, persistent) {
        (Ok(()), Ok(())) => Ok(()),
        (local, persistent) => bail!("Could not save all manager update results (recovery record: {local:?}; startup record: {persistent:?}). Recovery files remain in {}", stage.display()),
    }
}

#[cfg(target_os = "macos")]
fn apply_macos(plan_path: &Path) -> Result<()> {
    let result = apply_macos_trusted(plan_path);
    if let Err(original) = result {
        // Even a rejected legacy/tampered plan gets an actionable next-startup
        // error, but never writes to or launches a path supplied by that plan.
        let reporting = macos_data_home()
            .context("No macOS user data directory")
            .and_then(|home| {
                files::write_json(
                    &home.join("self-update-result.json"),
                    &UpdateResult {
                        status: UpdateStatus::Failed,
                        message: format!("Manager update failed: {original:#}"),
                    },
                )
            });
        if let Err(error) = reporting {
            bail!("{original:#}. Could not persist the update failure: {error:#}");
        }
        return Err(original);
    }
    result
}
#[cfg(target_os = "macos")]
fn apply_macos_trusted(plan_path: &Path) -> Result<()> {
    let plan: Plan = files::read_json(plan_path)?;
    let helper = std::env::current_exe()?.canonicalize()?;
    let stage = validate_macos_layout(&plan, plan_path, &helper)?;
    let previous_hash = files::hash(&staged_executable(&plan.target))?;
    let destination = macos_data_home()
        .context("No macOS user data directory")?
        .join("self-update-result.json");
    let result = (|| -> Result<()> {
        validate_macos_replacement(&plan)?;
        check_update_directory(plan.target.parent().context("Missing application folder")?)?;
        let deadline = Instant::now() + Duration::from_secs(60);
        while unsafe { libc::kill(i32::try_from(plan.parent_pid)?, 0) } == 0 {
            if Instant::now() >= deadline {
                bail!("Manager did not close; update was not applied");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        files::no_links(&plan.target)?;
        files::no_links(&plan.staged)?;
        verify_bundle(&plan.target)?;
        if files::hash(&staged_executable(&plan.target))? != previous_hash {
            bail!("Previous manager changed while waiting; update was not applied");
        }
        swap_macos(
            &plan,
            &stage,
            |from, to| fs::rename(from, to),
            |target| {
                files::no_links(target)?;
                verify_bundle(target)?;
                if files::hash(&staged_executable(target))? != plan.hash {
                    bail!("Published manager checksum mismatch");
                }
                Ok(())
            },
            || {
                report_macos_result(&stage, &destination, &UpdateResult {
                    status: UpdateStatus::LaunchPending,
                    message: format!("Replacement verified; manager launch request pending. Previous manager: {}", stage.join("previous.app").display()),
                })?;
                launch_macos(&plan)
            },
        )
    })();
    let message = match &result {
        Ok(()) => format!(
            "Manager replaced; macOS accepted the launch request. Previous manager: {}",
            stage.join("previous.app").display()
        ),
        Err(error) => format!(
            "Manager update failed: {error:#}. Recovery files: {}",
            stage.display()
        ),
    };
    let reporting = report_macos_result(
        &stage,
        &destination,
        &UpdateResult {
            status: if result.is_ok() {
                UpdateStatus::LaunchRequested
            } else {
                UpdateStatus::Failed
            },
            message,
        },
    );
    if let Err(original) = result {
        // Only reopen the validated original, never a failed or tampered new
        // target. Report first so the recovered manager sees the diagnostic.
        let recovery = if files::no_links(&plan.target).is_ok()
            && files::hash(&staged_executable(&plan.target)).is_ok_and(|hash| hash == previous_hash)
            && verify_bundle(&plan.target).is_ok()
            && unsafe { libc::kill(i32::try_from(plan.parent_pid)?, 0) } != 0
        {
            launch_macos(&plan)
        } else {
            Ok(())
        };
        let mut message = format!("{original:#}");
        if let Err(error) = reporting {
            message.push_str(&format!(". Result reporting failed: {error:#}"));
        }
        if let Err(error) = recovery {
            message.push_str(&format!(
                ". Previous manager launch request failed: {error:#}"
            ));
        }
        bail!("{message}");
    }
    reporting.context(
        "Manager was replaced and its launch request accepted, but result reporting failed",
    )
}

/// The file whose checksum the update plan records.
fn staged_executable(staged: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        staged.join("Contents/MacOS/craft-apps-manager")
    } else {
        staged.to_path_buf()
    }
}
/// The `.app` bundle that contains the running executable.
#[cfg(target_os = "macos")]
fn app_bundle(executable: &Path) -> Result<PathBuf> {
    executable
        .ancestors()
        .nth(3)
        .filter(|bundle| {
            bundle.extension().is_some_and(|e| e == "app")
                && staged_executable(bundle) == executable
        })
        .map(Path::to_path_buf)
        .context("Only Craft Apps Manager.app can update itself. Development builds cannot.")
}
/// The helper is a copy of the running executable, which the user already
/// approved. Copying keeps a browser download's quarantine flag, which would
/// make Gatekeeper block the helper, so drop it from the copy.
#[cfg(target_os = "macos")]
fn clear_quarantine(path: &Path) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    if unsafe {
        libc::removexattr(
            path.as_ptr(),
            c"com.apple.quarantine".as_ptr(),
            libc::XATTR_NOFOLLOW,
        )
    } != 0
    {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ENOATTR) {
            return Err(error).context("Could not prepare the update helper");
        }
    }
    Ok(())
}
#[cfg(target_os = "macos")]
fn verify_bundle(bundle: &Path) -> Result<()> {
    if crate::platform::bundle_value(bundle, "CFBundleIdentifier").as_deref() != Some(BUNDLE_ID) {
        bail!("Update package is not Craft Apps Manager");
    }
    let out = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(bundle)
        .output()?;
    if !out.status.success() {
        bail!(
            "Update package signature is invalid: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}
fn helper_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "update-helper.exe"
    } else {
        "update-helper"
    }
}
#[cfg(target_os = "windows")]
fn installer_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    let text = if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
    };
    PathBuf::from(text)
}
#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    #[test]
    fn system_installation_reports_package_update_before_download() {
        use std::os::unix::fs::PermissionsExt;
        let directory = std::env::temp_dir().join(format!("craft-update-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        super::check_update_directory(&directory).unwrap();
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o555)).unwrap();
        let result = super::check_update_directory(&directory);
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::remove_dir_all(directory).unwrap();
        if unsafe { libc::geteuid() } != 0 {
            assert!(result.unwrap_err().to_string().contains("DEB or RPM"));
        }
    }
}
#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;
    #[test]
    fn windows_installer_paths_do_not_use_extended_prefixes() {
        assert_eq!(
            installer_path(Path::new(r"\\?\C:\App Data\release.msi")),
            PathBuf::from(r"C:\App Data\release.msi")
        );
        assert_eq!(
            installer_path(Path::new(r"\\?\UNC\server\share\release.msi")),
            PathBuf::from(r"\\server\share\release.msi")
        );
    }
    #[test]
    fn installer_updates_require_msi_and_portable_updates_require_zip() {
        let arch = crate::model::MANAGER_ARCH;
        let release = |extension: &str| Release {
            tag_name: "v0.5.0".into(),
            draft: false,
            prerelease: false,
            assets: vec![Asset {
                name: format!("Craft-Apps-Manager-0.5.0-windows-{arch}.{extension}"),
                size: 1,
                digest: None,
                browser_download_url: format!(
                    "{REPOSITORY}/releases/download/v0.5.0/package.{extension}"
                ),
            }],
        };
        assert!(select_package(release("zip"), "0.4.0", true).is_err());
        assert!(select_package(release("msi"), "0.4.0", false).is_err());
        assert!(select_package(release("msi"), "0.4.0", true)
            .unwrap()
            .is_some());
        assert!(select_package(release("zip"), "0.4.0", false)
            .unwrap()
            .is_some());
    }
    #[test]
    fn self_update_never_switches_architecture() {
        let other = if crate::model::MANAGER_ARCH == "x86" {
            "x64"
        } else {
            "x86"
        };
        let asset = |arch: &str| Asset {
            name: format!("Craft-Apps-Manager-0.4.0-windows-{arch}.zip"),
            size: 1,
            digest: None,
            browser_download_url: format!("{REPOSITORY}/releases/download/v0.4.0/{arch}.zip"),
        };
        let mut release = Release {
            tag_name: "v0.4.0".into(),
            draft: false,
            prerelease: false,
            assets: vec![asset(other)],
        };
        assert!(select(release.clone(), "0.3.0").is_err());
        release.assets.push(asset(crate::model::MANAGER_ARCH));
        let selected = select(release, "0.3.0").unwrap().unwrap();
        assert!(selected
            .asset
            .browser_download_url
            .ends_with(&format!("/{}.zip", crate::model::MANAGER_ARCH)));
    }
    #[test]
    fn only_new_stable_release_from_our_repository() {
        let release = |tag: &str, prerelease, url: &str| Release {
            tag_name: tag.into(),
            draft: false,
            prerelease,
            assets: vec![Asset {
                name: format!(
                    "Craft-Apps-Manager-{}-windows-{}.zip",
                    tag.trim_start_matches('v'),
                    crate::model::MANAGER_ARCH
                ),
                size: 1,
                digest: None,
                browser_download_url: url.into(),
            }],
        };
        let url = format!("{REPOSITORY}/releases/download/v0.3.0/test.zip");
        assert!(select(release("v0.2.1", false, &url), "0.2.1")
            .unwrap()
            .is_none());
        assert!(select(release("v0.3.0", true, &url), "0.2.1")
            .unwrap()
            .is_none());
        assert!(select(release("v0.3.0", false, &url), "0.2.1")
            .unwrap()
            .is_some());
        assert!(select(
            release("v0.3.0", false, "https://example.com/file.zip"),
            "0.2.1"
        )
        .is_err());
    }
}
#[cfg(all(test, target_os = "macos"))]
mod macos_tests {
    use super::*;
    #[test]
    fn selects_universal_zip_from_configured_repository() {
        let release = |name: &str, url: &str| Release {
            tag_name: "v0.5.0".into(),
            draft: false,
            prerelease: false,
            assets: vec![Asset {
                name: name.into(),
                size: 1,
                digest: None,
                browser_download_url: url.into(),
            }],
        };
        let url = format!("{REPOSITORY}/releases/download/v0.5.0/package.zip");
        let selected = select(
            release("Craft-Apps-Manager-0.5.0-macos-universal.zip", &url),
            "0.4.0",
        )
        .unwrap()
        .unwrap();
        assert_eq!(selected.version, "0.5.0");
        assert!(select(
            release("Craft-Apps-Manager-0.5.0-linux-x64.zip", &url),
            "0.4.0"
        )
        .is_err());
        assert!(select(
            release(
                "Craft-Apps-Manager-0.5.0-macos-universal.zip",
                "https://example.com/package.zip"
            ),
            "0.4.0"
        )
        .is_err());
    }
    #[test]
    fn update_helper_drops_inherited_quarantine() {
        let file = std::env::temp_dir().join(format!("craft-helper-{}", uuid::Uuid::new_v4()));
        fs::write(&file, "helper").unwrap();
        clear_quarantine(&file).unwrap();
        let status = Command::new("/usr/bin/xattr")
            .args(["-w", "com.apple.quarantine", "0083;6720f000;Safari;"])
            .arg(&file)
            .status()
            .unwrap();
        assert!(status.success());
        clear_quarantine(&file).unwrap();
        let out = Command::new("/usr/bin/xattr").arg(&file).output().unwrap();
        assert!(!String::from_utf8_lossy(&out.stdout).contains("com.apple.quarantine"));
        fs::remove_file(file).unwrap();
    }
    #[test]
    fn only_app_bundles_update_themselves() {
        let bundle = Path::new("/Applications/Craft Apps Manager.app");
        assert_eq!(
            app_bundle(&bundle.join("Contents/MacOS/craft-apps-manager")).unwrap(),
            bundle
        );
        assert!(app_bundle(Path::new("/repo/target/release/craft-apps-manager")).is_err());
        assert!(app_bundle(&bundle.join("Contents/MacOS/other")).is_err());
    }
    struct Fixture {
        home: PathBuf,
        stage: PathBuf,
        plan: Plan,
    }
    impl Fixture {
        fn new() -> Self {
            let home =
                std::env::temp_dir().join(format!("craft-self-update-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&home).unwrap();
            let home = home.canonicalize().unwrap();
            let stage = create_macos_stage(&home).unwrap();
            let target = home.join("Craft Apps Manager.app");
            let staged = stage.join("package/Craft Apps Manager.app");
            for bundle in [&target, &staged] {
                fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
                fs::create_dir_all(bundle.join("Contents/Resources")).unwrap();
                fs::copy("/usr/bin/true", staged_executable(bundle)).unwrap();
                fs::write(bundle.join("Contents/Info.plist"), format!(
                    "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>{BUNDLE_ID}</string><key>CFBundleExecutable</key><string>craft-apps-manager</string><key>CFBundlePackageType</key><string>APPL</string></dict></plist>"
                )).unwrap();
                let output = Command::new("/usr/bin/codesign")
                    .args(["--force", "--sign", "-"])
                    .arg(bundle)
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                verify_bundle(bundle).unwrap();
            }
            fs::write(target.join("Contents/Resources/old-marker"), "old bytes").unwrap();
            fs::write(staged.join("Contents/Resources/new-marker"), "new bytes").unwrap();
            // Marker files are part of the signed resources, so resign after adding them.
            for bundle in [&target, &staged] {
                assert!(Command::new("/usr/bin/codesign")
                    .args(["--force", "--sign", "-"])
                    .arg(bundle)
                    .output()
                    .unwrap()
                    .status
                    .success());
            }
            fs::copy(staged_executable(&target), stage.join(helper_name())).unwrap();
            let plan = Plan {
                layout: 1,
                msi: false,
                parent_pid: u32::MAX,
                hash: files::hash(&staged_executable(&staged)).unwrap(),
                target,
                staged,
                root: home.join("separate-library"),
                tools: home.join("tools"),
            };
            files::write_json(&stage.join("plan.json"), &plan).unwrap();
            Self { home, stage, plan }
        }
        fn validate(&self) -> Result<PathBuf> {
            validate_macos_layout(
                &self.plan,
                &self.stage.join("plan.json"),
                &self.stage.join(helper_name()),
            )
        }
        fn old_is_present(&self) -> bool {
            fs::read(self.plan.target.join("Contents/Resources/old-marker"))
                .is_ok_and(|bytes| bytes == b"old bytes")
        }
        fn swap(&self, failures: &[usize], verify_fails: bool, launch_fails: bool) -> Result<()> {
            let mut calls = 0;
            swap_macos(
                &self.plan,
                &self.stage,
                |from, to| {
                    calls += 1;
                    if failures.contains(&calls) {
                        return Err(std::io::Error::from_raw_os_error(libc::EACCES));
                    }
                    fs::rename(from, to)
                },
                |_| {
                    if verify_fails {
                        bail!("injected verification failure");
                    }
                    Ok(())
                },
                || {
                    if launch_fails {
                        bail!("injected open rejection");
                    }
                    Ok(())
                },
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.home);
        }
    }
    #[test]
    fn private_sibling_layout_keeps_library_independent() {
        use std::os::unix::fs::MetadataExt;
        let fixture = Fixture::new();
        assert_eq!(fixture.validate().unwrap(), fixture.stage);
        assert_eq!(
            fs::metadata(&fixture.stage).unwrap().dev(),
            fs::metadata(&fixture.plan.target).unwrap().dev()
        );
        validate_macos_replacement(&fixture.plan).unwrap();
        fixture.swap(&[], false, false).unwrap();
        assert_eq!(
            fs::read(fixture.plan.target.join("Contents/Resources/new-marker")).unwrap(),
            b"new bytes"
        );
        assert_eq!(
            fs::read(
                fixture
                    .stage
                    .join("previous.app/Contents/Resources/old-marker")
            )
            .unwrap(),
            b"old bytes"
        );
        assert!(fixture.swap(&[], false, false).is_err());
    }
    #[test]
    fn legacy_plans_are_rejected_before_mutation() {
        let mut fixture = Fixture::new();
        fixture.plan.layout = 0;
        assert!(fixture
            .validate()
            .unwrap_err()
            .to_string()
            .contains("legacy layout"));
        assert!(fixture.old_is_present());
    }
    #[test]
    fn corrupt_replacement_is_rejected_before_swap() {
        let fixture = Fixture::new();
        fs::write(staged_executable(&fixture.plan.staged), "corrupt").unwrap();
        assert!(validate_macos_replacement(&fixture.plan)
            .unwrap_err()
            .to_string()
            .contains("checksum"));
        assert!(fixture.old_is_present());
    }
    #[test]
    fn first_move_failure_is_prompt_and_preserves_target() {
        let fixture = Fixture::new();
        let started = Instant::now();
        let error = fixture.swap(&[1], false, false).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(error.to_string().contains("permissions"));
        assert!(fixture.old_is_present());
        assert!(!fixture.stage.join("previous.app").exists());
    }
    #[test]
    fn publish_failure_restores_original_bytes() {
        let fixture = Fixture::new();
        let error = fixture.swap(&[2], false, false).unwrap_err();
        assert!(format!("{error:#}").contains("Could not publish"));
        assert!(fixture.old_is_present());
    }
    #[test]
    fn published_verification_and_launch_failures_restore_original() {
        for (verify, launch) in [(true, false), (false, true)] {
            let fixture = Fixture::new();
            let error = fixture.swap(&[], verify, launch).unwrap_err();
            assert!(error.to_string().contains("restored"));
            assert!(fixture.old_is_present());
            assert_eq!(
                fs::read(
                    fixture
                        .stage
                        .join("failed.app/Contents/Resources/new-marker")
                )
                .unwrap(),
                b"new bytes"
            );
        }
    }
    #[test]
    fn rollback_and_reporting_failures_keep_original_recovery_bytes() {
        // Failure publishing followed by failure restoring; and failure moving
        // the rejected replacement aside, or failure restoring after that move.
        for (failures, verify) in [(vec![2, 3], false), (vec![3], true), (vec![4], true)] {
            let fixture = Fixture::new();
            let error = fixture.swap(&failures, verify, false).unwrap_err();
            assert!(error.to_string().contains("previous.app"));
            fs::write(fixture.home.join("blocked"), "not a directory").unwrap();
            let record = UpdateResult {
                status: UpdateStatus::Failed,
                message: format!("{error:#}"),
            };
            assert!(report_macos_result(
                &fixture.stage,
                &fixture.home.join("blocked/result.json"),
                &record
            )
            .is_err());
            assert_eq!(
                fs::read(
                    fixture
                        .stage
                        .join("previous.app/Contents/Resources/old-marker")
                )
                .unwrap(),
                b"old bytes"
            );
            let retained: UpdateResult =
                files::read_json(&fixture.stage.join("result.json")).unwrap();
            assert!(retained.message.contains("Recovery failed"));
        }
    }
    #[test]
    fn missing_local_result_does_not_prevent_persistent_result() {
        let fixture = Fixture::new();
        fs::create_dir(fixture.stage.join("result.json")).unwrap();
        let path = fixture.home.join("startup-result.json");
        let record = UpdateResult {
            status: UpdateStatus::Failed,
            message: "original failure".into(),
        };
        assert!(report_macos_result(&fixture.stage, &path, &record).is_err());
        let retained: UpdateResult = files::read_json(&path).unwrap();
        assert_eq!(retained.message, "original failure");
        assert!(fixture.old_is_present());
    }
    #[test]
    fn open_exit_status_is_checked() {
        use std::os::unix::process::ExitStatusExt;
        assert!(check_open_status(std::process::ExitStatus::from_raw(0), b"").is_ok());
        let error = check_open_status(std::process::ExitStatus::from_raw(256), b"launch denied")
            .unwrap_err();
        assert!(error.to_string().contains("launch denied"));
    }
    #[test]
    fn link_escape_and_untrusted_helper_plans_are_rejected() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let mut fixture = Fixture::new();
        let staged = fixture.plan.staged.clone();
        fixture.plan.staged = fixture
            .stage
            .join("package/../package/Craft Apps Manager.app");
        assert!(fixture.validate().is_err());
        fixture.plan.staged = fixture.plan.target.clone();
        assert!(fixture.validate().is_err());
        fixture.plan.staged = staged;
        assert!(fixture.validate().is_ok());
        fs::set_permissions(&fixture.stage, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(fixture.validate().is_err());
        fs::set_permissions(&fixture.stage, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(
            fixture.stage.join(helper_name()),
            "not the approved manager",
        )
        .unwrap();
        assert!(fixture.validate().is_err());
        fs::copy(
            staged_executable(&fixture.plan.target),
            fixture.stage.join(helper_name()),
        )
        .unwrap();
        symlink(&fixture.plan.target, fixture.stage.join("linked.app")).unwrap();
        assert!(fixture.validate().is_err());
        fs::remove_file(fixture.stage.join("linked.app")).unwrap();
        let alias = fixture.home.join("Alias.app");
        symlink(&fixture.plan.target, &alias).unwrap();
        fixture.plan.target = alias;
        assert!(fixture.validate().is_err());
    }
    #[test]
    fn unrelated_identity_and_invalid_resource_signature_are_rejected() {
        let fixture = Fixture::new();
        fs::write(fixture.plan.staged.join("Contents/Info.plist"), "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>unrelated.app</string></dict></plist>").unwrap();
        assert!(validate_macos_replacement(&fixture.plan).is_err());
        fs::write(
            fixture.plan.target.join("Contents/Resources/old-marker"),
            "tampered resources",
        )
        .unwrap();
        assert!(fixture.validate().is_err());
    }
    #[test]
    fn unwritable_target_fails_preflight_without_removing_original() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = Fixture::new();
        fs::set_permissions(&fixture.home, fs::Permissions::from_mode(0o555)).unwrap();
        let result = check_update_directory(&fixture.home);
        fs::set_permissions(&fixture.home, fs::Permissions::from_mode(0o755)).unwrap();
        if unsafe { libc::geteuid() } != 0 {
            assert!(result.unwrap_err().to_string().contains("cannot write"));
        }
        assert!(fixture.old_is_present());
    }
    #[test]
    fn prelaunch_reporting_failure_restores_previous_manager() {
        let fixture = Fixture::new();
        fs::write(fixture.home.join("blocked"), "not a directory").unwrap();
        let error = swap_macos(
            &fixture.plan,
            &fixture.stage,
            |from, to| fs::rename(from, to),
            |_| Ok(()),
            || {
                report_macos_result(
                    &fixture.stage,
                    &fixture.home.join("blocked/result.json"),
                    &UpdateResult {
                        status: UpdateStatus::LaunchPending,
                        message: "replacement verified; launch pending".into(),
                    },
                )
            },
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("Could not save"));
        assert!(fixture.old_is_present());
        assert!(fixture
            .stage
            .join("failed.app/Contents/Resources/new-marker")
            .exists());
    }
    #[test]
    fn pending_and_success_results_replace_stale_failure_before_startup() {
        let fixture = Fixture::new();
        let path = fixture.home.join("startup-result.json");
        let failed = UpdateResult {
            status: UpdateStatus::Failed,
            message: "earlier failure".into(),
        };
        report_macos_result(&fixture.stage, &path, &failed).unwrap();
        for status in [UpdateStatus::LaunchPending, UpdateStatus::LaunchRequested] {
            let record = UpdateResult {
                status,
                message: "replacement verified".into(),
            };
            report_macos_result(&fixture.stage, &path, &record).unwrap();
            let result: UpdateResult = files::read_json(&path).unwrap();
            assert!(!matches!(result.status, UpdateStatus::Failed));
            assert!(!result.message.contains("earlier failure"));
        }
    }
}
