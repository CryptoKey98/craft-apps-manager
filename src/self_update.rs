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

pub const REPOSITORY: &str = "https://github.com/CryptoKey98/craft-apps-manager";
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
    let names = if cfg!(target_os = "linux") {
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
    let release = Network::new(&paths.root)?
        .json("https://api.github.com/repos/CryptoKey98/craft-apps-manager/releases/latest")?;
    select(release, env!("CARGO_PKG_VERSION"))
}
#[derive(Serialize, Deserialize)]
pub struct Plan {
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
    let target = std::env::current_exe()?.canonicalize()?;
    let home = target.parent().context("No executable folder")?;
    #[cfg(target_os = "linux")]
    check_update_directory(home)?;
    let msi = installed_with_msi();
    if available.asset.name.ends_with(".msi") != msi {
        bail!("Update package does not match this installation type");
    }
    let stage_home = if msi { paths.root.as_path() } else { home };
    let stage = stage_home
        .join("runtime/self-update")
        .join(uuid::Uuid::new_v4().simple().to_string());
    fs::create_dir_all(&stage)?;
    let result = (|| {
        let zip = stage.join(if msi { "release.msi" } else { "release.zip" });
        Network::new(&paths.root)?.asset(&available.asset, &zip, job)?;
        if msi {
            let plan = stage.join("plan.json");
            files::write_json(
                &plan,
                &Plan {
                    msi: true,
                    parent_pid: std::process::id(),
                    target: target.clone(),
                    staged: zip.clone(),
                    hash: files::hash(&zip)?,
                    root: paths.root.clone(),
                    tools: paths.tools.clone(),
                },
            )?;
            fs::copy(&target, stage.join(helper_name()))?;
            return Ok(plan);
        }
        let extracted = stage.join("package");
        files::extract_zip(&zip, &extracted, job)?;
        let manager = extracted.join(if cfg!(target_os = "linux") {
            "Craft Apps Manager Linux/craft-apps-manager"
        } else {
            "Craft Apps Manager/CraftApps-Manager.exe"
        });
        let staged = if manager.is_file() {
            manager
        } else {
            extracted.join("Craft Apps Updater/CraftApps-Updater.exe")
        };
        files::no_links(&staged)?;
        if !staged.is_file() {
            bail!("Release does not contain the manager executable");
        }
        let hash = files::hash(&staged)?;
        let plan = stage.join("plan.json");
        files::write_json(
            &plan,
            &Plan {
                msi: false,
                parent_pid: std::process::id(),
                target: target.clone(),
                staged,
                hash,
                root: paths.root.clone(),
                tools: paths.tools.clone(),
            },
        )?;
        fs::copy(&target, stage.join(helper_name()))?;
        fs::remove_file(zip)?;
        Ok(plan)
    })();
    if result.is_err() {
        let _ = files::remove_managed(&stage, &stage_home.join("runtime/self-update"));
    }
    result
}
#[cfg(target_os = "linux")]
fn check_update_directory(home: &Path) -> Result<()> {
    let probe = home.join(format!(".craft-manager-update-{}", uuid::Uuid::new_v4()));
    let file = fs::OpenOptions::new().write(true).create_new(true).open(&probe)
        .context("This installation is managed by the system package manager. Install a newer DEB or RPM package to update Craft Apps Manager")?;
    drop(file);
    fs::remove_file(probe)?;
    Ok(())
}
pub fn launch(plan: &Path) -> Result<()> {
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
    if files::hash(&plan.staged)? != plan.hash {
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
    #[cfg(target_os = "linux")]
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
        Command::new(&plan.target)
            .arg("--root")
            .arg(&plan.root)
            .arg("--tools")
            .arg(&plan.tools)
            .spawn()?;
        Ok(())
    })();
    if result.is_err() {
        if plan.target.exists() {
            fs::remove_file(&plan.target)?;
        }
        fs::rename(&backup, &plan.target).context("Could not restore previous manager")?;
    }
    result
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
