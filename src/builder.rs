use crate::{
    files,
    jobs::Job,
    model::{BuildInfo, Paths, Source},
    platform, tools, updates,
};
use anyhow::{bail, Context, Result};
use std::{collections::BTreeMap, fs, path::PathBuf, process::Command};
pub fn build(paths: &Paths, app: &str, latest: bool, job: &Job) -> Result<()> {
    crate::model::valid_app(app)?;
    let _lock = platform::Lock::take("Local\\CraftAppsSourceBuilder")?;
    job.log(&format!(
        "\nCraft Apps Builder — {app} — {}",
        chrono::Utc::now().to_rfc3339()
    ));
    job.stage("Checking tools", None, "Checking prerequisites");
    tools::preflight(paths, app)?;
    if latest {
        updates::sources(paths, &[app.into()], job)?;
    }
    let index: BTreeMap<String, Source> = files::read_json(&paths.at("sources/source-index.json"))
        .context("No managed source ZIP. Use Update source files first.")?;
    let commit = index.get(app).context("No managed source for this app")?;
    let archive = paths.at(format!("sources/{app}-source.zip"));
    if !files::hash(&archive)?.eq_ignore_ascii_case(&commit.archive_sha256) {
        bail!("Source checksum differs. Update source files first.");
    }
    files::verify_source(&archive, app, &commit.sha)?;
    let app_workspace = paths.at(format!("workspace/{app}"));
    let mut work = app_workspace.join(&commit.sha[..12]);
    if work.exists() && !work.join(".extracted").exists() {
        work = app_workspace.join(format!(
            "{}-{}",
            &commit.sha[..12],
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        ));
    }
    let project = work.join("src");
    if !work.join(".extracted").exists() {
        files::extract_zip(&archive, &work, job)?;
        let renamed_root = work.join(format!("{}-{}", crate::model::repository(app), commit.sha));
        let source_root = if renamed_root.exists() {
            renamed_root
        } else {
            work.join(format!("{app}-{}", commit.sha))
        };
        fs::rename(source_root, &project)?;
        fs::write(work.join(".extracted"), &commit.sha)?;
    } else {
        if fs::read_to_string(work.join(".extracted"))?.trim() != commit.sha {
            bail!("Workspace source commit differs")
        };
        job.log("Reusing the extracted source workspace.");
    }
    let target = paths.at(format!("workspace/cache/{app}"));
    let mut env = tools::environment(paths)?;
    env.insert("CARGO_TARGET_DIR".into(), target.display().to_string());
    env.insert("CARGO_BUILD_JOBS".into(), "4".into());
    if app == "artcraftx" {
        env.insert("VITE_ENVIRONMENT_TYPE".into(), "production".into());
        env.insert("SQLX_OFFLINE".into(), "true".into());
        env.insert("NX_DAEMON".into(), "false".into());
        let node = tools::find(paths, "node", "node.exe").unwrap();
        #[cfg(target_os = "windows")]
        let npm = node
            .parent()
            .unwrap()
            .join("node_modules/npm/bin/npm-cli.js");
        #[cfg(unix)]
        let npm = std::fs::canonicalize(tools::system("npm").context("npm is missing")?)?;
        let frontend = project.join("frontend");
        job.stage(
            "Frontend dependencies",
            None,
            "Installing original locked dependencies",
        );
        job.run(
            Command::new(&node)
                .arg(npm)
                .args(["ci", "--no-audit", "--no-fund"])
                .current_dir(&frontend)
                .envs(&env),
            false,
        )?;
        job.stage("Frontend build", None, "Building the ArtCraft X interface");
        job.run(
            Command::new(node)
                .args(["node_modules/nx/bin/nx.js", "run", "artcraft:build"])
                .current_dir(&frontend)
                .envs(&env),
            false,
        )?;
        if !frontend.join("apps/artcraft/dist/index.html").exists() {
            bail!("Frontend output is missing")
        }
    }
    job.stage("Compiling", None, "Starting release compilation");
    job.log(&format!("Source commit: {}", commit.sha));
    let cargo = tools::cargo(paths).unwrap();
    let mut cmd = Command::new(cargo);
    cmd.args([
        "build",
        "--release",
        "--locked",
        "-p",
        app,
        "--bin",
        app,
        "--message-format=json-render-diagnostics",
    ]);
    if app == "artcraftx" {
        cmd.args(["--features", "tauri/custom-protocol"]);
    }
    cmd.current_dir(&project).envs(&env);
    job.run(&mut cmd, true)?;
    job.check()?;
    let executable = job
        .state
        .lock()
        .unwrap()
        .executable
        .clone()
        .context("Cargo did not report a finished executable")?;
    if executable.file_stem().is_none_or(|n| n != app) {
        bail!("Unexpected build executable")
    }
    let out = paths.at(format!(
        "builds/{app}/{}-{}-{}",
        &commit.sha[..7],
        chrono::Local::now().format("%Y%m%d-%H%M%S"),
        &uuid::Uuid::new_v4().simple().to_string()[..4]
    ));
    fs::create_dir_all(&out)?;
    job.stage("Packaging", None, "Saving the executable and runtime files");
    fs::copy(
        &executable,
        out.join(crate::model::build_executable_name(app)),
    )?;
    for folder in [executable.parent().unwrap(), project.as_path()] {
        for e in fs::read_dir(folder)? {
            let e = e?;
            let name = e.file_name().to_string_lossy().into_owned();
            if e.file_type()?.is_file()
                && (name.ends_with(".dll") || name.starts_with("LICENSE") || name == "README.md")
            {
                fs::copy(e.path(), out.join(name))?;
            }
        }
    }
    files::write_json(
        &out.join("build-info.json"),
        &BuildInfo {
            app: app.into(),
            commit: commit.sha.clone(),
            source_branch: commit.branch.clone(),
            built_at: chrono::Utc::now().to_rfc3339(),
            profile: "release".into(),
            log: job.log_path.display().to_string(),
        },
    )?;
    job.state.lock().unwrap().output = Some(out.clone());
    job.log(&format!("Build complete: {}", out.display()));
    let prefs = paths.builder_preferences()?;
    job.stage(
        "Cleaning up",
        None,
        "Removing successful build intermediates",
    );
    for (enabled, path, root) in [
        (
            prefs.delete_cache_after_success,
            target,
            paths.at("workspace/cache"),
        ),
        (
            prefs.delete_workspace_after_success,
            app_workspace,
            paths.at("workspace"),
        ),
    ] {
        if enabled {
            if let Err(e) = files::remove_managed(&path, &root) {
                job.log(&format!("Cleanup warning: {e:#}"));
            }
        }
    }
    Ok(())
}
pub fn launch_options(paths: &Paths, app: &str) -> Result<crate::apps::LaunchSettings> {
    crate::model::valid_app(app)?;
    let all: BTreeMap<String, crate::apps::LaunchSettings> =
        files::read_or_default(&paths.at("runtime/build-launch-settings.json"))?;
    Ok(all.get(app).cloned().unwrap_or_default())
}
pub fn save_launch_options(
    paths: &Paths,
    app: &str,
    value: &crate::apps::LaunchSettings,
) -> Result<()> {
    crate::model::valid_app(app)?;
    if !value.executable.is_empty() {
        bail!("Local builds use their compiled executable.");
    }
    let file = paths.at("runtime/build-launch-settings.json");
    let mut all: BTreeMap<String, crate::apps::LaunchSettings> = files::read_or_default(&file)?;
    all.insert(app.to_owned(), value.clone());
    files::write_json(&file, &all)
}
/// Launch the newest completed build independently of installed releases.
pub fn launch_local(paths: &Paths, app: &str) -> Result<()> {
    crate::model::valid_app(app)?;
    let folder =
        history(paths, app).context("No completed local build was found. Build the app first.")?;
    let executable = folder.join(crate::model::build_executable_name(app));
    files::inside(&executable, &paths.at(format!("builds/{app}")))?;
    if !executable.is_file() {
        bail!("The build executable is missing. Rebuild the app first.");
    }
    let options = launch_options(paths, app)?;
    Command::new(&executable)
        .args(options.arguments)
        .current_dir(&folder)
        .spawn()
        .with_context(|| format!("Could not launch the local build of {app}"))?;
    Ok(())
}

/// Remove only the selected completed build, preserving other builds and releases.
pub fn delete_local(paths: &Paths, app: &str, folder: &std::path::Path) -> Result<()> {
    crate::model::valid_app(app)?;
    let _lock = platform::Lock::take("Local\\CraftAppsSourceBuilder")?;
    let root = paths.at(format!("builds/{app}"));
    files::inside(folder, &root)?;
    if folder.parent() != Some(root.as_path()) {
        bail!("Only a completed build folder can be deleted here.");
    }
    let info: BuildInfo = files::read_json(&folder.join("build-info.json"))?;
    if info.app != app {
        bail!("The build belongs to another app.");
    }
    if platform::running_app(app)? {
        bail!("Close the app before deleting its local build.");
    }
    files::remove_managed(folder, &root)
}
pub fn history(paths: &Paths, app: &str) -> Option<PathBuf> {
    let root = paths.at(format!("builds/{app}"));
    let mut builds: Vec<_> = fs::read_dir(root)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .join(crate::model::build_executable_name(app))
                .exists()
        })
        .filter_map(|e| {
            if files::linked(&e.path()).ok()? {
                return None;
            }
            let info: BuildInfo = files::read_json(&e.path().join("build-info.json")).ok()?;
            if info.app != app {
                return None;
            }
            let time = chrono::DateTime::parse_from_rfc3339(&info.built_at).ok()?;
            Some((time, e.path()))
        })
        .collect();
    builds.sort_by_key(|v| v.0);
    builds.pop().map(|v| v.1)
}
pub fn clean(paths: &Paths) -> Result<()> {
    let _lock = platform::Lock::take("Local\\CraftAppsSourceBuilder")?;
    let workspace = paths.at("workspace");
    for name in crate::model::sources()
        .into_iter()
        .chain(["cache".into(), "cache-previous".into()])
    {
        files::remove_managed(&workspace.join(name), &workspace)?;
    }
    Ok(())
}

#[cfg(test)]
mod local_build_tests {
    use super::*;

    #[test]
    fn build_launch_options_are_persisted_separately_from_release_options() {
        let temp =
            std::env::temp_dir().join(format!("craft-build-options-{}", uuid::Uuid::new_v4()));
        let paths = Paths::new(temp.clone(), None);
        let release = crate::apps::LaunchSettings {
            executable: String::new(),
            arguments: vec!["--release".into()],
        };
        crate::apps::save(&paths, "filmcraft", &release).unwrap();
        let build = crate::apps::LaunchSettings {
            executable: String::new(),
            arguments: vec!["--build".into(), "a value with spaces".into()],
        };
        save_launch_options(&paths, "filmcraft", &build).unwrap();
        assert_eq!(
            launch_options(&paths, "filmcraft").unwrap().arguments,
            build.arguments
        );
        assert_eq!(
            crate::apps::settings(&paths, "filmcraft")
                .unwrap()
                .arguments,
            release.arguments
        );
        assert!(launch_options(&paths, "soundcraft")
            .unwrap()
            .arguments
            .is_empty());
        let invalid = crate::apps::LaunchSettings {
            executable: "../other.exe".into(),
            arguments: vec![],
        };
        assert!(save_launch_options(&paths, "filmcraft", &invalid).is_err());
        fs::remove_dir_all(temp).unwrap();
    }
    #[test]
    fn local_build_actions_reject_missing_and_unrelated_outputs() {
        let temp =
            std::env::temp_dir().join(format!("craft-build-actions-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp).unwrap();
        let paths = Paths::new(temp.join("library"), None);
        let outside = temp.join("unrelated");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep.txt"), "keep").unwrap();
        assert!(launch_local(&paths, "filmcraft").is_err());
        assert!(delete_local(&paths, "filmcraft", &outside).is_err());
        let wrong = paths.at("builds/filmcraft/wrong");
        fs::create_dir_all(&wrong).unwrap();
        files::write_json(
            &wrong.join("build-info.json"),
            &BuildInfo {
                app: "soundcraft".into(),
                commit: "abcdef".into(),
                source_branch: "main".into(),
                built_at: chrono::Utc::now().to_rfc3339(),
                profile: "release".into(),
                log: String::new(),
            },
        )
        .unwrap();
        assert!(delete_local(&paths, "filmcraft", &wrong).is_err());
        assert!(wrong.exists());
        assert!(outside.join("keep.txt").exists());
        fs::remove_dir_all(temp).unwrap();
    }
}
