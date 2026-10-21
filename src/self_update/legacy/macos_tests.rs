use super::*;
#[test]
fn selects_universal_zip_from_configured_repository() {
    let release = |name: &str, url: &str| Release {
        tag_name: "v0.5.0".into(),
        name: None,
        draft: false,
        prerelease: false,
        assets: vec![Asset {
            name: name.into(),
            size: 1,
            digest: None,
            browser_download_url: url.into(),
        }],
    };
    let url = format!("{REPOSITORY}/releases/download/v0.5.0/package.zip");
    let selected = select(
        release("Craft-Apps-Manager-0.5.0-macos-universal.zip", &url),
        "0.4.0",
    )
    .unwrap()
    .unwrap();
    assert_eq!(selected.version, "0.5.0");
    assert!(select(
        release("Craft-Apps-Manager-0.5.0-linux-x64.zip", &url),
        "0.4.0"
    )
    .unwrap()
    .is_none());
    assert!(select(
        release(
            "Craft-Apps-Manager-0.5.0-macos-universal.zip",
            "https://example.com/package.zip"
        ),
        "0.4.0"
    )
    .is_err());
}
#[test]
fn update_helper_drops_inherited_quarantine() {
    let file = std::env::temp_dir().join(format!("craft-helper-{}", uuid::Uuid::new_v4()));
    fs::write(&file, "helper").unwrap();
    clear_quarantine(&file).unwrap();
    let status = Command::new("/usr/bin/xattr")
        .args(["-w", "com.apple.quarantine", "0083;6720f000;Safari;"])
        .arg(&file)
        .status()
        .unwrap();
    assert!(status.success());
    clear_quarantine(&file).unwrap();
    let out = Command::new("/usr/bin/xattr").arg(&file).output().unwrap();
    assert!(!String::from_utf8_lossy(&out.stdout).contains("com.apple.quarantine"));
    fs::remove_file(file).unwrap();
}
#[test]
fn only_app_bundles_update_themselves() {
    let bundle = Path::new("/Applications/Craft Apps Manager.app");
    assert_eq!(
        app_bundle(&bundle.join("Contents/MacOS/craft-apps-manager")).unwrap(),
        bundle
    );
    assert!(app_bundle(Path::new("/repo/target/release/craft-apps-manager")).is_err());
    assert!(app_bundle(&bundle.join("Contents/MacOS/other")).is_err());
}
struct Fixture {
    home: PathBuf,
    stage: PathBuf,
    plan: Plan,
}
impl Fixture {
    fn new() -> Self {
        let home = std::env::temp_dir().join(format!("craft-self-update-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&home).unwrap();
        let home = home.canonicalize().unwrap();
        let stage = create_macos_stage(&home).unwrap();
        let target = home.join("Craft Apps Manager.app");
        let staged = stage.join("package/Craft Apps Manager.app");
        for bundle in [&target, &staged] {
            fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
            fs::create_dir_all(bundle.join("Contents/Resources")).unwrap();
            fs::copy("/usr/bin/true", staged_executable(bundle)).unwrap();
            fs::write(bundle.join("Contents/Info.plist"), format!(
                    "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>{BUNDLE_ID}</string><key>CFBundleExecutable</key><string>craft-apps-manager</string><key>CFBundlePackageType</key><string>APPL</string></dict></plist>"
                )).unwrap();
            let output = Command::new("/usr/bin/codesign")
                .args(["--force", "--sign", "-"])
                .arg(bundle)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            verify_bundle(bundle).unwrap();
        }
        fs::write(target.join("Contents/Resources/old-marker"), "old bytes").unwrap();
        fs::write(staged.join("Contents/Resources/new-marker"), "new bytes").unwrap();
        // Marker files are part of the signed resources, so resign after adding them.
        for bundle in [&target, &staged] {
            assert!(Command::new("/usr/bin/codesign")
                .args(["--force", "--sign", "-"])
                .arg(bundle)
                .output()
                .unwrap()
                .status
                .success());
        }
        fs::copy(staged_executable(&target), stage.join(helper_name())).unwrap();
        let plan = Plan {
            expected_identity: None,
            layout: 1,
            msi: false,
            linux_package: false,
            parent_pid: u32::MAX,
            hash: files::hash(&staged_executable(&staged)).unwrap(),
            target,
            staged,
            root: home.join("separate-library"),
            tools: home.join("tools"),
        };
        files::write_json(&stage.join("plan.json"), &plan).unwrap();
        Self { home, stage, plan }
    }
    fn validate(&self) -> Result<PathBuf> {
        validate_macos_layout(
            &self.plan,
            &self.stage.join("plan.json"),
            &self.stage.join(helper_name()),
        )
    }
    fn old_is_present(&self) -> bool {
        fs::read(self.plan.target.join("Contents/Resources/old-marker"))
            .is_ok_and(|bytes| bytes == b"old bytes")
    }
    fn swap(&self, failures: &[usize], verify_fails: bool, launch_fails: bool) -> Result<()> {
        let mut calls = 0;
        swap_macos(
            &self.plan,
            &self.stage,
            |from, to| {
                calls += 1;
                if failures.contains(&calls) {
                    return Err(std::io::Error::from_raw_os_error(libc::EACCES));
                }
                fs::rename(from, to)
            },
            |_| {
                if verify_fails {
                    bail!("injected verification failure");
                }
                Ok(())
            },
            || {
                if launch_fails {
                    bail!("injected open rejection");
                }
                Ok(())
            },
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.home);
    }
}
#[test]
fn private_sibling_layout_keeps_library_independent() {
    use std::os::unix::fs::MetadataExt;
    let fixture = Fixture::new();
    assert_eq!(fixture.validate().unwrap(), fixture.stage);
    assert_eq!(
        fs::metadata(&fixture.stage).unwrap().dev(),
        fs::metadata(&fixture.plan.target).unwrap().dev()
    );
    validate_macos_replacement(&fixture.plan).unwrap();
    fixture.swap(&[], false, false).unwrap();
    assert_eq!(
        fs::read(fixture.plan.target.join("Contents/Resources/new-marker")).unwrap(),
        b"new bytes"
    );
    assert_eq!(
        fs::read(
            fixture
                .stage
                .join("previous.app/Contents/Resources/old-marker")
        )
        .unwrap(),
        b"old bytes"
    );
    assert!(fixture.swap(&[], false, false).is_err());
}
#[test]
fn legacy_plans_are_rejected_before_mutation() {
    let mut fixture = Fixture::new();
    fixture.plan.layout = 0;
    assert!(fixture
        .validate()
        .unwrap_err()
        .to_string()
        .contains("legacy layout"));
    assert!(fixture.old_is_present());
}
#[test]
fn corrupt_replacement_is_rejected_before_swap() {
    let fixture = Fixture::new();
    fs::write(staged_executable(&fixture.plan.staged), "corrupt").unwrap();
    assert!(validate_macos_replacement(&fixture.plan)
        .unwrap_err()
        .to_string()
        .contains("checksum"));
    assert!(fixture.old_is_present());
}
#[test]
fn first_move_failure_is_prompt_and_preserves_target() {
    let fixture = Fixture::new();
    let started = Instant::now();
    let error = fixture.swap(&[1], false, false).unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(error.to_string().contains("permissions"));
    assert!(fixture.old_is_present());
    assert!(!fixture.stage.join("previous.app").exists());
}
#[test]
fn publish_failure_restores_original_bytes() {
    let fixture = Fixture::new();
    let error = fixture.swap(&[2], false, false).unwrap_err();
    assert!(format!("{error:#}").contains("Could not publish"));
    assert!(fixture.old_is_present());
}
#[test]
fn published_verification_and_launch_failures_restore_original() {
    for (verify, launch) in [(true, false), (false, true)] {
        let fixture = Fixture::new();
        let error = fixture.swap(&[], verify, launch).unwrap_err();
        assert!(error.to_string().contains("restored"));
        assert!(fixture.old_is_present());
        assert_eq!(
            fs::read(
                fixture
                    .stage
                    .join("failed.app/Contents/Resources/new-marker")
            )
            .unwrap(),
            b"new bytes"
        );
    }
}
#[test]
fn rollback_and_reporting_failures_keep_original_recovery_bytes() {
    // Failure publishing followed by failure restoring; and failure moving
    // the rejected replacement aside, or failure restoring after that move.
    for (failures, verify) in [(vec![2, 3], false), (vec![3], true), (vec![4], true)] {
        let fixture = Fixture::new();
        let error = fixture.swap(&failures, verify, false).unwrap_err();
        assert!(error.to_string().contains("previous.app"));
        fs::write(fixture.home.join("blocked"), "not a directory").unwrap();
        let record = UpdateResult {
            status: UpdateStatus::Failed,
            message: format!("{error:#}"),
        };
        assert!(report_macos_result(
            &fixture.stage,
            &fixture.home.join("blocked/result.json"),
            &record
        )
        .is_err());
        assert_eq!(
            fs::read(
                fixture
                    .stage
                    .join("previous.app/Contents/Resources/old-marker")
            )
            .unwrap(),
            b"old bytes"
        );
        let retained: UpdateResult = files::read_json(&fixture.stage.join("result.json")).unwrap();
        assert!(retained.message.contains("Recovery failed"));
    }
}
#[test]
fn missing_local_result_does_not_prevent_persistent_result() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.stage.join("result.json")).unwrap();
    let path = fixture.home.join("startup-result.json");
    let record = UpdateResult {
        status: UpdateStatus::Failed,
        message: "original failure".into(),
    };
    assert!(report_macos_result(&fixture.stage, &path, &record).is_err());
    let retained: UpdateResult = files::read_json(&path).unwrap();
    assert_eq!(retained.message, "original failure");
    assert!(fixture.old_is_present());
}
#[test]
fn open_exit_status_is_checked() {
    use std::os::unix::process::ExitStatusExt;
    assert!(check_open_status(std::process::ExitStatus::from_raw(0), b"").is_ok());
    let error =
        check_open_status(std::process::ExitStatus::from_raw(256), b"launch denied").unwrap_err();
    assert!(error.to_string().contains("launch denied"));
}
#[test]
fn link_escape_and_untrusted_helper_plans_are_rejected() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let mut fixture = Fixture::new();
    let staged = fixture.plan.staged.clone();
    fixture.plan.staged = fixture
        .stage
        .join("package/../package/Craft Apps Manager.app");
    assert!(fixture.validate().is_err());
    fixture.plan.staged = fixture.plan.target.clone();
    assert!(fixture.validate().is_err());
    fixture.plan.staged = staged;
    assert!(fixture.validate().is_ok());
    fs::set_permissions(&fixture.stage, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(fixture.validate().is_err());
    fs::set_permissions(&fixture.stage, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(
        fixture.stage.join(helper_name()),
        "not the approved manager",
    )
    .unwrap();
    assert!(fixture.validate().is_err());
    fs::copy(
        staged_executable(&fixture.plan.target),
        fixture.stage.join(helper_name()),
    )
    .unwrap();
    symlink(&fixture.plan.target, fixture.stage.join("linked.app")).unwrap();
    assert!(fixture.validate().is_err());
    fs::remove_file(fixture.stage.join("linked.app")).unwrap();
    let alias = fixture.home.join("Alias.app");
    symlink(&fixture.plan.target, &alias).unwrap();
    fixture.plan.target = alias;
    assert!(fixture.validate().is_err());
}
#[test]
fn unrelated_identity_and_invalid_resource_signature_are_rejected() {
    let fixture = Fixture::new();
    fs::write(fixture.plan.staged.join("Contents/Info.plist"), "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>unrelated.app</string></dict></plist>").unwrap();
    assert!(validate_macos_replacement(&fixture.plan).is_err());
    fs::write(
        fixture.plan.target.join("Contents/Resources/old-marker"),
        "tampered resources",
    )
    .unwrap();
    assert!(fixture.validate().is_err());
}
#[test]
fn unwritable_target_fails_preflight_without_removing_original() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    fs::set_permissions(&fixture.home, fs::Permissions::from_mode(0o555)).unwrap();
    let result = check_update_directory(&fixture.home);
    fs::set_permissions(&fixture.home, fs::Permissions::from_mode(0o755)).unwrap();
    if unsafe { libc::geteuid() } != 0 {
        assert!(result.unwrap_err().to_string().contains("cannot write"));
    }
    assert!(fixture.old_is_present());
}
#[test]
fn prelaunch_reporting_failure_restores_previous_manager() {
    let fixture = Fixture::new();
    fs::write(fixture.home.join("blocked"), "not a directory").unwrap();
    let error = swap_macos(
        &fixture.plan,
        &fixture.stage,
        |from, to| fs::rename(from, to),
        |_| Ok(()),
        || {
            report_macos_result(
                &fixture.stage,
                &fixture.home.join("blocked/result.json"),
                &UpdateResult {
                    status: UpdateStatus::LaunchPending,
                    message: "replacement verified; launch pending".into(),
                },
            )
        },
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("Could not save"));
    assert!(fixture.old_is_present());
    assert!(fixture
        .stage
        .join("failed.app/Contents/Resources/new-marker")
        .exists());
}
#[test]
fn pending_and_success_results_replace_stale_failure_before_startup() {
    let fixture = Fixture::new();
    let path = fixture.home.join("startup-result.json");
    let failed = UpdateResult {
        status: UpdateStatus::Failed,
        message: "earlier failure".into(),
    };
    report_macos_result(&fixture.stage, &path, &failed).unwrap();
    for status in [UpdateStatus::LaunchPending, UpdateStatus::LaunchRequested] {
        let record = UpdateResult {
            status,
            message: "replacement verified".into(),
        };
        report_macos_result(&fixture.stage, &path, &record).unwrap();
        let result: UpdateResult = files::read_json(&path).unwrap();
        assert!(!matches!(result.status, UpdateStatus::Failed));
        assert!(!result.message.contains("earlier failure"));
    }
}
#[test]
fn full_apply_waits_for_closing_parent_then_reports_and_recovers_early_failure() {
    use std::{cell::RefCell, rc::Rc};
    let mut fixture = Fixture::new();
    let parent = Rc::new(RefCell::new(
        Command::new("/bin/sleep").arg("0.3").spawn().unwrap(),
    ));
    fixture.plan.parent_pid = parent.borrow().id();
    files::write_json(&fixture.stage.join("plan.json"), &fixture.plan).unwrap();
    fs::write(
        staged_executable(&fixture.plan.staged),
        "corrupt before parent closes",
    )
    .unwrap();
    let destination = fixture.home.join("startup-result.json");
    let mut recovery_launches = 0;
    let mut observed_parent_alive = false;
    let error = apply_macos_with(
        &fixture.stage.join("plan.json"),
        &fixture.stage.join(helper_name()),
        &destination,
        Duration::from_secs(2),
        |pid| {
            assert_eq!(pid, parent.borrow().id());
            let alive = parent.borrow_mut().try_wait().unwrap().is_none();
            observed_parent_alive |= alive;
            Ok(alive)
        },
        |plan| {
            assert!(parent.borrow_mut().try_wait().unwrap().is_some());
            assert_eq!(plan.target, fixture.plan.target);
            assert!(fixture.old_is_present());
            let before_launch: UpdateResult = files::read_json(&destination).unwrap();
            assert!(matches!(before_launch.status, UpdateStatus::Failed));
            assert!(before_launch.message.contains("checksum mismatch"));
            assert!(before_launch
                .message
                .contains(&fixture.stage.display().to_string()));
            recovery_launches += 1;
            Ok(())
        },
    )
    .unwrap_err();
    assert!(observed_parent_alive);
    assert_eq!(recovery_launches, 1);
    assert!(fixture.old_is_present());
    assert!(!fixture.stage.join("previous.app").exists());
    assert!(error.to_string().contains("checksum mismatch"));
    assert!(error
        .to_string()
        .contains(&fixture.stage.display().to_string()));
    let final_result: UpdateResult = files::read_json(&destination).unwrap();
    assert!(matches!(final_result.status, UpdateStatus::Failed));
    assert!(final_result.message.contains("Recovery files:"));
    assert!(final_result
        .message
        .contains(&fixture.stage.display().to_string()));
}
#[test]
fn full_apply_ignores_links_inside_other_applications() {
    // 0.8.1-0.8.3 helpers scanned the whole application folder and rolled
    // back beside apps with framework links, such as Electron apps.
    let fixture = Fixture::new();
    let framework = fixture
        .home
        .join("Other.app/Contents/Frameworks/Example.framework");
    fs::create_dir_all(framework.join("Versions/A")).unwrap();
    fs::write(framework.join("Versions/A/Example"), "framework").unwrap();
    std::os::unix::fs::symlink("Versions/A/Example", framework.join("Example")).unwrap();
    assert!(files::no_links(&fixture.home).is_err());
    let destination = fixture.home.join("startup-result.json");
    let mut launches = 0;
    apply_macos_with(
        &fixture.stage.join("plan.json"),
        &fixture.stage.join(helper_name()),
        &destination,
        Duration::from_secs(2),
        |_| Ok(false),
        |plan| {
            assert_eq!(plan.target, fixture.plan.target);
            launches += 1;
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(launches, 1);
    assert_eq!(
        fs::read(fixture.plan.target.join("Contents/Resources/new-marker")).unwrap(),
        b"new bytes"
    );
    assert!(fixture.stage.join("previous.app").exists());
    assert!(framework.join("Example").is_symlink());
    let result: UpdateResult = files::read_json(&destination).unwrap();
    assert!(matches!(result.status, UpdateStatus::LaunchRequested));
}
#[test]
fn trusted_failure_retains_recovery_path_and_secondary_launch_error() {
    let fixture = Fixture::new();
    fs::write(staged_executable(&fixture.plan.staged), "corrupt").unwrap();
    let destination = fixture.home.join("startup-result.json");
    let error = apply_macos_with(
        &fixture.stage.join("plan.json"),
        &fixture.stage.join(helper_name()),
        &destination,
        Duration::from_millis(20),
        |_| Ok(false),
        |_| bail!("injected recovery launch failure"),
    )
    .unwrap_err();
    let final_result: UpdateResult = files::read_json(&destination).unwrap();
    assert!(matches!(final_result.status, UpdateStatus::Failed));
    for expected in [
        "checksum mismatch",
        "Recovery files:",
        "Previous manager launch request failed",
        "injected recovery launch failure",
    ] {
        assert!(final_result.message.contains(expected));
        assert!(error.to_string().contains(expected));
    }
    assert!(final_result
        .message
        .contains(&fixture.stage.display().to_string()));
    let local: UpdateResult = files::read_json(&fixture.stage.join("result.json")).unwrap();
    assert_eq!(local.message, final_result.message);
    assert!(fixture.old_is_present());
}
#[test]
fn parent_shutdown_timeout_never_swaps_or_opens_recovery_window() {
    let fixture = Fixture::new();
    let destination = fixture.home.join("startup-result.json");
    let started = Instant::now();
    let error = apply_macos_with(
        &fixture.stage.join("plan.json"),
        &fixture.stage.join(helper_name()),
        &destination,
        Duration::from_millis(20),
        |_| Ok(true),
        |_| panic!("must not open a window while the parent is alive"),
    )
    .unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(error.to_string().contains("Manager did not close"));
    assert!(fixture.old_is_present());
    assert!(!fixture.stage.join("previous.app").exists());
    let final_result: UpdateResult = files::read_json(&destination).unwrap();
    assert!(final_result.message.contains("Recovery files:"));
    assert!(final_result
        .message
        .contains(&fixture.stage.display().to_string()));
}
#[test]
fn pretrust_failure_is_reported_without_waiting_or_launching() {
    let mut fixture = Fixture::new();
    fixture.plan.layout = 0;
    files::write_json(&fixture.stage.join("plan.json"), &fixture.plan).unwrap();
    let destination = fixture.home.join("startup-result.json");
    let error = apply_macos_with(
        &fixture.stage.join("plan.json"),
        &fixture.stage.join(helper_name()),
        &destination,
        Duration::from_millis(20),
        |_| panic!("untrusted plan must not wait"),
        |_| panic!("untrusted plan must not launch"),
    )
    .unwrap_err();
    assert!(error.to_string().contains("legacy layout"));
    let final_result: UpdateResult = files::read_json(&destination).unwrap();
    assert!(final_result.message.contains("legacy layout"));
    assert!(matches!(final_result.status, UpdateStatus::Failed));
    assert!(!fixture.stage.join("result.json").exists());
    assert!(fixture.old_is_present());
}
#[test]
fn target_changed_during_parent_shutdown_is_never_recovery_launched() {
    let fixture = Fixture::new();
    fs::write(staged_executable(&fixture.plan.staged), "corrupt").unwrap();
    let destination = fixture.home.join("startup-result.json");
    let error = apply_macos_with(
        &fixture.stage.join("plan.json"),
        &fixture.stage.join(helper_name()),
        &destination,
        Duration::from_millis(20),
        |_| {
            fs::write(
                fixture.plan.target.join("Contents/Resources/old-marker"),
                "tampered after trust validation",
            )
            .unwrap();
            Ok(false)
        },
        |_| panic!("changed target must not be launched"),
    )
    .unwrap_err();
    assert!(error.to_string().contains("checksum mismatch"));
    assert!(!fixture.stage.join("previous.app").exists());
}
