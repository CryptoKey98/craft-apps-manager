use crate::{
    files,
    model::{Installed, Paths},
    platform,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf, process::Command};

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct LaunchSettings {
    pub executable: String,
    pub arguments: Vec<String>,
}
pub fn settings(paths: &Paths, app: &str) -> Result<LaunchSettings> {
    let all: BTreeMap<String, LaunchSettings> =
        files::read_or_default(&paths.at("runtime/launch-settings.json"))?;
    Ok(all.get(app).cloned().unwrap_or_default())
}
pub fn save(paths: &Paths, app: &str, value: &LaunchSettings) -> Result<()> {
    crate::model::valid_app(app)?;
    let file = paths.at("runtime/launch-settings.json");
    let mut all: BTreeMap<String, LaunchSettings> = files::read_or_default(&file)?;
    all.insert(app.into(), value.clone());
    files::write_json(&file, &all)
}
pub fn installed(paths: &Paths, app: &str) -> Result<Installed> {
    paths
        .config()?
        .apps
        .into_iter()
        .find(|a| a.name == app && !a.path.is_empty())
        .context("This app is not installed")
}
pub fn executables(paths: &Paths, app: &str) -> Result<Vec<String>> {
    let installed = installed(paths, app)?;
    let root = PathBuf::from(installed.path);
    if installed.install_kind != "installer" {
        files::inside(&root, &paths.at("releases"))?;
    }
    let mut names = Vec::new();
    for entry in std::fs::read_dir(&root)? {
        let entry = entry?;
        if !entry.file_type()?.is_symlink()
            && crate::model::is_executable(&entry.path())
            && valid_launch_item(&entry.path(), app)
            && (cfg!(target_os = "windows")
                && entry
                    .path()
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
                || cfg!(not(target_os = "windows"))
                    && crate::model::executable_names(app)
                        .contains(&entry.file_name().to_string_lossy().into_owned()))
        {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    names.sort();
    Ok(names)
}
fn valid_launch_item(item: &std::path::Path, app: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        crate::installers::is_owned_bundle(item, app)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (item, app);
        true
    }
}
fn launch_executable(root: &std::path::Path, app: &str, selected: &str) -> Result<String> {
    let default = || {
        crate::model::installed_executable(root, app)
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .context("App executable is missing")
    };
    if selected.is_empty() {
        return default();
    }
    #[cfg(target_os = "macos")]
    {
        let aliases = crate::model::executable_names(app);
        let chosen = std::path::Path::new(selected);
        let recognized = aliases
            .iter()
            .any(|name| chosen == std::path::Path::new(name) || chosen == root.join(name));
        if aliases.len() > 1 && recognized {
            return default();
        }
    }
    Ok(selected.into())
}
pub fn launch(paths: &Paths, app: &str) -> Result<()> {
    let installed = installed(paths, app)?;
    let settings = settings(paths, app)?;
    let executable = launch_executable(&PathBuf::from(&installed.path), app, &settings.executable)?;
    if !executables(paths, app)?.contains(&executable) {
        bail!("Selected executable is missing. Check launch settings.");
    }
    let root = PathBuf::from(installed.path);
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("/usr/bin/open");
        command.arg("-a").arg(root.join(executable)).arg("--args");
        command
    };
    #[cfg(not(target_os = "macos"))]
    let mut command = Command::new(root.join(executable));
    #[cfg(target_os = "linux")]
    if installed.install_kind != "installer" {
        command.env("APPIMAGE_EXTRACT_AND_RUN", "1");
    }
    command.args(settings.arguments).current_dir(root).spawn()?;
    Ok(())
}
pub fn uninstall(paths: &Paths, app: &str) -> Result<()> {
    crate::model::valid_app(app)?;
    let _lock = platform::Lock::take("Local\\CraftAppsManager")?;
    uninstall_locked(paths, app)
}
fn uninstall_locked(paths: &Paths, app: &str) -> Result<()> {
    if platform::running_app(app)? {
        bail!("Close the app before uninstalling it.");
    }
    let mut config = paths.config()?;
    let installed = installed(paths, app)?;
    if installed.install_kind == "installer" {
        crate::installers::uninstall(&installed)?;
        for entry in config.apps.iter_mut().filter(|a| a.name == app) {
            entry.path.clear();
            entry.version.clear();
            entry.install_kind.clear();
            entry.product_code.clear();
        }
        return paths.save_config(&config);
    }
    let root = PathBuf::from(&installed.path);
    let releases = paths.at("releases");
    files::inside(&root, &releases)?;
    if root.parent() != Some(releases.as_path()) {
        bail!("Only managed portable app folders can be uninstalled here.");
    }
    let temporary = releases.join(format!(".uninstall-{app}-{}", uuid::Uuid::new_v4()));
    std::fs::rename(&root, &temporary)?;
    config
        .apps
        .iter_mut()
        .filter(|a| a.name == app)
        .for_each(|a| {
            a.path.clear();
            a.version.clear();
        });
    if let Err(error) = paths.save_config(&config) {
        std::fs::rename(&temporary, &root)?;
        return Err(error);
    }
    files::remove_managed(&temporary, &releases)
}
pub fn uninstall_with_profile(paths: &Paths, app: &str, delete_profile: bool) -> Result<()> {
    crate::model::valid_app(app)?;
    let _lock = platform::Lock::take("Local\\CraftAppsManager")?;

    if platform::running_app(app)? {
        bail!("Close the app before uninstalling it.");
    }
    let targets = if delete_profile {
        crate::profiles::targets(paths, app)?
    } else {
        Vec::new()
    };
    if delete_profile {
        crate::profiles::validate(&targets)?;
    }
    let preserved = if delete_profile {
        None
    } else {
        crate::profiles::preserve_portable(paths, app)?
    };
    if let Err(error) = uninstall_locked(paths, app) {
        if let Some((original, kept)) = preserved {
            std::fs::rename(kept, original)
                .context("Uninstall failed; could not restore the retained profile")?;
        }
        return Err(error);
    }
    if delete_profile {
        crate::profiles::remove(&targets)
            .context("App was uninstalled, but profile cleanup failed")?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn repair_linux_shortcuts(paths: &Paths) -> Result<()> {
    for app in crate::model::APPS {
        let old = paths.at(format!("releases/{app}.lnk"));
        if !old.is_file() {
            continue;
        }
        files::inside(&old, &paths.at("releases"))?;
        let target = paths.at(format!("releases/{app}/{app}"));
        if !target.is_file() {
            continue;
        }
        files::inside(&target, &paths.at("releases"))?;
        let expected = format!("Exec=\"{}\" ", target.display());
        if std::fs::metadata(&old)?.len() > 65536 {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(&old) else {
            continue;
        };
        if !contents.starts_with("[Desktop Entry]\n")
            || !contents.lines().any(|line| line == expected)
        {
            continue;
        }
        platform::shortcut(
            &paths.at(format!("releases/{app}.desktop")),
            &target,
            "",
            target.parent().unwrap(),
        )?;
        std::fs::remove_file(old)?;
    }
    Ok(())
}

#[cfg(all(test, target_os = "macos"))]
mod macos_launch_tests {
    use super::*;
    #[test]
    fn renamed_bundle_selection_follows_current_app_without_changing_arguments() {
        let root = std::env::temp_dir().join(format!("craft-launch-{}", uuid::Uuid::new_v4()));
        let paths = Paths::new(root.clone(), None);
        let folder = paths.at("releases/printcraft");
        std::fs::create_dir_all(&folder).unwrap();
        files::write_json(
            &paths.at("manager-settings.json"),
            &crate::model::Preferences {
                release_format: "portable".into(),
                ..Default::default()
            },
        )
        .unwrap();
        for (name, id) in [
            ("PrintCraft.app", "ai.storyteller.printcraft"),
            ("PdfCraft.app", "ai.storyteller.pdfcraft"),
        ] {
            let contents = folder.join(name).join("Contents");
            std::fs::create_dir_all(&contents).unwrap();
            std::fs::write(contents.join("Info.plist"), format!(r#"<plist version="1.0"><dict><key>CFBundleIdentifier</key><string>{id}</string><key>CFBundleShortVersionString</key><string>0.4.0</string></dict></plist>"#)).unwrap();
        }
        for selected in [
            String::new(),
            "PrintCraft.app".into(),
            folder.join("PrintCraft.app").display().to_string(),
        ] {
            let value = LaunchSettings {
                executable: selected.clone(),
                arguments: vec!["--profile".into(), "a folder with spaces".into()],
            };
            save(&paths, "printcraft", &value).unwrap();
            let loaded = settings(&paths, "printcraft").unwrap();
            assert_eq!(
                launch_executable(&folder, "printcraft", &loaded.executable).unwrap(),
                "PdfCraft.app"
            );
            assert_eq!(loaded.executable, selected);
            assert_eq!(loaded.arguments, value.arguments);
        }
        std::fs::remove_dir_all(folder.join("PrintCraft.app")).unwrap();
        assert_eq!(
            launch_executable(&folder, "printcraft", "PrintCraft.app").unwrap(),
            "PdfCraft.app"
        );
        assert_eq!(executables(&paths, "printcraft").unwrap(), ["PdfCraft.app"]);
        for custom in ["custom-tool", "/elsewhere/PrintCraft.app"] {
            assert_eq!(
                launch_executable(&folder, "printcraft", custom).unwrap(),
                custom
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
