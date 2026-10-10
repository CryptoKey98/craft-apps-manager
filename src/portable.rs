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
        if std::fs::symlink_metadata(cursor).is_ok() && files::linked(cursor)? {
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
            shortcut,
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
pub fn build_desktop_shortcut(paths: &Paths, app: &str, target: &Path) -> Result<()> {
    crate::model::valid_app(app)?;
    let marker = paths.at(format!("runtime/build-shortcuts/{app}.json"));
    let owner = if marker.exists() {
        files::read_json(&marker)?
    } else {
        Ownership {
            app: app.into(),
            library: std::path::absolute(&paths.root)?,
            shortcut: None,
        }
    };
    create_named_shortcut(
        &marker,
        owner,
        target,
        &desktop()?,
        &format!("{} Build", crate::model::title(app)),
    )
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
    if shortcut.exists() {
        anyhow::ensure!(
            owner.shortcut.as_ref().is_some_and(
                |(p, hash)| p == &shortcut && shortcut_proof(p).is_ok_and(|h| &h == hash)
            ),
            "An existing desktop shortcut was left unchanged"
        );
    }
    std::fs::create_dir_all(desktop)?;
    let executable =
        crate::model::installed_executable(target, app).context("Missing installed executable")?;
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
    if let Some((path, expected)) = owner.shortcut {
        let known_path = desktop.join(format!(
            "{}.{}",
            crate::model::title(&owner.app),
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

    use super::*;
    use crate::model::Preferences;
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
