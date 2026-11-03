use crate::{apps, files, model::Paths};
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Target {
    pub path: PathBuf,
    pub root: PathBuf,
}
#[cfg(target_os = "windows")]
pub(crate) fn known_folder(id: &windows::core::GUID) -> Result<PathBuf> {
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
                crate::portable::validate(paths, app, &root)?;
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
    crate::portable::validate(paths, app, data.parent().unwrap())?;
    files::inside(&data, data.parent().unwrap())?;
    let kept = paths.at("runtime/app-profiles/photocraft");
    files::inside(&kept, &paths.at("runtime/app-profiles"))?;
    if kept.exists() {
        bail!("A retained PhotoCraft profile already exists; leaving both profiles intact");
    }
    std::fs::create_dir_all(kept.parent().unwrap())?;
    files::move_verified(&data, &kept)?;
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
    crate::portable::validate(paths, app, data.parent().unwrap())?;
    files::inside(&data, data.parent().unwrap())?;
    if data.exists() {
        bail!("PhotoCraftData already exists; retained profile was not overwritten");
    }
    files::move_verified(&kept, &data)?;
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn targets(_: &Paths, _: &str) -> Result<Vec<Target>> {
    bail!(
        "Linux profile deletion is unavailable until each upstream app's paths have been verified"
    )
}
#[cfg(target_os = "macos")]
pub fn targets(_: &Paths, app: &str) -> Result<Vec<Target>> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("The current user's home folder is unavailable"))?;
    macos_targets(app, &home)
}

#[cfg(any(target_os = "macos", test))]
fn macos_targets(app: &str, home: &Path) -> Result<Vec<Target>> {
    // These names come from upstream profile/config code, not display titles or
    // bundle IDs. Unknown and custom apps must never inherit guessed paths.
    let (settings, caches): (&[&str], &[&str]) = match app {
        "designcraft" => (&["DesignCraft"], &[]),
        "effectcraft" => (&["EffectCraft"], &["EffectCraft"]),
        "filmcraft" => (&["FilmCraft"], &[]),
        "lightcraft" => (&["LightCraft"], &[]),
        "photocraft" => (&["Photocraft"], &[]),
        "printcraft" => (&["PdfCraft", "PrintCraft"], &["PdfCraft"]),
        "vectorcraft" => (&["VectorCraft", "DrawCraft"], &[]),
        "wordcraft" => (&["WordCraft"], &[]),
        "gridcraft" => (&["GridCraft"], &[]),
        "deckcraft" => (&["DeckCraft"], &[]),
        "soundcraft" => (&["SoundCraft"], &[]),
        "cadcraft" => (&["CADCraft"], &[]),
        "figcraft" => (&["FigCraft"], &[]),
        "texturecraft" => (&["TextureCraft"], &[]),
        "solidcraft" => (
            &["SolidCraft", "SolveCraft", "ai.storyteller.solvecraft"],
            &["SolidCraft"],
        ),
        _ => bail!("This app has no verified macOS settings or cache locations"),
    };
    anyhow::ensure!(home.is_absolute(), "The home folder must be absolute");
    let settings_root = home.join("Library/Application Support");
    let cache_root = home.join("Library/Caches");
    // Include Library in the containment check; a link above either app-specific
    // root must not redirect cleanup to another user's files or a shared folder.
    files::inside(&settings_root, home)?;
    files::inside(&cache_root, home)?;
    Ok(settings
        .iter()
        .map(|name| Target {
            path: settings_root.join(name),
            root: settings_root.clone(),
        })
        .chain(caches.iter().map(|name| Target {
            path: cache_root.join(name),
            root: cache_root.clone(),
        }))
        .collect())
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

    #[test]
    fn macos_cleanup_keeps_other_apps_projects_and_custom_locations() {
        let home =
            std::env::temp_dir().join(format!("craft-mac-profiles-{}", uuid::Uuid::new_v4()));
        let support = home.join("Library/Application Support");
        let cache = home.join("Library/Caches");
        let own = support.join("EffectCraft");
        let own_cache = cache.join("EffectCraft");
        let other = support.join("FilmCraft");
        let custom = home.join("custom-profile");
        let projects = home.join("Documents");
        for path in [&own, &own_cache, &other, &custom, &projects] {
            std::fs::create_dir_all(path).unwrap();
            std::fs::write(path.join("keep-or-remove"), "fixture").unwrap();
        }
        let targets = macos_targets("effectcraft", &home).unwrap();
        assert_eq!(targets.len(), 2);
        remove(&targets).unwrap();
        assert!(!own.exists() && !own_cache.exists());
        for path in [&other, &custom, &projects] {
            assert!(path.join("keep-or-remove").is_file());
        }
        assert!(support.is_dir() && cache.is_dir());
        // Already removed paths are harmless on repeated cleanup.
        remove(&targets).unwrap();
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn macos_cleanup_requires_verified_app_identity_and_absolute_home() {
        let home =
            std::env::temp_dir().join(format!("craft-mac-profiles-{}", uuid::Uuid::new_v4()));
        assert!(macos_targets("unknown", &home).is_err());
        assert!(macos_targets("someone/effectcraft", &home).is_err());
        assert!(macos_targets("effectcraft", Path::new("relative")).is_err());
        assert!(macos_targets("photocraft", &home)
            .unwrap()
            .iter()
            .any(|target| target.path == home.join("Library/Application Support/Photocraft")));
    }

    #[test]
    fn macos_solidcraft_cleanup_includes_verified_legacy_settings_and_slicer_cache() {
        let home =
            std::env::temp_dir().join(format!("craft-solid-profiles-{}", uuid::Uuid::new_v4()));
        let targets = macos_targets("solidcraft", &home).unwrap();
        let expected = [
            "Library/Application Support/SolidCraft",
            "Library/Application Support/SolveCraft",
            "Library/Application Support/ai.storyteller.solvecraft",
            "Library/Caches/SolidCraft",
        ];
        assert_eq!(targets.len(), expected.len());
        for (target, relative) in targets.iter().zip(expected) {
            assert_eq!(target.path, home.join(relative));
            std::fs::create_dir_all(&target.path).unwrap();
            std::fs::write(target.path.join("fixture"), "data").unwrap();
        }
        let other = home.join("Library/Application Support/OtherCAD");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("project.sc"), "user project").unwrap();
        remove(&targets).unwrap();
        assert!(targets.iter().all(|target| !target.path.exists()));
        assert!(other.join("project.sc").is_file());
        std::fs::remove_dir_all(home).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn macos_cleanup_rejects_linked_library_roots_and_app_folders() {
        use std::os::unix::fs::symlink;
        let fixture =
            std::env::temp_dir().join(format!("craft-mac-profiles-{}", uuid::Uuid::new_v4()));
        let home = fixture.join("home");
        let outside = fixture.join("outside");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("untouched"), "fixture").unwrap();
        symlink(&outside, home.join("Library")).unwrap();
        assert!(macos_targets("effectcraft", &home).is_err());
        std::fs::remove_file(home.join("Library")).unwrap();
        let support = home.join("Library/Application Support");
        std::fs::create_dir_all(&support).unwrap();
        symlink(&outside, support.join("EffectCraft")).unwrap();
        assert!(remove(&macos_targets("effectcraft", &home).unwrap()).is_err());
        assert!(outside.join("untouched").is_file());
        std::fs::remove_dir_all(fixture).unwrap();
    }
}
