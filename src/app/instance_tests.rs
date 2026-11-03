use super::*;

#[test]
fn duplicate_window_is_rejected_but_builder_and_other_libraries_are_independent() {
    let root = std::env::temp_dir().join(format!("craft-instance-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let manager = Instance::claim(&root, false).unwrap().unwrap();
    assert!(!manager.take_activation());
    assert!(Instance::claim(&root, false).unwrap().is_none());
    assert!(manager.take_activation());
    assert!(!manager.take_activation());
    assert!(Instance::claim(&root.join("."), false).unwrap().is_none());
    let builder = Instance::claim(&root, true).unwrap().unwrap();
    assert!(Instance::claim(&root, true).unwrap().is_none());
    let other = Instance::claim(&root.join("other"), false)
        .unwrap()
        .unwrap();
    drop((manager, builder, other));
    let restarted = Instance::claim(&root, false).unwrap().unwrap();
    drop(restarted);
    std::fs::remove_dir_all(root).unwrap();
}

fn temporary_root() -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("craft-instance-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn headless_commands_do_not_claim_or_activate_gui_slots() {
    let root = temporary_root();
    let owner = Instance::claim(&root, false).unwrap().unwrap();
    for command in [
        "--build-identity",
        "--native-bridge-admin",
        "--approved-admin-batch",
        "--apply-self-update",
        "--check-app-updates",
        "--check-source-updates",
        "--install-shortcuts",
        "--list-apps",
        "--install-app",
        "--uninstall-app",
        "--update",
        "--update-source",
        "--diagnostics",
        "--release-format",
        "--architecture",
    ] {
        let args = vec!["manager".into(), command.into()];
        assert!(!super::is_gui_launch(&args), "Headless command: {command}");
    }
    assert!(super::is_gui_launch(&["manager".into()]));
    assert!(super::is_gui_launch(&[
        "manager".into(),
        "--builder".into()
    ]));
    assert!(!owner.take_activation());
    drop(owner);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn simultaneous_launches_elect_one_owner() {
    let root = temporary_root();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let owners = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let (root, barrier, owners) = (root.clone(), barrier.clone(), owners.clone());
            std::thread::spawn(move || {
                barrier.wait();
                let owner = Instance::claim(&root, false).unwrap();
                if owner.is_some() {
                    owners.fetch_add(1, Ordering::SeqCst);
                }
                barrier.wait();
                assert_eq!(owners.load(Ordering::SeqCst), 1);
                drop(owner);
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn activation_requires_current_nonce_and_wakes_the_owner() {
    let root = temporary_root();
    let owner = Instance::claim(&root, false).unwrap().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    owner.set_wake(move || {
        let _ = tx.send(());
    });
    let mut bad = TcpStream::connect(owner.address).unwrap();
    bad.write_all(&[b'x'; 36]).unwrap();
    bad.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    assert_ne!(bad.read(&mut [0; 7]).unwrap_or(0), 7);
    assert!(!owner.take_activation());
    assert!(Instance::claim(&root, false).unwrap().is_none());
    rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(owner.take_activation());
    drop(owner);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn stale_endpoint_is_replaced_and_lock_errors_are_not_duplicates() {
    let root = temporary_root();
    std::fs::create_dir_all(root.join("runtime/windows")).unwrap();
    std::fs::write(root.join("runtime/windows/manager.json"), b"stale").unwrap();
    let owner = Instance::claim(&root, false).unwrap().unwrap();
    assert!(Instance::claim(&root, false).unwrap().is_none());
    drop(owner);
    std::fs::create_dir(root.join("runtime/windows/builder.lock")).unwrap();
    assert!(Instance::claim(&root, true).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

struct Child(std::process::Child);
impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn child(root: &Path, action: &str) -> Child {
    Child(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "app::instance::tests::instance_child",
                "--ignored",
                "--nocapture",
            ])
            .env("CRAFT_INSTANCE_TEST_ROOT", root)
            .env("CRAFT_INSTANCE_TEST_ACTION", action)
            .spawn()
            .unwrap(),
    )
}

#[test]
#[ignore = "launched by the cross-process ownership test"]
fn instance_child() {
    let root = std::path::PathBuf::from(std::env::var_os("CRAFT_INSTANCE_TEST_ROOT").unwrap());
    let action = std::env::var("CRAFT_INSTANCE_TEST_ACTION").unwrap();
    if action == "duplicate" {
        assert!(Instance::claim(&root, false).unwrap().is_none());
    } else {
        let _owner = Instance::claim(&root, false).unwrap().unwrap();
        std::fs::write(root.join("ready"), b"ready").unwrap();
        loop {
            std::thread::park();
        }
    }
}

#[test]
fn another_process_activates_owner_and_crash_releases_ownership() {
    let root = temporary_root();
    let owner = Instance::claim(&root, false).unwrap().unwrap();
    let mut duplicate = child(&root, "duplicate");
    assert!(duplicate.0.wait().unwrap().success());
    assert!(owner.take_activation());
    drop(owner);
    let mut crashed = child(&root, "hold");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !root.join("ready").exists() {
        assert!(
            crashed.0.try_wait().unwrap().is_none(),
            "Owner exited before becoming ready"
        );
        assert!(Instant::now() < deadline, "Owner did not become ready");
        std::thread::sleep(Duration::from_millis(10));
    }
    crashed.0.kill().unwrap();
    crashed.0.wait().unwrap();
    let recovered = Instance::claim(&root, false).unwrap().unwrap();
    drop(recovered);
    std::fs::remove_dir_all(root).unwrap();
}
