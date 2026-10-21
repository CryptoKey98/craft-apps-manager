pub use super::shortcuts::*;
use crate::{files, model::Paths};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

pub(super) const MARKER: &str = ".craft-manager-install.json";

/// Remove empty legacy UUID containers, never a user-selected parent.
pub fn cleanup_empty_installation_parent(paths: &Paths, app: &str, target: &Path) -> Result<()> {
    crate::model::valid_app(app)?;
    let Some(parent) = target.parent() else {
        return Ok(());
    };
    let library = paths.at("releases");
    if target.file_name().is_none_or(|name| name != app)
        || parent.parent() != Some(library.as_path())
        || !parent.exists()
    {
        return Ok(());
    }
    let Some(suffix) = parent
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix(&format!("{app}-")))
    else {
        return Ok(());
    };
    let id = match suffix.split_once('-') {
        Some(("x64" | "x86" | "arm64", id)) => id,
        None => suffix,
        _ => return Ok(()),
    };
    if id.len() != 32
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || files::linked(parent)?
    {
        return Ok(());
    }
    files::inside(parent, &library)?;
    match std::fs::remove_dir(parent) {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

#[derive(Serialize, Deserialize)]
pub(super) struct Ownership {
    pub(super) app: String,
    pub(super) library: PathBuf,
    #[serde(default)]
    pub(super) shortcut: Option<(PathBuf, String)>,
}

pub fn destination(parent: &Path, app: &str) -> Result<PathBuf> {
    crate::model::valid_app(app)?;
    if !parent.is_absolute()
        || parent
            .components()
            .any(|c| matches!(c, Component::ParentDir))
    {
        bail!("Choose an absolute folder path without .. components");
    }
    let target = parent.join(app);
    validate_path(&target)?;
    Ok(target)
}

/// Library copies live in flat numbered folders. Custom destinations keep
/// their existing parent/app layout and collision checks.
pub fn additional_destination(paths: &Paths, parent: &Path, app: &str) -> Result<PathBuf> {
    let target = destination(parent, app)?;
    if parent != paths.at("releases") {
        return Ok(target);
    }
    match std::fs::symlink_metadata(&target) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(target),
        Err(error) => return Err(error.into()),
    }
    for number in 2..=u32::MAX {
        let candidate = parent.join(format!("{app}-{number}"));
        validate_path(&candidate)?;
        match std::fs::symlink_metadata(&candidate) {
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(candidate),
            Err(error) => return Err(error.into()),
        }
    }
    bail!("No unused installation folder is available")
}

fn numbered_library_copy(paths: &Paths, app: &str, target: &Path) -> bool {
    target.parent() == Some(paths.at("releases").as_path())
        && target
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix(&format!("{app}-")))
            .is_some_and(|number| {
                number
                    .parse::<u32>()
                    .is_ok_and(|value| value >= 2 && number == value.to_string())
            })
}

fn validate_path(target: &Path) -> Result<()> {
    let parent = target.parent().context("Missing app parent folder")?;
    files::inside(target, parent)?;
    let mut cursor = parent;
    loop {
        if std::fs::symlink_metadata(cursor).is_ok()
            && files::linked(cursor)?
            && !system_directory_alias(cursor)
        {
            bail!("Choose a folder without linked parent directories");
        }
        if let Some(next) = cursor.parent() {
            cursor = next;
        } else {
            break;
        }
    }
    Ok(())
}

fn system_directory_alias(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        // macOS exposes these OS-owned locations through fixed root aliases.
        // No user-controlled link below them is accepted.
        let expected = match path.to_str() {
            Some("/var") => Path::new("/private/var"),
            Some("/tmp") => Path::new("/private/tmp"),
            _ => return false,
        };
        std::fs::read_link(path).is_ok_and(|target| {
            let resolved = if target.is_absolute() {
                target
            } else {
                path.parent().unwrap_or(Path::new("/")).join(target)
            };
            resolved == expected
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        false
    }
}

pub fn validate(paths: &Paths, app: &str, target: &Path) -> Result<()> {
    crate::model::valid_app(app)?;
    if target.starts_with(paths.at("releases")) {
        return files::inside(target, &paths.at("releases"));
    }
    validate_path(target)?;
    if target.file_name() != Some(std::ffi::OsStr::new(app)) {
        bail!("Portable apps must use a separate app folder");
    }
    let marker = target.join(MARKER);
    files::inside(&marker, target)?;
    let owner: Ownership =
        files::read_json(&marker).context("This folder is not managed by this library")?;
    if owner.app != app
        || !crate::portable_mode::same_library(&owner.library, &std::path::absolute(&paths.root)?)
    {
        bail!("This app folder belongs to another library");
    }
    Ok(())
}

pub fn validate_install(paths: &Paths, app: &str, target: &Path) -> Result<()> {
    let parent = target.parent().context("Missing app parent folder")?;
    if numbered_library_copy(paths, app, target) {
        validate_path(target)?;
    }
    if target.starts_with(&paths.root) && !target.starts_with(paths.at("releases")) {
        bail!("Choose a folder outside the manager's sources, builds and runtime folders");
    }
    if (!target.starts_with(paths.at("releases")) || !target.exists())
        && !numbered_library_copy(paths, app, target)
    {
        anyhow::ensure!(destination(parent, app)? == target, "Invalid app folder");
    }
    // Never replace a pre-existing folder merely because the user chose its parent.
    if std::fs::symlink_metadata(target).is_ok() {
        let record = paths.config()?.apps.into_iter().find(|a| a.name == app);
        anyhow::ensure!(
            record.is_some_and(|a| a.install_kind != "installer" && Path::new(&a.path) == target),
            "That app folder already exists. Choose another parent folder to keep its files safe."
        );
        validate(paths, app, target)?;
    }
    Ok(())
}

pub fn mark(paths: &Paths, app: &str, stage: &Path) -> Result<()> {
    mark_copy(paths, app, stage, false)
}

pub fn mark_additional(paths: &Paths, app: &str, stage: &Path) -> Result<()> {
    mark_copy(paths, app, stage, true)
}

fn mark_copy(paths: &Paths, app: &str, stage: &Path, additional: bool) -> Result<()> {
    let marker = stage.join(MARKER);
    anyhow::ensure!(!marker.exists(), "Release includes a reserved manager file");
    let shortcut = paths
        .config()?
        .apps
        .into_iter()
        .find(|a| a.name == app)
        .and_then(|a| files::read_json::<Ownership>(&PathBuf::from(a.path).join(MARKER)).ok())
        .and_then(|owner| owner.shortcut);
    files::write_json(
        &marker,
        &Ownership {
            app: app.into(),
            library: std::path::absolute(&paths.root)?,
            shortcut: if additional { None } else { shortcut },
        },
    )
}

pub fn mark_restored(paths: &Paths, app: &str, stage: &Path) -> Result<()> {
    let marker = stage.join(MARKER);
    if marker.exists() {
        files::inside(&marker, stage)?;
        let owner: Ownership = files::read_json(&marker)?;
        anyhow::ensure!(
            owner.app == app
                && crate::portable_mode::same_library(
                    &owner.library,
                    &std::path::absolute(&paths.root)?
                ),
            "Backup belongs to another library"
        );
        std::fs::remove_file(marker)?;
    }
    mark(paths, app, stage)
}

pub fn mark_moved(paths: &Paths, app: &str, target: &Path) -> Result<()> {
    let marker = target.join(MARKER);
    if marker.exists() {
        let owner: Ownership = files::read_json(&marker)?;
        anyhow::ensure!(
            owner.app == app
                && crate::portable_mode::same_library(
                    &owner.library,
                    &std::path::absolute(&paths.root)?
                ),
            "App folder belongs to another library"
        );
        return Ok(());
    }
    mark(paths, app, target)
}

pub fn pick_folder(initial: &Path) -> Result<Option<PathBuf>> {
    #[cfg(target_os = "windows")]
    unsafe {
        use windows::Win32::{System::Com::*, UI::Shell::*};
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let result = (|| -> Result<Option<PathBuf>> {
            let dialog: IFileOpenDialog =
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
            dialog.SetOptions(dialog.GetOptions()? | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM)?;
            if initial.is_dir() {
                let text: Vec<u16> = initial
                    .as_os_str()
                    .to_string_lossy()
                    .encode_utf16()
                    .chain(Some(0))
                    .collect();
                if let Ok(folder) = SHCreateItemFromParsingName::<_, _, IShellItem>(
                    windows::core::PCWSTR(text.as_ptr()),
                    None,
                ) {
                    let _ = dialog.SetFolder(&folder);
                }
            }
            if let Err(error) = dialog.Show(None) {
                if error.code().0 as u32 == 0x800704c7 {
                    return Ok(None);
                }
                return Err(error.into());
            }
            let item = dialog.GetResult()?;
            let value = item.GetDisplayName(SIGDN_FILESYSPATH)?;
            let text = value.to_string();
            CoTaskMemFree(Some(value.0.cast()));
            Ok(Some(PathBuf::from(text?)))
        })();
        CoUninitialize();
        result
    }
    #[cfg(target_os = "linux")]
    {
        let output = std::process::Command::new("zenity")
            .args([
                "--file-selection",
                "--directory",
                "--title=Choose install location",
            ])
            .arg(format!("--filename={}/", initial.display()))
            .output()
            .or_else(|_| {
                std::process::Command::new("kdialog")
                    .arg("--getexistingdirectory")
                    .arg(initial)
                    .output()
            })
            .context("Folder picker unavailable. Enter the folder path directly.")?;
        if !output.status.success() {
            return Ok(None);
        }
        Ok(Some(PathBuf::from(
            String::from_utf8(output.stdout)?.trim(),
        )))
    }
    #[cfg(target_os = "macos")]
    {
        let _ = initial;
        let output = std::process::Command::new("/usr/bin/osascript")
            .args([
                "-e",
                "POSIX path of (choose folder with prompt \"Choose install location\")",
            ])
            .output()?;
        if !output.status.success() {
            return Ok(None);
        }
        Ok(Some(PathBuf::from(
            String::from_utf8(output.stdout)?.trim(),
        )))
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
