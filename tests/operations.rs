//! Real process boundaries with simulated work; no installers or system changes.
use craft_apps_manager::{
    files,
    jobs::{is_cancelled, Job},
    operations::{Lifecycle, Manager},
};
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
fn root() -> PathBuf {
    std::env::temp_dir().join(format!("operation-test-{}", uuid::Uuid::new_v4()))
}
fn wait(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !predicate() {
        assert!(Instant::now() < deadline, "Worker did not resolve");
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn simulated_process_worker() {
    let Some(root) = std::env::var_os("CRAFT_OPERATION_TEST_ROOT") else {
        return;
    };
    let job = Job::new(
        PathBuf::from(root).join("logs/mock.log"),
        &Default::default(),
    );
    let result = job.tracked("Build", "fixture", |job| {
        job.stage("Compiling", None, "Simulated compilation only");
        for _ in 0..1000 {
            job.check()?;
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    });
    assert!(result.is_err_and(|error| is_cancelled(&error)));
}
#[test]
fn separate_process_observation_and_cancellation_resolve() {
    let root = root();
    let mut process = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "simulated_process_worker", "--test-threads=1"])
        .env("CRAFT_OPERATION_TEST_ROOT", &root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let manager = Manager::new(&root);
    wait(|| {
        manager
            .list()
            .iter()
            .any(|op| op.lifecycle == Lifecycle::Running)
    });
    let operation = manager.list().remove(0);
    assert_ne!(operation.owner, std::process::id());
    assert!(Manager::new(&root).request_cancel(&operation.id).unwrap());
    wait(|| process.try_wait().unwrap().is_some());
    assert!(process.wait().unwrap().success());
    assert_eq!(
        manager.read(&operation.id).unwrap().lifecycle,
        Lifecycle::Cancelled
    );
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn worker_disappearance_is_unknown_and_never_confirmed_success() {
    let root = root();
    let manager = Manager::new(&root);
    let handle = manager.register("Install", "fixture", None);
    handle.start();
    let mut record = manager.read(&handle.id()).unwrap();
    record.heartbeat = chrono::Utc::now().timestamp_millis() - 60_000;
    files::write_json(
        &root.join(format!("runtime/operations/{}.json", record.id)),
        &record,
    )
    .unwrap();
    let observed = manager.list().remove(0);
    assert_eq!(observed.lifecycle, Lifecycle::Interrupted);
    assert!(observed.result.unwrap().contains("unknown"));
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn duplicate_start_and_late_worker_updates_do_not_corrupt_new_operation() {
    let root = root();
    let mut job = Job::new(root.join("logs/mock.log"), &Default::default());
    job.describe("Build", "first");
    let (tx, rx) = std::sync::mpsc::channel();
    let (done, stop) = std::sync::mpsc::channel();
    job.spawn(move |worker| {
        tx.send(worker).unwrap();
        stop.recv().unwrap();
        Ok(())
    });
    let old_worker = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    job.spawn(|_| panic!("duplicate must not start"));
    assert_eq!(job.operations.list().len(), 1);
    done.send(()).unwrap();
    wait(|| !job.snapshot().busy);
    job.describe("Build", "second");
    let (done, stop) = std::sync::mpsc::channel();
    job.spawn(move |worker| {
        worker.stage("Compiling", None, "second");
        stop.recv().unwrap();
        Ok(())
    });
    old_worker.stage("Installing", Some(0.7), "stale update");
    assert_ne!(job.snapshot().stage, "Installing");
    done.send(()).unwrap();
    wait(|| !job.snapshot().busy);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn mixed_batch_bridge_keeps_ids_options_and_individual_results() {
    let root = root();
    let parent = Job::new(root.join("logs/mock.log"), &Default::default());
    let result = parent.tracked("Bulk update", "Application library", |parent| {
        parent.prepare_child("native", "Update", "installer", "x64");
        let portable = parent.prepare_child("portable", "Update", "portable", "arm64");
        parent.prepare_child("pending", "Update", "portable", "x64");
        let native_ids = parent
            .child_ids()
            .into_iter()
            .filter(|(app, _)| app == "native")
            .collect();
        let mut bridge = Job::new(
            root.join("runtime/admin-batches/mock/installers.log"),
            &Default::default(),
        );
        bridge
            .attach_children(&root, parent.operation_id(), &native_ids)
            .unwrap();
        bridge
            .prepare_child("native", "Update", "installer", "x64")
            .tracked("Update", "native", |job| {
                job.stage("Running installer", None, "Simulated approved installer");
                Ok(())
            })
            .unwrap();
        portable.tracked::<()>("Update", "portable", |_| {
            Err(anyhow::anyhow!("Simulated extraction failure"))
        })?;
        Ok(())
    });
    assert!(result.is_err());
    let records = parent.operations.list();
    let find = |name: &str| records.iter().find(|op| op.resource == name).unwrap();
    assert_eq!(find("native").lifecycle, Lifecycle::Completed);
    assert_eq!(find("native").format.as_deref(), Some("installer"));
    assert_eq!(find("portable").lifecycle, Lifecycle::Failed);
    assert_eq!(find("portable").architecture.as_deref(), Some("arm64"));
    assert_eq!(find("pending").lifecycle, Lifecycle::Interrupted);
    assert_eq!(find("Application library").lifecycle, Lifecycle::Failed);
    assert_eq!(records.len(), 4);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn cancellation_keeps_completed_children_and_confirms_only_unstarted_ones() {
    let root = root();
    let parent = Job::new(root.join("logs/mock.log"), &Default::default());
    let error = parent
        .tracked::<()>("Bulk update", "Application library", |job| {
            job.prepare_child("finished", "Update", "portable", "x64")
                .tracked("Update", "finished", |_| Ok(()))?;
            job.prepare_child("pending", "Update", "installer", "x64");
            assert!(job.request_cancel());
            job.check()
        })
        .unwrap_err();
    assert!(is_cancelled(&error));
    let records = parent.operations.list();
    assert_eq!(
        records
            .iter()
            .find(|op| op.resource == "finished")
            .unwrap()
            .lifecycle,
        Lifecycle::Completed
    );
    assert_eq!(
        records
            .iter()
            .find(|op| op.resource == "pending")
            .unwrap()
            .lifecycle,
        Lifecycle::Cancelled
    );
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn independent_jobs_keep_progress_and_failures_separate() {
    let root = root();
    let mut first = Job::new(root.join("logs/first.log"), &Default::default());
    let mut second = Job::new(root.join("logs/second.log"), &Default::default());
    first.describe("Source download", "one");
    second.describe("Build", "two");
    let (done_first, wait_first) = std::sync::mpsc::channel();
    let (done_second, wait_second) = std::sync::mpsc::channel();
    first.spawn(move |job| {
        job.stage("Downloading", Some(0.2), "one");
        wait_first.recv().unwrap();
        Ok(())
    });
    second.spawn(move |job| {
        job.stage("Compiling", Some(0.8), "two");
        wait_second.recv().unwrap();
        Err(anyhow::anyhow!("Compiler failed"))
    });
    wait(|| first.snapshot().progress == Some(0.2) && second.snapshot().progress == Some(0.8));
    assert_eq!(first.operations.list().len(), 2);
    done_first.send(()).unwrap();
    wait(|| !first.snapshot().busy);
    assert!(second.snapshot().busy);
    done_second.send(()).unwrap();
    wait(|| !second.snapshot().busy);
    assert_eq!(first.snapshot().stage, "Complete");
    assert_eq!(second.snapshot().stage, "Failed");
    assert!(second.snapshot().outcome.contains("Compiler failed"));
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn worker_panic_while_holding_progress_lock_resolves_as_failed() {
    let root = root();
    let job = Job::new(root.join("logs/mock.log"), &Default::default());
    job.spawn(|job| {
        let _guard = job.state.lock().unwrap();
        panic!("Simulated poisoned worker");
    });
    wait(|| !job.snapshot().busy);
    assert_eq!(job.snapshot().stage, "Failed");
    assert_eq!(job.operations.list()[0].lifecycle, Lifecycle::Failed);
    std::fs::remove_dir_all(root).unwrap();
}
