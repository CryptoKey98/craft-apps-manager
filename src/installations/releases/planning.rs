use super::*;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ReleaseEntry {
    pub app: String,
    pub action: String,
    pub version: String,
    pub asset: Option<Asset>,
    pub destination: Option<PathBuf>,
    pub error: Option<String>,
    pub preferences: Preferences,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ReleasePlan {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) native_rollback: Option<String>,
    #[serde(default)]
    pub additional_installation: bool,
    #[serde(default)]
    pub(super) single_app: bool,
    pub entries: Vec<ReleaseEntry>,
    pub preferences: Preferences,
    pub requested_tag: Option<String>,
    pub(super) snapshot: Vec<u8>,
    pub desktop_shortcut: bool,
    pub installer_destination: Option<PathBuf>,
}

impl ReleasePlan {
    pub fn installer_downgrade(&self) -> bool {
        if self.requested_tag.is_none() || self.entries.len() != 1 {
            return false;
        }
        let entry = &self.entries[0];
        if entry.error.is_some()
            || entry.action != "Update"
            || entry.preferences.release_format != "installer"
        {
            return false;
        }
        let Ok((_, config, _)) =
            serde_json::from_slice::<(Preferences, crate::model::Config, serde_json::Value)>(
                &self.snapshot,
            )
        else {
            return false;
        };
        config.apps.iter().any(|app| {
            app.name == entry.app
                && app.install_kind == "installer"
                && !app.path.is_empty()
                && version(&entry.version)
                    .ok()
                    .zip(version(&app.version).ok())
                    .is_some_and(|(selected, installed)| selected < installed)
        })
    }
    pub fn executable_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.action == "Install" || e.action == "Update")
            .count()
    }
}

pub(crate) fn plan_native_rollback(
    paths: &Paths,
    installed: &crate::model::Installed,
    id: &str,
    version: &str,
    tag: &str,
    asset: Asset,
) -> Result<ReleasePlan> {
    let config = paths.config()?;
    let preferences = paths.read_preferences_locked()?;
    let options = crate::install_options::for_installed(&preferences, installed)?;
    Ok(ReleasePlan {
        native_rollback: Some(id.into()),
        additional_installation: false,
        single_app: false,
        entries: vec![ReleaseEntry {
            app: installed.name.clone(),
            action: "Update".into(),
            version: version.into(),
            asset: Some(asset),
            destination: None,
            error: None,
            preferences: options.clone(),
        }],
        snapshot: state_snapshot(paths, &preferences, &config)?,
        preferences: options,
        requested_tag: Some(tag.into()),
        desktop_shortcut: false,
        installer_destination: None,
    })
}

pub(super) fn state_snapshot(
    paths: &Paths,
    prefs: &Preferences,
    config: &crate::model::Config,
) -> Result<Vec<u8>> {
    let destinations: Vec<_> = config
        .apps
        .iter()
        .map(|a| {
            let target = if a.install_kind == "installer" && !a.path.is_empty() {
                PathBuf::from(&a.path)
            } else if a.path.is_empty() || a.install_kind == "installer" {
                paths.at(format!("releases/{}", a.name))
            } else {
                PathBuf::from(&a.path)
            };
            let metadata = fs::symlink_metadata(&target).ok();
            let executable = crate::model::installed_executable(&target, &a.name);
            let executable_state = executable.as_ref().map(|path| {
                let metadata = fs::symlink_metadata(path).ok();
                (
                    path.clone(),
                    metadata.map(|m| (m.len(), m.modified().ok())),
                    crate::platform::executable_version(path),
                )
            });
            let ownership = (a.install_kind == "portable" && !a.path.is_empty())
                .then(|| {
                    fs::symlink_metadata(target.join(crate::installations::portable::MARKER)).ok()
                })
                .flatten()
                .map(|metadata| {
                    (
                        metadata.len(),
                        metadata.modified().ok(),
                        metadata.file_type().is_symlink(),
                    )
                });
            (
                target,
                executable_state,
                metadata
                    .as_ref()
                    .map(|m| (m.len(), m.modified().ok(), m.file_type().is_symlink())),
                ownership,
            )
        })
        .collect();
    Ok(serde_json::to_vec(&(prefs, config, destinations))?)
}

pub fn plan_releases(paths: &Paths, job: &Job) -> Result<ReleasePlan> {
    let network = Network::new(&paths.root)?;
    plan_with(paths, job, None, |name| {
        network.release_json(&format!(
            "https://api.github.com/repos/{}/releases/latest",
            crate::model::repository(name)
        ))
    })
}

pub fn plan_app(paths: &Paths, job: &Job, app: &str) -> Result<ReleasePlan> {
    crate::model::valid_app(app)?;
    let network = Network::new(&paths.root)?;
    plan_with(paths, job, Some(app), |name| {
        network.release_json(&format!(
            "https://api.github.com/repos/{}/releases/latest",
            crate::model::repository(name)
        ))
    })
}

/// Interactive previews may reuse the window's observed inventory. Execution
/// still reloads inventory and validates the selected installation under its lock.
pub fn plan_install_preview(
    paths: &Paths,
    job: &Job,
    app: &str,
    tag: Option<&str>,
    additional: bool,
    config: crate::model::Config,
) -> Result<ReleasePlan> {
    let network = Network::new(&paths.root)?;
    plan_preview_with(paths, job, app, tag, additional, config, |app| {
        let mut url = reqwest::Url::parse(&format!(
            "https://api.github.com/repos/{}/releases/",
            crate::model::repository(app)
        ))?;
        if let Some(tag) = tag {
            url.path_segments_mut()
                .map_err(|_| anyhow::anyhow!("Invalid release endpoint"))?
                .pop_if_empty()
                .push("tags")
                .push(tag);
        } else {
            url.path_segments_mut()
                .map_err(|_| anyhow::anyhow!("Invalid release endpoint"))?
                .pop_if_empty()
                .push("latest");
        }
        network.release_json(url.as_str())
    })
}

pub(super) fn plan_preview_with(
    paths: &Paths,
    job: &Job,
    app: &str,
    tag: Option<&str>,
    additional: bool,
    mut config: crate::model::Config,
    mut fetch: impl FnMut(&str) -> Result<Release>,
) -> Result<ReleasePlan> {
    crate::model::valid_app(app)?;
    anyhow::ensure!(
        Path::new(&config.apps_root) == paths.root,
        "Preview inventory belongs to another library"
    );
    anyhow::ensure!(
        config.apps.iter().any(|entry| entry.name == app),
        "App missing from preview inventory"
    );
    if let Some(tag) = tag {
        release_version(tag)?;
    }
    let prefs = paths.read_preferences()?;
    // Empty observations carry preference-derived placeholder kinds, not
    // installation identities. Bring those up to date when options changed
    // since the window's inventory refresh; preserve every installed fact.
    for entry in config.apps.iter_mut().filter(|entry| {
        entry.path.is_empty() && entry.version.is_empty() && entry.product_code.is_empty()
    }) {
        entry.install_kind =
            if crate::install_options::preferred(&prefs, &entry.name).release_format == "installer"
            {
                "installer"
            } else {
                "portable"
            }
            .into();
    }
    let release = fetch(app)?;
    if let Some(tag) = tag {
        anyhow::ensure!(
            release.tag_name == tag,
            "The release tag changed; choose the version again"
        );
    }
    let mut plan = if additional {
        plan_additional_from_config(paths, job, app, prefs, config, |_| Ok(release.clone()))?
    } else if tag.is_some() {
        plan_selected_version_from_config(paths, job, app, release, prefs, config)?
    } else {
        plan_from_config(
            paths,
            job,
            Some(app),
            prefs,
            config,
            |_| Ok(release.clone()),
        )?
    };
    plan.requested_tag = tag.map(str::to_owned);
    Ok(plan)
}

/// Preview a new copy without changing the currently selected installation.
pub fn plan_additional(paths: &Paths, job: &Job, app: &str) -> Result<ReleasePlan> {
    let network = Network::new(&paths.root)?;
    plan_additional_with(paths, job, app, |name| {
        network.release_json(&format!(
            "https://api.github.com/repos/{}/releases/latest",
            crate::model::repository(name)
        ))
    })
}

/// Prepare a separate installation of an explicitly chosen release.
pub fn plan_additional_version(
    paths: &Paths,
    job: &Job,
    app: &str,
    tag: &str,
) -> Result<ReleasePlan> {
    crate::model::valid_app(app)?;
    release_version(tag)?;
    let mut url = reqwest::Url::parse(&format!(
        "https://api.github.com/repos/{}/releases/tags/",
        crate::model::repository(app)
    ))?;
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("Invalid release endpoint"))?
        .pop_if_empty()
        .push(tag);
    let release: Release = Network::new(&paths.root)?.release_json(url.as_str())?;
    anyhow::ensure!(
        release.tag_name == tag,
        "The release tag changed; choose the version again"
    );
    let mut plan = plan_additional_with(paths, job, app, |_| Ok(release.clone()))?;
    plan.requested_tag = Some(tag.into());
    Ok(plan)
}

pub(super) fn plan_additional_with(
    paths: &Paths,
    job: &Job,
    app: &str,
    fetch: impl FnMut(&str) -> Result<Release>,
) -> Result<ReleasePlan> {
    crate::model::valid_app(app)?;
    let prefs = paths.read_preferences()?;
    let config = paths.config()?;
    plan_additional_from_config(paths, job, app, prefs, config, fetch)
}

fn plan_additional_from_config(
    paths: &Paths,
    job: &Job,
    app: &str,
    prefs: Preferences,
    mut config: crate::model::Config,
    fetch: impl FnMut(&str) -> Result<Release>,
) -> Result<ReleasePlan> {
    let snapshot = state_snapshot(paths, &prefs, &config)?;
    let native_exists = config
        .installations
        .iter()
        .any(|copy| copy.name == app && copy.install_kind == "installer" && !copy.path.is_empty());
    let selected = config
        .apps
        .iter_mut()
        .find(|copy| copy.name == app)
        .context("App missing")?;
    *selected = crate::model::Installed {
        name: app.into(),
        ..Default::default()
    };
    let mut plan = plan_from_config(paths, job, Some(app), prefs, config, fetch)?;
    plan.snapshot = snapshot;
    plan.additional_installation = true;
    for entry in &mut plan.entries {
        if entry.preferences.release_format == "installer" && native_exists {
            entry.error = Some("An installer copy already exists. Select it to update it, or choose Portable for another installation.".into());
        }
        if entry.preferences.release_format == "portable" {
            entry.destination = Some(crate::portable::additional_destination(
                paths,
                &paths.at("releases"),
                app,
            )?);
        }
    }
    Ok(plan)
}

pub fn choose_destination(
    paths: &Paths,
    plan: &mut ReleasePlan,
    parent: &Path,
    shortcut: bool,
) -> Result<()> {
    anyhow::ensure!(
        plan.entries.len() == 1,
        "Choose a location for one app at a time"
    );
    anyhow::ensure!(
        plan.preferences.release_format == "portable",
        "Native installer locations are managed by the installer"
    );
    let entry = &mut plan.entries[0];
    let mut target = if plan.additional_installation {
        crate::portable::additional_destination(paths, parent, &entry.app)?
    } else {
        crate::portable::destination(parent, &entry.app)?
    };
    let current = paths
        .config()?
        .apps
        .into_iter()
        .find(|a| a.name == entry.app);
    if plan.additional_installation {
        anyhow::ensure!(
            !target.exists(),
            "The new installation requires an unused folder"
        );
    }
    if let Some(current) = current.filter(|a| {
        !plan.additional_installation && a.install_kind != "installer" && !a.path.is_empty()
    }) {
        let old = Path::new(&current.path);
        if old.parent() == Some(parent) {
            target = old.to_path_buf();
        }
        anyhow::ensure!(
            Path::new(&current.path) == target,
            "Uninstall this portable app before changing its location"
        );
    }
    crate::portable::validate_install(paths, &entry.app, &target)?;
    entry.destination = Some(target);
    plan.desktop_shortcut = shortcut;
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn choose_installer_destination(
    paths: &Paths,
    plan: &mut ReleasePlan,
    parent: &Path,
) -> Result<()> {
    anyhow::ensure!(
        plan.entries.len() == 1 && plan.preferences.release_format == "installer",
        "Choose a location for one installer at a time"
    );
    let entry = &plan.entries[0];
    anyhow::ensure!(
        crate::installers::custom_location_supported(&entry.app) && entry.action == "Install",
        "This installer keeps its own location"
    );
    let target = crate::portable::destination(parent, &entry.app)?;
    validate_installer_destination(paths, &target)?;
    plan.installer_destination = Some(target);
    Ok(())
}

#[cfg(target_os = "windows")]
pub(super) fn validate_installer_destination(paths: &Paths, target: &Path) -> Result<()> {
    crate::installers::validate_custom_location(target)?;
    // Resolve the existing ancestor: the new leaf need not exist yet.
    // Windows casing and short-path aliases must not bypass this boundary.
    let mut ancestor = target;
    let mut suffix = Vec::new();
    while !ancestor.exists() {
        suffix.push(
            ancestor
                .file_name()
                .context("Missing install folder name")?,
        );
        ancestor = ancestor.parent().context("Missing install parent")?;
    }
    let mut resolved = ancestor.canonicalize()?;
    for component in suffix.into_iter().rev() {
        resolved.push(component);
    }
    let root = paths.root.canonicalize()?;
    let resolved = PathBuf::from(resolved.to_string_lossy().to_lowercase());
    let root = PathBuf::from(root.to_string_lossy().to_lowercase());
    anyhow::ensure!(
        !resolved.starts_with(root),
        "Choose an installer location outside the manager's data and releases folders"
    );
    Ok(())
}

/// Pin a user-selected release, preserving the normal verification and review flow.
pub fn plan_version(paths: &Paths, job: &Job, app: &str, tag: &str) -> Result<ReleasePlan> {
    crate::model::valid_app(app)?;
    release_version(tag)?;
    let mut url = reqwest::Url::parse(&format!(
        "https://api.github.com/repos/{}/releases/tags/",
        crate::model::repository(app)
    ))?;
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("Invalid release endpoint"))?
        .pop_if_empty()
        .push(tag);
    let release: Release = Network::new(&paths.root)?.release_json(url.as_str())?;
    anyhow::ensure!(
        release.tag_name == tag,
        "The release tag changed; choose the version again"
    );
    plan_selected_version(paths, job, app, release)
}

pub(super) fn plan_selected_version(
    paths: &Paths,
    job: &Job,
    app: &str,
    release: Release,
) -> Result<ReleasePlan> {
    let prefs = paths.read_preferences()?;
    let config = paths.config()?;
    plan_selected_version_from_config(paths, job, app, release, prefs, config)
}

fn plan_selected_version_from_config(
    paths: &Paths,
    job: &Job,
    app: &str,
    release: Release,
    prefs: Preferences,
    config: crate::model::Config,
) -> Result<ReleasePlan> {
    let tag = release.tag_name.clone();
    let mut plan = plan_from_config(paths, job, Some(app), prefs, config.clone(), |_| {
        Ok(release.clone())
    })?;
    anyhow::ensure!(
        !plan.entries.is_empty(),
        "App is missing from the current catalog"
    );
    for entry in &mut plan.entries {
        // Only an explicit selection can downgrade. Update all still skips older versions.
        if entry.error.is_none() && entry.action == "Skip" {
            if let Some(installed) = config.apps.iter().find(|a| a.name == app) {
                if version(&installed.version)? > version(&entry.version)? {
                    entry.action = "Update".into();
                }
            }
        }
    }
    plan.requested_tag = Some(tag);
    Ok(plan)
}

pub(super) fn plan_with(
    paths: &Paths,
    job: &Job,
    selected: Option<&str>,
    fetch: impl FnMut(&str) -> Result<Release>,
) -> Result<ReleasePlan> {
    let prefs = paths.read_preferences()?;
    let config = paths.config()?;
    plan_from_config(paths, job, selected, prefs, config, fetch)
}

pub(super) fn plan_from_config(
    paths: &Paths,
    job: &Job,
    selected: Option<&str>,
    prefs: Preferences,
    config: crate::model::Config,
    mut fetch: impl FnMut(&str) -> Result<Release>,
) -> Result<ReleasePlan> {
    let snapshot = state_snapshot(paths, &prefs, &config)?;
    let mut entries = Vec::new();
    let mut targets: Vec<_> = config
        .apps
        .iter()
        .filter(|a| {
            selected.map(|name| a.name == name).unwrap_or_else(|| {
                prefs.selected_apps.contains(&a.name) && installed_for_updates(a, &prefs)
            })
        })
        .collect();
    if selected.is_none() {
        super::selection::sort_update_targets(&mut targets, &prefs.app_order, &prefs.hidden_apps);
    }
    for app in targets {
        if let Err(error) = job.check() {
            let previous: Vec<_> = entries
                .iter()
                .filter_map(|entry: &ReleaseEntry| entry.error.as_ref())
                .collect();
            if !previous.is_empty() {
                bail!(
                    "Planning failed before cancellation: {}",
                    previous
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join("; ")
                );
            }
            return Err(error);
        }
        job.stage("Planning releases", None, crate::model::title(&app.name));
        let mut entry = ReleaseEntry {
            app: app.name.clone(),
            action: "Error".into(),
            version: String::new(),
            asset: None,
            destination: None,
            error: None,
            preferences: prefs.clone(),
        };
        let resolved = if selected.is_some() {
            if !prefs
                .app_install_options
                .contains_key(&crate::install_options::identity(&app.name))
                && !app.path.is_empty()
            {
                crate::install_options::for_installed(&prefs, app)
            } else {
                Ok(crate::install_options::preferred(&prefs, &app.name))
            }
        } else {
            crate::install_options::for_installed(&prefs, app)
        };
        let result = (|| -> Result<()> {
            let mut prefs = resolved?;
            entry.preferences = prefs.clone();
            if !app.path.is_empty()
                && crate::model::installed_executable(Path::new(&app.path), &app.name).is_some()
            {
                let actual = crate::install_options::for_installed(&prefs, app)?;
                anyhow::ensure!(actual.release_format == prefs.release_format && crate::model::release_arch(&actual.architecture) == crate::model::release_arch(&prefs.architecture), "Uninstall the current installation before changing its format or architecture. Its files and settings are preserved by the existing uninstall workflow.");
            }

            anyhow::ensure!(
                !crate::model::installer_architecture_conflict(app, &prefs),
                "{}",
                crate::model::installer_architecture_conflict_message(app, &prefs)
            );
            let release = fetch(&app.name)?;
            if release.draft || release.prerelease {
                bail!("Not a stable release");
            }
            if selected.is_some()
                && app.path.is_empty()
                && !prefs
                    .app_install_options
                    .contains_key(&crate::install_options::identity(&app.name))
            {
                let preferred = crate::install_options::InstallOptions {
                    release_format: prefs.release_format.clone(),
                    architecture: prefs.architecture.clone(),
                };
                if let Some(option) = crate::install_options::preserve_or_default(
                    &crate::install_options::available(&release, &app.name, &prefs),
                    &preferred,
                ) {
                    prefs = crate::install_options::apply(&prefs, &option);
                    entry.preferences = prefs.clone();
                }
            }
            let asset = select_asset(&release, &app.name, &prefs)?;
            // Remember a new file name so extraction and detection find the program.
            if let Some(parsed) = crate::catalog::parse_asset(&asset.name) {
                crate::catalog::learn_alias(&paths.root, &app.name, &parsed.prefix)?;
            }
            if !expected_asset_url(
                &asset.browser_download_url,
                &crate::model::repository(&app.name),
            ) {
                bail!("Unexpected asset URL");
            }
            files::safe_relative(&asset.name)?;
            let digest = asset
                .digest
                .as_deref()
                .and_then(|digest| digest.strip_prefix("sha256:"))
                .context("Release asset has no published SHA-256 digest")?;
            if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                bail!("Release asset has an invalid SHA-256 digest");
            }

            entry.version = release_version(&release.tag_name)?;
            entry.asset = Some(asset.clone());
            let installer = prefs.release_format == "installer";
            let target = if installer {
                PathBuf::from(&app.path)
            } else if app.path.is_empty() || app.install_kind == "installer" {
                paths.at(format!("releases/{}", app.name))
            } else {
                PathBuf::from(&app.path)
            };
            if !installer {
                if target.exists() {
                    crate::portable::validate(paths, &app.name, &target)?;
                } else {
                    crate::portable::validate_install(paths, &app.name, &target)?;
                }
                entry.destination = Some(target.clone());
            }
            let exists = !app.path.is_empty()
                && (app.install_kind == "installer") == installer
                && crate::model::installed_architecture_matches(app, &prefs)
                && crate::model::installed_executable(&target, &app.name).is_some();
            entry.action = if exists
                && !app.version.is_empty()
                && version(&app.version)? >= version(&entry.version)?
                && crate::model::installed_architecture_matches(app, &prefs)
            {
                "Skip"
            } else if exists {
                "Update"
            } else {
                "Install"
            }
            .into();
            Ok(())
        })();
        if let Err(error) = result {
            if crate::jobs::is_cancelled(&error) {
                let previous: Vec<_> = entries
                    .iter()
                    .filter_map(|entry: &ReleaseEntry| entry.error.as_deref())
                    .collect();
                if !previous.is_empty() {
                    bail!(
                        "Planning failed before cancellation: {}",
                        previous.join("; ")
                    );
                }
                return Err(error);
            }
            entry.error = Some(format!("{error:#}"));
        }
        entries.push(entry);
    }
    if let Err(error) = job.check() {
        let previous: Vec<_> = entries
            .iter()
            .filter_map(|entry| entry.error.as_deref())
            .collect();
        if !previous.is_empty() {
            bail!(
                "Planning failed before cancellation: {}",
                previous.join("; ")
            );
        }
        return Err(error);
    }
    Ok(ReleasePlan {
        native_rollback: None,
        additional_installation: false,
        single_app: selected.is_some(),
        requested_tag: None,
        desktop_shortcut: false,
        installer_destination: None,
        preferences: entries
            .first()
            .map(|entry| entry.preferences.clone())
            .unwrap_or_else(|| prefs.clone()),
        entries,
        snapshot,
    })
}
