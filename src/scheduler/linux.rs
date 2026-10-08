use crate::model::Paths;
use anyhow::{bail, Context, Result};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
pub fn name(source: bool) -> &'static str {
    if source {
        "craft-apps-manager-sources"
    } else {
        "craft-apps-manager-releases"
    }
}
pub fn enabled(source: bool) -> bool {
    status(source).unwrap_or(false)
}
pub fn status(source: bool) -> Result<bool> {
    Ok(Command::new("systemctl")
        .args(["--user", "is-enabled", &format!("{}.timer", name(source))])
        .output()?
        .status
        .success())
}
fn unit_path() -> Result<PathBuf> {
    Ok(std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .context("No Linux config directory")?
        .join("systemd/user"))
}
fn systemctl(args: &[&str]) -> Result<()> {
    let out = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .output()?;
    if !out.status.success() {
        bail!(
            "systemd user service error: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
}
fn quoted(path: &Path) -> Result<String> {
    let text = path.to_str().context("Service path is not UTF-8")?;
    if text.contains(['\n', '\r']) {
        bail!("Invalid service path");
    }
    Ok(format!(
        "\"{}\"",
        text.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
            .replace('$', "$$")
    ))
}
pub fn set(paths: &Paths, source: bool, on: bool) -> Result<()> {
    let timer = format!("{}.timer", name(source));
    if !on {
        if enabled(source) {
            systemctl(&["disable", "--now", &timer])?;
        }
        return Ok(());
    }
    let base = unit_path()?;
    fs::create_dir_all(&base)?;
    let argument = if source {
        "--check-source-updates"
    } else {
        "--check-app-updates"
    };
    fs::write(base.join(format!("{}.service",name(source))),format!("[Unit]\nDescription=Craft Apps Manager update check\n[Service]\nType=oneshot\nExecStart={} {argument} --root {} --tools {} --background\n",quoted(&std::env::current_exe()?)?,quoted(&paths.root)?,quoted(&paths.tools)?))?;
    fs::write(base.join(&timer),"[Unit]\nDescription=Craft Apps Manager hourly update check\n[Timer]\nOnStartupSec=2min\nOnUnitActiveSec=1h\n[Install]\nWantedBy=timers.target\n")?;
    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", "--now", &timer])
}
