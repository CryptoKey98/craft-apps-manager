use anyhow::{bail, Context, Result};
use std::{
    fs::{self, File, OpenOptions},
    os::unix::{fs::OpenOptionsExt, process::CommandExt},
    path::Path,
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
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".cache"))
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
    let status = Command::new("xdg-open").arg(path).status()?;
    if !status.success() {
        bail!("Could not open {}", path.display());
    }
    Ok(())
}
pub fn executable_version(_: &Path) -> Option<String> {
    None
}
pub fn running_app(name: &str) -> Result<bool> {
    for entry in fs::read_dir("/proc")?.flatten() {
        if entry.file_name().to_string_lossy().parse::<u32>().is_err() {
            continue;
        }
        if let Ok(exe) = fs::read_link(entry.path().join("exe")) {
            if exe.file_name().is_some_and(|s| {
                crate::model::executable_names(name)
                    .iter()
                    .any(|name| s == name.as_str())
            }) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
pub fn notify(_: &Path, message: &str) -> Result<()> {
    let status = Command::new("notify-send")
        .args([
            "--app-name=Craft Apps Manager",
            "Craft updates available",
            message,
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
pub fn shortcut(path: &Path, target: &Path, args: &str, working: &Path) -> Result<()> {
    fn quoted(path: &Path) -> Result<String> {
        let text = path
            .to_str()
            .context("Desktop shortcut path is not UTF-8")?;
        if text.contains(['\n', '\r']) {
            bail!("Invalid desktop shortcut path");
        }
        Ok(format!(
            "\"{}\"",
            text.replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('`', "\\`")
                .replace('$', "\\$")
                .replace('%', "%%")
        ))
    }
    if !["", "--builder"].contains(&args) {
        bail!("Unsupported shortcut arguments");
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let app = target
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| crate::model::apps().iter().any(|a| a == s));
    let title = if let Some(app) = app {
        crate::model::title(app)
    } else if args.is_empty() {
        "Craft Apps Manager".into()
    } else {
        "Craft Apps Builder".into()
    };
    let command = if app.is_some() {
        format!(
            "/usr/bin/env APPIMAGE_EXTRACT_AND_RUN=1 {}",
            quoted(target)?
        )
    } else {
        quoted(target)?
    };
    fs::write(path,format!("[Desktop Entry]\nType=Application\nName={title}\nExec={} {args}\nPath={}\nTerminal=false\nCategories=Utility;\n",command,working.display()))?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}
pub fn verify_microsoft_signature(_: &Path) -> Result<()> {
    bail!("Microsoft executable verification is only available on Windows")
}
pub fn run_elevated(_: &Path, _: &str, _: &crate::jobs::Job) -> Result<()> {
    bail!("Windows prerequisite installers cannot run on Linux")
}

pub fn shortcut_extension() -> &'static str {
    "desktop"
}

pub fn relaunch_executable() -> Result<std::path::PathBuf> {
    resolve_relaunch_executable(&std::env::current_exe()?)
}
fn resolve_relaunch_executable(current: &Path) -> Result<std::path::PathBuf> {
    if current.is_file() {
        return Ok(current.to_path_buf());
    }
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    if let Some(original) = current.as_os_str().as_bytes().strip_suffix(b" (deleted)") {
        let replacement = std::path::PathBuf::from(std::ffi::OsString::from_vec(original.to_vec()));
        if replacement.is_file() {
            return Ok(replacement);
        }
    }
    bail!("The manager executable has moved or been removed. Reopen Craft Apps Manager from its launcher.")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn launching_after_executable_replacement_uses_existing_binary() {
        let root = std::env::temp_dir().join(format!("craft-relaunch-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let executable = root.join("craft-apps-manager");
        fs::copy("/usr/bin/true", &executable).unwrap();
        let deleted = root.join("craft-apps-manager (deleted)");
        let resolved = resolve_relaunch_executable(&deleted).unwrap();
        assert!(Command::new(&resolved)
            .arg("--builder")
            .status()
            .unwrap()
            .success());
        assert_eq!(resolved, executable);
        fs::remove_file(executable).unwrap();
        assert!(resolve_relaunch_executable(&deleted).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
