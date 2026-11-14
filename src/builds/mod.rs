//! Source builds, archive management, toolchains and build preferences.
pub mod preferences;
pub mod types;

pub mod profiles;
pub mod sources;
pub mod toolchains;

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
        files::extract_source_zip(&archive, &project, job)?;
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
        env.insert(
            "TAURI_FRONTEND_PATH".into(),
            project.join("frontend").display().to_string(),
        );
        env.insert(
            "TAURI_APP_PATH".into(),
            project.join("crates/artcraftx").display().to_string(),
        );
        #[cfg(target_os = "windows")]
        let node = tools::find(paths, "node", "node.exe").unwrap();
        #[cfg(target_os = "windows")]
        let npm = node
            .parent()
            .unwrap()
            .join("node_modules/npm/bin/npm-cli.js");
        #[cfg(unix)]
        let (node, npm) = tools::node_npm(paths)?;
        let frontend = project.join("frontend");
        job.stage(
            "Frontend dependencies",
            None,
            "Installing original locked dependencies",
        );
        job.run(
            npm_install_command(&node, &npm)
                .args(["ci", "--no-audit", "--no-fund"])
                // Vite's TypeScript path resolver compares canonical source
                // paths with its working directory; Windows short aliases do
                // not match. Only the Rust compiler needs shortened paths.
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

fn npm_install_command(node: &std::path::Path, npm: &std::path::Path) -> Command {
    #[cfg(windows)]
    {
        let mut command = Command::new(node);
        command.arg(npm);
        command
    }
    #[cfg(unix)]
    {
        let _ = node;
        Command::new(npm)
    }
}

#[cfg(unix)]
#[test]
fn npm_install_supports_executable_shell_wrappers() {
    use std::os::unix::fs::PermissionsExt;
    let folder = std::env::temp_dir().join(format!("craft-npm-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&folder).unwrap();
    let npm = folder.join("npm");
    std::fs::write(&npm, "#!/bin/sh\nprintf 'npm-wrapper:%s' \"$1\"\n").unwrap();
    std::fs::set_permissions(&npm, std::fs::Permissions::from_mode(0o755)).unwrap();
    let output = npm_install_command(std::path::Path::new("must-not-run-node"), &npm)
        .arg("ci")
        .output()
        .unwrap();
    std::fs::remove_dir_all(folder).unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"npm-wrapper:ci");
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
    let config = crate::storage::config::ConfigStore::new(paths.root.clone()).load()?;
    let all = saved_build_options(&config)?;
    Ok(all.get(app).cloned().unwrap_or_default())
}

fn saved_build_options(
    config: &crate::storage::config::ConfigSnapshot,
) -> Result<BTreeMap<String, BuildLaunchOptions>> {
    crate::storage::config::document::section(&config.document, "build_launch")
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
    crate::storage::config::ConfigStore::new(paths.root.clone()).mutate_section::<BTreeMap<
        String,
        BuildLaunchOptions,
    >>("build_launch", |all| {
        all.insert(app.to_owned(), value.clone());
        Ok(())
    })?;
    Ok(())
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
    Ok(selected_build_info(paths, app, Some(&options)).map(|(path, _)| path))
}

/// Read build choices once and reuse the metadata already loaded while finding each build.
pub fn selected_builds(
    paths: &Paths,
    config: &crate::storage::config::ConfigSnapshot,
    names: &[String],
) -> Result<BTreeMap<String, (PathBuf, BuildInfo)>> {
    let all = saved_build_options(config)?;
    Ok(names
        .iter()
        .filter(|name| crate::model::valid_app(name).is_ok())
        .filter_map(|name| {
            selected_build_info(paths, name, all.get(name)).map(|build| (name.clone(), build))
        })
        .collect())
}

fn selected_build_info(
    paths: &Paths,
    app: &str,
    options: Option<&BuildLaunchOptions>,
) -> Option<(PathBuf, BuildInfo)> {
    let builds = completed_builds(paths, app);
    let selected = options
        .and_then(|options| options.selected_build.as_ref())
        .and_then(|selected| builds.iter().position(|(path, _)| path == selected))
        .unwrap_or_default();
    builds.into_iter().nth(selected)
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
#[path = "local_build_tests.rs"]
mod local_build_tests;

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
