use crate::{model::Paths, platform::escape};
use anyhow::{bail, Context, Result};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
pub fn name(source: bool) -> &'static str {
    if source {
        "io.github.craft-apps-manager.sources"
    } else {
        "io.github.craft-apps-manager.releases"
    }
}
fn domain() -> String {
    format!("gui/{}", unsafe { libc::getuid() })
}
pub fn enabled(source: bool) -> bool {
    status(source).unwrap_or(false)
}
pub fn status(source: bool) -> Result<bool> {
    Ok(agent_path(source)?.is_file()
        && Command::new("/bin/launchctl")
            .args(["print", &format!("{}/{}", domain(), name(source))])
            .output()?
            .status
            .success())
}
fn agent_path(source: bool) -> Result<PathBuf> {
    Ok(
        PathBuf::from(std::env::var_os("HOME").context("No macOS home directory")?)
            .join("Library/LaunchAgents")
            .join(format!("{}.plist", name(source))),
    )
}
fn launchctl(args: &[&str]) -> Result<()> {
    let out = Command::new("/bin/launchctl").args(args).output()?;
    if !out.status.success() {
        bail!(
            "launchd error: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}
fn string(path: &Path) -> Result<String> {
    Ok(format!(
        "<string>{}</string>",
        escape(path.to_str().context("Launch agent path is not UTF-8")?)
    ))
}
pub fn set(paths: &Paths, source: bool, on: bool) -> Result<()> {
    let plist = agent_path(source)?;
    let target = format!("{}/{}", domain(), name(source));
    if enabled(source) {
        launchctl(&["bootout", &target])?;
    }
    if !on {
        if plist.is_file() {
            fs::remove_file(&plist)?;
        }
        return Ok(());
    }
    fs::create_dir_all(plist.parent().unwrap())?;
    let argument = if source {
        "--check-source-updates"
    } else {
        "--check-app-updates"
    };
    // RunAtLoad covers sign-in; StartInterval repeats the check hourly.
    fs::write(&plist, format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n<key>Label</key><string>{}</string>\n<key>ProgramArguments</key>\n<array>{}<string>{argument}</string><string>--root</string>{}<string>--tools</string>{}<string>--background</string></array>\n<key>RunAtLoad</key><true/>\n<key>StartInterval</key><integer>3600</integer>\n<key>ProcessType</key><string>Background</string>\n</dict>\n</plist>\n", name(source), string(&std::env::current_exe()?)?, string(&paths.root)?, string(&paths.tools)?))?;
    launchctl(&[
        "bootstrap",
        &domain(),
        plist.to_str().context("Launch agent path is not UTF-8")?,
    ])
}
