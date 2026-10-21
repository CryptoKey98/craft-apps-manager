use super::*;
use std::cell::{Cell, RefCell};
/// Opt-in real DMG transactions; never uses /Applications or changes Gatekeeper.
#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires CRAFT_TEST_MAC_IDENTITY_MANIFEST pointing to approved real DMGs"]
fn real_custom_dmg_install_reload_reinstall_and_uninstall() {
    #[derive(serde::Deserialize)]
    struct Input {
        repository: String,
        image: PathBuf,
        bundle: String,
    }
    let manifest = std::env::var_os("CRAFT_TEST_MAC_IDENTITY_MANIFEST")
        .expect("missing approved DMG manifest");
    let inputs: Vec<Input> = crate::files::read_json(Path::new(&manifest)).unwrap();
    assert!(
        inputs.len() >= 2,
        "exercise independent repository identities"
    );
    let fixture = Fixture::new();
    let applications = fixture.folder("Applications");
    let mut entries = Vec::new();
    for (i, input) in inputs.iter().enumerate() {
        let mut entry = crate::catalog::builtin().remove(0);
        entry.key = format!("customnative{i}");
        entry.repository = input.repository.to_ascii_lowercase();
        entry.title = input.repository.split('/').next_back().unwrap().into();
        entry.aliases.clear();
        entry.bundle_ids.clear();
        entry.asset_prefix = "NativeFixture".into();
        entry.scheme = crate::catalog::Scheme::Generic;
        entry.download_rule = None;
        entry.download_rules.clear();
        entries.push(entry);
    }
    crate::files::write_json(&fixture.0.join("runtime/custom-apps.json"), &entries).unwrap();
    crate::catalog::load(&fixture.0);
    let extract = |image: &Path, stage: &Path, key: &str, bundle: &str, cancel: bool| {
        let job = Job::interactive(fixture.0.join("install.log"), &Default::default());
        std::thread::scope(|scope| {
            let worker = scope.spawn(|| extract_app(image, stage, key, &job));
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !worker.is_finished() {
                if let Some(choice) = job.identity_choice() {
                    let index = choice
                        .candidates
                        .iter()
                        .position(|candidate| candidate.starts_with(&format!("{bundle} (")))
                        .expect("expected explicit bundle choice");
                    choice.answer((!cancel).then_some(index));
                    break;
                }
                if std::time::Instant::now() > deadline {
                    job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            worker.join().unwrap()
        })
    };
    let mut installed = Vec::new();
    for (i, input) in inputs.iter().enumerate() {
        let key = &entries[i].key;
        assert!(crate::app_identity::native(key).is_empty());
        let cancelled = extract(
            &input.image,
            &fixture.folder(&format!("cancel-{i}")),
            key,
            &input.bundle,
            true,
        )
        .unwrap_err();
        assert!(
            crate::jobs::is_cancelled(&cancelled),
            "expected choice cancellation, got: {cancelled:#}"
        );
        assert!(crate::app_identity::native(key).is_empty());
        let staged = extract(
            &input.image,
            &fixture.folder(&format!("extract-{i}")),
            key,
            &input.bundle,
            false,
        )
        .unwrap();
        let current = publish_bundle(&staged, &applications, key, &NativeInstall).unwrap();
        assert_eq!(
            resolve_bundle(&applications, key),
            Some(applications.join(&input.bundle))
        );
        assert!(!current.product_code.is_empty());
        crate::catalog::load(&fixture.0);
        let detected = detect_in_folders(key, std::slice::from_ref(&applications))
            .unwrap()
            .unwrap();
        assert_eq!(detected.product_code, current.product_code);
        let staged = extract(
            &input.image,
            &fixture.folder(&format!("reinstall-{i}")),
            key,
            &input.bundle,
            false,
        )
        .unwrap();
        let current = publish_bundle(&staged, &applications, key, &NativeInstall).unwrap();
        let mut wrong = current.clone();
        wrong.product_code = "unrelated.identifier".into();
        assert!(uninstall_in_folders(&wrong, std::slice::from_ref(&applications)).is_err());
        assert!(applications.join(&input.bundle).is_dir());
        println!(
            "Verified real DMG install, reload, reinstall, cancellation and identity: {} ({})",
            input.repository, current.product_code
        );
        installed.push(current);
    }
    // Another signed app cannot replace an already-bound repository.
    assert!(extract(
        &inputs[1].image,
        &fixture.folder("wrong-repository"),
        &entries[0].key,
        &inputs[1].bundle,
        false
    )
    .is_err());
    for (i, current) in installed.iter().enumerate() {
        uninstall_in_folders(current, std::slice::from_ref(&applications)).unwrap();
        assert!(!applications.join(&inputs[i].bundle).exists());
        for other in inputs.iter().skip(i + 1) {
            assert!(applications.join(&other.bundle).is_dir());
        }
        println!("Verified selected-app removal: {}", inputs[i].repository);
    }
    crate::catalog::load(&fixture.0.join("empty"));
}

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("craft-bundles-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn folder(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir_all(&path).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn bundle(folder: &Path, name: &str, id: &str, version: &str) -> PathBuf {
    let path = folder.join(name);
    fs::create_dir_all(path.join("Contents")).unwrap();
    fs::write(path.join("Contents/Info.plist"), format!(r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>{id}</string><key>CFBundleShortVersionString</key><string>{version}</string></dict></plist>"#)).unwrap();
    fs::write(path.join("payload"), version).unwrap();
    path
}
#[derive(Default)]
struct Ops {
    running: bool,
    rename_count: Cell<usize>,
    fail_renames: Vec<usize>,
    verify_count: Cell<usize>,
    fail_verify: usize,
    corrupt_published: bool,
    fail_remember: bool,
    calls: RefCell<Vec<&'static str>>,
}
impl InstallOps for Ops {
    fn running(&self, _: &str) -> Result<bool> {
        self.calls.borrow_mut().push("running");
        Ok(self.running)
    }
    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        self.calls.borrow_mut().push("rename");
        let n = self.rename_count.get() + 1;
        self.rename_count.set(n);
        if self.fail_renames.contains(&n) {
            bail!("injected rename {n}");
        }
        fs::rename(from, to)?;
        Ok(())
    }
    fn verify(&self, path: &Path) -> Result<()> {
        self.calls.borrow_mut().push("verify");
        let n = self.verify_count.get() + 1;
        self.verify_count.set(n);
        if self.fail_verify == n {
            bail!("injected verification {n}");
        }
        if self.corrupt_published && n == 2 {
            let plist = path.join("Contents/Info.plist");
            let contents = fs::read_to_string(&plist)?;
            fs::write(plist, contents.replace("0.4.0", "99.0.0"))?;
        }
        Ok(())
    }
    fn remember(&self, _: &str, _: &str, _: &str) -> Result<()> {
        if self.fail_remember {
            bail!("injected identity persistence failure");
        }
        Ok(())
    }
}
fn rollback_dirs(folder: &Path) -> Vec<PathBuf> {
    fs::read_dir(folder)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".craft-rollback-")
        })
        .collect()
}
#[test]
fn custom_renamed_update_refuses_destination_copy_and_rolls_back_identity_save_failure() {
    for collision in [false, true] {
        let f = Fixture::new();
        let folder = f.folder("Applications");
        let stage = f.folder("stage");
        bundle(
            &folder,
            "PhotoCraft.app",
            "ai.storyteller.photocraft",
            "0.2.0",
        );
        if collision {
            bundle(&folder, "Renamed.app", "ai.storyteller.photocraft", "0.1.0");
        }
        let staged = bundle(&stage, "Renamed.app", "ai.storyteller.photocraft", "0.4.0");
        let ops = Ops {
            fail_remember: true,
            ..Default::default()
        };
        assert!(publish_bundle_with_policy(&staged, &folder, "photocraft", &ops, false).is_err());
        assert_eq!(
            fs::read(folder.join("PhotoCraft.app/payload")).unwrap(),
            b"0.2.0"
        );
        if collision {
            assert_eq!(ops.rename_count.get(), 0);
            assert_eq!(
                fs::read(folder.join("Renamed.app/payload")).unwrap(),
                b"0.1.0"
            );
        } else {
            assert!(!folder.join("Renamed.app").exists());
        }
    }
}
#[test]
fn custom_update_preserves_duplicate_id_manual_copy() {
    let f = Fixture::new();
    let folder = f.folder("Applications");
    let stage = f.folder("stage");
    let copy = bundle(&folder, "AaaCopy.app", "ai.storyteller.photocraft", "0.1.0");
    bundle(
        &folder,
        "PhotoCraft.app",
        "ai.storyteller.photocraft",
        "0.2.0",
    );
    let staged = bundle(&stage, "Renamed.app", "ai.storyteller.photocraft", "0.4.0");
    let installed =
        publish_bundle_with_policy(&staged, &folder, "photocraft", &Ops::default(), false).unwrap();
    assert_eq!(installed.version, "0.4.0");
    assert_eq!(fs::read(copy.join("payload")).unwrap(), b"0.1.0");
    assert!(!folder.join("PhotoCraft.app").exists());
    assert!(folder.join("Renamed.app").exists());
    assert_eq!(
        fs::read(rollback_dirs(&folder)[0].join("PhotoCraft.app/payload")).unwrap(),
        b"0.2.0"
    );
    assert!(!rollback_dirs(&folder)[0].join("AaaCopy.app").exists());
}
#[test]
fn uninstall_removes_selected_bundle_when_duplicate_ids_exist() {
    let f = Fixture::new();
    let folder = f.folder("Applications");
    let copy = bundle(&folder, "AaaCopy.app", "ai.storyteller.photocraft", "0.1.0");
    let selected = bundle(
        &folder,
        "PhotoCraft.app",
        "ai.storyteller.photocraft",
        "0.4.0",
    );
    let installed = installed_bundle(&selected, "photocraft").unwrap();
    assert_eq!(
        resolve_bundle(&folder, "photocraft"),
        Some(selected.clone())
    );
    uninstall_in_folders(&installed, std::slice::from_ref(&folder)).unwrap();
    assert!(!selected.exists());
    assert!(copy.exists());
}
#[test]
fn unknown_custom_bundle_is_not_adopted_from_its_guessed_name_or_id() {
    let f = Fixture::new();
    let folder = f.folder("Applications");
    let candidate = bundle(
        &folder,
        "UnregisteredFixture.app",
        "ai.storyteller.unregisteredfixture",
        "1.0.0",
    );
    assert_eq!(
        bundle_identity(&candidate).unwrap(),
        "ai.storyteller.unregisteredfixture"
    );
    assert!(verify_identity(&candidate, "unregisteredfixture").is_err());
    assert!(resolve_bundle(&folder, "unregisteredfixture").is_none());
}
#[test]
#[cfg(unix)]
fn resolver_does_not_follow_renamed_bundle_or_metadata_links() {
    let f = Fixture::new();
    let folder = f.folder("Applications");
    let outside = f.folder("outside");
    let target = bundle(&outside, "Real.app", "ai.storyteller.photocraft", "1.0.0");
    std::os::unix::fs::symlink(&target, folder.join("Linked.app")).unwrap();
    let candidate = folder.join("Metadata.app");
    fs::create_dir(&candidate).unwrap();
    std::os::unix::fs::symlink(target.join("Contents"), candidate.join("Contents")).unwrap();
    assert!(resolve_bundle(&folder, "photocraft").is_none());
}
#[test]
fn resolver_finds_renamed_bundle_only_by_exact_identity() {
    let f = Fixture::new();
    let folder = f.folder("Applications");
    bundle(&folder, "Unrelated.app", "another.owner", "99");
    let renamed = bundle(
        &folder,
        "Different.app",
        "ai.storyteller.photocraft",
        "0.4.0",
    );
    assert_eq!(resolve_bundle(&folder, "photocraft"), Some(renamed));
}
#[test]
fn renamed_bundle_update_preserves_identity_and_previous_bytes() {
    let f = Fixture::new();
    let folder = f.folder("Applications");
    let stage = f.folder("stage");
    bundle(&folder, "OldName.app", "ai.storyteller.photocraft", "0.1.0");
    let staged = bundle(&stage, "NewName.app", "ai.storyteller.photocraft", "0.4.0");
    let installed = publish_bundle(&staged, &folder, "photocraft", &Ops::default()).unwrap();
    assert_eq!(installed.product_code, "ai.storyteller.photocraft");
    assert!(!folder.join("OldName.app").exists());
    assert_eq!(
        fs::read(rollback_dirs(&folder)[0].join("OldName.app/payload")).unwrap(),
        b"0.1.0"
    );
}
#[test]
#[cfg(unix)]
fn resolver_prefers_valid_current_alias_and_preserves_folder_precedence() {
    let f = Fixture::new();
    let system = f.folder("system");
    let user = f.folder("user");
    let legacy = bundle(
        &system,
        "PrintCraft.app",
        "ai.storyteller.printcraft",
        "0.2.1",
    );
    bundle(&user, "PdfCraft.app", "ai.storyteller.pdfcraft", "0.4.0");
    assert_eq!(
        detect_in_folders("printcraft", &[system.clone(), user])
            .unwrap()
            .unwrap()
            .version,
        "0.2.1"
    );
    let canonical = bundle(&system, "PdfCraft.app", "unrelated.owner", "99");
    assert_eq!(resolve_bundle(&system, "printcraft"), Some(legacy.clone()));
    fs::remove_dir_all(&canonical).unwrap();
    std::os::unix::fs::symlink(&legacy, &canonical).unwrap();
    assert_eq!(resolve_bundle(&system, "printcraft"), Some(legacy));
    fs::remove_file(&canonical).unwrap();
    bundle(&system, "PdfCraft.app", "ai.storyteller.pdfcraft", "0.4.0");
    assert_eq!(resolve_bundle(&system, "printcraft"), Some(canonical));
}
#[test]
fn migration_moves_both_owned_aliases_and_uninstall_uses_current_bundle() {
    for both in [false, true] {
        let f = Fixture::new();
        let folder = f.folder("Applications");
        let other = f.folder("other-Applications");
        let stage = f.folder("stage");
        bundle(
            &folder,
            "PrintCraft.app",
            "ai.storyteller.printcraft",
            "0.2.1",
        );
        if both {
            bundle(&folder, "PdfCraft.app", "ai.storyteller.pdfcraft", "0.3.0");
        }
        let untouched = bundle(
            &other,
            "PrintCraft.app",
            "ai.storyteller.printcraft",
            "0.2.1",
        );
        let staged = bundle(&stage, "PdfCraft.app", "ai.storyteller.pdfcraft", "0.4.0");
        let installed = publish_bundle(&staged, &folder, "printcraft", &Ops::default()).unwrap();
        assert_eq!(installed.version, "0.4.0");
        assert!(!folder.join("PrintCraft.app").exists());
        assert!(untouched.exists());
        assert_eq!(rollback_dirs(&folder).len(), 1);
        assert_eq!(
            fs::read(rollback_dirs(&folder)[0].join("PrintCraft.app/payload")).unwrap(),
            b"0.2.1"
        );
        assert_eq!(
            detect_in_folders("printcraft", std::slice::from_ref(&folder))
                .unwrap()
                .unwrap()
                .version,
            "0.4.0"
        );
        uninstall_in_folders(&installed, std::slice::from_ref(&folder)).unwrap();
        assert!(!folder.join("PdfCraft.app").exists());
        assert!(detect_in_folders("printcraft", &[folder])
            .unwrap()
            .is_none());
    }
}
#[test]
fn ordinary_same_name_update_retains_one_previous_set() {
    let f = Fixture::new();
    let folder = f.folder("Applications");
    let stage = f.folder("stage");
    bundle(
        &folder,
        "PhotoCraft.app",
        "ai.storyteller.photocraft",
        "0.1.0",
    );
    for version in ["0.2.0", "0.3.0"] {
        let staged = bundle(
            &stage,
            "PhotoCraft.app",
            "ai.storyteller.photocraft",
            version,
        );
        publish_bundle(&staged, &folder, "photocraft", &Ops::default()).unwrap();
    }
    assert_eq!(rollback_dirs(&folder).len(), 1);
    assert_eq!(
        fs::read(rollback_dirs(&folder)[0].join("PhotoCraft.app/payload")).unwrap(),
        b"0.2.0"
    );
}
#[test]
#[cfg(unix)]
fn install_refuses_unrelated_and_link_destinations_without_moving_legacy() {
    for linked in [false, true] {
        let f = Fixture::new();
        let folder = f.folder("Applications");
        let stage = f.folder("stage");
        let legacy = bundle(
            &folder,
            "PrintCraft.app",
            "ai.storyteller.printcraft",
            "0.2.1",
        );
        if linked {
            std::os::unix::fs::symlink(&legacy, folder.join("PdfCraft.app")).unwrap();
        } else {
            bundle(&folder, "PdfCraft.app", "another.owner", "99");
        }
        let staged = bundle(&stage, "PdfCraft.app", "ai.storyteller.pdfcraft", "0.4.0");
        let ops = Ops::default();
        assert!(publish_bundle(&staged, &folder, "printcraft", &ops).is_err());
        assert_eq!(ops.rename_count.get(), 0);
        assert!(legacy.exists());
    }
}
#[test]
fn running_recheck_happens_after_stage_verification_before_old_bundle_moves() {
    let f = Fixture::new();
    let folder = f.folder("Applications");
    let stage = f.folder("stage");
    let old = bundle(
        &folder,
        "PrintCraft.app",
        "ai.storyteller.printcraft",
        "0.2.1",
    );
    let staged = bundle(&stage, "PdfCraft.app", "ai.storyteller.pdfcraft", "0.4.0");
    let ops = Ops {
        running: true,
        ..Default::default()
    };
    assert!(publish_bundle(&staged, &folder, "printcraft", &ops)
        .unwrap_err()
        .to_string()
        .contains("Close the app"));
    assert_eq!(*ops.calls.borrow(), ["verify", "running"]);
    assert!(old.exists());
    assert!(rollback_dirs(&folder).is_empty());
}
#[test]
fn move_publish_and_postvalidation_failures_restore_every_alias() {
    for (fail_renames, fail_verify) in [(vec![2], 0), (vec![3], 0), (vec![], 2)] {
        let f = Fixture::new();
        let folder = f.folder("Applications");
        let stage = f.folder("stage");
        bundle(&folder, "PdfCraft.app", "ai.storyteller.pdfcraft", "0.3.0");
        bundle(
            &folder,
            "PrintCraft.app",
            "ai.storyteller.printcraft",
            "0.2.1",
        );
        let staged = bundle(&stage, "PdfCraft.app", "ai.storyteller.pdfcraft", "0.4.0");
        let ops = Ops {
            fail_renames,
            fail_verify,
            ..Default::default()
        };
        assert!(publish_bundle(&staged, &folder, "printcraft", &ops).is_err());
        assert_eq!(
            fs::read(folder.join("PrintCraft.app/payload")).unwrap(),
            b"0.2.1"
        );
        assert_eq!(
            fs::read(folder.join("PdfCraft.app/payload")).unwrap(),
            b"0.3.0"
        );
        assert!(rollback_dirs(&folder).is_empty());
    }
}
#[test]
fn exact_destination_version_is_checked_before_commit() {
    let f = Fixture::new();
    let folder = f.folder("Applications");
    let stage = f.folder("stage");
    bundle(
        &folder,
        "PrintCraft.app",
        "ai.storyteller.printcraft",
        "0.2.1",
    );
    let staged = bundle(&stage, "PdfCraft.app", "ai.storyteller.pdfcraft", "0.4.0");
    let ops = Ops {
        corrupt_published: true,
        ..Default::default()
    };
    let error = publish_bundle(&staged, &folder, "printcraft", &ops).unwrap_err();
    assert!(error.to_string().contains("does not match"));
    assert_eq!(
        fs::read(folder.join("PrintCraft.app/payload")).unwrap(),
        b"0.2.1"
    );
    assert!(!folder.join("PdfCraft.app").exists());
}
#[test]
fn invalid_stage_is_rejected_before_moving_installed_app() {
    let f = Fixture::new();
    let folder = f.folder("Applications");
    let stage = f.folder("stage");
    let old = bundle(
        &folder,
        "PrintCraft.app",
        "ai.storyteller.printcraft",
        "0.2.1",
    );
    let staged = bundle(&stage, "PdfCraft.app", "ai.storyteller.pdfcraft", "0.4.0");
    let ops = Ops {
        fail_verify: 1,
        ..Default::default()
    };
    assert!(publish_bundle(&staged, &folder, "printcraft", &ops).is_err());
    assert_eq!(ops.rename_count.get(), 0);
    assert!(old.exists());
    assert!(rollback_dirs(&folder).is_empty());
}
#[test]
fn rollback_failure_keeps_old_bytes_outside_stage_and_preserves_original_error() {
    let f = Fixture::new();
    let folder = f.folder("Applications");
    let stage = f.folder("stage");
    bundle(
        &folder,
        "PrintCraft.app",
        "ai.storyteller.printcraft",
        "0.2.1",
    );
    let staged = bundle(&stage, "PdfCraft.app", "ai.storyteller.pdfcraft", "0.4.0");
    let ops = Ops {
        fail_renames: vec![2, 3],
        ..Default::default()
    };
    let error = publish_bundle(&staged, &folder, "printcraft", &ops).unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("injected rename 2"));
    assert!(message.contains("injected rename 3"));
    assert!(message.contains("Recovery incomplete"));
    fs::remove_dir_all(stage).unwrap();
    assert_eq!(
        fs::read(rollback_dirs(&folder)[0].join("PrintCraft.app/payload")).unwrap(),
        b"0.2.1"
    );
    // Later successful installs must not prune failed recovery evidence.
    let next_stage = f.folder("next-stage");
    let staged = bundle(
        &next_stage,
        "PdfCraft.app",
        "ai.storyteller.pdfcraft",
        "0.4.0",
    );
    publish_bundle(&staged, &folder, "printcraft", &Ops::default()).unwrap();
    assert_eq!(rollback_dirs(&folder).len(), 1);
}
#[test]
fn failed_withdrawal_preserves_previous_canonical_bytes_and_restores_legacy() {
    let f = Fixture::new();
    let folder = f.folder("Applications");
    let stage = f.folder("stage");
    bundle(&folder, "PdfCraft.app", "ai.storyteller.pdfcraft", "0.3.0");
    bundle(
        &folder,
        "PrintCraft.app",
        "ai.storyteller.printcraft",
        "0.2.1",
    );
    let staged = bundle(&stage, "PdfCraft.app", "ai.storyteller.pdfcraft", "0.4.0");
    let ops = Ops {
        fail_renames: vec![4],
        fail_verify: 2,
        ..Default::default()
    };
    let error = publish_bundle(&staged, &folder, "printcraft", &ops).unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("injected verification 2"));
    assert!(message.contains("injected rename 4"));
    fs::remove_dir_all(stage).unwrap();
    assert_eq!(
        fs::read(rollback_dirs(&folder)[0].join("PdfCraft.app/payload")).unwrap(),
        b"0.3.0"
    );
    assert_eq!(
        fs::read(folder.join("PrintCraft.app/payload")).unwrap(),
        b"0.2.1"
    );
}
/// Run explicitly on macOS with CRAFT_TEST_LEGACY_APP pointing to an
/// official PrintCraft 0.2.1 bundle and CRAFT_TEST_CURRENT_APP pointing to
/// an official PdfCraft 0.4.0 bundle. Optionally set CRAFT_TEST_CURRENT_DMG
/// to its official release DMG to exercise extraction as well.
/// All mutations use temporary copies;
/// no installed apps, user preferences, profiles or launch agents are used.
#[test]
#[ignore = "requires official signed macOS release bundle fixtures"]
fn official_renamed_release_migrates_in_isolated_applications_folders() {
    let legacy = PathBuf::from(
        std::env::var_os("CRAFT_TEST_LEGACY_APP").expect("Set CRAFT_TEST_LEGACY_APP"),
    );
    let current = PathBuf::from(
        std::env::var_os("CRAFT_TEST_CURRENT_APP").expect("Set CRAFT_TEST_CURRENT_APP"),
    );
    assert_eq!(platform::executable_version(&legacy).unwrap(), "0.2.1");
    assert_eq!(platform::executable_version(&current).unwrap(), "0.4.0");
    let copy = |from: &Path, to: &Path| {
        assert!(Command::new("/usr/bin/ditto")
            .args(["--noextattr", "--norsrc"])
            .arg(from)
            .arg(to)
            .status()
            .unwrap()
            .success());
    };
    for both in [false, true] {
        let f = Fixture::new();
        let folder = f.folder("Applications");
        let stage = f.folder("stage");
        copy(&legacy, &folder.join("PrintCraft.app"));
        if both {
            copy(&current, &folder.join("PdfCraft.app"));
        }
        let staged = stage.join("PdfCraft.app");
        if let Some(dmg) = std::env::var_os("CRAFT_TEST_CURRENT_DMG") {
            let job = Job::new(f.0.join("install.log"), &Default::default());
            assert_eq!(
                extract_app(&PathBuf::from(dmg), &stage, "printcraft", &job).unwrap(),
                staged
            );
        } else {
            copy(&current, &staged);
        }
        let installed = publish_bundle(&staged, &folder, "printcraft", &NativeInstall).unwrap();
        assert_eq!(installed.version, "0.4.0");
        assert_eq!(
            resolve_bundle(&folder, "printcraft"),
            Some(folder.join("PdfCraft.app"))
        );
        assert!(!folder.join("PrintCraft.app").exists());
        let detected = detect_in_folders("printcraft", std::slice::from_ref(&folder))
            .unwrap()
            .unwrap();
        assert_eq!(detected.version, "0.4.0");
        assert!(
            crate::updates::version(&detected.version).unwrap()
                >= crate::updates::version("0.4.0").unwrap()
        );
        assert_eq!(
            fs::read(folder.join("PdfCraft.app/Contents/MacOS/PdfCraft")).unwrap(),
            fs::read(current.join("Contents/MacOS/PdfCraft")).unwrap()
        );
        assert_eq!(
            fs::read(rollback_dirs(&folder)[0].join("PrintCraft.app/Contents/MacOS/PrintCraft"))
                .unwrap(),
            fs::read(legacy.join("Contents/MacOS/PrintCraft")).unwrap()
        );
        uninstall_in_folders(&installed, std::slice::from_ref(&folder)).unwrap();
        assert!(detect_in_folders("printcraft", &[folder])
            .unwrap()
            .is_none());
    }
}
#[test]
fn bundle_identities_cover_renamed_repository() {
    assert!(identities("printcraft").contains(&"ai.storyteller.pdfcraft".to_string()));
    assert!(identities("photocraft").contains(&"ai.storyteller.photocraft".to_string()));
    #[cfg(target_os = "macos")]
    assert_eq!(
        crate::model::executable_names("printcraft"),
        ["PdfCraft.app", "PrintCraft.app"]
    );
    #[cfg(target_os = "macos")]
    assert_eq!(
        crate::model::executable_name("photocraft"),
        "PhotoCraft.app"
    );
}
