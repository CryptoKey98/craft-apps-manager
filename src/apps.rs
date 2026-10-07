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
        if entry.file_type()?.is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
        {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    names.sort();
    Ok(names)
}
pub fn launch(paths: &Paths, app: &str) -> Result<()> {
    let installed = installed(paths, app)?;
    let settings = settings(paths, app)?;
    let executable = if settings.executable.is_empty() {
        format!("{app}.exe")
    } else {
        settings.executable
    };
    if !executables(paths, app)?.contains(&executable) {
        bail!("Selected executable is missing. Check launch settings.");
    }
    let root = PathBuf::from(installed.path);
    Command::new(root.join(executable))
        .args(settings.arguments)
        .current_dir(root)
        .spawn()?;
    Ok(())
}
pub fn uninstall(paths: &Paths, app: &str) -> Result<()> {
    crate::model::valid_app(app)?;
    let _lock = platform::Lock::take("Local\\CraftAppsUpdater")?;
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
        return files::write_json(&paths.at("settings.json"), &config);
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
    if let Err(error) = files::write_json(&paths.at("settings.json"), &config) {
        std::fs::rename(&temporary, &root)?;
        return Err(error);
    }
    files::remove_managed(&temporary, &releases)
}
