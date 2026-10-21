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

#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod linux;

/// The GitHub repository (owner/name) manager updates come from; set at build time.
pub const REPOSITORY_NAME: &str = env!("CRAFT_MANAGER_REPOSITORY");
pub const REPOSITORY: &str = concat!("https://github.com/", env!("CRAFT_MANAGER_REPOSITORY"));
/// Bundle identifier of the macOS app, as set by scripts/packaging/package-macos.sh.
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
/// Package ownership is checked once: never run package queries during painting.
pub fn installed_with_linux_package() -> bool {
    #[cfg(target_os = "linux")]
    {
        static INSTALLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *INSTALLED.get_or_init(|| {
            let Ok(executable) = std::env::current_exe().and_then(|p| p.canonicalize()) else {
                return false;
            };
            installed_with_linux_package_at(&executable)
        })
    }
    #[cfg(not(target_os = "linux"))]
    false
}

#[cfg(target_os = "linux")]
fn installed_with_linux_package_at(executable: &Path) -> bool {
    for (command, args) in [
        ("dpkg-query", vec!["-S"]),
        ("rpm", vec!["-qf", "--queryformat", "%{NAME}"]),
        ("pacman", vec!["-Qqo"]),
    ] {
        if let Ok(output) = Command::new(command).args(args).arg(executable).output() {
            if output.status.success()
                && linux_package_owner(command, &String::from_utf8_lossy(&output.stdout))
            {
                return true;
            }
        }
    }
    false
}

#[cfg(any(target_os = "linux", test))]
fn linux_package_owner(command: &str, output: &str) -> bool {
    if matches!(command, "rpm" | "pacman") {
        output.trim() == "craft-apps-manager"
    } else {
        output.lines().any(|line| {
            line.split_once(": ")
                .is_some_and(|(owner, _)| owner.split(':').next() == Some("craft-apps-manager"))
        })
    }
}
#[derive(Clone)]
pub struct Available {
    pub version: String,
    pub revision: u32,
    pub package_version: Option<String>,
    pub asset: Asset,
    pub selection: Option<crate::manager_updates::transaction::SignedSelection>,
}
impl Available {
    pub fn confirmation_hint(&self) -> String {
        #[cfg(feature = "service-updater")]
        if let Some(s) = self
            .selection
            .as_ref()
            .filter(|s| s.offer.route_ids.len() > 1)
        {
            let p = s
                .catalog
                .catalog()
                .packages
                .iter()
                .find(|p| p.id == s.offer.final_package_id)
                .expect("validated route");
            return format!("A compatibility update installs first. After restarting, the manager automatically finishes updating to version {}.",crate::update_revisions::display_version(&s.catalog.catalog().release(p).version,p.revision));
        }
        "Close other manager and builder windows first.".into()
    }
    pub fn display_version(&self) -> String {
        crate::update_revisions::display_version(&self.version, self.revision)
    }
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
        #[cfg(target_os = "linux")]
        {
            if installed_with_linux_package() {
                vec![linux::asset_name(
                    &version,
                    arch,
                    crate::installers::package_kind()?,
                )?]
            } else {
                vec![format!("Craft-Apps-Manager-{version}-linux-{arch}.zip")]
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            unreachable!()
        }
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
        .find_map(|name| assets.iter().position(|a| &a.name == name));
    let Some(index) = index else {
        return Ok(None);
    };
    let asset = assets.swap_remove(index);
    if !asset
        .browser_download_url
        .starts_with(&format!("{REPOSITORY}/releases/download/"))
    {
        bail!("Unexpected manager download location");
    }
    Ok(Some(Available {
        selection: None,
        version,
        revision: 1,
        package_version: None,
        asset,
    }))
}
pub fn check(paths: &Paths) -> Result<Option<Available>> {
    let result = crate::manager_updates::coordinator::check_status(
        paths,
        crate::manager_updates::check_state::CheckTrigger::Manual,
    )?;
    match result.status {
        crate::manager_updates::api::UpdateStatus::Current
        | crate::manager_updates::api::UpdateStatus::Available => Ok(result.available),
        crate::manager_updates::api::UpdateStatus::Blocked(reason) => bail!("{reason}"),
    }
}
pub(crate) fn check_legacy(paths: &Paths) -> Result<Option<Available>> {
    let network = Network::new(&paths.root)?;
    let url = format!("https://api.github.com/repos/{REPOSITORY_NAME}/releases/latest");
    let mut release: Release = network.json(&url)?;
    if let Some(asset) = release
        .assets
        .iter()
        .find(|asset| asset.name == "updates-v1.json")
    {
        anyhow::ensure!(
            asset.size <= 256 * 1024
                && asset.browser_download_url.starts_with(&format!(
                    "{REPOSITORY}/releases/download/{}/",
                    release.tag_name
                )),
            "Invalid revision manifest location or size"
        );
        let envelope: crate::update_revisions::Envelope =
            network.json_bounded(&crate::network::asset_download_url(asset)?, 256 * 1024)?;
        let version = release.tag_name.trim_start_matches('v');
        let manifest = crate::update_revisions::verify(
            &envelope,
            env!("CRAFT_UPDATE_PUBLIC_KEY_HEX"),
            REPOSITORY_NAME,
            version,
            chrono::Utc::now().timestamp(),
        )?;
        let architecture = if cfg!(target_os = "macos") {
            "universal"
        } else {
            crate::model::MANAGER_ARCH
        };
        #[cfg(target_os = "linux")]
        let format = if installed_with_linux_package() {
            match crate::installers::package_kind()? {
                crate::installers::PackageKind::Debian => "deb",
                crate::installers::PackageKind::Rpm => "rpm",
                crate::installers::PackageKind::Arch => "arch",
            }
        } else {
            "zip"
        };
        #[cfg(not(target_os = "linux"))]
        let format = if installed_with_msi() { "msi" } else { "zip" };
        // Hotfixes replace downloads under the same compatible filenames.
        // Refresh a cached release when its asset metadata no longer matches
        // the signed manifest; never weaken verification to accept stale data.
        if manifest.packages.iter().any(|package| {
            package.platform == crate::model::release_os()
                && package.architecture == architecture
                && package.format == format
                && !release.assets.iter().any(|asset| {
                    asset.name == package.asset
                        && asset.size == package.size
                        && asset.digest.as_ref().is_none_or(|digest| {
                            digest.eq_ignore_ascii_case(&format!("sha256:{}", package.sha256))
                        })
                })
        }) {
            release = network.json_fresh(&url)?;
        }
        return crate::update_revisions::select(
            &manifest,
            &release,
            (crate::model::release_os(), architecture, format),
            (
                env!("CARGO_PKG_VERSION"),
                crate::update_revisions::BUILD_REVISION,
            ),
        );
    }
    select(release, env!("CARGO_PKG_VERSION"))
}
#[derive(Serialize, Deserialize)]
pub struct Plan {
    #[serde(default)]
    pub expected_identity: Option<crate::manager_updates::api::ExpectedIdentity>,
    /// macOS layout 1 stages beside the target; older library plans are rejected.
    #[serde(default)]
    pub layout: u32,
    #[serde(default)]
    pub msi: bool,
    #[serde(default)]
    pub linux_package: bool,
    pub parent_pid: u32,
    pub target: PathBuf,
    pub staged: PathBuf,
    pub hash: String,
    pub root: PathBuf,
    pub tools: PathBuf,
}
pub fn prepare(paths: &Paths, available: &Available, job: &Job) -> Result<PathBuf> {
    #[cfg(feature = "service-updater")]
    if let Some(selection) = &available.selection {
        return crate::manager_updates::transaction::prepare_offer(
            paths,
            &selection.catalog,
            &selection.offer,
            job,
        );
    }
    crate::manager_updates::bridge::revoke(&paths.root)?;
    prepare_legacy(paths, available, job)
}
#[cfg(feature = "service-updater")]
pub fn prepare_continuation(
    paths: &Paths,
    available: &Available,
    journey: crate::manager_updates::bridge::Journey,
    job: &Job,
) -> Result<PathBuf> {
    let selection = available
        .selection
        .as_ref()
        .context("Automatic update requires signed service metadata")?;
    crate::manager_updates::transaction::prepare_continuation(paths, selection, journey, job)
}
pub(crate) fn prepare_legacy(paths: &Paths, available: &Available, job: &Job) -> Result<PathBuf> {
    #[cfg(target_os = "linux")]
    if installed_with_linux_package() {
        return linux::prepare(paths, available, job);
    }
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
                    expected_identity: None,
                    layout: 0,
                    msi: true,
                    linux_package: false,
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
                expected_identity: None,
                layout: u32::from(cfg!(target_os = "macos")),
                msi: false,
                linux_package: false,
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
#[cfg(all(target_os = "linux", feature = "service-updater"))]
pub(crate) fn prepare_native_v2(
    paths: &Paths,
    available: &Available,
    job: &Job,
) -> Result<PathBuf> {
    linux::prepare(paths, available, job)
}
#[cfg(all(target_os = "linux", feature = "service-updater"))]
pub(crate) fn validate_native_record(
    file: &Path,
    p: &crate::manager_updates::model::PackageRecord,
) -> Result<()> {
    linux::validate_record(file, p)
}
#[cfg(all(target_os = "linux", feature = "service-updater"))]
pub(crate) fn verify_native_registered(
    target: &Path,
    p: &crate::manager_updates::model::PackageRecord,
) -> Result<()> {
    linux::verify_registered(target, p)
}
#[cfg(unix)]
fn check_update_directory(home: &Path) -> Result<()> {
    let probe = home.join(format!(".craft-manager-update-{}", uuid::Uuid::new_v4()));
    let file = fs::OpenOptions::new().write(true).create_new(true).open(&probe)
        .context(if cfg!(target_os = "macos") {
            "Craft Apps Manager.app is in a folder you cannot write to. Move it to Applications or download the new version manually"
        } else {
            "The manager folder is not writable. Move the portable manager to a writable folder or download the new version manually"
        })?;
    drop(file);
    fs::remove_file(probe)?;
    Ok(())
}
pub fn launch(plan: &Path) -> Result<()> {
    if plan.file_name().is_some_and(|s| s == "authorization.json") {
        return crate::manager_updates::transaction::launch_v2(plan);
    }
    #[cfg(not(feature = "service-updater"))]
    crate::manager_updates::transaction::reject_file(plan)?;
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
    if plan_path
        .file_name()
        .is_some_and(|s| s == "authorization.json")
    {
        return crate::manager_updates::transaction::apply_v2(plan_path);
    }
    #[cfg(not(feature = "service-updater"))]
    crate::manager_updates::transaction::reject_file(plan_path)?;
    apply_legacy(plan_path)
}
pub(crate) fn apply_legacy(plan_path: &Path) -> Result<()> {
    apply_with_authorization(plan_path, None)
}
#[cfg(feature = "service-updater")]
pub(crate) fn apply_authorized(plan_path: &Path, authorization: &Path) -> Result<()> {
    apply_with_authorization(plan_path, Some(authorization))
}
fn apply_with_authorization(plan_path: &Path, authorization: Option<&Path>) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        apply_macos(plan_path, authorization)
    }
    #[cfg(not(target_os = "macos"))]
    apply_executable(plan_path, authorization)
}
#[cfg(not(target_os = "macos"))]
fn apply_executable(plan_path: &Path, authorization: Option<&Path>) -> Result<()> {
    let mut plan: Plan = files::read_json(plan_path)?;
    if plan.msi && plan.linux_package {
        bail!("Conflicting manager installation types in update plan");
    }
    files::no_links(&plan.target)?;
    plan.target = plan.target.canonicalize()?;
    plan.staged = plan.staged.canonicalize()?;
    let stage = plan_path
        .parent()
        .context("Missing update folder")?
        .canonicalize()?;
    let home = plan.target.parent().context("Missing application folder")?;
    let stage_home = if plan.msi || plan.linux_package {
        plan.root.as_path()
    } else {
        home
    };
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
    // Preparation released its execution locks before starting this helper. Reacquire
    // at commit so a Builder started in that interval cannot be replaced underneath.
    let _execution = plan
        .expected_identity
        .as_ref()
        .map(|_| crate::manager_updates::bridge::execution_gate())
        .transpose()?;
    let _authorization = authorization
        .map(|path| crate::manager_updates::transaction::authorize_plan(path, &plan))
        .transpose()?;
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
            let installation = (|| -> Result<()> {
                if let Some((code, text)) =
                    crate::manager_updates::transaction::install_native_batch(&plan, authorization)?
                {
                    fs::write(stage.join("install.log"), text)?;
                    if !matches!(code, 0 | 3010) {
                        bail!("Native MSI batch failed ({code})");
                    }
                    return Ok(());
                }
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
                    Some(0 | 3010) => Ok(()),
                    Some(1602) => bail!(
                        "Manager update was canceled; Windows Installer reported cancellation"
                    ),
                    code => bail!(
                        "Windows Installer could not update the manager (exit {code:?}). See {}",
                        stage.join("install.log").display()
                    ),
                }
            })();
            if let Err(error) = installation {
                return recover_failed_update(error, &windows_update_result_path()?, || {
                    files::no_links(&plan.target)?;
                    Command::new(&plan.target)
                        .arg("--root")
                        .arg(&plan.root)
                        .arg("--tools")
                        .arg(&plan.tools)
                        .spawn()?;
                    Ok(())
                });
            }
            if let Some(expected) = &plan.expected_identity {
                crate::manager_updates::transaction::verify_executable(
                    &plan.target,
                    expected,
                    &stage,
                )?;
            }
            crate::manager_updates::transaction::acknowledge_native_batch(&plan, authorization)?;
            let mut command = Command::new(&plan.target);
            crate::manager_updates::bridge::append_restart(&mut command, &plan);
            command
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
    if plan.linux_package {
        #[cfg(target_os = "linux")]
        {
            if !installed_with_linux_package_at(&plan.target) {
                bail!("Manager package ownership changed before restart");
            }
            if let Some(expected) = &plan.expected_identity {
                crate::manager_updates::transaction::verify_executable(
                    &plan.target,
                    expected,
                    &stage,
                )?;
            }
            let mut command = Command::new(&plan.target);
            crate::manager_updates::bridge::append_restart(&mut command, &plan);
            command
                .arg("--root")
                .arg(&plan.root)
                .arg("--tools")
                .arg(&plan.tools)
                .spawn()?;
            return Ok(());
        }
        #[cfg(not(target_os = "linux"))]
        bail!("Linux package updates are only supported on Linux");
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
        crate::portable_mode::adopt_updated_portable(home, &plan.root, Some(&plan.tools))?;
        let mut command = Command::new(&plan.target);
        crate::manager_updates::bridge::append_restart(&mut command, &plan);
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

#[cfg(target_os = "windows")]
fn windows_update_result_path() -> Result<PathBuf> {
    Ok(
        PathBuf::from(std::env::var_os("LOCALAPPDATA").context("No Windows user data directory")?)
            .join("Craft Apps Manager/self-update-result.txt"),
    )
}

#[cfg(target_os = "windows")]
fn recover_failed_update(
    error: anyhow::Error,
    result_path: &Path,
    mut reopen: impl FnMut() -> Result<()>,
) -> Result<()> {
    let message = format!("Manager update failed: {error:#}");
    let reporting = (|| -> Result<()> {
        fs::create_dir_all(result_path.parent().context("Missing result directory")?)?;
        fs::write(result_path, &message)?;
        Ok(())
    })();
    // Never assume a failed MSI left an executable; reopening errors remain explicit.
    let recovery = reopen();
    if let Err(recovery) = recovery {
        bail!("{message}. Could not reopen the manager: {recovery:#}. Result reporting: {reporting:?}");
    }
    reporting.context(format!("{message}. Could not persist the update failure"))?;
    Err(error)
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
        macos_startup_message_at(&path)
    }
    #[cfg(target_os = "windows")]
    {
        let path = windows_update_result_path().ok()?;
        let message = fs::read_to_string(&path).ok()?;
        let _ = fs::remove_file(path);
        Some(message)
    }
    #[cfg(target_os = "linux")]
    None
}

#[cfg(any(target_os = "macos", test))]
fn macos_startup_message_at(path: &Path) -> Option<String> {
    // Deliver once, like the Windows startup result. Keep a bounded diagnostic
    // copy outside the signed bundle; the helper also retains its recovery copy.
    if !path.exists() {
        return None;
    }
    let read = (|| -> Result<Vec<u8>> {
        anyhow::ensure!(!files::linked(path)?, "Linked manager update result");
        Ok(fs::read(path)?)
    })();
    let bytes = match read {
        Ok(bytes) => bytes,
        Err(error) => {
            return Some(format!(
                "Could not read the previous manager update result at {}: {error:#}",
                path.display()
            ))
        }
    };
    let message = match serde_json::from_slice::<UpdateResult>(&bytes) {
        Ok(result) if matches!(result.status, UpdateStatus::Failed) => result.message,
        Ok(_) => return None,
        Err(_) => format!(
            "Could not read the previous manager update result. See {}",
            path.display()
        ),
    };
    let acknowledged = (|| -> Result<()> {
        crate::settings::commit_metadata(&[(path.with_extension("acknowledged.json"), bytes)])?;
        fs::remove_file(path)?;
        Ok(())
    })();
    match acknowledged {
        Ok(()) => Some(message),
        Err(error) => Some(format!(
            "{message}\nCould not acknowledge this update notification: {error:#}"
        )),
    }
}

#[cfg(target_os = "macos")]
fn macos_data_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support/Craft Apps Manager"))
}
#[cfg(any(target_os = "macos", test))]
#[derive(Serialize, Deserialize)]
struct UpdateResult {
    status: UpdateStatus,
    message: String,
}
#[cfg(any(target_os = "macos", test))]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum UpdateStatus {
    LaunchPending,
    LaunchRequested,
    Failed,
}
#[cfg(target_os = "macos")]
const STAGE_PREFIX: &str = ".craft-manager-update-";

#[cfg(any(target_os = "macos", test))]
fn validate_macos_admin_paths(plan: &Plan, plan_path: &Path, helper: &Path) -> Result<PathBuf> {
    ensure_macos_admin_shape(plan, plan_path, helper)
}
#[cfg(any(target_os = "macos", test))]
fn ensure_macos_admin_shape(plan: &Plan, plan_path: &Path, helper: &Path) -> Result<PathBuf> {
    if plan.layout != 2 || plan.msi || plan.linux_package {
        bail!("Not a protected bundle plan");
    }
    let stage = plan_path.parent().context("Missing protected stage")?;
    if stage.parent() != Some(plan.root.join("runtime/self-update").as_path())
        || !plan.root.is_absolute()
        || plan_path != stage.join("plan.json")
        || helper != stage.join(helper_name())
        || plan.staged != stage.join("package/Craft Apps Manager.app")
    {
        bail!("Protected stage is outside its exact owned layout");
    }
    let id = stage
        .file_name()
        .and_then(|s| s.to_str())
        .context("Invalid protected stage name")?;
    if uuid::Uuid::parse_str(id)?.simple().to_string() != id {
        bail!("Invalid protected stage identifier");
    }
    Ok(stage.to_path_buf())
}

#[cfg(all(target_os = "macos", feature = "service-updater"))]
pub(crate) fn create_macos_admin_stage(root: &Path) -> Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    let parent = root.join("runtime/self-update");
    fs::create_dir_all(&parent)?;
    crate::files::canonical_directory(&parent)?;
    let stage = parent.join(uuid::Uuid::new_v4().simple().to_string());
    fs::DirBuilder::new().mode(0o700).create(&stage)?;
    Ok(stage)
}
#[cfg(target_os = "macos")]
pub(crate) fn validate_macos_admin_layout(
    plan: &Plan,
    plan_path: &Path,
    helper: &Path,
) -> Result<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let stage = validate_macos_admin_paths(plan, plan_path, helper)?;
    crate::files::canonical_directory(&stage)?;
    if plan.root.canonicalize()? != plan.root
        || plan.target.canonicalize()? != plan.target
        || plan.target.extension().is_none_or(|s| s != "app")
    {
        bail!("Noncanonical protected update paths");
    }
    let meta = fs::symlink_metadata(&stage)?;
    if meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o777 != 0o700 {
        bail!("Protected stage must be private and owned by this user");
    }
    if files::hash(helper)? != files::hash(&staged_executable(&plan.target))? {
        bail!("Protected helper differs from source manager");
    }
    files::no_links(&plan.target)?;
    files::no_links(helper)?;
    verify_bundle(&plan.target)?;
    Ok(stage)
}

#[cfg(test)]
mod macos_admin_path_tests {
    use super::*;
    #[test]
    fn protected_stage_is_private_library_child_with_exact_owned_files() {
        let root = std::env::temp_dir().join("craft-admin-path-fixture");
        let stage = root.join("runtime/self-update/0123456789abcdef0123456789abcdef");
        let mut plan = Plan {
            layout: 2,
            parent_pid: 42,
            msi: false,
            linux_package: false,
            target: root.join("Manager.app"),
            staged: stage.join("package/Craft Apps Manager.app"),
            hash: "a".repeat(64),
            root: root.clone(),
            tools: root.join("tools"),
            expected_identity: None,
        };
        let path = stage.join("plan.json");
        let helper = stage.join(helper_name());
        assert_eq!(
            validate_macos_admin_paths(&plan, &path, &helper).unwrap(),
            stage
        );
        assert!(validate_macos_admin_paths(&plan, &root.join("plan.json"), &helper).is_err());
        assert!(validate_macos_admin_paths(&plan, &path, &root.join(helper_name())).is_err());
        plan.staged = root.join("unrelated.app");
        assert!(validate_macos_admin_paths(&plan, &path, &helper).is_err());
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn create_macos_stage(home: &Path) -> Result<PathBuf> {
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
    if plan.layout != 1 || plan.msi || plan.linux_package {
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
    let mut command = Command::new("/usr/bin/open");
    command
        .arg("-n")
        .arg(&plan.target)
        .arg("--args")
        .arg("--root")
        .arg(&plan.root)
        .arg("--tools")
        .arg(&plan.tools);
    crate::manager_updates::bridge::append_restart(&mut command, plan);
    let output = command
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
fn apply_macos(plan_path: &Path, authorization: Option<&Path>) -> Result<()> {
    let destination = macos_data_home()
        .context("No macOS user data directory")?
        .join("self-update-result.json");
    apply_macos_with_authorization(
        plan_path,
        &std::env::current_exe()?.canonicalize()?,
        &destination,
        Duration::from_secs(60),
        parent_alive,
        launch_macos,
        authorization,
    )
}

#[cfg(target_os = "macos")]
fn parent_alive(pid: u32) -> Result<bool> {
    let pid = i32::try_from(pid)?;
    if pid <= 0 {
        bail!("Invalid manager process identifier");
    }
    if unsafe { libc::kill(pid, 0) } == 0 {
        return Ok(true);
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::ESRCH) => Ok(false),
        Some(libc::EPERM) => Ok(true),
        _ => Err(std::io::Error::last_os_error())
            .context("Could not check whether the manager closed"),
    }
}
#[cfg(target_os = "macos")]
fn wait_for_parent(
    pid: u32,
    timeout: Duration,
    alive: &mut impl FnMut(u32) -> Result<bool>,
) -> Result<()> {
    let deadline = Instant::now() + timeout;
    while alive(pid)? {
        if Instant::now() >= deadline {
            bail!(
                "Manager did not close; update was not applied and no recovery window was opened"
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

#[cfg(all(target_os = "macos", test))]
fn apply_macos_with(
    plan_path: &Path,
    helper: &Path,
    destination: &Path,
    timeout: Duration,
    alive: impl FnMut(u32) -> Result<bool>,
    launch: impl FnMut(&Plan) -> Result<()>,
) -> Result<()> {
    apply_macos_with_authorization(plan_path, helper, destination, timeout, alive, launch, None)
}
#[cfg(target_os = "macos")]
fn apply_macos_with_authorization(
    plan_path: &Path,
    helper: &Path,
    destination: &Path,
    timeout: Duration,
    mut alive: impl FnMut(u32) -> Result<bool>,
    mut launch: impl FnMut(&Plan) -> Result<()>,
    authorization: Option<&Path>,
) -> Result<()> {
    let trusted = (|| -> Result<_> {
        let plan: Plan = files::read_json(plan_path)?;
        #[cfg(not(feature = "service-updater"))]
        crate::manager_updates::transaction::reject(&plan, authorization)?;
        let stage = if plan.layout == 2 {
            let authorization =
                authorization.context("Protected bundle update requires signed authorization")?;
            if authorization
                != plan_path
                    .parent()
                    .context("Missing stage")?
                    .join("authorization.json")
            {
                bail!("Protected authorization is outside the stage");
            }
            crate::manager_updates::transaction::verify_macos_admin_plan(&plan, authorization)?;
            validate_macos_admin_layout(&plan, plan_path, helper)?
        } else {
            validate_macos_layout(&plan, plan_path, helper)?
        };
        let previous_hash = files::hash(&staged_executable(&plan.target))?;
        Ok((plan, stage, previous_hash))
    })();
    let (plan, stage, previous_hash) = match trusted {
        Ok(trusted) => trusted,
        Err(original) => {
            // Rejected legacy/tampered plans get a startup diagnostic without
            // writing to or launching any path supplied by the rejected plan.
            let reporting = files::write_json(
                destination,
                &UpdateResult {
                    status: UpdateStatus::Failed,
                    message: format!("Manager update failed: {original:#}"),
                },
            );
            if let Err(error) = reporting {
                bail!("{original:#}. Could not persist the update failure: {error:#}");
            }
            return Err(original);
        }
    };
    let mut parent_closed = false;
    let result = (|| -> Result<()> {
        // The GUI closes after spawning the helper. Wait before fallible
        // preparation so an early checksum/permission failure cannot skip
        // recovery merely because the GUI was still closing at that instant.
        wait_for_parent(plan.parent_pid, timeout, &mut alive)?;
        parent_closed = true;
        let _execution = plan
            .expected_identity
            .as_ref()
            .map(|_| crate::manager_updates::bridge::execution_gate())
            .transpose()?;
        validate_macos_replacement(&plan)?;
        if plan.layout == 1 {
            check_update_directory(plan.target.parent().context("Missing application folder")?)?;
        }
        files::no_links(&plan.target)?;
        files::no_links(&plan.staged)?;
        verify_bundle(&plan.target)?;
        if files::hash(&staged_executable(&plan.target))? != previous_hash {
            bail!("Previous manager changed while waiting; update was not applied");
        }
        let _authorization = authorization
            .map(|path| crate::manager_updates::transaction::authorize_plan(path, &plan))
            .transpose()?;
        if plan.layout == 2 {
            let authorization = authorization.context("Missing protected authorization")?;
            let verification = (|| -> Result<()> {
                crate::manager_updates::transaction::install_macos_batch(&plan, authorization)?;
                verify_bundle(&plan.target)?;
                if files::hash(&staged_executable(&plan.target))? != plan.hash {
                    bail!("Published bundle checksum mismatch");
                }
                crate::manager_updates::transaction::verify_executable(
                    &staged_executable(&plan.target),
                    plan.expected_identity
                        .as_ref()
                        .context("Missing bundle identity")?,
                    &stage,
                )?;
                crate::manager_updates::transaction::acknowledge_macos_batch(&plan, authorization)
            })();
            if let Err(error) = verification {
                crate::manager_updates::native_admin::revoke(&plan.root);
                crate::manager_updates::bridge::revoke(&plan.root)?;
                return Err(error);
            }
            // Acknowledgement is the commit boundary. No shared-parent portable metadata writes.
            report_macos_result(
                &stage,
                destination,
                &UpdateResult {
                    status: UpdateStatus::LaunchPending,
                    message: "Protected manager replacement verified; launch request pending"
                        .into(),
                },
            )?;
            return launch(&plan);
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
                crate::portable_mode::adopt_updated_portable(
                    plan.target
                        .parent()
                        .context("Missing portable manager folder")?,
                    &plan.root,
                    Some(&plan.tools),
                )?;
                report_macos_result(&stage, destination, &UpdateResult {
                    status: UpdateStatus::LaunchPending,
                    message: format!("Replacement verified; manager launch request pending. Previous manager: {}", stage.join("previous.app").display()),
                })?;
                launch(&plan)
            },
        )
    })();
    let message = match &result {
        Ok(()) if plan.layout == 2 => {
            "Protected manager replaced; macOS accepted the launch request".into()
        }
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
        destination,
        &UpdateResult {
            status: if result.is_ok() {
                UpdateStatus::LaunchRequested
            } else {
                UpdateStatus::Failed
            },
            message: message.clone(),
        },
    );
    if result.is_err() {
        // Report first, then reopen only the validated original after confirmed
        // parent shutdown. A wait timeout must not create a second window.
        let recovery = if parent_closed
            && files::no_links(&plan.target).is_ok()
            && files::hash(&staged_executable(&plan.target)).is_ok_and(|hash| hash == previous_hash)
            && verify_bundle(&plan.target).is_ok()
        {
            launch(&plan)
        } else {
            Ok(())
        };
        let mut detailed = message;
        let mut additional_error = false;
        if let Err(error) = reporting {
            additional_error = true;
            detailed.push_str(&format!(". Result reporting failed: {error:#}"));
        }
        if let Err(error) = recovery {
            additional_error = true;
            detailed.push_str(&format!(
                ". Previous manager launch request failed: {error:#}"
            ));
        }
        if additional_error {
            // Keep the complete trusted diagnostic, including its recovery
            // path, even if reporting or the recovery launch also failed.
            let _ = report_macos_result(
                &stage,
                destination,
                &UpdateResult {
                    status: UpdateStatus::Failed,
                    message: detailed.clone(),
                },
            );
        }
        bail!("{detailed}");
    }
    reporting.context(
        "Manager was replaced and its launch request accepted, but result reporting failed",
    )
}

/// The file whose checksum the update plan records.
pub(crate) fn staged_executable(staged: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        let name = crate::platform::bundle_value(staged, "CFBundleExecutable")
            .unwrap_or_else(|| "craft-apps-manager".into());
        if files::safe_relative(&name).is_err() || name.contains('/') {
            return staged.join("invalid-bundle-entrypoint");
        }
        staged.join("Contents/MacOS").join(name)
    }
    #[cfg(not(target_os = "macos"))]
    staged.to_path_buf()
}
/// The `.app` bundle that contains the running executable.
#[cfg(target_os = "macos")]
pub(crate) fn app_bundle(executable: &Path) -> Result<PathBuf> {
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
pub(crate) fn clear_quarantine(path: &Path) -> Result<()> {
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
pub(crate) fn verify_bundle(bundle: &Path) -> Result<()> {
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
pub(crate) fn helper_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "update-helper.exe"
    } else {
        "update-helper"
    }
}
#[cfg(target_os = "windows")]
pub(crate) fn installer_path(path: &Path) -> PathBuf {
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
    fn unwritable_portable_directory_reports_manual_update_before_download() {
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
            let error = result.unwrap_err();
            assert!(error.to_string().contains("manager folder is not writable"));
            assert!(error.chain().any(|cause| cause
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::PermissionDenied)));
        }
    }
}
#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;
    #[test]
    fn failed_update_records_error_before_reopening() {
        let root =
            std::env::temp_dir().join(format!("craft-update-recovery-{}", uuid::Uuid::new_v4()));
        let path = root.join("result.txt");
        let mut reopened = false;
        let result = recover_failed_update(anyhow::anyhow!("installer cancelled"), &path, || {
            assert!(fs::read_to_string(&path)?.contains("installer cancelled"));
            reopened = true;
            Ok(())
        });
        assert!(result.is_err());
        assert!(reopened);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_reopen_preserves_update_diagnostic() {
        let root =
            std::env::temp_dir().join(format!("craft-update-recovery-{}", uuid::Uuid::new_v4()));
        let path = root.join("result.txt");
        let error = recover_failed_update(anyhow::anyhow!("installer failed"), &path, || {
            bail!("executable missing")
        })
        .unwrap_err();
        assert!(error.to_string().contains("executable missing"));
        assert!(fs::read_to_string(&path)
            .unwrap()
            .contains("installer failed"));
        fs::remove_dir_all(root).unwrap();
    }

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
            name: None,
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
        assert!(select_package(release("zip"), "0.4.0", true)
            .unwrap()
            .is_none());
        assert!(select_package(release("msi"), "0.4.0", false)
            .unwrap()
            .is_none());
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
            name: None,
            draft: false,
            prerelease: false,
            assets: vec![asset(other)],
        };
        assert!(select(release.clone(), "0.3.0").unwrap().is_none());
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
            name: None,
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
#[path = "macos_tests.rs"]
mod macos_tests;

#[cfg(test)]
mod linux_package_tests {
    use super::linux_package_owner;
    #[test]
    fn exact_package_ownership_is_required() {
        assert!(linux_package_owner(
            "dpkg-query",
            "craft-apps-manager:amd64: /usr/bin/craft-apps-manager\n"
        ));
        assert!(linux_package_owner(
            "dpkg-query",
            "craft-apps-manager: /usr/bin/craft-apps-manager\n"
        ));
        assert!(linux_package_owner("rpm", "craft-apps-manager\n"));
        assert!(linux_package_owner("pacman", "craft-apps-manager\n"));
        assert!(!linux_package_owner("pacman", "other-craft-apps-manager\n"));
        assert!(!linux_package_owner("rpm", "other-craft-apps-manager"));
        assert!(!linux_package_owner(
            "dpkg-query",
            "other: /home/user/craft-apps-manager\n"
        ));
        assert!(!linux_package_owner(
            "dpkg-query",
            "dpkg-query: no path found matching pattern"
        ));
    }
}

#[cfg(test)]
mod startup_result_tests {
    use super::*;
    #[test]
    fn macos_failure_stays_pending_when_diagnostic_cannot_be_saved() {
        let root =
            std::env::temp_dir().join(format!("craft-startup-result-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("self-update-result.json");
        fs::create_dir(path.with_extension("acknowledged.json")).unwrap();
        let bytes = br#"{"status":"failed","message":"original failure"}"#;
        fs::write(&path, bytes).unwrap();
        let message = macos_startup_message_at(&path).unwrap();
        assert!(message.contains("original failure"));
        assert!(message.contains("Could not acknowledge"));
        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::remove_dir(path.with_extension("acknowledged.json")).unwrap();
        assert_eq!(
            macos_startup_message_at(&path).as_deref(),
            Some("original failure")
        );
        assert!(macos_startup_message_at(&path).is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn macos_malformed_result_is_reported_once_and_preserved() {
        let root =
            std::env::temp_dir().join(format!("craft-startup-result-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("self-update-result.json");
        fs::write(&path, b"interrupted JSON write").unwrap();
        assert!(macos_startup_message_at(&path)
            .unwrap()
            .contains("Could not read"));
        assert!(macos_startup_message_at(&path).is_none());
        assert_eq!(
            fs::read(path.with_extension("acknowledged.json")).unwrap(),
            b"interrupted JSON write"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn macos_success_and_missing_result_do_not_report_failure() {
        let root =
            std::env::temp_dir().join(format!("craft-startup-result-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("self-update-result.json");
        assert!(macos_startup_message_at(&path).is_none());
        for status in ["launch_pending", "launch_requested"] {
            let bytes =
                serde_json::to_vec(&serde_json::json!({"status":status,"message":"not a failure"}))
                    .unwrap();
            fs::write(&path, &bytes).unwrap();
            assert!(macos_startup_message_at(&path).is_none());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn macos_failure_is_reported_once_and_diagnostic_is_preserved() {
        let root =
            std::env::temp_dir().join(format!("craft-startup-result-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("self-update-result.json");
        let bytes = br#"{"status":"failed","message":"previous manager was restored"}"#;
        fs::write(&path, bytes).unwrap();
        assert_eq!(
            macos_startup_message_at(&path).as_deref(),
            Some("previous manager was restored")
        );
        assert!(
            macos_startup_message_at(&path).is_none(),
            "relaunch must not repeat an already delivered failure"
        );
        assert_eq!(
            fs::read(path.with_extension("acknowledged.json")).unwrap(),
            bytes
        );
        // A later failed attempt must still produce a new notification.
        fs::write(&path, br#"{"status":"failed","message":"new failure"}"#).unwrap();
        assert_eq!(
            macos_startup_message_at(&path).as_deref(),
            Some("new failure")
        );
        assert!(macos_startup_message_at(&path).is_none());
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(not(feature = "service-updater"))]
pub fn prepare_continuation(
    _: &Paths,
    _: &Available,
    journey: crate::manager_updates::bridge::Journey,
    _: &Job,
) -> Result<PathBuf> {
    match journey {}
}
