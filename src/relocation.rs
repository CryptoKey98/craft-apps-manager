//! Relocate app installations without changing external user-profile folders.
use crate::{
    apps, files,
    jobs::Job,
    model::{Installed, Paths},
    platform, portable,
};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub fn supported(app: &Installed) -> bool {
    if app.install_kind != "installer" {
        return true;
    }
    #[cfg(target_os = "windows")]
    {
        // Never back up or overlay a shared directory such as Program Files.
        let leaf = Path::new(&app.path)
            .file_name()
            .map(|name| {
                name.to_string_lossy()
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric())
                    .flat_map(char::to_lowercase)
                    .collect::<String>()
            })
            .unwrap_or_default();
        crate::installers::custom_location_supported(&app.name)
            && !app.product_code.is_empty()
            && (leaf == app.name || (app.name == "printcraft" && leaf == "pdfcraft"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

pub fn destination(paths: &Paths, app: &Installed, parent: &Path) -> Result<PathBuf> {
    anyhow::ensure!(
        supported(app),
        "This system-managed installation has a fixed location. Portable apps can be moved."
    );
    let target = portable::destination(parent, &app.name)?;
    let current = Path::new(&app.path);
    anyhow::ensure!(
        target != current && !target.starts_with(current) && !current.starts_with(&target),
        "Choose a different folder outside the current installation"
    );
    anyhow::ensure!(
        !target.exists(),
        "That app folder already exists. Choose another location."
    );
    anyhow::ensure!(
        !target.starts_with(&paths.root) || target.starts_with(paths.at("releases")),
        "Choose a location outside the manager's sources, builds and runtime folders"
    );
    #[cfg(target_os = "windows")]
    if app.install_kind == "installer" {
        crate::installers::validate_custom_location(&target)?;
    }
    Ok(target)
}

pub fn move_app(paths: &Paths, expected: &Installed, parent: &Path, job: &Job) -> Result<()> {
    let _lock = platform::Lock::take("Local\\CraftAppsManager")?;
    let current = apps::installed(paths, &expected.name)?;
    anyhow::ensure!(
        current.path == expected.path
            && current.version == expected.version
            && current.install_kind == expected.install_kind,
        "Installation changed. Reopen the Move dialog."
    );
    anyhow::ensure!(
        !platform::running_app(&current.name)?,
        "Close the app before moving it."
    );
    let target = destination(paths, &current, parent)?;
    job.check()?;
    if current.install_kind == "installer" {
        #[cfg(target_os = "windows")]
        return move_installer(paths, &current, &target, job);
        #[cfg(not(target_os = "windows"))]
        anyhow::bail!("This installation has a fixed location");
    }
    move_portable(paths, &current, &target, job)
}

fn save_location(paths: &Paths, app: &Installed) -> Result<()> {
    let mut config = paths.config()?;
    let record = config
        .apps
        .iter_mut()
        .find(|a| a.name == app.name)
        .context("App missing from library")?;
    *record = app.clone();
    paths.save_config(&config)
}

fn update_shortcuts(paths: &Paths, old: &Installed, new: &Installed, job: &Job) {
    let target = Path::new(&new.path);
    if let Some(executable) = crate::model::installed_executable(target, &new.name) {
        let shortcut = paths.at(format!(
            "releases/{}.{}",
            new.name,
            platform::shortcut_extension()
        ));
        if let Err(error) = platform::shortcut(&shortcut, &executable, "", target) {
            job.log(&format!("Library shortcut warning: {error:#}"));
        }
    }
    if let Err(error) =
        portable::refresh_moved_shortcut(paths, &new.name, target, new.install_kind == "installer")
    {
        job.log(&format!("Desktop shortcut warning: {error:#}"));
    }
    if let Ok(mut settings) = apps::settings(paths, &new.name) {
        if let Ok(relative) = Path::new(&settings.executable).strip_prefix(&old.path) {
            settings.executable = target.join(relative).display().to_string();
            if let Err(error) = apps::save(paths, &new.name, &settings) {
                job.log(&format!("Launch settings warning: {error:#}"));
            }
        }
    }
}

fn move_portable(paths: &Paths, current: &Installed, target: &Path, job: &Job) -> Result<()> {
    let source = Path::new(&current.path);
    portable::validate(paths, &current.name, source)?;
    portable::validate_install(paths, &current.name, target)?;
    files::no_links(source)?;
    anyhow::ensure!(
        crate::model::installed_executable(source, &current.name).is_some(),
        "App executable is missing"
    );
    job.stage(
        "Moving app",
        None,
        "Moving and verifying app files; settings inside the app folder move with it.",
    );
    files::move_verified(source, target)?;
    let mut moved = current.clone();
    moved.path = target.display().to_string();
    let commit = portable::mark_moved(paths, &current.name, target)
        .and_then(|()| save_location(paths, &moved));
    if let Err(error) = commit {
        files::move_verified(target, source)
            .context("Could not restore the original app location")?;
        return Err(error);
    }
    update_shortcuts(paths, current, &moved, job);
    job.log(&format!("{}: moved to {}", current.name, target.display()));
    Ok(())
}

#[cfg(target_os = "windows")]
fn move_installer(paths: &Paths, current: &Installed, target: &Path, job: &Job) -> Result<()> {
    use crate::{installers, model::Release, network::Network, updates};
    let network = Network::new(&paths.root)?;
    let mut prefs = paths.read_preferences()?;
    prefs.release_format = "installer".into();
    prefs.architecture = current.architecture.clone();
    job.stage(
        "Preparing move",
        None,
        "Finding the installed version's verified installer.",
    );
    let releases: Vec<Release> = network.json(&format!(
        "https://api.github.com/repos/{}/releases?per_page=100",
        crate::model::repository(&current.name)
    ))?;
    let release = releases
        .iter()
        .find(|r| {
            !r.draft
                && !r.prerelease
                && updates::release_version(&r.tag_name).is_ok_and(|v| v == current.version)
        })
        .context("The installed version is unavailable for a safe reinstall")?;
    let asset = updates::select_asset(release, &current.name, &prefs)?;
    anyhow::ensure!(
        asset.name.ends_with(".msi"),
        "This installer does not support relocation"
    );
    let installer = paths.at(format!(
        "releases/installers/{}/{}",
        current.name, asset.name
    ));
    if installer.exists() {
        crate::network::verify_asset(&installer, asset)?;
    } else {
        network.asset(asset, &installer, job)?;
    }
    anyhow::ensure!(
        installers::package_location_supported(&installer),
        "This MSI does not expose a supported install location"
    );
    job.check()?;
    anyhow::ensure!(
        !platform::running_app(&current.name)?,
        "Close the app before moving it."
    );
    let backup = paths.at(format!(
        "runtime/relocation/{}-{}",
        current.name,
        uuid::Uuid::new_v4()
    ));
    job.stage(
        "Preserving app files",
        None,
        "Keeping a verified recovery copy before reinstalling.",
    );
    files::copy_directory_verified(Path::new(&current.path), &backup)?;
    let attempt = (|| -> Result<Installed> {
        job.check()?;
        installers::uninstall(current)?;
        let installed =
            installers::run_with_destination(&installer, &current.name, Some(job), Some(target))?;
        anyhow::ensure!(
            installed.version == current.version,
            "Installer version did not match the installed app"
        );
        anyhow::ensure!(
            Path::new(&installed.path) == target,
            "Installer used a different location"
        );
        restore_installer_files(paths, &current.name, &backup, target, job)?;
        save_location(paths, &installed)?;
        Ok(installed)
    })();
    match attempt {
        Ok(moved) => {
            update_shortcuts(paths, current, &moved, job);
        }
        Err(error) => {
            job.stage(
                "Restoring previous installation",
                None,
                "Returning the app to its original location.",
            );
            let recovery = (|| -> Result<()> {
                if let Some(installed) = installers::detect(&current.name)? {
                    if installed.path == current.path {
                        restore_installer_files(
                            paths,
                            &current.name,
                            &backup,
                            Path::new(&current.path),
                            &Job::new(paths.at("logs/move-recovery.log"), &Default::default()),
                        )?;
                        return Ok(());
                    }
                    installers::uninstall(&installed)?;
                }
                let restored = installers::run_replacement(&installer, current, None)?;
                restore_installer_files(
                    paths,
                    &current.name,
                    &backup,
                    Path::new(&restored.path),
                    &Job::new(paths.at("logs/move-recovery.log"), &Default::default()),
                )?;
                save_location(paths, &restored)
            })();
            if let Err(recovery) = recovery {
                anyhow::bail!(
                    "Move failed: {error:#}. Recovery failed: {recovery:#}. Preserved files: {}",
                    backup.display()
                );
            }
            files::remove_managed(&backup, &paths.at("runtime/relocation"))?;
            return Err(error.context("Original installation restored"));
        }
    }
    if let Err(error) = files::remove_managed(&backup, &paths.at("runtime/relocation")) {
        job.log(&format!("Recovery copy retained: {error:#}"));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
pub(crate) fn restore_extra_files(source: &Path, target: &Path) -> Result<()> {
    files::no_links(source)?;
    files::no_links(target)?;
    for entry in walkdir::WalkDir::new(source).min_depth(1) {
        let entry = entry?;
        let destination = target.join(entry.path().strip_prefix(source)?);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(destination)?;
        } else {
            // Reinstalled MSI files are usually identical. Leave those alone:
            // rewriting them needs extra privileges and can hit executable locks.
            if destination.is_file() && files::hash(entry.path())? == files::hash(&destination)? {
                continue;
            }
            // This is the same version, not an upgrade: preserve every original
            // app-local file, including settings that an installer may recreate.
            std::fs::copy(entry.path(), &destination)?;
            anyhow::ensure!(
                files::hash(entry.path())? == files::hash(&destination)?,
                "App file restoration failed verification"
            );
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn restore_installer_files(
    paths: &Paths,
    app: &str,
    source: &Path,
    target: &Path,
    job: &Job,
) -> Result<()> {
    match restore_extra_files(source, target) {
        Ok(()) => Ok(()),
        Err(error)
            if error.chain().any(|cause| {
                cause
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied)
            }) =>
        {
            job.stage("Restoring app files", None, "Approve administrator access to preserve app-local settings in the protected install folder.");
            let plan = RestorePlan {
                app: app.into(),
                source: source.into(),
                target: target.into(),
            };
            let file = paths.at(format!(
                "runtime/relocation/restore-{}.json",
                uuid::Uuid::new_v4()
            ));
            files::write_json(&file, &plan)?;
            // Neither an embedded quote nor a trailing backslash is possible in
            // the generated file path; the library path is normalized first.
            let root = std::path::absolute(&paths.root)?.join(".");
            let args = format!(
                "--root \"{}\" --restore-relocation-files \"{}\"",
                root.display(),
                file.display()
            );
            let result = platform::run_elevated(&std::env::current_exe()?, &args, job);
            let _ = std::fs::remove_file(file);
            result.context("Could not restore app files with administrator permission")
        }
        Err(error) => Err(error),
    }
}

#[cfg(target_os = "windows")]
#[derive(serde::Serialize, serde::Deserialize)]
struct RestorePlan {
    app: String,
    source: PathBuf,
    target: PathBuf,
}

#[cfg(target_os = "windows")]
pub fn restore_elevated(paths: &Paths, file: &Path) -> Result<()> {
    let boundary = paths.at("runtime/relocation");
    files::inside(file, &boundary)?;
    let plan: RestorePlan = files::read_json(file)?;
    crate::model::valid_app(&plan.app)?;
    files::inside(&plan.source, &boundary)?;
    anyhow::ensure!(
        plan.source.file_name().is_some_and(|name| name
            .to_string_lossy()
            .starts_with(&format!("{}-", plan.app))),
        "Invalid recovery folder"
    );
    let installed = crate::installers::detect(&plan.app)?.context("App is no longer installed")?;
    anyhow::ensure!(
        supported(&installed) && Path::new(&installed.path) == plan.target,
        "Recovery target does not match the registered app installation"
    );
    restore_extra_files(&plan.source, &plan.target)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "windows")]
    #[test]
    fn installer_move_preserves_existing_app_local_settings_and_rejects_shared_roots() {
        let root =
            std::env::temp_dir().join(format!("craft-move-restore-{}", uuid::Uuid::new_v4()));
        let backup = root.join("backup");
        let target = root.join("target");
        std::fs::create_dir_all(&backup).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(backup.join("settings.json"), b"user settings").unwrap();
        std::fs::write(target.join("settings.json"), b"installer defaults").unwrap();
        restore_extra_files(&backup, &target).unwrap();
        assert_eq!(
            std::fs::read(target.join("settings.json")).unwrap(),
            b"user settings"
        );
        // An in-use or protected file must not be opened for writing when the
        // verified installed contents already match the recovery copy.
        use std::os::windows::fs::OpenOptionsExt;
        let locked = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(target.join("settings.json"))
            .unwrap();
        restore_extra_files(&backup, &target).unwrap();
        drop(locked);
        let mut app = Installed {
            name: "photocraft".into(),
            install_kind: "installer".into(),
            product_code: "{test}".into(),
            path: "C:\\Program Files".into(),
            ..Default::default()
        };
        assert!(!supported(&app));
        app.path = "C:\\Program Files\\PhotoCraft".into();
        assert!(supported(&app));
        std::fs::remove_dir_all(root).unwrap();
    }
    fn fixture() -> (Paths, Installed, Job) {
        let root = std::env::temp_dir().join(format!("craft-relocation-{}", uuid::Uuid::new_v4()));
        let paths = Paths {
            root: root.clone(),
            tools: root.join("tools"),
        };
        let source = paths.at("releases/photocraft");
        std::fs::create_dir_all(&source).unwrap();
        let executable = source.join(crate::model::executable_name("photocraft"));
        #[cfg(not(target_os = "macos"))]
        std::fs::write(executable, b"test executable").unwrap();
        #[cfg(target_os = "macos")]
        {
            std::fs::create_dir_all(executable.join("Contents")).unwrap();
            std::fs::write(executable.join("Contents/Info.plist"), "<plist><dict><key>CFBundleIdentifier</key><string>ai.storyteller.photocraft</string><key>CFBundleShortVersionString</key><string>0.6.0</string></dict></plist>").unwrap();
        }
        let app = Installed {
            name: "photocraft".into(),
            path: source.display().to_string(),
            version: "0.6.0".into(),
            install_kind: "portable".into(),
            ..Default::default()
        };
        let config = crate::model::Config {
            apps: vec![app.clone()],
            installations: vec![app.clone()],
            apps_root: paths.root.display().to_string(),
        };
        files::write_json(&paths.at("settings.json"), &config).unwrap();
        let prefs = crate::model::Preferences {
            release_format: "portable".into(),
            ..Default::default()
        };
        files::write_json(&paths.at("manager-settings.json"), &prefs).unwrap();
        portable::mark(&paths, &app.name, &source).unwrap();
        let job = Job::new(paths.at("logs/move.log"), &Default::default());
        (paths, app, job)
    }
    #[test]
    fn portable_move_preserves_settings_and_rejects_overwrite() {
        let (paths, app, job) = fixture();
        let source = Path::new(&app.path);
        std::fs::write(source.join("settings.json"), b"user settings").unwrap();
        std::fs::create_dir(source.join("PhotoCraftData")).unwrap();
        std::fs::write(source.join("PhotoCraftData/project.psd"), b"user project").unwrap();
        let external = paths.at("external-project.psd");
        std::fs::write(&external, b"external project").unwrap();
        let parent = paths
            .root
            .parent()
            .unwrap()
            .join(format!("craft-new-location-{}", uuid::Uuid::new_v4()));
        let target = destination(&paths, &app, &parent).unwrap();
        move_app(&paths, &app, &parent, &job).unwrap();
        assert!(!source.exists());
        assert_eq!(
            std::fs::read(target.join("settings.json")).unwrap(),
            b"user settings"
        );
        assert_eq!(
            std::fs::read(target.join("PhotoCraftData/project.psd")).unwrap(),
            b"user project"
        );
        assert_eq!(std::fs::read(external).unwrap(), b"external project");
        assert_eq!(
            apps::installed(&paths, &app.name).unwrap().path,
            target.display().to_string()
        );
        portable::validate(&paths, &app.name, &target).unwrap();
        assert!(destination(&paths, &app, &parent).is_err());
        std::fs::remove_dir_all(&parent).unwrap();
        std::fs::remove_dir_all(&paths.root).unwrap();
    }
    #[test]
    fn cancellation_and_invalid_destination_leave_source_intact() {
        let (paths, app, job) = fixture();
        assert!(destination(&paths, &app, Path::new(&app.path)).is_err());
        job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        let parent = paths
            .root
            .parent()
            .unwrap()
            .join(format!("craft-cancelled-move-{}", uuid::Uuid::new_v4()));
        assert!(move_app(&paths, &app, &parent, &job).is_err());
        assert!(Path::new(&app.path).is_dir());
        assert!(!parent.exists());
        std::fs::remove_dir_all(&paths.root).unwrap();
    }
}
