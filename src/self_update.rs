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
#[derive(Clone)]
pub struct Available {
    pub version: String,
    pub asset: Asset,
}
pub fn select(release: Release, current: &str) -> Result<Option<Available>> {
    if release.draft
        || release.prerelease
        || updates::version(&release.tag_name)? <= updates::version(current)?
    {
        return Ok(None);
    }
    let version = release.tag_name.trim_start_matches('v').to_owned();
    let arch = crate::model::MANAGER_ARCH;
    let names = [
        format!("Craft-Apps-Manager-{version}-windows-{arch}.zip"),
        format!("Craft-Apps-Updater-{version}-windows-{arch}.zip"),
    ];
    let mut assets = release.assets;
    let index = names
        .iter()
        .find_map(|name| assets.iter().position(|a| &a.name == name))
        .with_context(|| format!("This release has no supported Windows {arch} package"))?;
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
    let stage = home
        .join("runtime/self-update")
        .join(uuid::Uuid::new_v4().simple().to_string());
    fs::create_dir_all(&stage)?;
    let result = (|| {
        let zip = stage.join("release.zip");
        Network::new(&paths.root)?.asset(&available.asset, &zip, job)?;
        let extracted = stage.join("package");
        files::extract_zip(&zip, &extracted, job)?;
        let manager = extracted.join("Craft Apps Manager/CraftApps-Manager.exe");
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
                parent_pid: std::process::id(),
                target: target.clone(),
                staged,
                hash,
                root: paths.root.clone(),
                tools: paths.tools.clone(),
            },
        )?;
        fs::copy(&target, stage.join("update-helper.exe"))?;
        fs::remove_file(zip)?;
        Ok(plan)
    })();
    if result.is_err() {
        let _ = files::remove_managed(&stage, &home.join("runtime/self-update"));
    }
    result
}
pub fn launch(plan: &Path) -> Result<()> {
    Command::new(
        plan.parent()
            .context("Missing update folder")?
            .join("update-helper.exe"),
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
    files::inside(&stage, &home.join("runtime/self-update"))?;
    files::inside(&plan.staged, &stage)?;
    files::no_links(&plan.target)?;
    if std::env::current_exe()?.canonicalize()? != stage.join("update-helper.exe").canonicalize()? {
        bail!("Update helper location mismatch");
    }
    if files::hash(&plan.staged)? != plan.hash {
        bail!("Staged manager checksum mismatch");
    }
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

#[cfg(test)]
mod tests {
    use super::*;
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
