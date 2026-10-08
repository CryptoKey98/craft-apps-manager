use crate::{apps, files, model::Paths};
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Target {
    pub path: PathBuf,
    pub root: PathBuf,
}
#[cfg(target_os = "windows")]
fn known_folder(id: &windows::core::GUID) -> Result<PathBuf> {
    use windows::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{SHGetKnownFolderPath, KF_FLAG_DEFAULT},
    };
    unsafe {
        let value = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None)?;
        let text = value.to_string();
        CoTaskMemFree(Some(value.0.cast()));
        Ok(PathBuf::from(text?))
    }
}
#[cfg(any(target_os = "windows", test))]
fn mapped(app: &str, roaming: &Path, local: &Path) -> Result<Vec<Target>> {
    // Verified against each upstream app's settings, cache and recovery directory code.
    let (roaming_names, local_names): (&[&str], &[&str]) = match app {
        "designcraft" => (&["DesignCraft"], &[]),
        "effectcraft" => (&["EffectCraft"], &["EffectCraft"]),
        "filmcraft" => (&["FilmCraft"], &[]),
        "lightcraft" => (&["LightCraft"], &[]),
        "photocraft" => (&["Photocraft"], &[]),
        "printcraft" => (&["PdfCraft", "PrintCraft"], &["PdfCraft", "PrintCraft"]),
        "vectorcraft" => (&["VectorCraft", "DrawCraft"], &[]),
        "wordcraft" => (&["WordCraft"], &[]),
        "gridcraft" => (&["GridCraft"], &[]),
        "deckcraft" => (&["DeckCraft"], &[]),
        "soundcraft" => (&["SoundCraft"], &[]),
        _ => bail!("This app has no verified profile locations"),
    };
    Ok(roaming_names
        .iter()
        .map(|name| Target {
            path: roaming.join(name),
            root: roaming.into(),
        })
        .chain(local_names.iter().map(|name| Target {
            path: local.join(name),
            root: local.into(),
        }))
        .collect())
}
#[cfg(target_os = "windows")]
pub fn targets(paths: &Paths, app: &str) -> Result<Vec<Target>> {
    use windows::Win32::UI::Shell::{FOLDERID_LocalAppData, FOLDERID_RoamingAppData};
    let mut targets = mapped(
        app,
        &known_folder(&FOLDERID_RoamingAppData)?,
        &known_folder(&FOLDERID_LocalAppData)?,
    )?;
    if app == "photocraft" {
        if let Ok(installed) = apps::installed(paths, app) {
            if installed.install_kind != "installer" {
                let root = PathBuf::from(installed.path);
                files::inside(&root, &paths.at("releases"))?;
                targets.push(Target {
                    path: root.join("PhotoCraftData"),
                    root,
                });
            }
        }
        let root = paths.at("runtime/app-profiles");
        targets.push(Target {
            path: root.join("photocraft"),
            root,
        });
    }
    Ok(targets)
}
pub fn validate(targets: &[Target]) -> Result<()> {
    for target in targets {
        files::inside(&target.path, &target.root)?;
        if target.path.parent() != Some(target.root.as_path()) {
            bail!("Profile is not an app-specific folder");
        }
    }
    Ok(())
}
pub fn remove(targets: &[Target]) -> Result<()> {
    validate(targets)?;
    for target in targets {
        files::remove_managed(&target.path, &target.root)?;
    }
    Ok(())
}
pub fn preserve_portable(paths: &Paths, app: &str) -> Result<Option<(PathBuf, PathBuf)>> {
    if app != "photocraft" {
        return Ok(None);
    }
    let installed = apps::installed(paths, app)?;
    if installed.install_kind == "installer" {
        return Ok(None);
    }
    let data = PathBuf::from(installed.path).join("PhotoCraftData");
    if !data.exists() {
        return Ok(None);
    }
    files::inside(&data, &paths.at("releases"))?;
    let kept = paths.at("runtime/app-profiles/photocraft");
    files::inside(&kept, &paths.at("runtime/app-profiles"))?;
    if kept.exists() {
        bail!("A retained PhotoCraft profile already exists; leaving both profiles intact");
    }
    std::fs::create_dir_all(kept.parent().unwrap())?;
    std::fs::rename(&data, &kept)?;
    Ok(Some((data, kept)))
}
pub fn restore_portable(paths: &Paths, app: &str, target: &Path) -> Result<()> {
    if app != "photocraft" {
        return Ok(());
    }
    let kept = paths.at("runtime/app-profiles/photocraft");
    if !kept.exists() {
        return Ok(());
    }
    files::inside(&kept, &paths.at("runtime/app-profiles"))?;
    let data = target.join("PhotoCraftData");
    files::inside(&data, &paths.at("releases"))?;
    if data.exists() {
        bail!("PhotoCraftData already exists; retained profile was not overwritten");
    }
    std::fs::rename(kept, data)?;
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn targets(_: &Paths, _: &str) -> Result<Vec<Target>> {
    bail!(
        "Linux profile deletion is unavailable until each upstream app's paths have been verified"
    )
}
#[cfg(target_os = "macos")]
pub fn targets(_: &Paths, _: &str) -> Result<Vec<Target>> {
    bail!(
        "macOS profile deletion is unavailable until each upstream app's paths have been verified"
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_cleanup_only_removes_verified_app_folders() {
        let root = std::env::temp_dir().join(format!("craft-profiles-{}", uuid::Uuid::new_v4()));
        let roaming = root.join("roaming");
        let local = root.join("local");
        std::fs::create_dir_all(roaming.join("FilmCraft")).unwrap();
        std::fs::create_dir_all(roaming.join("DesignCraft")).unwrap();
        std::fs::write(roaming.join("FilmCraft/ui.json"), "preferences").unwrap();
        remove(&mapped("filmcraft", &roaming, &local).unwrap()).unwrap();
        assert!(!roaming.join("FilmCraft").exists());
        assert!(roaming.join("DesignCraft").exists());
        let pdf = mapped("printcraft", &roaming, &local).unwrap();
        assert_eq!(pdf.len(), 4);
        assert!(pdf.iter().any(|t| t.path == local.join("PdfCraft")));
        assert!(mapped("unknown", &roaming, &local).is_err());
        assert!(remove(&[Target {
            path: root.clone(),
            root: root.clone()
        }])
        .is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
