//! Ownership and destination rules for portable apps outside the library.
use crate::{files, model::Paths};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

const MARKER: &str = ".craft-manager-install.json";
#[derive(Serialize, Deserialize)]
struct Ownership {
    app: String,
    library: PathBuf,
    #[serde(default)]
    shortcut: Option<(PathBuf, String)>,
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
    if owner.app != app || owner.library != std::path::absolute(&paths.root)? {
        bail!("This app folder belongs to another library");
    }
    Ok(())
}
pub fn validate_install(paths: &Paths, app: &str, target: &Path) -> Result<()> {
    let parent = target.parent().context("Missing app parent folder")?;
    if target.starts_with(&paths.root) && !target.starts_with(paths.at("releases")) {
        bail!("Choose a folder outside the manager's sources, builds and runtime folders");
    }
    if !target.starts_with(paths.at("releases")) || !target.exists() {
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
            owner.app == app && owner.library == std::path::absolute(&paths.root)?,
            "Backup belongs to another library"
        );
        std::fs::remove_file(marker)?;
    }
    mark(paths, app, stage)
}
pub fn desktop_shortcut(paths: &Paths, app: &str, target: &Path) -> Result<()> {
    validate(paths, app, target)?;
    let marker = target.join(MARKER);
    let owner: Ownership = files::read_json(&marker)?;
    create_shortcut(&marker, owner, target, &desktop()?)
}
pub fn refresh_moved_shortcut(
    paths: &Paths,
    app: &str,
    target: &Path,
    installer: bool,
) -> Result<()> {
    let marker = if installer {
        paths.at(format!("runtime/desktop-shortcuts/{app}.json"))
    } else {
        target.join(MARKER)
    };
    if !marker.exists() {
        return Ok(());
    }
    let owner: Ownership = files::read_json(&marker)?;
    if owner.shortcut.is_some() {
        create_shortcut(&marker, owner, target, &desktop()?)?;
    }
    Ok(())
}
pub fn mark_moved(paths: &Paths, app: &str, target: &Path) -> Result<()> {
    let marker = target.join(MARKER);
    if marker.exists() {
        let owner: Ownership = files::read_json(&marker)?;
        anyhow::ensure!(
            owner.app == app && owner.library == std::path::absolute(&paths.root)?,
            "App folder belongs to another library"
        );
        return Ok(());
    }
    mark(paths, app, target)
}
pub fn installer_desktop_shortcut(paths: &Paths, app: &str, target: &Path) -> Result<()> {
    crate::model::valid_app(app)?;
    let marker = paths.at(format!("runtime/desktop-shortcuts/{app}.json"));
    let owner: Ownership = if marker.exists() {
        files::read_json(&marker)?
    } else {
        Ownership {
            app: app.into(),
            library: std::path::absolute(&paths.root)?,
            shortcut: None,
        }
    };
    anyhow::ensure!(
        owner.app == app && owner.library == std::path::absolute(&paths.root)?,
        "Shortcut record belongs to another app library"
    );
    create_shortcut(&marker, owner, target, &desktop()?)
}
#[derive(Serialize, Deserialize)]
struct BuildShortcutOwnership {
    #[serde(flatten)]
    owner: Ownership,
    #[serde(default)]
    build_folder: Option<PathBuf>,
    #[serde(default)]
    shortcut_title: Option<String>,
}
fn build_shortcut_identity(paths: &Paths, app: &str, target: &Path) -> Result<(PathBuf, String)> {
    use sha2::{Digest, Sha256};
    crate::model::valid_app(app)?;
    let target = std::path::absolute(target)?;
    let digest = format!(
        "{:x}",
        Sha256::digest(target.as_os_str().as_encoded_bytes())
    );
    let info: crate::model::BuildInfo = files::read_json(&target.join("build-info.json"))?;
    anyhow::ensure!(info.app == app, "The build belongs to another app");
    let date = chrono::DateTime::parse_from_rfc3339(&info.built_at)?
        .with_timezone(&chrono::Local)
        .format("%b %d %Y %H-%M-%S")
        .to_string();
    let commit: String = info
        .commit
        .chars()
        .take(7)
        .filter(|c| c.is_ascii_hexdigit())
        .collect();
    let title = format!(
        "{} Build ({date} · {commit} · {})",
        crate::model::title(app),
        &digest[..8]
    );
    Ok((
        paths.at(format!("runtime/build-shortcuts/{app}/{digest}.json")),
        title,
    ))
}
pub fn build_desktop_shortcut(paths: &Paths, app: &str, target: &Path) -> Result<()> {
    build_desktop_shortcut_at(paths, app, target, &desktop()?)
}
fn build_desktop_shortcut_at(
    paths: &Paths,
    app: &str,
    target: &Path,
    desktop: &Path,
) -> Result<()> {
    let (marker, title) = build_shortcut_identity(paths, app, target)?;
    let target = std::path::absolute(target)?;
    let owner = if marker.exists() {
        files::read_json(&marker)?
    } else {
        Ownership {
            app: app.into(),
            library: std::path::absolute(&paths.root)?,
            shortcut: None,
        }
    };
    anyhow::ensure!(
        owner.app == app && owner.library == std::path::absolute(&paths.root)?,
        "Build shortcut belongs to another app library"
    );
    create_named_shortcut(&marker, owner, &target, desktop, &title)?;
    let owner = files::read_json(&marker)?;
    files::write_json(
        &marker,
        &BuildShortcutOwnership {
            owner,
            build_folder: Some(target),
            shortcut_title: Some(title),
        },
    )
}
pub fn remove_build_shortcut(paths: &Paths, app: &str, folder: &Path) -> Result<()> {
    // Keep legacy ownership records supported alongside separate records for each build.
    let legacy = paths.at(format!("runtime/build-shortcuts/{app}.json"));
    let directory = paths.at(format!("runtime/build-shortcuts/{app}"));
    if !legacy.exists() && !directory.exists() {
        return Ok(());
    }
    remove_build_shortcut_at(paths, app, folder, &desktop()?)
}
fn remove_build_shortcut_at(paths: &Paths, app: &str, folder: &Path, desktop: &Path) -> Result<()> {
    use sha2::{Digest, Sha256};
    crate::model::valid_app(app)?;
    let folder = std::path::absolute(folder)?;
    let digest = format!(
        "{:x}",
        Sha256::digest(folder.as_os_str().as_encoded_bytes())
    );
    for marker in [
        paths.at(format!("runtime/build-shortcuts/{app}/{digest}.json")),
        paths.at(format!("runtime/build-shortcuts/{app}.json")),
    ] {
        if !marker.exists() {
            continue;
        }
        let record: BuildShortcutOwnership = files::read_json(&marker)?;
        anyhow::ensure!(
            record.owner.app == app && record.owner.library == std::path::absolute(&paths.root)?,
            "Build shortcut belongs to another app library"
        );
        let belongs =
            match &record.build_folder {
                Some(owned) => owned == &folder,
                None => record.owner.shortcut.as_ref().is_some_and(|(shortcut, _)| {
                    legacy_build_shortcut_matches(shortcut, &folder, app)
                }),
            };
        if belongs {
            let title = record
                .shortcut_title
                .unwrap_or_else(|| format!("{} Build", crate::model::title(app)));
            anyhow::ensure!(
                Path::new(&title).components().count() == 1 && !title.contains(['/', '\\']),
                "Invalid build shortcut name"
            );
            remove_owned_named_shortcut(record.owner, desktop, &title)?;
            std::fs::remove_file(marker)?;
        }
    }
    Ok(())
}
fn legacy_build_shortcut_matches(shortcut: &Path, folder: &Path, app: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        crate::platform::shortcut_target(shortcut).is_some_and(|target| {
            same_shortcut_target(
                &target,
                &folder.join(crate::model::build_executable_name(app)),
            )
        })
    }
    #[cfg(target_os = "macos")]
    {
        std::fs::read_link(shortcut)
            .is_ok_and(|target| target == folder.join(crate::model::build_executable_name(app)))
    }
    #[cfg(target_os = "linux")]
    {
        let _ = app;
        std::fs::read_to_string(shortcut).is_ok_and(|contents| {
            contents.lines().any(|line| {
                line.strip_prefix("Path=")
                    .is_some_and(|working| Path::new(working) == folder)
            })
        })
    }
}

fn create_shortcut(marker: &Path, owner: Ownership, target: &Path, desktop: &Path) -> Result<()> {
    let title = crate::model::title(&owner.app);
    create_named_shortcut(marker, owner, target, desktop, &title)
}
fn create_named_shortcut(
    marker: &Path,
    mut owner: Ownership,
    target: &Path,
    desktop: &Path,
    title: &str,
) -> Result<()> {
    let app = &owner.app;
    let shortcut = desktop.join(format!(
        "{}.{}",
        title,
        crate::platform::shortcut_extension()
    ));
    std::fs::create_dir_all(desktop)?;
    let executable =
        crate::model::installed_executable(target, app).context("Missing installed executable")?;
    if shortcut.exists() {
        let owned = owner
            .shortcut
            .as_ref()
            .is_some_and(|(p, hash)| p == &shortcut && shortcut_proof(p).is_ok_and(|h| &h == hash));
        if !owned && !repairable_shortcut(&shortcut, &executable) {
            anyhow::bail!("An existing desktop shortcut was left unchanged");
        }
    }
    #[cfg(not(target_os = "macos"))]
    crate::platform::shortcut(&shortcut, &executable, "", target)?;
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::symlink;
        if shortcut.exists() {
            std::fs::remove_file(&shortcut)?;
        }
        symlink(&executable, &shortcut)?;
    }
    owner.shortcut = Some((shortcut.clone(), shortcut_proof(&shortcut)?));
    files::write_json(marker, &owner)
}

fn repairable_shortcut(shortcut: &Path, executable: &Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        match crate::platform::shortcut_target(shortcut) {
            Some(existing) => {
                same_shortcut_target(&existing, executable)
                    || crate::platform::advertised_shortcut_target(shortcut)
                        .is_some_and(|target| same_shortcut_target(&target, executable))
            }
            None => false,
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (shortcut, executable);
        false
    }
}

#[cfg(target_os = "windows")]
fn same_shortcut_target(existing: &str, executable: &Path) -> bool {
    if Path::new(existing)
        .to_string_lossy()
        .eq_ignore_ascii_case(&executable.to_string_lossy())
    {
        return true;
    }
    // Shortcuts store long paths; Windows temp directories may use 8.3 names.
    match (
        std::fs::canonicalize(existing),
        std::fs::canonicalize(executable),
    ) {
        (Ok(a), Ok(b)) => a
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy()),
        _ => false,
    }
}

pub fn remove_shortcut(target: &Path) -> Result<()> {
    let marker = target.join(MARKER);
    if !marker.exists() {
        return Ok(());
    }
    let owner: Ownership = files::read_json(&marker)?;
    remove_owned_shortcut(owner, &desktop()?)
}
pub fn remove_installer_shortcut(paths: &Paths, app: &str) -> Result<()> {
    crate::model::valid_app(app)?;
    let marker = paths.at(format!("runtime/desktop-shortcuts/{app}.json"));
    if !marker.exists() {
        return Ok(());
    }
    let owner: Ownership = files::read_json(&marker)?;
    anyhow::ensure!(
        owner.app == app && owner.library == std::path::absolute(&paths.root)?,
        "Shortcut record belongs to another app library"
    );
    remove_owned_shortcut(owner, &desktop()?)?;
    std::fs::remove_file(marker)?;
    Ok(())
}
fn remove_owned_shortcut(owner: Ownership, desktop: &Path) -> Result<()> {
    let title = crate::model::title(&owner.app);
    remove_owned_named_shortcut(owner, desktop, &title)
}
fn remove_owned_named_shortcut(owner: Ownership, desktop: &Path, title: &str) -> Result<()> {
    if let Some((path, expected)) = owner.shortcut {
        let known_path = desktop.join(format!(
            "{}.{}",
            title,
            crate::platform::shortcut_extension()
        ));
        // A user-edited shortcut is theirs to keep.
        if path == known_path && shortcut_proof(&path).is_ok_and(|hash| hash == expected) {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}
fn shortcut_proof(path: &Path) -> Result<String> {
    if files::linked(path)? {
        Ok(format!("link:{}", std::fs::read_link(path)?.display()))
    } else {
        files::hash(path)
    }
}
fn desktop() -> Result<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        crate::profiles::known_folder(&windows::Win32::UI::Shell::FOLDERID_Desktop)
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(output) = std::process::Command::new("xdg-user-dir")
            .arg("DESKTOP")
            .output()
        {
            if output.status.success() {
                let path = PathBuf::from(String::from_utf8(output.stdout)?.trim());
                if path.is_absolute() {
                    return Ok(path);
                }
            }
        }
        Ok(PathBuf::from(std::env::var_os("HOME").context("Missing home folder")?).join("Desktop"))
    }
    #[cfg(target_os = "macos")]
    {
        Ok(PathBuf::from(std::env::var_os("HOME").context("Missing home folder")?).join("Desktop"))
    }
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
mod tests {
    #[cfg(target_os = "windows")]
    #[test]
    fn installer_shortcuts_are_tracked_without_writing_to_app_folder() {
        use super::*;
        let root = std::env::temp_dir().join(format!("craft-shortcut-{}", uuid::Uuid::new_v4()));
        let target = root.join("installed-app");
        let desktop = root.join("desktop");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("photocraft.exe"), b"fixture").unwrap();
        let marker = root.join("runtime/desktop-shortcuts/photocraft.json");
        let owner = Ownership {
            app: "photocraft".into(),
            library: root.clone(),
            shortcut: None,
        };
        create_shortcut(&marker, owner, &target, &desktop).unwrap();
        assert!(!target.join(MARKER).exists());
        let shortcut = desktop.join("PhotoCraft.lnk");
        assert!(shortcut.is_file());
        let owner: Ownership = files::read_json(&marker).unwrap();
        create_shortcut(&marker, owner, &target, &desktop).unwrap();
        std::fs::write(&shortcut, b"user-edited shortcut").unwrap();
        let owner: Ownership = files::read_json(&marker).unwrap();
        remove_owned_shortcut(owner, &desktop).unwrap();
        assert_eq!(std::fs::read(&shortcut).unwrap(), b"user-edited shortcut");
        let owner: Ownership = files::read_json(&marker).unwrap();
        assert!(create_shortcut(&marker, owner, &target, &desktop).is_err());
        std::fs::remove_file(&shortcut).unwrap();
        let owner: Ownership = files::read_json(&marker).unwrap();
        create_shortcut(&marker, owner, &target, &desktop).unwrap();
        let owner: Ownership = files::read_json(&marker).unwrap();
        remove_owned_shortcut(owner, &desktop).unwrap();
        assert!(!shortcut.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn shortcuts_repair_when_the_target_still_points_at_the_app() {
        use super::*;
        let root = std::env::temp_dir().join(format!("craft-shortcut-{}", uuid::Uuid::new_v4()));
        let target = root.join("installed-app");
        let desktop = root.join("desktop");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("photocraft.exe"), b"fixture").unwrap();
        let marker = root.join("runtime/desktop-shortcuts/photocraft.json");
        let owner = Ownership {
            app: "photocraft".into(),
            library: root.clone(),
            shortcut: None,
        };
        create_shortcut(&marker, owner, &target, &desktop).unwrap();
        let shortcut = desktop.join("PhotoCraft.lnk");
        // Rewriting the shortcut with different metadata keeps the same target
        // but breaks the recorded proof hash, as an update or repair would.
        crate::platform::shortcut(
            &shortcut,
            &target.join("photocraft.exe"),
            "",
            &target.join("elsewhere"),
        )
        .unwrap();
        let owner: Ownership = files::read_json(&marker).unwrap();
        create_shortcut(&marker, owner, &target, &desktop).unwrap();
        let owner: Ownership = files::read_json(&marker).unwrap();
        remove_owned_shortcut(owner, &desktop).unwrap();
        assert!(!shortcut.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn installer_cache_path_alone_does_not_prove_shortcut_ownership() {
        use super::*;
        let root = std::env::temp_dir().join(format!("craft-shortcut-{}", uuid::Uuid::new_v4()));
        let target = root.join("installed-app");
        let desktop = root.join("desktop");
        std::fs::create_dir_all(&target).unwrap();
        let exe = target.join("photocraft.exe");
        std::fs::write(&exe, b"fixture").unwrap();
        // Installer updates leave advertised shortcuts whose target lives in
        // the Windows Installer cache and breaks on the next major upgrade.
        let advertised = PathBuf::from(
            r"C:\Windows\Installer\{01234567-89AB-CDEF-0123-456789ABCDEF}\PhotocraftIcon.ico",
        );
        std::fs::create_dir_all(&desktop).unwrap();
        let shortcut = desktop.join("PhotoCraft.lnk");
        crate::platform::shortcut(&shortcut, &advertised, "", &target).unwrap();
        let marker = root.join("runtime/desktop-shortcuts/photocraft.json");
        let owner = Ownership {
            app: "photocraft".into(),
            library: root.clone(),
            shortcut: None,
        };
        let before = std::fs::read(&shortcut).unwrap();
        assert!(create_shortcut(&marker, owner, &target, &desktop).is_err());
        assert_eq!(std::fs::read(&shortcut).unwrap(), before);
        assert!(crate::platform::advertised_shortcut_target(&shortcut).is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn shortcuts_pointing_elsewhere_are_left_unchanged() {
        use super::*;
        let root = std::env::temp_dir().join(format!("craft-shortcut-{}", uuid::Uuid::new_v4()));
        let target = root.join("installed-app");
        let desktop = root.join("desktop");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("photocraft.exe"), b"fixture").unwrap();
        std::fs::write(target.join("other.exe"), b"other").unwrap();
        std::fs::create_dir_all(&desktop).unwrap();
        let shortcut = desktop.join("PhotoCraft.lnk");
        crate::platform::shortcut(&shortcut, &target.join("other.exe"), "", &target).unwrap();
        let before = std::fs::read(&shortcut).unwrap();
        let marker = root.join("runtime/desktop-shortcuts/photocraft.json");
        let owner = Ownership {
            app: "photocraft".into(),
            library: root.clone(),
            shortcut: None,
        };
        assert!(create_shortcut(&marker, owner, &target, &desktop).is_err());
        assert_eq!(std::fs::read(&shortcut).unwrap(), before);
        crate::platform::shortcut(
            &shortcut,
            &target.join("photocraft.exe"),
            "--user-option",
            &target,
        )
        .unwrap();
        let before = std::fs::read(&shortcut).unwrap();
        let owner = Ownership {
            app: "photocraft".into(),
            library: root.clone(),
            shortcut: None,
        };
        assert!(create_shortcut(&marker, owner, &target, &desktop).is_err());
        assert_eq!(std::fs::read(&shortcut).unwrap(), before);
        std::fs::remove_dir_all(root).unwrap();
    }

    use super::*;
    use crate::model::Preferences;
    #[cfg(unix)]
    #[test]
    fn destination_rejects_custom_links_even_below_system_temp_directory() {
        let root = std::env::temp_dir().join(format!("craft-linked-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("real")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("linked")).unwrap();
        assert!(destination(&root.join("real"), "photocraft").is_ok());
        assert!(destination(&root.join("linked"), "photocraft").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    fn fixture() -> (PathBuf, Paths) {
        let root = std::env::temp_dir().join(format!("craft-custom-{}", uuid::Uuid::new_v4()));
        let paths = Paths::new(root.join("library"), None);
        files::write_json(
            &paths.at("manager-settings.json"),
            &Preferences {
                release_format: "portable".into(),
                ..Default::default()
            },
        )
        .unwrap();
        (root, paths)
    }
    #[test]
    fn destinations_reject_relative_traversal_internal_and_existing_unmanaged_folders() {
        let (root, paths) = fixture();
        assert!(destination(Path::new("relative"), "photocraft").is_err());
        assert!(destination(&root.join(".."), "photocraft").is_err());
        assert!(validate_install(&paths, "photocraft", &paths.at("sources/photocraft")).is_err());
        let target = destination(&root.join("apps"), "photocraft").unwrap();
        assert!(!target.exists());
        validate_install(&paths, "photocraft", &target).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("work.png"), b"personal").unwrap();
        assert!(validate_install(&paths, "photocraft", &target).is_err());
        assert_eq!(std::fs::read(target.join("work.png")).unwrap(), b"personal");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn custom_folder_ownership_is_bound_to_app_and_library() {
        let (root, paths) = fixture();
        let target = destination(&root.join("apps"), "photocraft").unwrap();
        std::fs::create_dir_all(&target).unwrap();
        mark(&paths, "photocraft", &target).unwrap();
        validate(&paths, "photocraft", &target).unwrap();
        assert!(validate(&paths, "filmcraft", &target).is_err());
        let other = Paths::new(root.join("other-library"), None);
        assert!(validate(&other, "photocraft", &target).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn custom_portable_uninstall_keeps_projects_and_retains_photo_data() {
        let (root, paths) = fixture();
        let parent = root.join("apps");
        let target = destination(&parent, "photocraft").unwrap();
        std::fs::create_dir_all(target.join("PhotoCraftData")).unwrap();
        let exe = target.join(crate::model::executable_name("photocraft"));
        std::fs::write(&exe, b"fixture").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::write(target.join("PhotoCraftData/recovery.pcraft"), b"recovery").unwrap();
        std::fs::write(parent.join("my-image.png"), b"image").unwrap();
        mark(&paths, "photocraft", &target).unwrap();
        let mut config = paths.config().unwrap();
        let app = config
            .apps
            .iter_mut()
            .find(|a| a.name == "photocraft")
            .unwrap();
        app.path = target.display().to_string();
        app.version = "0.3.0".into();
        app.install_kind = "portable".into();
        paths.save_config(&config).unwrap();
        assert_eq!(
            crate::apps::executables(&paths, "photocraft")
                .unwrap()
                .len(),
            1
        );
        validate_install(&paths, "photocraft", &target).unwrap();
        crate::apps::uninstall_with_profile(&paths, "photocraft", false).unwrap();
        assert!(!target.exists());
        assert_eq!(
            std::fs::read(parent.join("my-image.png")).unwrap(),
            b"image"
        );
        assert_eq!(
            std::fs::read(paths.at("runtime/app-profiles/photocraft/recovery.pcraft")).unwrap(),
            b"recovery"
        );
        assert!(!crate::apps::installed(&paths, "photocraft").is_ok());
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(all(test, target_os = "windows"))]
mod build_shortcut_tests {
    use super::*;
    #[test]
    fn multiple_build_shortcuts_coexist_and_remove_independently() {
        let root = std::env::temp_dir().join(format!(
            "craft-build-shortcuts-many-{}",
            uuid::Uuid::new_v4()
        ));
        let paths = Paths::new(root.join("library"), None);
        let desktop = root.join("desktop");
        let make = |name: &str| {
            let folder = paths.at(format!("builds/filmcraft/{name}"));
            files::write_json(
                &folder.join("build-info.json"),
                &crate::model::BuildInfo {
                    app: "filmcraft".into(),
                    commit: "a".repeat(40),
                    source_branch: "main".into(),
                    built_at: "2026-10-12T12:00:00Z".into(),
                    profile: "release".into(),
                    log: String::new(),
                },
            )
            .unwrap();
            std::fs::write(folder.join("filmcraft.exe"), b"test build").unwrap();
            build_desktop_shortcut_at(&paths, "filmcraft", &folder, &desktop).unwrap();
            let (marker, title) = build_shortcut_identity(&paths, "filmcraft", &folder).unwrap();
            (folder, marker, desktop.join(format!("{title}.lnk")))
        };
        // Even the same commit and timestamp need separate shortcuts.
        let first = make("first");
        let second = make("second");
        assert_ne!(first.2, second.2);
        assert!(first.2.is_file() && second.2.is_file());
        assert_eq!(
            PathBuf::from(crate::platform::shortcut_target(&first.2).unwrap())
                .canonicalize()
                .unwrap(),
            first.0.join("filmcraft.exe").canonicalize().unwrap()
        );
        assert_eq!(
            PathBuf::from(crate::platform::shortcut_target(&second.2).unwrap())
                .canonicalize()
                .unwrap(),
            second.0.join("filmcraft.exe").canonicalize().unwrap()
        );
        build_desktop_shortcut_at(&paths, "filmcraft", &first.0, &desktop).unwrap();
        assert_eq!(std::fs::read_dir(&desktop).unwrap().count(), 2);
        remove_build_shortcut_at(&paths, "filmcraft", &first.0, &desktop).unwrap();
        assert!(!first.2.exists() && !first.1.exists());
        assert!(second.2.exists() && second.1.exists());
        std::fs::write(&second.2, b"User edited shortcut").unwrap();
        remove_build_shortcut_at(&paths, "filmcraft", &second.0, &desktop).unwrap();
        assert_eq!(std::fs::read(&second.2).unwrap(), b"User edited shortcut");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn removing_build_shortcuts_preserves_other_builds_and_user_edits() {
        for (legacy, other_build, edited) in [
            (false, false, false),
            (true, false, false),
            (false, true, false),
            (true, true, false),
            (false, false, true),
        ] {
            let root =
                std::env::temp_dir().join(format!("craft-build-shortcut-{}", uuid::Uuid::new_v4()));
            let paths = Paths::new(root.join("library"), None);
            let desktop = root.join("desktop");
            let selected = root.join("selected");
            let other = root.join("other");
            std::fs::create_dir_all(&selected).unwrap();
            std::fs::create_dir_all(&other).unwrap();
            for folder in [&selected, &other] {
                std::fs::write(folder.join("filmcraft.exe"), b"test build").unwrap();
            }
            let marker = paths.at("runtime/build-shortcuts/filmcraft.json");
            let target = if other_build { &other } else { &selected };
            create_named_shortcut(
                &marker,
                Ownership {
                    app: "filmcraft".into(),
                    library: std::path::absolute(&paths.root).unwrap(),
                    shortcut: None,
                },
                target,
                &desktop,
                "FilmCraft Build",
            )
            .unwrap();
            if !legacy {
                let owner = files::read_json(&marker).unwrap();
                files::write_json(
                    &marker,
                    &BuildShortcutOwnership {
                        owner,
                        build_folder: Some(target.clone()),
                        shortcut_title: None,
                    },
                )
                .unwrap();
            }
            let shortcut = desktop.join("FilmCraft Build.lnk");
            if edited {
                std::fs::write(&shortcut, b"user changed shortcut").unwrap();
            }
            let original = std::fs::read(&shortcut).unwrap();
            remove_build_shortcut_at(&paths, "filmcraft", &selected, &desktop).unwrap();
            if other_build || edited {
                assert_eq!(std::fs::read(&shortcut).unwrap(), original);
            } else {
                assert!(!shortcut.exists());
            }
            if other_build {
                assert!(marker.exists());
            }
            assert!(other.join("filmcraft.exe").exists());
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}
