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
    job.tracked("Build", app, |job| {
        build_owned(paths, app, latest, parent, shortcut, job)
    })
}
fn build_owned(
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
    let cargo_target = cargo_target(app, commit);
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
        let folder = crate::model::repository_name(app);
        let renamed_root = work.join(format!("{}-{}", folder, commit.sha));
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
        cargo_target,
        "--bin",
        cargo_target,
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
    if executable.file_stem().is_none_or(|n| n != cargo_target) {
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
            job.report(
                "Desktop shortcut",
                format!("Could not be created: {error:#}"),
            );
            job.log(&format!("Shortcut warning: {error:#}"));
        } else {
            job.report("Desktop shortcut", "Created successfully");
        }
    } else {
        job.report("Desktop shortcut", "Not requested");
    }
    job.report("Build location", out.display().to_string());
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

/// PDFCraft retained its `printcraft` library key after the upstream package rename.
/// The source record identifies which Cargo package the downloaded archive contains.
fn cargo_target<'a>(app: &'a str, source: &Source) -> &'a str {
    if app == "printcraft" && source.repository == "storytold/pdfcraft" {
        "pdfcraft"
    } else {
        app
    }
}

#[cfg(test)]
#[test]
fn pdfcraft_build_target_follows_downloaded_source() {
    let mut source = Source {
        sha: String::new(),
        branch: String::new(),
        repository: "storytold/printcraft".into(),
        archive_sha256: String::new(),
        downloaded_at: String::new(),
    };
    assert_eq!(cargo_target("printcraft", &source), "printcraft");
    source.repository = "storytold/pdfcraft".into();
    assert_eq!(cargo_target("printcraft", &source), "pdfcraft");
    assert_eq!(cargo_target("filmcraft", &source), "filmcraft");
}
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildLaunchOptions {
    #[serde(flatten)]
    pub launch: crate::apps::LaunchSettings,
    #[serde(default)]
    pub selected_build: Option<PathBuf>,
}
pub fn build_options(paths: &Paths, app: &str) -> Result<BuildLaunchOptions> {
    crate::model::valid_app(app)?;
    let all: BTreeMap<String, BuildLaunchOptions> =
        files::read_or_default(&paths.at("runtime/build-launch-settings.json"))?;
    Ok(all.get(app).cloned().unwrap_or_default())
}
pub fn launch_options(paths: &Paths, app: &str) -> Result<crate::apps::LaunchSettings> {
    Ok(build_options(paths, app)?.launch)
}
pub fn save_build_options(paths: &Paths, app: &str, value: &BuildLaunchOptions) -> Result<()> {
    crate::model::valid_app(app)?;
    anyhow::ensure!(
        value.launch.executable.is_empty(),
        "Local builds use their compiled executable."
    );
    if let Some(folder) = &value.selected_build {
        anyhow::ensure!(
            completed_builds(paths, app)
                .iter()
                .any(|(path, _)| path == folder),
            "The selected completed build is no longer available."
        );
    }
    let file = paths.at("runtime/build-launch-settings.json");
    let mut all: BTreeMap<String, BuildLaunchOptions> = files::read_or_default(&file)?;
    all.insert(app.to_owned(), value.clone());
    files::write_json(&file, &all)
}
pub fn save_launch_options(
    paths: &Paths,
    app: &str,
    value: &crate::apps::LaunchSettings,
) -> Result<()> {
    let mut options = build_options(paths, app)?;
    options.launch = value.clone();
    save_build_options(paths, app, &options)
}
/// A removed selected build falls back to the newest valid completed build.
pub fn selected_build(paths: &Paths, app: &str) -> Result<Option<PathBuf>> {
    let options = build_options(paths, app)?;
    let builds = completed_builds(paths, app);
    Ok(options
        .selected_build
        .filter(|selected| builds.iter().any(|(path, _)| path == selected))
        .or_else(|| builds.first().map(|(path, _)| path.clone())))
}
/// Launch the chosen completed build independently of installed releases.
pub fn launch_local(paths: &Paths, app: &str) -> Result<()> {
    crate::model::valid_app(app)?;
    let folder = selected_build(paths, app)?
        .context("No completed local build was found. Build the app first.")?;
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
    crate::portable::remove_build_shortcut(paths, app, folder)?;
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
/// Interrupted builds have no completed build record to select.
pub fn has_app_cache(paths: &Paths, app: &str) -> Result<bool> {
    Ok(app_cache_paths(paths, app, None)?
        .iter()
        .any(|(path, _)| path.exists()))
}
pub fn clean_app_cache(paths: &Paths, app: &str) -> Result<()> {
    let _lock = platform::Lock::take(r"Local\CraftAppsSourceBuilder")?;
    let targets = app_cache_paths(paths, app, None)?;
    for (path, root) in targets {
        files::remove_managed(&path, &root)?;
    }
    Ok(())
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
    app_cache_paths(paths, app, Some(&info.commit))
}
fn app_cache_paths(
    paths: &Paths,
    app: &str,
    commit: Option<&str>,
) -> Result<Vec<(PathBuf, PathBuf)>> {
    crate::model::valid_app(app)?;
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
                let extracted = fs::read_to_string(folder.join(".extracted"))?;
                let extracted = extracted.trim();
                if extracted.len() == 40
                    && extracted.bytes().all(|b| b.is_ascii_hexdigit())
                    && commit.is_none_or(|commit| extracted == commit)
                {
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
/// Newest first; only direct, unlinked completed builds belonging to this app.
pub fn completed_builds(paths: &Paths, app: &str) -> Vec<(PathBuf, BuildInfo)> {
    let root = output_root(paths, app);
    let Ok(entries) = fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut builds: Vec<_> = entries
        .filter_map(|entry| {
            let folder = entry.ok()?.path();
            if files::no_links(&folder).is_err()
                || !folder.is_dir()
                || !folder
                    .join(crate::model::build_executable_name(app))
                    .is_file()
            {
                return None;
            }
            let info: BuildInfo = files::read_json(&folder.join("build-info.json")).ok()?;
            if info.app != app {
                return None;
            }
            let time = chrono::DateTime::parse_from_rfc3339(&info.built_at).ok()?;
            Some((time, folder, info))
        })
        .collect();
    builds.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    builds
        .into_iter()
        .map(|(_, folder, info)| (folder, info))
        .collect()
}
pub fn history(paths: &Paths, app: &str) -> Option<PathBuf> {
    completed_builds(paths, app)
        .into_iter()
        .next()
        .map(|(folder, _)| folder)
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
    fn build_selection_survives_restart_and_keeps_latest_independent() {
        let root =
            std::env::temp_dir().join(format!("craft-build-choice-{}", uuid::Uuid::new_v4()));
        let paths = Paths::new(root.clone(), None);
        let make = |name: &str, time: &str| {
            let folder = paths.at(format!("builds/filmcraft/{name}"));
            files::write_json(
                &folder.join("build-info.json"),
                &BuildInfo {
                    app: "filmcraft".into(),
                    commit: "a".repeat(40),
                    source_branch: "main".into(),
                    built_at: time.into(),
                    profile: "release".into(),
                    log: String::new(),
                },
            )
            .unwrap();
            fs::write(
                folder.join(crate::model::build_executable_name("filmcraft")),
                b"test executable",
            )
            .unwrap();
            folder
        };
        let older = make("older", "2026-10-10T12:00:00Z");
        let newer = make("newer", "2026-10-11T12:00:00Z");
        assert_eq!(
            selected_build(&paths, "filmcraft").unwrap(),
            Some(newer.clone())
        );
        save_build_options(
            &paths,
            "filmcraft",
            &BuildLaunchOptions {
                launch: crate::apps::LaunchSettings {
                    executable: String::new(),
                    arguments: vec!["--test".into()],
                },
                selected_build: Some(older.clone()),
            },
        )
        .unwrap();
        let restarted = Paths::new(root.clone(), None);
        assert_eq!(
            selected_build(&restarted, "filmcraft").unwrap(),
            Some(older.clone())
        );
        assert_eq!(history(&paths, "filmcraft"), Some(newer.clone()));
        let newest = make("newest", "2026-10-12T12:00:00Z");
        assert_eq!(
            selected_build(&paths, "filmcraft").unwrap(),
            Some(older.clone())
        );
        assert_eq!(completed_builds(&paths, "filmcraft").len(), 3);
        fs::remove_dir_all(&older).unwrap();
        assert_eq!(
            selected_build(&paths, "filmcraft").unwrap(),
            Some(newest.clone())
        );
        assert!(save_build_options(
            &paths,
            "filmcraft",
            &BuildLaunchOptions {
                selected_build: Some(root.join("outside")),
                ..Default::default()
            }
        )
        .is_err());
        save_build_options(&paths, "filmcraft", &BuildLaunchOptions::default()).unwrap();
        assert_eq!(selected_build(&paths, "filmcraft").unwrap(), Some(newest));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_launch_settings_load_without_a_build_selection() {
        let root =
            std::env::temp_dir().join(format!("craft-build-legacy-{}", uuid::Uuid::new_v4()));
        let paths = Paths::new(root.clone(), None);
        files::write_json(
            &paths.at("runtime/build-launch-settings.json"),
            &serde_json::json!({"filmcraft":{"executable":"","arguments":["--legacy"]}}),
        )
        .unwrap();
        let options = build_options(&paths, "filmcraft").unwrap();
        assert!(options.selected_build.is_none());
        assert_eq!(options.launch.arguments, ["--legacy"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cache_availability_follows_each_build_commit() {
        let root =
            std::env::temp_dir().join(format!("craft-cache-choice-{}", uuid::Uuid::new_v4()));
        let paths = Paths::new(root.clone(), None);
        let make = |name: &str, commit: String| {
            let folder = paths.at(format!("builds/filmcraft/{name}"));
            files::write_json(
                &folder.join("build-info.json"),
                &BuildInfo {
                    app: "filmcraft".into(),
                    commit,
                    source_branch: "main".into(),
                    built_at: chrono::Utc::now().to_rfc3339(),
                    profile: "release".into(),
                    log: String::new(),
                },
            )
            .unwrap();
            folder
        };
        let older = make("older", "a".repeat(40));
        let newer = make("newer", "b".repeat(40));
        let workspace = paths.at("workspace/filmcraft/older");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join(".extracted"), "a".repeat(40)).unwrap();
        assert!(has_local_cache(&paths, "filmcraft", &older).unwrap());
        assert!(!has_local_cache(&paths, "filmcraft", &newer).unwrap());
        fs::remove_dir_all(&workspace).unwrap();
        assert!(!has_local_cache(&paths, "filmcraft", &older).unwrap());
        fs::remove_dir_all(root).unwrap();
    }

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
    fn interrupted_build_cache_cleanup_is_locked_and_sources_can_be_extracted_again() {
        use std::io::Write;

        let root =
            std::env::temp_dir().join(format!("craft-interrupted-cache-{}", uuid::Uuid::new_v4()));
        let paths = Paths::new(root.clone(), None);
        let commit = "a".repeat(40);
        let work = paths.at(format!("workspace/filmcraft/{}", &commit[..12]));
        let cache = paths.at("workspace/cache/filmcraft");
        let archive = paths.at("sources/filmcraft-source.zip");
        fs::create_dir_all(archive.parent().unwrap()).unwrap();
        let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        zip.start_file(
            format!("filmcraft-{commit}/Cargo.toml"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(b"[package]\nname = 'filmcraft'\nversion = '0.1.0'\n")
            .unwrap();
        zip.finish().unwrap();
        let hash = files::hash(&archive).unwrap();
        let job = Job::new(
            paths.at("logs/test.log"),
            &crate::model::BuilderPreferences::default(),
        );
        let extract = || {
            files::verify_source(&archive, "filmcraft", &commit).unwrap();
            files::extract_zip(&archive, &work, &job).unwrap();
            fs::rename(work.join(format!("filmcraft-{commit}")), work.join("src")).unwrap();
            fs::write(work.join(".extracted"), &commit).unwrap();
            fs::create_dir_all(&cache).unwrap();
            fs::write(cache.join("intermediate"), b"partial compilation").unwrap();
        };
        extract();
        for name in [
            "workspace/cache/soundcraft",
            "workspace/filmcraft/user-files",
            "builds/filmcraft/completed",
        ] {
            fs::create_dir_all(paths.at(name)).unwrap();
            fs::write(paths.at(format!("{name}/keep.txt")), b"keep").unwrap();
        }
        // No completed build record is required to discover these intermediates.
        assert!(completed_builds(&paths, "filmcraft").is_empty());
        assert!(has_app_cache(&paths, "filmcraft").unwrap());
        {
            let _active_builder = platform::Lock::take(r"Local\CraftAppsSourceBuilder").unwrap();
            // Windows mutexes are reentrant on their owner thread: simulate a
            // separate cleanup worker, as used by both windows.
            let cleanup_paths = paths.clone();
            assert!(
                std::thread::spawn(move || clean_app_cache(&cleanup_paths, "filmcraft"))
                    .join()
                    .unwrap()
                    .is_err()
            );
            assert!(cache.join("intermediate").is_file());
            assert!(work.join("src/Cargo.toml").is_file());
        }
        clean_app_cache(&paths, "filmcraft").unwrap();
        assert!(!has_app_cache(&paths, "filmcraft").unwrap());
        assert!(!work.exists());
        assert_eq!(files::hash(&archive).unwrap(), hash);
        for name in [
            "workspace/cache/soundcraft",
            "workspace/filmcraft/user-files",
            "builds/filmcraft/completed",
        ] {
            assert!(paths.at(format!("{name}/keep.txt")).is_file());
        }
        // Exercise the same extraction and compiler-cache creation used by
        // build_owned; a fresh build can recreate both after cleanup.
        extract();
        assert!(work.join("src/Cargo.toml").is_file());
        assert!(cache.join("intermediate").is_file());
        clean_app_cache(&paths, "filmcraft").unwrap();
        clean_app_cache(&paths, "filmcraft").unwrap();
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
