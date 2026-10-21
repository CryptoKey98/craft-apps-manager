use super::StagedPayload;
use crate::{files, jobs::Job, manager_updates::model::*};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
pub struct Mount {
    pub device: String,
    pub path: PathBuf,
}
pub trait DmgTools {
    fn attach(&mut self, image: &Path) -> Result<Mount>;
    fn detach(&mut self, mount: &Mount) -> Result<()>;
}
pub fn stage_with_tools(
    p: &PackageRecord,
    image: &Path,
    stage: &Path,
    job: &Job,
    tools: &mut dyn DmgTools,
) -> Result<StagedPayload> {
    ensure!(!stage.exists(), "Staging folder already exists");
    ensure!(p.layout.schema == 1, "Unsupported DMG layout");
    relative(&p.layout.root)?;
    relative(&p.layout.entrypoint)?;
    fs::create_dir(stage)?;
    let mount = tools.attach(image)?;
    let result = (|| {
        job.check()?;
        let source = mount.path.join(&p.layout.root);
        files::inside(&source, &mount.path)?;
        files::no_links(&source)?;
        let root = stage.join(&p.layout.root);
        let mut total = 0;
        let mut count = 0;
        for entry in walkdir::WalkDir::new(&source).follow_links(false) {
            job.check()?;
            let entry = entry?;
            count += 1;
            ensure!(
                count <= super::archive::MAX_ENTRIES,
                "Too many bundle files"
            );
            let relative = entry.path().strip_prefix(&source)?;
            let destination = root.join(relative);
            if entry.file_type().is_dir() {
                fs::create_dir_all(&destination)?;
                continue;
            }
            ensure!(entry.file_type().is_file(), "Special file in bundle");
            let size = entry.metadata()?.len();
            total += size;
            ensure!(
                size <= super::archive::MAX_FILE && total <= super::archive::MAX_TOTAL,
                "Bundle extraction limit"
            );
            fs::copy(entry.path(), &destination)?;
            ensure!(
                files::hash(entry.path())? == files::hash(&destination)?,
                "Bundle copy checksum mismatch"
            );
        }
        let entrypoint = root.join(&p.layout.entrypoint);
        files::inside(&entrypoint, stage)?;
        ensure!(entrypoint.is_file(), "DMG manager entrypoint missing");
        Ok(StagedPayload {
            hash: files::hash(&entrypoint)?,
            root,
            entrypoint,
        })
    })();
    if let Err(detach) = tools.detach(&mount) {
        files::write_json(&stage.join("dmg-cleanup.json"), &mount)?;
        anyhow::bail!(
            "DMG detach failed; cleanup record retained: {detach:#}; staging result: {}",
            result
                .as_ref()
                .err()
                .map(|e: &anyhow::Error| format!("{e:#}"))
                .unwrap_or_else(|| "payload staged".into())
        );
    }
    if result.is_err() {
        files::remove_managed(stage, stage.parent().context("Missing stage parent")?)?;
    }
    result
}
#[cfg(target_os = "macos")]
pub fn stage(p: &PackageRecord, image: &Path, stage: &Path, job: &Job) -> Result<StagedPayload> {
    let mut tools = SystemTools {
        mountpoint: stage.join(".mount"),
    };
    stage_with_tools(p, image, stage, job, &mut tools)
}
#[cfg(target_os = "macos")]
struct SystemTools {
    mountpoint: PathBuf,
}
#[cfg(target_os = "macos")]
impl DmgTools for SystemTools {
    fn attach(&mut self, image: &Path) -> Result<Mount> {
        use std::process::Command;
        fs::create_dir(&self.mountpoint)?;
        let output = Command::new("/usr/bin/hdiutil")
            .env_clear()
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("LANG", "C")
            .args([
                "attach",
                "-readonly",
                "-nobrowse",
                "-noautoopen",
                "-plist",
                "-mountpoint",
            ])
            .arg(&self.mountpoint)
            .arg(image)
            .output()?;
        let result = (|| {
            ensure!(
                output.status.success() && output.stdout.len() <= 65536,
                "Cannot mount update DMG"
            );
            let plist = plist::Value::from_reader_xml(output.stdout.as_slice())?;
            let entities = plist
                .as_dictionary()
                .and_then(|v| v.get("system-entities"))
                .and_then(|v| v.as_array())
                .context("Missing DMG mount identity")?;
            for entity in entities {
                if let Some(dict) = entity.as_dictionary() {
                    if dict
                        .get("mount-point")
                        .and_then(|v| v.as_string())
                        .map(Path::new)
                        == Some(self.mountpoint.as_path())
                    {
                        let device = dict
                            .get("dev-entry")
                            .and_then(|v| v.as_string())
                            .context("Missing DMG device")?;
                        ensure!(
                            device.starts_with("/dev/disk")
                                && device[9..].bytes().all(|c| c.is_ascii_alphanumeric()),
                            "Invalid DMG device"
                        );
                        return Ok(Mount {
                            device: device.into(),
                            path: self.mountpoint.clone(),
                        });
                    }
                }
            }
            anyhow::bail!("DMG mounted at an unexpected location")
        })();
        if result.is_err() {
            let status = Command::new("/usr/bin/hdiutil")
                .env_clear()
                .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
                .env("LANG", "C")
                .arg("detach")
                .arg(&self.mountpoint)
                .status()?;
            if !status.success() {
                files::write_json(
                    &self.mountpoint.parent().unwrap().join("dmg-cleanup.json"),
                    &Mount {
                        device: String::new(),
                        path: self.mountpoint.clone(),
                    },
                )?;
            }
        }
        result
    }
    fn detach(&mut self, mount: &Mount) -> Result<()> {
        let status = std::process::Command::new("/usr/bin/hdiutil")
            .env_clear()
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("LANG", "C")
            .arg("detach")
            .arg(&mount.device)
            .status()?;
        ensure!(status.success(), "Could not detach update DMG");
        Ok(())
    }
}
