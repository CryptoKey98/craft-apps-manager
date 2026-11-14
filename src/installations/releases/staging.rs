use super::*;
use std::{collections::BTreeMap, sync::Mutex, thread};

pub(crate) struct StagedPackages<'a> {
    paths: &'a Paths,
    job: &'a Job,
    installers: Mutex<Vec<(String, PathBuf)>>,
    portable: BTreeMap<String, PortableDownload>,
    portable_cache: Option<PathBuf>,
    failed: BTreeMap<String, String>,
}

impl StagedPackages<'_> {
    pub(crate) fn portable_archive(&self, app: &str) -> Option<&Path> {
        self.portable
            .get(app)
            .map(|download| download.archive.as_path())
    }

    pub(crate) fn apply_failures(&self, plan: &mut ReleasePlan) {
        for entry in &mut plan.entries {
            if let Some(error) = self.failed.get(&entry.app) {
                entry.error = Some(error.clone());
            }
        }
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn preserve_for_worker(&mut self, plan: &ReleasePlan) {
        // A wait error can leave the elevated worker running. Only release
        // cleanup ownership for packages handed to that worker.
        self.installers
            .get_mut()
            .unwrap_or_else(|error| error.into_inner())
            .retain(|(app, _)| !plan.entries.iter().any(|entry| &entry.app == app));
    }
}

impl Drop for StagedPackages<'_> {
    fn drop(&mut self) {
        self.portable.clear();
        if let Some(cache) = &self.portable_cache {
            let _ = fs::remove_dir(cache);
        }
        let Ok(_lock) = platform::Lock::take(r"Local\CraftAppsManager") else {
            return;
        };
        let ids = self.job.child_ids();
        for (app, path) in self
            .installers
            .get_mut()
            .unwrap_or_else(|error| error.into_inner())
        {
            // An installer that started may still need its setup file for recovery.
            if ids
                .get(app)
                .and_then(|id| self.job.operations.read(id).ok())
                .is_some_and(|record| record.started.is_none())
            {
                let _ =
                    crate::installations::rollbacks::native::discard_new_download(self.paths, path);
            }
        }
    }
}

pub(super) fn stage_packages<'a>(
    paths: &'a Paths,
    plan: &ReleasePlan,
    entries: &[&ReleaseEntry],
    job: &'a Job,
) -> Result<StagedPackages<'a>> {
    let network = Network::new(&paths.root)?;
    stage_packages_with(paths, plan, entries, job, &|asset, destination, child| {
        network.asset(asset, destination, child)
    })
}

pub(super) fn stage_packages_with<'a>(
    paths: &'a Paths,
    plan: &ReleasePlan,
    entries: &[&ReleaseEntry],
    job: &'a Job,
    fetch: &(impl Fn(&Asset, &Path, &Job) -> Result<()> + Sync),
) -> Result<StagedPackages<'a>> {
    let mut staged = StagedPackages {
        paths,
        job,
        installers: Default::default(),
        portable: BTreeMap::new(),
        portable_cache: None,
        failed: BTreeMap::new(),
    };
    let lock = platform::Lock::take(r"Local\CraftAppsManager")?;
    job.check()?;
    let cache = paths.at("runtime/downloads");
    if entries
        .iter()
        .any(|entry| entry.preferences.release_format != "installer")
    {
        files::inside(&cache, &paths.root)?;
        fs::create_dir_all(&cache)?;
        staged.portable_cache = Some(cache);
    }
    for pair in entries.chunks(2) {
        job.check()?;
        let outcomes = thread::scope(|scope| {
            let installers = &staged.installers;
            let workers: Vec<_> = pair
                .iter()
                .map(|&entry| {
                    let child = job.prepare_child(
                        &entry.app,
                        &entry.action,
                        &entry.preferences.release_format,
                        &entry.preferences.architecture,
                    );
                    (
                        entry,
                        scope.spawn(move || {
                            child.prepare_package(|child| {
                                let result =
                                    stage_one(paths, plan, entry, child, installers, fetch);
                                match &result {
                                    Ok(_) => {
                                        child.stage("Waiting to install", None, "Package verified")
                                    }
                                    Err(error) if !crate::jobs::is_cancelled(error) => {
                                        child.stage("Download failed", None, format!("{error:#}"));
                                    }
                                    Err(_) => {}
                                }
                                result
                            })
                        }),
                    )
                })
                .collect();
            workers
                .into_iter()
                .map(|(entry, worker)| {
                    (
                        entry,
                        worker
                            .join()
                            .map_err(|_| anyhow::anyhow!("Package preparation worker stopped"))
                            .and_then(|result| result),
                    )
                })
                .collect::<Vec<_>>()
        });
        for (entry, outcome) in outcomes {
            match outcome {
                Ok(Some(download)) => {
                    staged.portable.insert(entry.app.clone(), download);
                }
                Ok(None) => {}
                Err(error) if crate::jobs::is_cancelled(&error) => return Err(error),
                Err(error) => {
                    staged
                        .failed
                        .insert(entry.app.clone(), format!("{error:#}"));
                }
            }
        }
    }
    drop(lock);
    job.check()?;
    if !staged.failed.is_empty() {
        let failures = staged
            .failed
            .iter()
            .map(|(app, error)| format!("{}: {error}", crate::model::title(app)))
            .collect::<Vec<_>>();
        if staged.failed.len() == entries.len() || !job.identity_choices_available() {
            bail!("Could not prepare packages: {}", failures.join("; "));
        }
        job.confirm_partial_batch(failures)?;
        job.check()?;
    }
    Ok(staged)
}

fn stage_one(
    paths: &Paths,
    plan: &ReleasePlan,
    entry: &ReleaseEntry,
    job: &Job,
    installers: &Mutex<Vec<(String, PathBuf)>>,
    fetch: &impl Fn(&Asset, &Path, &Job) -> Result<()>,
) -> Result<Option<PortableDownload>> {
    job.check()?;
    let asset = entry.asset.as_ref().context("Planned asset missing")?;
    if entry.preferences.release_format == "installer" {
        let destination = planned_installer_path(paths, plan, &entry.app, asset)?;
        files::inside(&destination, &paths.root)?;
        if destination.exists() {
            job.stage("Verifying checksum", None, "Checking the cached installer");
            crate::network::verify_asset(&destination, asset)?;
        } else {
            installers
                .lock()
                .unwrap()
                .push((entry.app.clone(), destination.clone()));
            fetch(asset, &destination, job)?;
        }
        return Ok(None);
    }
    files::safe_relative(&asset.name)?;
    anyhow::ensure!(
        Path::new(&asset.name)
            .file_name()
            .and_then(|name| name.to_str())
            == Some(&asset.name),
        "Invalid download filename"
    );
    let cache = paths.at("runtime/downloads");
    let directory = cache.join(uuid::Uuid::new_v4().simple().to_string());
    files::inside(&directory, &paths.root)?;
    fs::create_dir(&directory)?;
    let download = PortableDownload {
        archive: directory.join(&asset.name),
        directory,
    };
    fetch(asset, &download.archive, job)?;
    Ok(Some(download))
}

struct PortableDownload {
    directory: PathBuf,
    archive: PathBuf,
}

impl Drop for PortableDownload {
    fn drop(&mut self) {
        if let Some(parent) = self.directory.parent() {
            let _ = files::remove_managed(&self.directory, parent);
        }
    }
}
