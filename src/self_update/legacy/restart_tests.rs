use super::*;

#[test]
fn replacement_retains_the_operation_lock_but_restart_can_load_existing_settings() {
    let root = std::env::temp_dir().join(format!("manager-restart-lock-{}", uuid::Uuid::new_v4()));
    files::write_bytes(&root.join("config.yaml"), br#"{"schema_version":1,"manager":{},"builder":{},"custom_apps":[],"launch":{},"build_launch":{},"legacy_extensions":{}}"#).unwrap();
    let guard = crate::platform::Lock::take("Local\\CraftAppsManager").unwrap();
    let startup_root = root.clone();
    let result = commit_then_restart(
        guard,
        || {
            assert!(std::thread::spawn(
                || crate::platform::Lock::take("Local\\CraftAppsManager").is_err()
            )
            .join()
            .unwrap());
            Ok(())
        },
        || {
            std::thread::spawn(move || {
                crate::storage::config::migration::ensure_migrated(
                    &crate::storage::config::ConfigStore::new(startup_root),
                )
                .map(|_| ())
            })
            .join()
            .unwrap()
        },
        || panic!("A successful restart must not restore the old executable"),
    );
    let _ = fs::remove_dir_all(&root);
    assert!(result.is_ok(), "Updated Manager startup failed: {result:?}");
}

fn lock_is_held(name: &'static str) -> bool {
    std::thread::spawn(move || crate::platform::Lock::take(name).is_err())
        .join()
        .unwrap()
}

#[test]
fn failed_commit_keeps_its_guard_and_never_attempts_restart() {
    let guard = crate::platform::Lock::take("Local\\CraftAppsManager").unwrap();
    let recovered = std::cell::Cell::new(false);
    let result = commit_then_restart(
        guard,
        || anyhow::bail!("injected replacement failure"),
        || panic!("Failed replacement must not launch the new Manager"),
        || {
            assert!(lock_is_held("Local\\CraftAppsManager"));
            recovered.set(true);
            Ok(())
        },
    );
    assert!(recovered.get());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("replacement failure"));
}

#[test]
fn failed_restart_reacquires_execution_ownership_before_recovery() {
    let builder = crate::platform::Lock::take("Local\\CraftAppsSourceBuilder").unwrap();
    let manager = crate::platform::Lock::take("Local\\CraftAppsManager").unwrap();
    let recovered = std::cell::Cell::new(false);
    let result = commit_then_restart(
        (builder, manager),
        || Ok(()),
        || {
            assert!(!lock_is_held("Local\\CraftAppsSourceBuilder"));
            assert!(!lock_is_held("Local\\CraftAppsManager"));
            anyhow::bail!("injected launch failure")
        },
        || {
            assert!(lock_is_held("Local\\CraftAppsSourceBuilder"));
            assert!(lock_is_held("Local\\CraftAppsManager"));
            recovered.set(true);
            Ok(())
        },
    );
    assert!(recovered.get());
    assert!(result.unwrap_err().to_string().contains("launch failure"));
}
