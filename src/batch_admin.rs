//! A temporary Windows worker for the exact installer plan approved by the user.
use crate::{
    files,
    jobs::Job,
    model::{BuilderPreferences, Paths},
    network::Network,
    updates::{self, ReleasePlan},
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

#[derive(Serialize, Deserialize)]
struct Request {
    root: PathBuf,
    tools: PathBuf,
    profile: PathBuf,
    plan: ReleasePlan,
    build: BuilderPreferences,
    #[serde(default)]
    operation_parent: Option<String>,
    #[serde(default)]
    child_operations: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    recovery_packages: std::collections::BTreeMap<String, crate::model::Asset>,
}
#[derive(Serialize, Deserialize)]
struct Outcome {
    error: Option<String>,
    cancelled: bool,
}

pub fn eligible(plan: &ReleasePlan) -> bool {
    plan.preferences.batch_admin
        && ((plan.requested_tag.is_none()
            && plan
                .entries
                .iter()
                .filter(|entry| {
                    entry.preferences.release_format == "installer"
                        && matches!(entry.action.as_str(), "Install" | "Update")
                        && entry.error.is_none()
                })
                .count()
                > 1)
            || (plan.installer_downgrade()
                && plan.entries[0]
                    .asset
                    .as_ref()
                    .is_some_and(|asset| asset.name.to_ascii_lowercase().ends_with(".msi"))))
}

pub fn execute(paths: &Paths, plan: &ReleasePlan, job: &Job) -> Result<()> {
    let mut native = plan.clone();
    native.entries.retain(|entry| {
        entry.preferences.release_format == "installer"
            && matches!(entry.action.as_str(), "Install" | "Update")
            && entry.error.is_none()
    });
    native.desktop_shortcut = false; // Shortcut ownership belongs to the unelevated user.
    let network = Network::new(&paths.root)?;
    for entry in &native.entries {
        job.check()?;
        let asset = entry.asset.as_ref().context("Missing approved installer")?;
        files::safe_relative(&asset.name)?;
        let destination = paths.at(format!("releases/installers/{}/{}", entry.app, asset.name));
        if destination.exists() {
            crate::network::verify_asset(&destination, asset)?;
        } else {
            network.asset(asset, &destination, job)?;
        }
    }
    let mut recovery_packages = std::collections::BTreeMap::new();
    if native.installer_downgrade() {
        let config = paths.config()?;
        for entry in &native.entries {
            let installed = config
                .apps
                .iter()
                .find(|app| app.name == entry.app)
                .context("Downgrade installation is missing")?;
            let (asset, _) = updates::prepare_downgrade_recovery(paths, installed, job)?;
            recovery_packages.insert(entry.app.clone(), asset);
        }
    }
    let folder = paths.at(format!("runtime/admin-batches/{}", uuid::Uuid::new_v4()));
    let manifest = folder.join("approved.json");
    let child_operations = job
        .child_ids()
        .into_iter()
        .filter(|(app, _)| native.entries.iter().any(|entry| entry.app == *app))
        .collect();
    let request = Request {
        root: paths.root.clone(),
        tools: paths.tools.clone(),
        profile: std::env::var_os("USERPROFILE")
            .context("Missing Windows profile")?
            .into(),
        plan: native,
        build: paths.builder_preferences()?,
        operation_parent: job.operation_id(),
        child_operations,
        recovery_packages,
    };
    files::write_json(&manifest, &request)?;
    let checksum = format!("{:x}", Sha256::digest(std::fs::read(&manifest)?));
    job.stage(
        "Awaiting administrator approval",
        None,
        "Approve access once for this installer batch",
    );
    run_worker(&manifest, &checksum, job)?;
    let outcome: Outcome = files::read_json(&folder.join("result.json"))?;
    if let Ok(log) = std::fs::read_to_string(folder.join("installers.log")) {
        for line in log.lines() {
            job.log(line);
        }
    }
    if outcome.cancelled {
        return Err(crate::jobs::Cancelled.into());
    }
    let mut errors = Vec::new();
    if let Some(error) = outcome.error {
        errors.push(error);
    }
    let config = paths.config()?;
    if plan.desktop_shortcut {
        for entry in &request.plan.entries {
            if let Some(installed) = config.apps.iter().find(|installed| {
                installed.name == entry.app
                    && installed.install_kind == "installer"
                    && installed.version == entry.version
            }) {
                if let Err(error) = crate::portable::installer_desktop_shortcut(
                    paths,
                    &entry.app,
                    Path::new(&installed.path),
                ) {
                    job.log(&format!("Desktop shortcut warning: {error:#}"));
                }
            }
        }
    }
    let mut remaining = plan.clone();
    remaining.entries.retain(|entry| {
        !request
            .plan
            .entries
            .iter()
            .any(|native| native.app == entry.app)
    });
    if !remaining.entries.is_empty() {
        updates::refresh_plan_snapshot(paths, &mut remaining)?;
        if let Err(error) = updates::execute_plan_direct(paths, &remaining, job) {
            errors.push(format!("{error:#}"));
        }
    }
    anyhow::ensure!(errors.is_empty(), "{}", errors.join("; "));
    Ok(())
}

fn run_worker(manifest: &Path, checksum: &str, job: &Job) -> Result<()> {
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::{CloseHandle, WAIT_OBJECT_0, WAIT_TIMEOUT},
            System::Threading::{GetExitCodeProcess, WaitForSingleObject},
            UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW},
        },
    };
    let executable = crate::platform::wide(std::env::current_exe()?);
    let verb = crate::platform::wide("runas");
    let text = manifest.to_str().context("Invalid batch path")?;
    anyhow::ensure!(!text.contains(['"', '\r', '\n']), "Invalid batch path");
    let arguments = crate::platform::wide(format!("--approved-admin-batch \"{text}\" {checksum}"));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(executable.as_ptr()),
        lpParameters: PCWSTR(arguments.as_ptr()),
        nShow: 0,
        ..Default::default()
    };
    unsafe {
        ShellExecuteExW(&mut info)
            .context("Administrator approval was declined or the batch worker could not start")?;
    }
    // SHELLEXECUTEINFOW is packed on x86: copy its handle before borrowing it.
    let process = info.hProcess;
    let folder = manifest.parent().context("Missing batch folder")?;
    let result = (|| -> Result<()> {
        anyhow::ensure!(
            !process.is_invalid(),
            "Batch worker returned no process handle"
        );
        loop {
            if job.cancel.load(Ordering::Relaxed) {
                files::write_bytes(&folder.join("cancel"), b"cancel")?;
            }
            if let Ok(state) = files::read_json::<crate::jobs::State>(&folder.join("progress.json"))
            {
                job.stage(&state.stage, state.progress, state.detail);
                job.state.lock().unwrap().units = state.units;
            }
            let status = unsafe { WaitForSingleObject(process, 150) };
            if status == WAIT_OBJECT_0 {
                let mut code = 0;
                unsafe {
                    GetExitCodeProcess(process, &mut code)?;
                }
                anyhow::ensure!(
                    code == 0,
                    "Administrator batch failed ({code}); see the batch log"
                );
                return Ok(());
            }
            anyhow::ensure!(
                status == WAIT_TIMEOUT,
                "Could not wait for administrator batch"
            );
        }
    })();
    unsafe {
        let _ = CloseHandle(process);
    }
    result
}

pub fn worker(manifest: &Path, checksum: &str) -> Result<()> {
    let bytes = std::fs::read(manifest)?;
    anyhow::ensure!(
        format!("{:x}", Sha256::digest(&bytes)) == checksum,
        "The approved batch changed; review it again"
    );
    anyhow::ensure!(
        unsafe { windows::Win32::UI::Shell::IsUserAnAdmin().as_bool() },
        "The batch worker requires administrator approval"
    );
    let request: Request = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(
        eligible(&request.plan)
            && request
                .plan
                .entries
                .iter()
                .all(|entry| entry.preferences.release_format == "installer"
                    && matches!(entry.action.as_str(), "Install" | "Update")
                    && entry.error.is_none()),
        "The approved worker only accepts installer batches"
    );
    anyhow::ensure!(std::env::var_os("USERPROFILE").map(PathBuf::from).as_ref() == Some(&request.profile),
        "Use administrator approval for the same Windows account; another account would change per-user installation ownership");
    let paths = Paths::new(request.root, Some(request.tools));
    crate::catalog::load(&paths.root);
    // Deny writes/deletion while the already verified packages are being executed.
    use std::os::windows::fs::OpenOptionsExt;
    let mut locked = Vec::new();
    for entry in &request.plan.entries {
        let asset = entry.asset.as_ref().context("Missing approved package")?;
        files::safe_relative(&asset.name)?;
        crate::model::valid_app(&entry.app)?;
        let file = paths.at(format!("releases/installers/{}/{}", entry.app, asset.name));
        locked.push(
            std::fs::OpenOptions::new()
                .read(true)
                .share_mode(1)
                .open(&file)?,
        );
        crate::network::verify_asset(&file, asset)?;
    }
    for (app, asset) in &request.recovery_packages {
        anyhow::ensure!(
            request.plan.installer_downgrade()
                && request.plan.entries.iter().any(|entry| &entry.app == app),
            "Invalid recovery package scope"
        );
        crate::model::valid_app(app)?;
        files::safe_relative(&asset.name)?;
        let file = paths.at(format!("releases/installers/{app}/{}", asset.name));
        locked.push(
            std::fs::OpenOptions::new()
                .read(true)
                .share_mode(1)
                .open(&file)?,
        );
        crate::network::verify_asset(&file, asset)?;
    }
    anyhow::ensure!(
        !request.plan.installer_downgrade() || request.recovery_packages.len() == 1,
        "Downgrade recovery must be prepared before administrator approval"
    );
    let folder = manifest
        .parent()
        .context("Missing batch directory")?
        .to_path_buf();
    let mut job = Job::new(folder.join("installers.log"), &request.build);
    anyhow::ensure!(
        request.child_operations.keys().all(|app| request
            .plan
            .entries
            .iter()
            .any(|entry| entry.app == *app)),
        "Invalid batch operation scope"
    );
    job.attach_children(
        &paths.root,
        request.operation_parent,
        &request.child_operations,
    )?;
    let stopped = Arc::new(AtomicBool::new(false));
    let stop = stopped.clone();
    let watcher_job = job.clone();
    let watcher_folder = folder.clone();
    let watcher = std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            if watcher_folder.join("cancel").exists() {
                watcher_job.cancel.store(true, Ordering::Relaxed);
            }
            let state = {
                let current = watcher_job.state.lock().unwrap();
                crate::jobs::State {
                    stage: current.stage.clone(),
                    detail: current.detail.clone(),
                    progress: current.progress,
                    units: current.units,
                    ..Default::default()
                }
            };
            let _ = files::write_json(&watcher_folder.join("progress.json"), &state);
            std::thread::sleep(Duration::from_millis(100));
        }
    });
    let result = updates::execute_approved_downgrade(
        &paths,
        &request.plan,
        &job,
        &request.recovery_packages,
    );
    stopped.store(true, Ordering::Relaxed);
    let _ = watcher.join();
    let outcome = match result {
        Ok(()) => Outcome {
            error: None,
            cancelled: false,
        },
        Err(error) => Outcome {
            cancelled: crate::jobs::is_cancelled(&error),
            error: Some(format!("{error:#}")),
        },
    };
    files::write_json(&folder.join("result.json"), &outcome)?;
    drop(locked);
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn single_msi_downgrade_shares_batch_approval_but_upgrade_does_not() {
        let prefs = crate::model::Preferences::default();
        let snapshot = serde_json::to_vec(&serde_json::json!([
            prefs, {"appsRoot":"fixture", "apps":[{"name":"photocraft", "version":"2.0.0",
                "path":"fixture", "install_kind":"installer", "architecture":"x64"}], "installations":[]}, []
        ])).unwrap();
        let mut plan: crate::updates::ReleasePlan = serde_json::from_value(serde_json::json!({
            "preferences":prefs, "requested_tag":"1.0.0", "snapshot":snapshot,
            "desktop_shortcut":false, "installer_destination":null,
            "entries":[{"app":"photocraft", "action":"Update", "version":"1.0.0",
                "preferences":prefs, "destination":null, "error":null,
                "asset":{"name":"photocraft.msi", "browser_download_url":"https://example.invalid/installer", "size":1, "digest":null}}]
        })).unwrap();
        plan.entries[0].preferences.release_format = "installer".into();
        assert!(super::eligible(&plan));
        plan.preferences.batch_admin = false;
        assert!(!super::eligible(&plan));
        plan.preferences.batch_admin = true;
        plan.entries[0].version = "3.0.0".into();
        assert!(!super::eligible(&plan));
        plan.entries[0].version = "1.0.0".into();
        plan.entries[0].preferences.release_format = "portable".into();
        assert!(!super::eligible(&plan));
        plan.entries[0].preferences.release_format = "installer".into();
        plan.entries[0].asset.as_mut().unwrap().name = "setup.exe".into();
        assert!(!super::eligible(&plan));
    }
    #[test]
    fn worker_is_only_used_for_multiple_approved_installer_operations() {
        let preferences = crate::model::Preferences::default();
        let entries: Vec<_> = ["photocraft", "filmcraft"]
            .into_iter()
            .map(|name| {
                let mut prefs = preferences.clone();
                prefs.release_format = "installer".into();
                crate::updates::ReleaseEntry {
                    app: name.into(),
                    action: "Update".into(),
                    version: "1.0".into(),
                    asset: None,
                    destination: None,
                    error: None,
                    preferences: prefs,
                }
            })
            .collect();
        let mut plan: crate::updates::ReleasePlan = serde_json::from_value(serde_json::json!({
            "entries": entries, "preferences": preferences, "requested_tag": null,
            "snapshot": [], "desktop_shortcut": false, "installer_destination": null
        }))
        .unwrap();
        assert!(super::eligible(&plan));
        plan.preferences.batch_admin = false;
        assert!(!super::eligible(&plan));
        plan.preferences.batch_admin = true;
        plan.requested_tag = Some("1.0".into());
        assert!(!super::eligible(&plan));
        plan.requested_tag = None;
        plan.entries[0].preferences.release_format = "portable".into();
        assert!(!super::eligible(&plan));
        plan.entries[0].preferences.release_format = "installer".into();
        plan.entries[0].error = Some("No compatible update".into());
        assert!(!super::eligible(&plan));
    }
    #[test]
    fn changed_manifest_is_rejected_before_any_elevation_or_installation() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&root).unwrap();
        let manifest = root.join("approved.json");
        std::fs::write(&manifest, b"altered request").unwrap();
        let error = super::worker(&manifest, "incorrect-checksum").unwrap_err();
        assert!(error.to_string().contains("approved batch changed"));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn batch_elevation_defaults_on_but_preserves_opt_out() {
        let settings: crate::model::Preferences = serde_json::from_str("{}").unwrap();
        assert!(settings.batch_admin);
        let enabled: crate::model::Preferences =
            serde_json::from_str(r#"{"batchAdmin":true}"#).unwrap();
        assert!(enabled.batch_admin);
        let disabled: crate::model::Preferences =
            serde_json::from_str(r#"{"batchAdmin":false}"#).unwrap();
        assert!(!disabled.batch_admin);
    }
}
