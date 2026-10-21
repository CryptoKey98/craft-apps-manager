use super::*;

pub fn releases(paths: &Paths, job: &Job, background: bool) -> Result<()> {
    if background {
        return crate::hourly::run(paths, job);
    }
    job.tracked("Bulk update", "Application library", |job| {
        releases_for(paths, job, None)
    })
}

pub fn install_app(paths: &Paths, app: &str, job: &Job) -> Result<()> {
    crate::model::valid_app(app)?;
    if !crate::model::apps().iter().any(|a| a == app) {
        bail!("This app has no managed release");
    }
    job.tracked("Install / update", app, |job| {
        releases_for(paths, job, Some(app))
    })
}

pub fn execute_plan(paths: &Paths, plan: &ReleasePlan, job: &Job) -> Result<()> {
    let resource = if plan.entries.len() == 1 {
        plan.entries[0].app.as_str()
    } else {
        "Application library"
    };
    job.tracked(
        if plan.installer_downgrade() {
            "Downgrade"
        } else if plan.entries.len() > 1 {
            "Bulk update"
        } else {
            "Install / update"
        },
        resource,
        |job| execute_plan_owned(paths, plan, job),
    )
}

pub(super) fn execute_plan_owned(paths: &Paths, plan: &ReleasePlan, job: &Job) -> Result<()> {
    for entry in &plan.entries {
        job.prepare_child(
            &entry.app,
            &entry.action,
            &entry.preferences.release_format,
            &entry.preferences.architecture,
        );
    }
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    if crate::batch_admin::eligible(plan) {
        return crate::batch_admin::execute(paths, plan, job);
    }
    execute_plan_direct(paths, plan, job)
}

pub(crate) fn execute_plan_direct(paths: &Paths, plan: &ReleasePlan, job: &Job) -> Result<()> {
    execute_validated(paths, plan, |config| {
        execute_entries(paths, plan, config, job, None, None)
    })
}

#[cfg(target_os = "windows")]
pub(crate) fn execute_approved_downgrade(
    paths: &Paths,
    plan: &ReleasePlan,
    job: &Job,
    recovery: &std::collections::BTreeMap<String, Asset>,
    release_packages: &mut dyn FnMut(&str),
) -> Result<()> {
    execute_validated(paths, plan, |config| {
        execute_entries(
            paths,
            plan,
            config,
            job,
            Some(recovery),
            Some(release_packages),
        )
    })
}

#[cfg(target_os = "windows")]
pub(crate) fn refresh_plan_snapshot(paths: &Paths, plan: &mut ReleasePlan) -> Result<()> {
    plan.snapshot = state_snapshot(paths, &paths.read_preferences()?, &paths.config()?)?;
    Ok(())
}

pub(super) fn execute_validated(
    paths: &Paths,
    plan: &ReleasePlan,
    execute: impl FnOnce(&mut crate::model::Config) -> Result<()>,
) -> Result<()> {
    let _lock = platform::Lock::take("Local\\CraftAppsManager")?;
    let prefs = paths.read_preferences()?;
    let mut config = paths.config()?;
    if state_snapshot(paths, &prefs, &config)? != plan.snapshot {
        bail!("Release preferences, installations, or destinations changed. Review a new plan before installing.");
    }
    if plan.additional_installation {
        anyhow::ensure!(
            plan.entries.len() == 1,
            "Additional installations must use a single install plan"
        );
        let entry = &plan.entries[0];
        if entry.preferences.release_format == "installer" {
            anyhow::ensure!(
                !config
                    .installations
                    .iter()
                    .any(|copy| copy.name == entry.app
                        && copy.install_kind == "installer"
                        && !copy.path.is_empty()),
                "An installer copy already exists"
            );
        } else {
            let target = entry
                .destination
                .as_ref()
                .context("Choose a separate installation folder")?;
            anyhow::ensure!(
                !target.exists(),
                "The new installation requires an unused folder"
            );
            anyhow::ensure!(
                !config.installations.iter().any(|copy| !copy.path.is_empty()
                    && (target.starts_with(&copy.path)
                        || Path::new(&copy.path).starts_with(target))),
                "Installation folders must not overlap"
            );
        }
        let selected = config
            .apps
            .iter_mut()
            .find(|copy| copy.name == entry.app)
            .context("App missing")?;
        *selected = crate::model::Installed {
            name: entry.app.clone(),
            ..Default::default()
        };
    }
    for entry in &plan.entries {
        if let Some(installed) = config.apps.iter().find(|app| app.name == entry.app) {
            anyhow::ensure!(
                !crate::model::installer_architecture_conflict(installed, &entry.preferences),
                "{}",
                crate::model::installer_architecture_conflict_message(
                    installed,
                    &entry.preferences
                )
            );
        }
        if let Some(target) = &entry.destination {
            crate::portable::validate_install(paths, &entry.app, target)?;
        }
    }
    #[cfg(target_os = "windows")]
    if let Some(target) = &plan.installer_destination {
        validate_installer_destination(paths, target)?;
    }
    execute(&mut config)
}

pub(super) fn execute_entries(
    paths: &Paths,
    plan: &ReleasePlan,
    config: &mut crate::model::Config,
    job: &Job,
    _approved_recovery: Option<&std::collections::BTreeMap<String, Asset>>,
    mut release_packages: Option<&mut dyn FnMut(&str)>,
) -> Result<()> {
    let network = Network::new(&paths.root)?;
    execute_with(plan, job, |entry, job| {
        let prefs = &entry.preferences;
        let i = config
            .apps
            .iter()
            .position(|a| a.name == entry.app)
            .context("Planned app missing")?;
        let app = config.apps[i].clone();
        let asset = entry.asset.as_ref().context("Planned asset missing")?;
        let result = (|| -> Result<bool> {
            if prefs.release_format == "installer" {
                if platform::running_app(&app.name)? {
                    bail!("Close {} before installing", app.name);
                }
                let dest = paths.at(format!("releases/installers/{}/{}", app.name, asset.name));
                if dest.exists() {
                    job.stage(
                        "Verifying checksum",
                        None,
                        "Checking SHA-256 of the cached installer",
                    );
                    crate::network::verify_asset(&dest, asset)?;
                } else {
                    network.asset(asset, &dest, job)?;
                }
                #[cfg(target_os = "windows")]
                crate::installers::prepare_custom_identity(&paths.root, &dest, &app.name, job)?;
                job.stage(
                    "Installing",
                    None,
                    if cfg!(target_os = "windows") {
                        "Complete the Windows installation; Windows may ask for administrator permission."
                    } else {
                        "Installing the app."
                    },
                );
                #[cfg(target_os = "windows")]
                let mut installed = if plan.requested_tag.is_some()
                    && app.install_kind == "installer"
                    && !app.path.is_empty()
                    && version(&entry.version)? < version(&app.version)?
                {
                    anyhow::ensure!(
                        dest.extension()
                            .is_some_and(|e| e.eq_ignore_ascii_case("msi")),
                        "This installer does not support a safe downgrade"
                    );
                    anyhow::ensure!(
                        crate::model::release_arch(&app.architecture)
                            == crate::model::release_arch(&prefs.architecture),
                        "Keep the installed architecture selected when downgrading"
                    );
                    let recovery = downgrade_recovery_path(paths, &app, job, _approved_recovery)?;
                    job.check()?;
                    let remove_step = job.prepare_child(
                        &format!("{} · Current version", app.name),
                        "Uninstall",
                        "installer",
                        &prefs.architecture,
                    );
                    let install_step = job.prepare_child(
                        &format!("{} · Older version", app.name),
                        "Install",
                        "installer",
                        &prefs.architecture,
                    );
                    replace_installer_transaction(
                        || {
                            job.stage(
                                "Downgrading",
                                None,
                                "Removing the current installation; user data is kept",
                            );
                            remove_step.tracked("Uninstall", &app.name, |step| {
                                step.stage(
                                    "Uninstalling",
                                    None,
                                    "Removing the current version; user data is kept",
                                );
                                crate::installers::uninstall(&app)
                            })
                        },
                        || {
                            install_step.tracked("Install", &app.name, |step| {
                                step.check()?;
                                job.stage(
                                    "Installing older version",
                                    None,
                                    "Running the selected older installer",
                                );
                                step.stage(
                                    "Installing",
                                    None,
                                    "Running the selected older installer",
                                );
                                let installed =
                                    crate::installers::run_replacement(&dest, &app, Some(step))?;
                                anyhow::ensure!(
                                    version(&installed.version)? == version(&entry.version)?,
                                    "Installer did not install the selected version"
                                );
                                Ok(installed)
                            })
                        },
                        || {
                            let restore_step = job.prepare_child(
                                &format!("{} · Recovery", app.name),
                                "Restore installation",
                                "installer",
                                &prefs.architecture,
                            );
                            restore_step.tracked("Restore installation", &app.name, |step| {
                            step.stage("Restoring previous version", None, "Restoring the verified previous installer");
                            job.stage("Restoring previous version", None, "The downgrade did not complete; restoring the verified previous installer");
                            if let Some(partial) = crate::installers::detect(&app.name)? {
                                if version(&partial.version)? == version(&app.version)? {
                                    return Ok(());
                                }
                                crate::installers::uninstall(&partial)?;
                            }
                            let restored =
                                crate::installers::run_replacement(&recovery, &app, None)?;
                            anyhow::ensure!(
                                version(&restored.version)? == version(&app.version)?,
                                "Recovery installer did not restore the previous version"
                            );
                            Ok(())
                            })
                        },
                    )?
                } else {
                    crate::installers::run_for_version(
                        &dest,
                        &app.name,
                        Some(job),
                        plan.installer_destination.as_deref(),
                        &entry.version,
                        &app,
                    )?
                };
                #[cfg(target_os = "macos")]
                let mut installed = crate::installers::run_with_job(&dest, &app.name, Some(job))?;
                #[cfg(target_os = "linux")]
                let mut installed = crate::installers::run_selected_with_job(
                    &dest,
                    &app.name,
                    Some(job),
                    plan.requested_tag.is_some()
                        && !app.version.is_empty()
                        && version(&entry.version)? < version(&app.version)?,
                )?;
                #[cfg(target_os = "windows")]
                anyhow::ensure!(crate::model::installed_architecture_matches(&installed,prefs), "Installer registered a different architecture than the selected package. Check Windows Installed apps; another copy was not selected.");
                installed.architecture = if cfg!(target_os = "macos") {
                    "universal".into()
                } else {
                    prefs.architecture.clone()
                };
                config.apps[i] = installed;
                paths.save_config(config)?;
                if let Some(release) = release_packages.as_mut() {
                    release(&entry.app);
                }
                if let Err(error) =
                    crate::hourly::refresh_after_install(paths, &config.apps[i], prefs)
                {
                    job.log(&format!("Update availability refresh warning: {error:#}"));
                }
                job.report("Installation location", &config.apps[i].path);
                job.report("Version", &config.apps[i].version);
                job.report("Action", &entry.action);
                if plan.desktop_shortcut {
                    job.stage(
                        "Creating desktop shortcut",
                        None,
                        crate::model::title(&app.name),
                    );
                    if let Err(error) = crate::portable::installer_desktop_shortcut(
                        paths,
                        &app.name,
                        Path::new(&config.apps[i].path),
                    ) {
                        job.report(
                            "Desktop shortcut",
                            format!("Could not be created: {error:#}"),
                        );
                        job.log(&format!(
                            "{}: installed; desktop shortcut could not be created: {error:#}",
                            app.name
                        ));
                    } else {
                        job.report("Desktop shortcut", "Created successfully");
                    }
                } else {
                    job.report("Desktop shortcut", "Not requested from the manager");
                }
                platform::refresh_file_associations();
                job.log(&format!("{}: installer completed", app.name));
                crate::installer_cleanup::finish(paths, &config.apps[i], &dest, job);
                return Ok(true);
            }
            let v = entry.version.as_str();
            let target = entry
                .destination
                .clone()
                .context("Missing portable destination")?;
            if platform::running_app(&app.name)? {
                job.log(&format!("{}: app is open; skipped", app.name));
                job.report(
                    "Result",
                    "Skipped because the application was open. No installation changes were made.",
                );
                return Ok(false);
            }
            let cache = paths.at("runtime/downloads");
            let archive = cache.join(&asset.name);
            let stage = cache.join(uuid::Uuid::new_v4().simple().to_string());
            let attempt = (|| -> Result<()> {
                network.asset(asset, &archive, job)?;
                if cfg!(target_os = "macos") {
                    #[cfg(target_os = "macos")]
                    crate::installers::extract_app(&archive, &stage, &app.name, job)?;
                } else if cfg!(target_os = "linux") {
                    fs::create_dir_all(&stage)?;
                    let executable = stage.join(crate::model::executable_name(&app.name));
                    fs::copy(&archive, &executable)?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))?;
                    }
                } else {
                    files::extract_zip(&archive, &stage, job)?;
                }
                #[cfg_attr(not(target_os = "windows"), allow(unused_mut))]
                let mut executables: Vec<_> = walkdir::WalkDir::new(&stage)
                    .into_iter()
                    .collect::<std::result::Result<Vec<_>, _>>()?
                    .into_iter()
                    .filter(|e| {
                        !e.path_is_symlink()
                            && crate::model::is_executable(e.path())
                            && crate::model::executable_names(&app.name)
                                .iter()
                                .any(|name| {
                                    if cfg!(target_os = "windows") {
                                        e.file_name().to_string_lossy().eq_ignore_ascii_case(name)
                                    } else {
                                        e.file_name().to_string_lossy() == *name
                                    }
                                })
                    })
                    .collect();
                #[cfg(target_os = "windows")]
                if crate::catalog::get(&app.name)
                    .is_some_and(|entry| entry.repository.contains('/'))
                {
                    let previous = crate::app_identity::portable(&target, &app.name)
                        .and_then(|path| path.strip_prefix(&target).ok().map(Path::to_path_buf));
                    let candidates: Vec<_> = walkdir::WalkDir::new(&stage)
                        .max_depth(8)
                        .into_iter()
                        .filter_map(std::result::Result::ok)
                        .filter(|entry| {
                            entry.file_type().is_file()
                                && !entry.path_is_symlink()
                                && entry
                                    .path()
                                    .extension()
                                    .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
                                && crate::downloads::verify_desktop_executable(
                                    entry.path(),
                                    &prefs.architecture,
                                )
                                .is_ok()
                        })
                        .collect();
                    let remembered = previous.and_then(|relative| {
                        candidates
                            .iter()
                            .position(|entry| entry.path() == stage.join(relative.clone()))
                    });
                    let index = if let Some(index) = remembered {
                        index
                    } else if executables.len() == 1 {
                        candidates
                            .iter()
                            .position(|entry| entry.path() == executables[0].path())
                            .context(
                                "The expected executable is not a compatible desktop application",
                            )?
                    } else {
                        job.choose_identity("Choose the program to launch from this package. Your choice will be remembered for this installation.", candidates.iter().map(|entry| entry.path().strip_prefix(&stage).unwrap().display().to_string()).collect())?
                    };
                    executables = vec![candidates[index].clone()];
                }
                if executables.len() != 1 {
                    bail!("Release must contain exactly one app executable");
                }
                #[cfg(target_os = "windows")]
                if crate::catalog::get(&app.name)
                    .is_some_and(|entry| entry.repository.contains('/'))
                {
                    crate::downloads::verify_desktop_executable(
                        executables[0].path(),
                        &prefs.architecture,
                    )?;
                }
                job.check()?;
                if platform::running_app(&app.name)? {
                    bail!("App opened during download; update deferred")
                }
                let backup = if target.exists() {
                    version(&app.version)?;
                    Some(backup_path(paths, &app.name, &app.version, false))
                } else {
                    None
                };
                let custom = crate::catalog::get(&app.name)
                    .is_some_and(|entry| entry.repository.contains('/'));
                let staged_folder = if custom && cfg!(target_os = "windows") {
                    stage.as_path()
                } else {
                    executables[0].path().parent().unwrap()
                };
                if custom && cfg!(target_os = "windows") {
                    crate::app_identity::remember_portable(
                        staged_folder,
                        &app.name,
                        executables[0].path(),
                    )?;
                }
                // PhotoCraft stores user data beside its portable executable.
                // Copy it before activation; rollback keeps the original intact.
                if app.name == "photocraft" && target.join("PhotoCraftData").is_dir() {
                    files::copy_directory_verified(
                        &target.join("PhotoCraftData"),
                        &staged_folder.join("PhotoCraftData"),
                    )?;
                }
                if plan.additional_installation {
                    crate::portable::mark_additional(paths, &app.name, staged_folder)?;
                } else {
                    crate::portable::mark(paths, &app.name, staged_folder)?;
                }
                let old = config.apps[i].clone();
                config.apps[i].path = target.to_string_lossy().into_owned();
                config.apps[i].version = v.into();
                config.apps[i].architecture = if cfg!(target_os = "macos") {
                    "universal".into()
                } else {
                    prefs.architecture.clone()
                };
                config.apps[i].install_kind = "portable".into();
                config.apps[i].package_type =
                    if asset.name.to_ascii_lowercase().ends_with(".appimage") {
                        "appimage"
                    } else {
                        ""
                    }
                    .into();
                config.apps[i].product_code.clear();
                if let Err(e) =
                    replace_transaction(staged_folder, &target, backup.as_deref(), || {
                        paths.save_config(config)
                    })
                {
                    config.apps[i] = old;
                    return Err(e);
                }
                if let Err(e) =
                    backups::finish(paths, prefs, backup.as_deref(), &app.name, false, job)
                {
                    job.log(&format!("Backup cleanup warning: {e:#}"))
                }
                if let Err(error) = crate::profiles::restore_portable(paths, &app.name, &target) {
                    job.log(&format!(
                        "Retained profile was kept for recovery: {error:#}"
                    ));
                }
                if plan.desktop_shortcut {
                    job.stage(
                        "Creating desktop shortcut",
                        None,
                        crate::model::title(&app.name),
                    );
                    if let Err(error) = crate::portable::desktop_shortcut(paths, &app.name, &target)
                    {
                        job.report(
                            "Desktop shortcut",
                            format!("Could not be created: {error:#}"),
                        );
                        job.log(&format!("Desktop shortcut warning: {error:#}"));
                    } else {
                        job.report("Desktop shortcut", "Created successfully");
                    }
                } else {
                    job.report("Desktop shortcut", "Not requested from the manager");
                }
                job.report("Installation location", &config.apps[i].path);
                job.report("Version", &config.apps[i].version);
                job.report("Action", &entry.action);
                platform::refresh_file_associations();
                if let Err(error) =
                    crate::hourly::refresh_after_install(paths, &config.apps[i], prefs)
                {
                    job.log(&format!("Update availability refresh warning: {error:#}"));
                }
                platform::refresh_file_associations();
                job.log(&format!("{}: updated to {v}", app.name));
                Ok(())
            })();
            for p in [&stage, &archive] {
                if p.exists() {
                    let _ = files::remove_managed(p, &cache);
                }
            }
            attempt.map(|()| true)
        })();
        if matches!(result, Ok(true)) && app.path.is_empty() {
            // Select only a newly installed app, preserving deliberate exclusions
            // for apps that were already installed and merely updated.
            if let Err(error) = include_new_install(paths, &app.name) {
                job.log(&format!("Update All selection warning: {error:#}"));
            }
        }
        result
    })
}

pub(super) fn include_new_install(paths: &Paths, app: &str) -> Result<()> {
    let mut preferences = paths.read_preferences()?;
    if let Some(installed) = paths
        .config()?
        .apps
        .iter()
        .find(|copy| copy.name == app && !copy.path.is_empty())
    {
        if let Some(options) = crate::install_options::InstallOptions::installed(installed) {
            preferences
                .app_install_options
                .entry(crate::install_options::identity(app))
                .or_insert(options);
        }
    }
    if !preferences.selected_apps.iter().any(|name| name == app) {
        preferences.selected_apps.push(app.into());
    }
    crate::files::write_json(&paths.at("manager-settings.json"), &preferences)?;
    Ok(())
}

#[derive(Debug, Default)]
pub struct ReleaseSummary {
    pub completed: usize,
    pub skipped: usize,
    pub failed: usize,
    pub remaining: usize,
    pub current: Option<String>,
}

impl std::fmt::Display for ReleaseSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} completed, {} skipped, {} failed, {} remaining{}",
            self.completed,
            self.skipped,
            self.failed,
            self.remaining,
            self.current
                .as_ref()
                .map(|app| format!("; interrupted: {app}"))
                .unwrap_or_default()
        )
    }
}

pub(super) fn execute_with(
    plan: &ReleasePlan,
    job: &Job,
    mut execute: impl FnMut(&ReleaseEntry, &Job) -> Result<bool>,
) -> Result<()> {
    let mut summary = ReleaseSummary {
        failed: plan.entries.iter().filter(|e| e.error.is_some()).count(),
        skipped: plan
            .entries
            .iter()
            .filter(|e| e.action == "Skip" && e.error.is_none())
            .count(),
        ..Default::default()
    };
    let executable: Vec<_> = plan
        .entries
        .iter()
        .filter(|e| (e.action == "Install" || e.action == "Update") && e.error.is_none())
        .collect();
    for entry in &plan.entries {
        let child = job.prepare_child(
            &entry.app,
            &entry.action,
            &entry.preferences.release_format,
            &entry.preferences.architecture,
        );
        if let Some(error) = &entry.error {
            let _ = child.tracked::<()>(&entry.action, &entry.app, |_| {
                Err(anyhow::anyhow!(error.clone()))
            });
        } else if entry.action == "Skip" {
            child.tracked("Check", &entry.app, |child| {
                child.report("Result", "No installation changes were needed.");
                Ok(())
            })?;
        }
    }
    let mut cancelled = false;
    let mut handled = 0;
    for entry in &executable {
        if job.check().is_err() {
            cancelled = true;
            break;
        }
        summary.current = Some(entry.app.clone());
        let child = job.prepare_child(
            &entry.app,
            &entry.action,
            &entry.preferences.release_format,
            &entry.preferences.architecture,
        );
        let result = child.tracked(&entry.action, &entry.app, |child| {
            let result = execute(entry, child);
            if matches!(result, Ok(false)) {
                child.report(
                    "Result",
                    "The operation was skipped. No package was installed.",
                );
            }
            result
        });
        match result {
            Ok(true) => summary.completed += 1,
            Ok(false) => summary.skipped += 1,
            Err(error) if crate::jobs::is_cancelled(&error) => {
                cancelled = true;
                break;
            }
            Err(error) => {
                summary.failed += 1;
                job.log(&format!("{}: {error:#}", entry.app));
                if error.to_string().contains("API limit") {
                    handled += 1;
                    summary.current = None;
                    break;
                }
            }
        }
        handled += 1;
        summary.current = None;
    }
    summary.remaining = executable.len().saturating_sub(handled);
    job.report("Batch results", summary.to_string());
    job.log(&summary.to_string());
    if summary.failed > 0 {
        bail!("Release operations: {summary}");
    }
    if cancelled {
        return Err(anyhow::Error::new(crate::jobs::Cancelled).context(summary));
    }
    Ok(())
}

pub(super) fn releases_for(paths: &Paths, job: &Job, selected: Option<&str>) -> Result<()> {
    // CLI retains its deliberate install/update scope; GUI uses plan + confirmation.
    let _lock = platform::Lock::take("Local\\CraftAppsManager")?;
    let network = Network::new(&paths.root)?;
    let plan = plan_with(paths, job, selected, |name| {
        network.json(&format!(
            "https://api.github.com/repos/{}/releases/latest",
            crate::model::repository(name)
        ))
    })?;
    let errors: Vec<_> = plan
        .entries
        .iter()
        .filter_map(|e| e.error.as_ref().map(|error| format!("{}: {error}", e.app)))
        .collect();
    for error in &errors {
        job.log(error);
    }
    let mut config = paths.config()?;
    let result = execute_entries(paths, &plan, &mut config, job, None, None);
    if !errors.is_empty() {
        bail!("{} release planning error(s); see the log", errors.len());
    }
    result
}
