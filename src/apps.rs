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
    let (key, folder) = launch_settings_key(paths, app);
    if let Some(settings) = all.get(&key) {
        return Ok(settings.clone());
    }
    let mut settings = all
        .get(app)
        .cloned()
        .or_else(|| {
            if folder.is_none() {
                let mut previous = all
                    .iter()
                    .filter(|(key, _)| key.starts_with(&format!("{app}|")));
                let first = previous.next();
                if previous.next().is_none() {
                    return first.map(|(_, settings)| settings.clone());
                }
            }
            None
        })
        .unwrap_or_default();
    if let Some(folder) = folder {
        let executable = std::path::Path::new(&settings.executable);
        if executable.is_absolute() && !executable.starts_with(folder) {
            settings.executable.clear();
        }
    }
    Ok(settings)
}
fn launch_settings_key(paths: &Paths, app: &str) -> (String, Option<PathBuf>) {
    let folder = files::read_json::<crate::model::Config>(&paths.at("settings.json"))
        .ok()
        .and_then(|config| {
            config
                .apps
                .into_iter()
                .find(|copy| copy.name == app && !copy.path.is_empty())
        })
        .map(|copy| PathBuf::from(copy.path));
    (
        folder
            .as_ref()
            .map(|folder| format!("{app}|{}", folder.display()))
            .unwrap_or_else(|| app.into()),
        folder,
    )
}
pub fn save(paths: &Paths, app: &str, value: &LaunchSettings) -> Result<()> {
    crate::model::valid_app(app)?;
    let file = paths.at("runtime/launch-settings.json");
    let mut all: BTreeMap<String, LaunchSettings> = files::read_or_default(&file)?;
    all.insert(launch_settings_key(paths, app).0, value.clone());
    files::write_json(&file, &all)
}
/// Keep the old key until relocation commits, so a failed move still has its options.
pub(crate) fn carry_launch_settings(paths: &Paths, app: &str, old: &str, new: &str) -> Result<()> {
    let file = paths.at("runtime/launch-settings.json");
    let mut all: BTreeMap<String, LaunchSettings> = files::read_or_default(&file)?;
    if let Some(mut settings) = all
        .get(&format!("{app}|{old}"))
        .or_else(|| all.get(app))
        .cloned()
    {
        if let Ok(relative) = std::path::Path::new(&settings.executable).strip_prefix(old) {
            settings.executable = std::path::Path::new(new)
                .join(relative)
                .display()
                .to_string();
        }
        all.insert(format!("{app}|{new}"), settings);
        files::write_json(&file, &all)?;
    }
    Ok(())
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
    executables_for_installed(paths, app, &installed)
}
fn executables_for_installed(
    paths: &Paths,
    app: &str,
    installed: &Installed,
) -> Result<Vec<String>> {
    let root = PathBuf::from(&installed.path);
    if installed.install_kind != "installer" {
        crate::portable::validate(paths, app, &root)?;
    }
    let mut names = Vec::new();
    let aliases = crate::model::executable_names(app);
    for entry in std::fs::read_dir(&root)? {
        let entry = entry?;
        // Bundle identity checks spawn plutil, so first reject unrelated
        // filenames in potentially large Applications folders.
        if (cfg!(target_os = "windows")
            && entry
                .path()
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
            || cfg!(not(target_os = "windows"))
                && aliases.contains(&entry.file_name().to_string_lossy().into_owned()))
            && !entry.file_type()?.is_symlink()
            && crate::model::is_executable(&entry.path())
            && valid_launch_item(&entry.path(), app)
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
    #[cfg(any(target_os = "macos", target_os = "linux"))]
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
    if !executables_for_installed(paths, app, &installed)?
        .iter()
        .any(|name| launch_name_matches(name, &executable))
    {
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
fn launch_name_matches(actual: &str, selected: &str) -> bool {
    if cfg!(target_os = "windows") {
        actual.eq_ignore_ascii_case(selected)
    } else {
        actual == selected
    }
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
        if let Err(error) = crate::portable::remove_installer_shortcut(paths, app) {
            eprintln!("Desktop shortcut cleanup: {error:#}");
        }
        forget_installation(&mut config, &installed);
        return paths.save_config(&config);
    }
    let root = PathBuf::from(&installed.path);
    crate::portable::validate(paths, app, &root)?;
    let releases = root
        .parent()
        .context("Missing app parent folder")?
        .to_path_buf();
    let temporary = releases.join(format!(".uninstall-{app}-{}", uuid::Uuid::new_v4()));
    std::fs::rename(&root, &temporary)?;
    forget_installation(&mut config, &installed);
    if let Err(error) = paths.save_config(&config) {
        std::fs::rename(&temporary, &root)?;
        return Err(error);
    }
    if let Err(error) = crate::portable::remove_shortcut(&temporary) {
        eprintln!("Shortcut cleanup: {error:#}");
    }
    files::remove_managed(&temporary, &releases)
}
/// Remove only the completed uninstall target; keep other verified copies
/// available immediately and preserve the format of an empty selected record.
pub(crate) fn forget_installation(config: &mut crate::model::Config, removed: &Installed) {
    config.installations.retain(|record| {
        !(record.name == removed.name
            && record.path == removed.path
            && (record.install_kind == "installer") == (removed.install_kind == "installer"))
    });
    let remaining = config
        .installations
        .iter()
        .find(|record| {
            record.name == removed.name
                && !record.path.is_empty()
                && crate::model::installed_executable(
                    std::path::Path::new(&record.path),
                    &record.name,
                )
                .is_some()
        })
        .cloned();
    for entry in config.apps.iter_mut().filter(|a| a.name == removed.name) {
        if let Some(record) = &remaining {
            *entry = record.clone();
        } else {
            entry.path.clear();
            entry.version.clear();
            entry.product_code.clear();
            entry.install_kind = removed.install_kind.clone();
            entry.architecture = removed.architecture.clone();
        }
    }
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
            files::move_verified(&kept, &original)
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
    for app in crate::model::apps() {
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

#[cfg(all(test, target_os = "linux"))]
mod linux_launch_tests {
    use super::*;
    #[test]
    fn renamed_linux_launch_uses_current_binary_and_falls_back_to_legacy() {
        let root = std::env::temp_dir().join(format!(
            "craft-linux-alias-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("printcraft"), b"legacy").unwrap();
        assert_eq!(
            launch_executable(&root, "printcraft", "").unwrap(),
            "printcraft"
        );
        std::fs::write(root.join("pdfcraft"), b"current").unwrap();
        assert_eq!(
            crate::model::installed_executable(&root, "printcraft"),
            Some(root.join("pdfcraft"))
        );
        for selected in [
            String::new(),
            "printcraft".into(),
            "pdfcraft".into(),
            root.join("printcraft").display().to_string(),
        ] {
            assert_eq!(
                launch_executable(&root, "printcraft", &selected).unwrap(),
                "pdfcraft"
            );
        }
        for selected in ["custom-tool", "/elsewhere/printcraft"] {
            assert_eq!(
                launch_executable(&root, "printcraft", selected).unwrap(),
                selected
            );
        }
        std::fs::remove_file(root.join("pdfcraft")).unwrap();
        assert_eq!(
            launch_executable(&root, "printcraft", "pdfcraft").unwrap(),
            "printcraft"
        );
    }
}

#[cfg(test)]
mod launch_name_tests {
    #[test]
    fn launch_names_follow_platform_case_rules_without_matching_other_programs() {
        assert_eq!(
            super::launch_name_matches("Example.exe", "example.exe"),
            cfg!(target_os = "windows")
        );
        assert!(super::launch_name_matches("Example.exe", "Example.exe"));
        assert!(!super::launch_name_matches(
            "Example-helper.exe",
            "example.exe"
        ));
    }
}
