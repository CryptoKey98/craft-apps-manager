use anyhow::{bail, Context, Result};
use std::{
    fs::{self, File, OpenOptions},
    os::unix::{fs::OpenOptionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Output},
};

pub fn hidden(command: &mut Command) -> &mut Command {
    command.process_group(0)
}
pub fn output(command: &mut Command) -> Result<Output> {
    Ok(hidden(command).output()?)
}
pub fn atomic_replace(from: &Path, to: &Path) -> Result<()> {
    fs::rename(from, to)?;
    File::open(to.parent().context("Missing parent directory")?)?.sync_all()?;
    Ok(())
}
pub struct Lock(File);
impl Lock {
    pub fn take(name: &str) -> Result<Self> {
        // TMPDIR is a private per-user folder on macOS.
        let base = std::env::var_os("TMPDIR")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Caches"))
            })
            .context("No per-user lock directory available")?
            .join("craft-apps-manager");
        fs::create_dir_all(&base)?;
        let filename: String = name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(base.join(format!("{filename}.lock")))?;
        use std::os::fd::AsRawFd;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            bail!(
                "Another operation is already running: {}",
                std::io::Error::last_os_error()
            );
        }
        Ok(Self(file))
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
pub struct ProcessGroup(i32);
impl ProcessGroup {
    pub fn attach(child: &std::process::Child) -> Result<Self> {
        let pid = i32::try_from(child.id())?;
        if unsafe { libc::getpgid(pid) } != pid {
            bail!("Child process was not started in its own process group");
        }
        Ok(Self(pid))
    }
}
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        unsafe {
            libc::kill(-self.0, libc::SIGKILL);
        }
    }
}
pub fn open(path: &Path) -> Result<()> {
    let status = Command::new("/usr/bin/open").arg(path).status()?;
    if !status.success() {
        bail!("Could not open {}", path.display());
    }
    Ok(())
}
/// Reads a string value from an app bundle's Info.plist.
pub fn bundle_value(bundle: &Path, key: &str) -> Option<String> {
    let out = Command::new("/usr/bin/plutil")
        .args(["-extract", key, "raw", "-o", "-"])
        .arg(bundle.join("Contents/Info.plist"))
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}
pub fn executable_version(bundle: &Path) -> Option<String> {
    bundle_value(bundle, "CFBundleShortVersionString")
}
pub fn running_app(name: &str) -> Result<bool> {
    let out = Command::new("/bin/ps")
        .args(["-axww", "-o", "comm="])
        .output()?;
    if !out.status.success() {
        bail!("Could not list running processes");
    }
    let processes = String::from_utf8_lossy(&out.stdout);
    let bundles: Vec<_> = crate::model::executable_names(name)
        .into_iter()
        .map(|bundle| format!("/{}/Contents/MacOS/", bundle.to_lowercase()))
        .collect();
    Ok(processes
        .lines()
        .map(str::to_lowercase)
        .any(|line| bundles.iter().any(|bundle| line.contains(bundle))))
}
pub fn notify(exe: &Path, message: &str) -> Result<()> {
    notify_with_title(exe, "Craft updates available", message)
}
pub fn notify_with_title(_: &Path, title: &str, message: &str) -> Result<()> {
    // Pass the message as an argument so it is never parsed as AppleScript.
    let status = Command::new("/usr/bin/osascript")
        .args([
            "-e",
            "on run argv",
            "-e",
            "display notification (item 1 of argv) with title \"Craft Apps Manager\" subtitle (item 2 of argv)",
            "-e",
            "end run",
            message,
            title,
        ])
        .status()?;
    if !status.success() {
        bail!("Desktop notification could not be delivered");
    }
    Ok(())
}
pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
/// App bundles open directly from Finder, so macOS needs no separate shortcut files.
pub fn shortcut(_: &Path, _: &Path, args: &str, _: &Path) -> Result<()> {
    if !["", "--builder"].contains(&args) {
        bail!("Unsupported shortcut arguments");
    }
    Ok(())
}
pub fn verify_microsoft_signature(_: &Path) -> Result<()> {
    bail!("Microsoft executable verification is only available on Windows")
}
pub fn run_elevated(_: &Path, _: &str, _: &crate::jobs::Job) -> Result<()> {
    bail!("Windows prerequisite installers cannot run on macOS")
}

pub fn shortcut_extension() -> &'static str {
    "alias"
}

pub fn relaunch_executable() -> Result<PathBuf> {
    let current = std::env::current_exe()?;
    if current.is_file() {
        return Ok(current);
    }
    bail!("The manager executable has moved or been removed. Reopen Craft Apps Manager.")
}
