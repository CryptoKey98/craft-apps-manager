use crate::{
    files,
    jobs::Job,
    model::{BuildInfo, Paths, Source},
    platform, tools, updates,
};
use anyhow::{bail, Context, Result};
use std::{collections::BTreeMap, fs, path::PathBuf, process::Command};
pub fn build(paths: &Paths, app: &str, latest: bool, job: &Job) -> Result<()> {
    build_to(paths, app, latest, None, false, job)
}

pub fn output_root(paths: &Paths, app: &str) -> PathBuf {
    files::read_json::<PathBuf>(&paths.at(format!("runtime/build-locations/{app}.json")))
        .ok()
        .and_then(|p| crate::portable::destination(&p, app).ok())
        .unwrap_or_else(|| paths.at(format!("builds/{app}")))
}

pub fn build_to(
    paths: &Paths,
    app: &str,
    latest: bool,
    parent: Option<&std::path::Path>,
    shortcut: bool,
    job: &Job,
) -> Result<()> {
    crate::model::valid_app(app)?;
    let root = match parent {
        Some(parent) => crate::portable::destination(parent, app)?,
        None => output_root(paths, app),
    };
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
    tools::configure_build_paths(paths, &project, &mut env)?;
    fs::create_dir_all(&target)?;
    env.insert(
        "CARGO_TARGET_DIR".into(),
        tools::build_path(&target)?.display().to_string(),
    );
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
                .current_dir(tools::build_path(&frontend)?)
                .envs(&env),
            false,
        )?;
        job.stage("Frontend build", None, "Building the ArtCraft X interface");
        job.run(
            Command::new(node)
                .args(["node_modules/nx/bin/nx.js", "run", "artcraft:build"])
                .current_dir(tools::build_path(&frontend)?)
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
    let mut cmd = Command::new(tools::build_path(&cargo)?);
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
    cmd.current_dir(tools::build_path(&project)?).envs(&env);
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
    let out = root.join(format!(
        "{}-{}-{}",
        &commit.sha[..7],
        chrono::Local::now().format("%Y%m%d-%H%M%S"),
        &uuid::Uuid::new_v4().simple().to_string()[..4]
    ));
    files::inside(&out, &root)?;
    fs::create_dir_all(&root)?;
    fs::create_dir(&out)?;
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
    files::write_json(
        &paths.at(format!("runtime/build-locations/{app}.json")),
        &root.parent().context("Missing build parent")?.to_path_buf(),
    )?;
    if shortcut {
        if let Err(error) = crate::portable::build_desktop_shortcut(paths, app, &out) {
            job.log(&format!("Shortcut warning: {error:#}"));
        }
    }
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
    files::inside(&executable, &output_root(paths, app))?;
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
    let root = output_root(paths, app);
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
/// Clean intermediates for the selected build, never its compiled output.
/// The compiler cache is shared only within this app.
pub fn clean_local(paths: &Paths, app: &str, folder: &std::path::Path) -> Result<()> {
    crate::model::valid_app(app)?;
    let _lock = platform::Lock::take(r"Local\CraftAppsSourceBuilder")?;
    let targets = local_cache_paths(paths, app, folder)?;
    // Validate every boundary before deleting anything.
    for (path, root) in &targets {
        files::inside(path, root)?;
    }
    for (path, root) in targets {
        files::remove_managed(&path, &root)?;
    }
    Ok(())
}
pub fn has_local_cache(paths: &Paths, app: &str, folder: &std::path::Path) -> Result<bool> {
    Ok(local_cache_paths(paths, app, folder)?
        .iter()
        .any(|(path, _)| path.exists()))
}
fn local_cache_paths(
    paths: &Paths,
    app: &str,
    folder: &std::path::Path,
) -> Result<Vec<(PathBuf, PathBuf)>> {
    crate::model::valid_app(app)?;
    let root = output_root(paths, app);
    files::inside(folder, &root)?;
    anyhow::ensure!(
        folder.parent() == Some(root.as_path()),
        "Select a completed build folder"
    );
    let info: BuildInfo = files::read_json(&folder.join("build-info.json"))?;
    anyhow::ensure!(info.app == app, "The build belongs to another app");
    anyhow::ensure!(
        info.commit.len() == 40 && info.commit.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid build commit"
    );
    let work_root = paths.at(format!("workspace/{app}"));
    let mut targets = vec![(
        paths.at(format!("workspace/cache/{app}")),
        paths.at("workspace/cache"),
    )];
    if work_root.exists() {
        files::inside(&work_root, &paths.at("workspace"))?;
        for entry in fs::read_dir(&work_root)? {
            let folder = entry?.path();
            files::inside(&folder, &work_root)?;
            if folder.is_dir() && folder.join(".extracted").is_file() {
                files::inside(&folder.join(".extracted"), &work_root)?;
                if fs::read_to_string(folder.join(".extracted"))?.trim() == info.commit {
                    targets.push((folder, work_root.clone()));
                }
            }
        }
    }
    for (path, root) in &targets {
        files::inside(path, root)?;
    }
    Ok(targets)
}
pub fn history(paths: &Paths, app: &str) -> Option<PathBuf> {
    let root = output_root(paths, app);
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
    fn selected_build_cleanup_keeps_outputs_sources_and_other_workspaces() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let paths = Paths::new(root.clone(), None);
        let commit = "a".repeat(40);
        let folder = paths.at("builds/filmcraft/selected");
        files::write_json(
            &folder.join("build-info.json"),
            &BuildInfo {
                app: "filmcraft".into(),
                commit: commit.clone(),
                source_branch: "main".into(),
                built_at: chrono::Utc::now().to_rfc3339(),
                profile: "release".into(),
                log: String::new(),
            },
        )
        .unwrap();
        fs::write(
            folder.join(crate::model::build_executable_name("filmcraft")),
            b"compiled",
        )
        .unwrap();
        for name in [
            "workspace/filmcraft/selected",
            "workspace/filmcraft/other",
            "workspace/cache/filmcraft",
            "workspace/cache/soundcraft",
            "sources",
            "builds/filmcraft/other",
        ] {
            fs::create_dir_all(paths.at(name)).unwrap();
            fs::write(paths.at(format!("{name}/keep.txt")), b"data").unwrap();
        }
        fs::write(paths.at("workspace/filmcraft/selected/.extracted"), &commit).unwrap();
        fs::write(
            paths.at("workspace/filmcraft/other/.extracted"),
            "b".repeat(40),
        )
        .unwrap();
        assert!(clean_local(&paths, "soundcraft", &folder).is_err());
        assert!(paths.at("workspace/cache/filmcraft").exists());
        assert!(has_local_cache(&paths, "filmcraft", &folder).unwrap());
        clean_local(&paths, "filmcraft", &folder).unwrap();
        assert!(!has_local_cache(&paths, "filmcraft", &folder).unwrap());
        assert!(!paths.at("workspace/cache/filmcraft").exists());
        assert!(!paths.at("workspace/filmcraft/selected").exists());
        for name in [
            "workspace/filmcraft/other",
            "workspace/cache/soundcraft",
            "sources",
            "builds/filmcraft/other",
        ] {
            assert!(paths.at(format!("{name}/keep.txt")).is_file());
        }
        assert!(folder
            .join(crate::model::build_executable_name("filmcraft"))
            .is_file());
        clean_local(&paths, "filmcraft", &folder).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
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

#[cfg(test)]
mod destination_tests {
    use super::*;
    #[test]
    fn custom_builds_launch_from_record_and_delete_only_selected_build() {
        let temporary =
            std::env::temp_dir().join(format!("craft-build-location-{}", uuid::Uuid::new_v4()));
        let paths = Paths::new(temporary.join("library"), None);
        let parent = temporary.join("Custom Builds");
        let root = parent.join("photocraft");
        let folder = root.join("completed");
        fs::create_dir_all(&folder).unwrap();
        fs::write(root.join("keep.txt"), b"personal").unwrap();
        fs::write(
            folder.join(crate::model::build_executable_name("photocraft")),
            b"fixture",
        )
        .unwrap();
        files::write_json(
            &folder.join("build-info.json"),
            &BuildInfo {
                app: "photocraft".into(),
                commit: "a".repeat(40),
                source_branch: "main".into(),
                built_at: chrono::Utc::now().to_rfc3339(),
                profile: "release".into(),
                log: String::new(),
            },
        )
        .unwrap();
        files::write_json(
            &paths.at("runtime/build-locations/photocraft.json"),
            &parent,
        )
        .unwrap();
        assert_eq!(output_root(&paths, "photocraft"), root);
        assert_eq!(history(&paths, "photocraft"), Some(folder.clone()));
        assert!(delete_local(&paths, "photocraft", &root).is_err());
        assert!(delete_local(&paths, "filmcraft", &folder).is_err());
        delete_local(&paths, "photocraft", &folder).unwrap();
        assert_eq!(fs::read(root.join("keep.txt")).unwrap(), b"personal");
        fs::remove_dir_all(temporary).unwrap();
    }
}
