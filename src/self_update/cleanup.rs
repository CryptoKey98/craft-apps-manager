//! Bounded cleanup of completed Manager-owned update stages, never app rollback storage.
use crate::{files, model::Paths};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn owned_name(name: &str) -> bool {
    matches!(
        name,
        "plan.json"
            | "authorization.json"
            | "completed.json"
            | "release.msi"
            | "release.zip"
            | "download.package"
            | "package"
            | "payload"
            | "previous.exe"
            | "previous.app"
            | "update-helper.exe"
            | "update-helper"
            | "install.log"
            | "verify.log"
            | "result.json"
    )
}

// Linux package managers require the original package suffix. Unlike portable
// payloads, their staged downloads therefore retain release-specific names.
fn native_download(stage: &Path, plan: &super::Plan) -> Result<Option<PathBuf>> {
    if !plan.linux_package {
        return Ok(None);
    }
    let Some(name) = plan.staged.file_name() else {
        return Ok(None);
    };
    let text = name.to_string_lossy();
    if ![".deb", ".rpm", ".pkg.tar.zst"]
        .iter()
        .any(|suffix| text.ends_with(suffix))
        || plan.staged.parent().map(location).transpose()? != Some(location(stage)?)
    {
        return Ok(None);
    }
    let file = stage.join(name);
    if file.exists() {
        files::no_links(&file)?;
        ensure!(file.is_file(), "Native update download is not a file");
        ensure!(
            files::hash(&file)? == plan.hash,
            "Changed native download was preserved"
        );
    }
    Ok(Some(file))
}

fn remove_completed_stage(stage: &Path) -> Result<()> {
    files::no_links(stage)?;
    ensure!(
        fs::metadata(stage.join("plan.json"))?.len() <= 64 * 1024,
        "Oversized update plan"
    );
    let plan: super::Plan = files::read_json(&stage.join("plan.json"))?;
    let download = native_download(stage, &plan)?;
    let entries = fs::read_dir(stage)?.collect::<std::io::Result<Vec<_>>>()?;
    ensure!(
        entries
            .iter()
            .all(|e| owned_name(&e.file_name().to_string_lossy())
                || download.as_ref() == Some(&e.path())),
        "Unknown update content was preserved"
    );
    let helper = stage.join(super::legacy::helper_name());
    if helper.exists() {
        fs::remove_file(helper)?;
    }
    for entry in entries {
        let name = entry.file_name();
        if name != "plan.json" && name != "completed.json" && name != super::legacy::helper_name() {
            files::remove_managed(&entry.path(), stage)?;
        }
    }
    ensure!(
        fs::read_dir(stage)?.all(|e| e
            .is_ok_and(|e| matches!(e.file_name().to_str(), Some("plan.json" | "completed.json")))),
        "New update content was preserved"
    );
    if stage.join("completed.json").exists() {
        fs::remove_file(stage.join("completed.json"))?;
    }
    fs::remove_file(stage.join("plan.json"))?;
    fs::remove_dir(stage)?;
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct Completion {
    schema: u32,
    target: PathBuf,
    root: PathBuf,
    hash: String,
}

pub(crate) fn completed(plan: &super::Plan, stage: &Path) -> Result<()> {
    let hash = files::hash(&super::legacy::staged_executable(&plan.target))?;
    let record = Completion {
        schema: 1,
        target: plan.target.clone(),
        root: plan.root.clone(),
        hash,
    };
    files::write_json(&stage.join("completed.json"), &record)
}

fn location(path: &Path) -> Result<PathBuf> {
    let path = if path.exists() {
        path.canonicalize()?
    } else {
        std::path::absolute(path)?
    };
    ensure!(
        !path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir)),
        "Ambiguous update path"
    );
    #[cfg(windows)]
    let path = PathBuf::from(
        super::legacy::installer_path(&path)
            .to_string_lossy()
            .to_lowercase(),
    );
    Ok(path)
}

fn eligible(
    stage: &Path,
    root: &Path,
    target: &Path,
    hash: &str,
    version: &str,
    revision: u32,
) -> Result<bool> {
    files::no_links(stage)?;
    ensure!(
        fs::metadata(stage.join("plan.json"))?.len() <= 64 * 1024,
        "Oversized update plan"
    );
    let plan: super::Plan = files::read_json(&stage.join("plan.json"))?;
    if !target.exists() {
        return Ok(false);
    }
    if plan.root.canonicalize()? != root.canonicalize()?
        || plan.target.canonicalize()? != target.canonicalize()?
    {
        return Ok(false);
    }
    let stage_location = location(stage)?;
    if location(target)?.starts_with(&stage_location)
        || location(&plan.tools)?.starts_with(&stage_location)
        || !location(&plan.staged)?.starts_with(&stage_location)
    {
        return Ok(false);
    }
    let mut has_recovery = false;
    let download = native_download(stage, &plan)?;
    for entry in fs::read_dir(stage)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !owned_name(&name) && download.as_ref() != Some(&entry.path()) {
            return Ok(false);
        }
        has_recovery |= matches!(name.as_ref(), "previous.exe" | "previous.app");
    }
    // A running, healthy target with the exact replacement hash proves the old
    // portable transaction no longer needs its recovery executable.
    if plan.hash == hash {
        return Ok(true);
    }
    let not_newer = plan.expected_identity.as_ref().is_some_and(|expected| {
        crate::updates::version(&expected.version)
            .ok()
            .zip(crate::updates::version(version).ok())
            .is_some_and(|(wanted, current)| {
                wanted < current || wanted == current && expected.revision <= revision
            })
    });
    if stage.join("completed.json").exists() {
        ensure!(
            fs::metadata(stage.join("completed.json"))?.len() <= 8192,
            "Oversized completion record"
        );
        let completed: Completion = files::read_json(&stage.join("completed.json"))?;
        return Ok(completed.schema == 1
            && completed.root == plan.root
            && completed.target == plan.target
            && completed.hash.len() == 64
            && completed.hash.bytes().all(|b| b.is_ascii_hexdigit())
            && (completed.hash == hash || not_newer));
    }
    if !not_newer {
        return Ok(false);
    }
    // Retained by older installers: no rollback executable, an obsolete target
    // identity, and no recent modification. Unknown recovery is never aged out.
    Ok(!has_recovery
        && fs::metadata(stage)?
            .modified()?
            .elapsed()
            .is_ok_and(|age| age.as_secs() >= 3600))
}

/// Runs outside the UI thread. Failed/locked deletion retains the plan for retry.
pub fn schedule(paths: &Paths) {
    let paths = paths.clone();
    std::thread::spawn(move || {
        for delay in [2, 30, 120].into_iter().chain(std::iter::repeat(1800)) {
            std::thread::sleep(std::time::Duration::from_secs(delay));
            if let Err(error) = sweep(&paths) {
                eprintln!("Manager update cleanup deferred: {error:#}");
            }
        }
    });
}

fn sweep(paths: &Paths) -> Result<()> {
    let executable = std::env::current_exe()?.canonicalize()?;
    #[cfg(target_os = "macos")]
    let target = super::legacy::app_bundle(&executable)?;
    #[cfg(not(target_os = "macos"))]
    let target = executable.clone();
    sweep_at(
        paths,
        &target,
        &executable,
        env!("CARGO_PKG_VERSION"),
        super::revisions::BUILD_REVISION,
    )
}

fn sweep_at(
    paths: &Paths,
    target: &Path,
    executable: &Path,
    version: &str,
    revision: u32,
) -> Result<()> {
    let home = target.parent().context("Missing Manager folder")?;
    let mut bases = vec![
        paths.at("runtime/self-update"),
        home.join("runtime/self-update"),
    ];
    #[cfg(target_os = "macos")]
    bases.push(home.to_path_buf());
    bases.sort();
    bases.dedup();
    let mut stages = Vec::new();
    for base in bases {
        if !base.exists() {
            continue;
        }
        crate::portable::validate_path(&base)?;
        for entry in fs::read_dir(&base)?.take(1024) {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if base == home && !name.starts_with(".craft-manager-update-") {
                continue;
            }
            let id = name.strip_prefix(".craft-manager-update-").unwrap_or(&name);
            if uuid::Uuid::parse_str(id).is_ok() && entry.file_type()?.is_dir() {
                stages.push((entry.metadata()?.modified()?, entry.path(), base.clone()));
            }
        }
    }
    stages.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    if stages.is_empty() {
        return Ok(());
    }
    let hash = files::hash(executable)?;
    for (_, stage, base) in stages {
        let Ok(_lock) = crate::platform::Lock::take("Local\\CraftAppsManager") else {
            return Ok(());
        };
        if paths
            .at("runtime/manager-updates/bridge-approval.json")
            .exists()
            || crate::operations::Manager::new(&paths.root)
                .list()
                .iter()
                .any(|op| !op.lifecycle.terminal())
        {
            return Ok(());
        }
        if !eligible(&stage, &paths.root, target, &hash, version, revision).unwrap_or(false) {
            continue;
        }
        files::inside(&stage, &base)?;
        let stage_location = location(&stage)?;
        if let Some(inventory) = paths.read_inventory()? {
            if inventory
                .apps
                .iter()
                .chain(&inventory.installations)
                .any(|app| {
                    !app.path.is_empty()
                        && location(Path::new(&app.path))
                            .map_or(true, |path| path.starts_with(&stage_location))
                })
            {
                continue;
            }
        }
        // Recheck ownership and keep the plan until all payloads are gone.
        let deletion = remove_completed_stage(&stage);
        if let Err(error) = deletion {
            eprintln!("Manager update cleanup will retry: {error:#}");
        }
        if base != home {
            let _ = fs::remove_dir(&base);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn fixture() -> (Fixture, PathBuf, PathBuf, String) {
        let root = std::env::temp_dir().join(format!("manager-cleanup-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let stage = root
            .join("runtime/self-update")
            .join(uuid::Uuid::new_v4().simple().to_string());
        fs::create_dir_all(&stage).unwrap();
        let target = root.join("manager");
        fs::write(&target, b"new manager").unwrap();
        let hash = files::hash(&target).unwrap();
        let plan = super::super::Plan {
            expected_identity: None,
            layout: 0,
            msi: false,
            linux_package: false,
            parent_pid: 1,
            target: target.clone(),
            staged: stage.join("package/manager"),
            hash: hash.clone(),
            root: root.clone(),
            tools: root.join("tools"),
        };
        files::write_json(&stage.join("plan.json"), &plan).unwrap();
        fs::write(stage.join("previous.exe"), b"old manager").unwrap();
        (Fixture(root), stage, target, hash)
    }
    #[test]
    fn completed_matching_update_is_eligible_but_unknown_content_and_other_targets_are_not() {
        let (f, stage, target, hash) = fixture();
        assert!(eligible(&stage, &f.0, &target, &hash, "1.1.1", 1).unwrap());
        fs::write(stage.join("personal.txt"), b"keep").unwrap();
        assert!(!eligible(&stage, &f.0, &target, &hash, "1.1.1", 1).unwrap());
        fs::remove_file(stage.join("personal.txt")).unwrap();
        assert!(!eligible(
            &stage,
            &f.0,
            &f.0.join("another-manager"),
            &hash,
            "1.1.1",
            1
        )
        .unwrap());
    }
    #[test]
    fn completed_legacy_installer_receipt_binds_to_the_running_executable() {
        let (f, stage, target, hash) = fixture();
        let mut plan: super::super::Plan = files::read_json(&stage.join("plan.json")).unwrap();
        plan.msi = true;
        plan.hash = "0".repeat(64);
        files::write_json(&stage.join("plan.json"), &plan).unwrap();
        files::write_json(
            &stage.join("completed.json"),
            &Completion {
                schema: 1,
                target: plan.target.clone(),
                root: plan.root.clone(),
                hash: hash.clone(),
            },
        )
        .unwrap();
        assert!(eligible(&stage, &f.0, &target, &hash, "1.1.1", 1).unwrap());
        assert!(!eligible(&stage, &f.0, &target, &"f".repeat(64), "1.1.1", 1).unwrap());
    }
    #[test]
    fn native_release_filename_is_cleaned_only_when_bound_to_the_completed_plan() {
        for extension in ["deb", "rpm", "pkg.tar.zst"] {
            let (f, stage, target, hash) = fixture();
            let package = stage.join(format!("craft-apps-manager_1.1.1_amd64.{extension}"));
            fs::write(&package, b"verified native download").unwrap();
            let mut plan: super::super::Plan = files::read_json(&stage.join("plan.json")).unwrap();
            plan.linux_package = true;
            plan.staged = package.clone();
            plan.hash = files::hash(&package).unwrap();
            files::write_json(&stage.join("plan.json"), &plan).unwrap();
            files::write_json(
                &stage.join("completed.json"),
                &Completion {
                    schema: 1,
                    target: plan.target.clone(),
                    root: plan.root.clone(),
                    hash: hash.clone(),
                },
            )
            .unwrap();
            assert!(eligible(&stage, &f.0, &target, &hash, "1.1.1", 1).unwrap());
            fs::write(stage.join("unrelated.deb"), b"keep unknown package").unwrap();
            assert!(!eligible(&stage, &f.0, &target, &hash, "1.1.1", 1).unwrap());
            assert!(remove_completed_stage(&stage).is_err());
            fs::remove_file(stage.join("unrelated.deb")).unwrap();
            fs::write(&package, b"unexpected replacement").unwrap();
            assert!(!eligible(&stage, &f.0, &target, &hash, "1.1.1", 1).unwrap_or(false));
            assert!(remove_completed_stage(&stage).is_err());
            fs::write(&package, b"verified native download").unwrap();
            remove_completed_stage(&stage).unwrap();
            assert!(!stage.exists() && target.exists());
        }
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_completion_hashes_the_bundle_entrypoint() {
        let (f, stage, _, _) = fixture();
        let bundle = f.0.join("Manager.app");
        let executable = bundle.join("Contents/MacOS/craft-apps-manager");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(&executable, b"replacement bundle executable").unwrap();
        let mut plan: super::super::Plan = files::read_json(&stage.join("plan.json")).unwrap();
        plan.target = bundle.clone();
        plan.hash = "0".repeat(64);
        files::write_json(&stage.join("plan.json"), &plan).unwrap();
        completed(&plan, &stage).unwrap();
        let record: Completion = files::read_json(&stage.join("completed.json")).unwrap();
        let hash = files::hash(&executable).unwrap();
        assert_eq!(record.hash, hash);
        assert!(eligible(&stage, &f.0, &bundle, &hash, "1.1.1", 1).unwrap());
    }
    #[test]
    fn incomplete_recovery_and_corrupt_plans_are_retained() {
        let (f, stage, target, _) = fixture();
        assert!(!eligible(&stage, &f.0, &target, &"0".repeat(64), "1.1.1", 1).unwrap());
        fs::write(stage.join("plan.json"), b"not JSON").unwrap();
        assert!(eligible(&stage, &f.0, &target, &"0".repeat(64), "1.1.1", 1).is_err());
        assert!(stage.join("previous.exe").exists());
    }
    #[test]
    fn deletion_removes_completed_payloads_but_preserves_new_unknown_content() {
        let (f, stage, target, hash) = fixture();
        fs::write(stage.join("personal.txt"), b"keep").unwrap();
        assert!(remove_completed_stage(&stage).is_err());
        assert!(stage.join("plan.json").exists() && stage.join("previous.exe").exists());
        fs::remove_file(stage.join("personal.txt")).unwrap();
        assert!(eligible(&stage, &f.0, &target, &hash, "1.1.1", 1).unwrap());
        remove_completed_stage(&stage).unwrap();
        assert!(!stage.exists());
        assert!(target.exists());
    }
    #[test]
    fn sweep_removes_multiple_completed_stages_and_preserves_unknown_directories() {
        let (f, stage, target, _) = fixture();
        let base = stage.parent().unwrap();
        let older = base.join(uuid::Uuid::new_v4().simple().to_string());
        files::copy_directory_verified(&stage, &older).unwrap();
        let mut plan: super::super::Plan = files::read_json(&older.join("plan.json")).unwrap();
        plan.staged = older.join("package/manager");
        files::write_json(&older.join("plan.json"), &plan).unwrap();
        let unknown = base.join(uuid::Uuid::new_v4().simple().to_string());
        fs::create_dir(&unknown).unwrap();
        fs::write(unknown.join("personal.txt"), b"keep").unwrap();
        let paths = Paths::new(f.0.clone(), None);
        sweep_at(&paths, &target, &target, "1.1.1", 1).unwrap();
        assert!(!stage.exists() && !older.exists());
        assert!(unknown.join("personal.txt").exists() && target.exists());
    }
    #[test]
    fn active_bridge_and_registered_app_paths_prevent_stage_deletion() {
        let (f, stage, target, _) = fixture();
        let paths = Paths::new(f.0.clone(), None);
        let bridge = paths.at("runtime/manager-updates/bridge-approval.json");
        files::write_bytes(&bridge, b"active bridge").unwrap();
        sweep_at(&paths, &target, &target, "1.1.1", 1).unwrap();
        assert!(stage.join("previous.exe").exists());
        fs::remove_file(bridge).unwrap();
        let config = crate::model::Config {
            apps_root: f.0.display().to_string(),
            apps: vec![crate::model::Installed {
                name: "fixture".into(),
                path: stage.join("package/app").display().to_string(),
                ..Default::default()
            }],
            installations: Vec::new(),
            portable_label_counters: Default::default(),
        };
        files::write_json(&paths.at("installations.json"), &config).unwrap();
        sweep_at(&paths, &target, &target, "1.1.1", 1).unwrap();
        assert!(stage.join("previous.exe").exists());
    }
    #[cfg(windows)]
    #[test]
    fn canonical_windows_plan_paths_are_accepted_without_losing_containment_checks() {
        let (f, stage, target, hash) = fixture();
        let mut plan: super::super::Plan = files::read_json(&stage.join("plan.json")).unwrap();
        plan.staged = stage.canonicalize().unwrap().join("package/manager");
        files::write_json(&stage.join("plan.json"), &plan).unwrap();
        assert!(eligible(&stage, &f.0, &target, &hash, "1.1.1", 1).unwrap());
        plan.tools = stage.canonicalize().unwrap().join("tools");
        files::write_json(&stage.join("plan.json"), &plan).unwrap();
        assert!(!eligible(&stage, &f.0, &target, &hash, "1.1.1", 1).unwrap());
    }
    #[cfg(windows)]
    #[test]
    fn locked_helper_preserves_plan_and_payload_for_a_later_retry() {
        use std::os::windows::fs::OpenOptionsExt;
        let (_f, stage, _, _) = fixture();
        let helper = stage.join(super::super::legacy::helper_name());
        fs::write(&helper, b"helper").unwrap();
        let guard = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(helper)
            .unwrap();
        assert!(remove_completed_stage(&stage).is_err());
        assert!(stage.join("plan.json").exists() && stage.join("previous.exe").exists());
        drop(guard);
        remove_completed_stage(&stage).unwrap();
        assert!(!stage.exists());
    }
}
