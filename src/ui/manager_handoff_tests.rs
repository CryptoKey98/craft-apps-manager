#[test]
fn manager_handoff_does_not_run_on_the_ui_poll_path() {
    let ui = include_str!("mod.rs");
    let poll = ui.split("fn poll(&mut self").nth(1).unwrap();
    let poll = poll.split("fn apply_release_check(").next().unwrap();
    assert!(
        !poll.contains("self_update::launch"),
        "The UI poll still performs the blocking update verification and helper launch"
    );
}

use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::time::{Duration, Instant};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("craft-handoff-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn job(&self, name: &str) -> Job {
        Job::new(self.0.join(format!("logs/{name}.log")), &Default::default())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn wait_until_finished(job: &Job) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while job.snapshot().busy {
        assert!(Instant::now() < deadline, "Update worker did not settle");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Hold the expensive validation step behind a channel, not a timing threshold.
fn worker_case(after_validation_starts: impl FnOnce(&Job, &Job)) -> (Result<(), Failure>, bool) {
    let fixture = Fixture::new();
    let mut job = fixture.job("updates");
    job.describe("Manager update download", "Manager");
    let build = fixture.job("build");
    let build_for_worker = build.clone();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    let spawned = Arc::new(AtomicBool::new(false));
    let spawned_on_worker = spawned.clone();
    let ui_thread = std::thread::current().id();
    job.spawn_background(move |job| {
        let result = run(
            &job,
            &build_for_worker,
            || {
                assert_ne!(std::thread::current().id(), ui_thread);
                Ok(PathBuf::from("prepared-plan"))
            },
            |_, guard| {
                assert_ne!(std::thread::current().id(), ui_thread);
                ready_tx.send(()).unwrap();
                resume_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                guard()?;
                spawned_on_worker.store(true, Ordering::SeqCst);
                Ok(())
            },
        );
        result_tx.send(display_result(&result)).unwrap();
        result
    });
    ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        job.snapshot().busy,
        "Job must stay busy during handoff verification"
    );
    assert!(matches!(
        result_rx.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    assert!(!spawned.load(Ordering::SeqCst));
    after_validation_starts(&job, &build);
    resume_tx.send(()).unwrap();
    let result = result_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    wait_until_finished(&job);
    (result, spawned.load(Ordering::SeqCst))
}

#[test]
fn verification_and_handoff_stay_on_worker_while_ui_can_poll() {
    assert_eq!(worker_case(|_, _| {}), (Ok(()), true));
}

#[test]
fn cancellation_during_verification_prevents_helper_spawn() {
    assert_eq!(
        worker_case(|job, _| {
            job.request_cancel();
        }),
        (Err(Failure::Cancelled), false)
    );
}

#[test]
fn newly_started_builder_prevents_helper_spawn() {
    assert_eq!(
        worker_case(|_, build| build.state.lock().unwrap().busy = true),
        (Err(Failure::Paused), false)
    );
}

#[test]
fn other_process_operation_is_checked_again_after_verification() {
    let mut other = None;
    let result = worker_case(|job, _| {
        let handle = job.operations.register("Build", "FilmCraft", None);
        handle.start();
        other = Some(handle);
    });
    assert_eq!(result, (Err(Failure::Paused), false));
}

#[test]
fn cancellation_before_preparation_does_no_work() {
    let fixture = Fixture::new();
    let job = fixture.job("update");
    job.cancel.store(true, Ordering::Relaxed);
    let result = run(
        &job,
        &fixture.job("build"),
        || panic!("Preparation after cancellation"),
        |_, _| panic!("Launch after cancellation"),
    );
    assert_eq!(display_result(&result), Err(Failure::Cancelled));
}

#[test]
fn preparation_failure_never_launches_and_verification_failure_stays_an_error() {
    let fixture = Fixture::new();
    let job = fixture.job("update");
    let build = fixture.job("build");
    let result = run(
        &job,
        &build,
        || anyhow::bail!("Download verification failed"),
        |_, _| panic!("Failed preparation launched"),
    );
    assert_eq!(
        display_result(&result),
        Err(Failure::Failed("Download verification failed".into()))
    );
    let result = run(
        &job,
        &build,
        || Ok("prepared".into()),
        |_, _| anyhow::bail!("Update helper mismatch"),
    );
    assert_eq!(
        display_result(&result),
        Err(Failure::Failed("Update helper mismatch".into()))
    );
}

#[test]
fn panic_disconnects_result_only_after_job_is_finalized() {
    let fixture = Fixture::new();
    let job = fixture.job("update");
    let (tx, rx) = mpsc::channel::<Result<(), Failure>>();
    job.spawn_background(move |_| {
        let _tx = tx;
        panic!("Injected update worker failure")
    });
    wait_until_finished(&job);
    let result = poll_result(&job, &rx);
    assert!(matches!(result, Some(Err(Failure::Failed(_)))));
}

#[test]
fn success_waits_for_persisted_terminal_operation_before_ui_can_close() {
    let fixture = Fixture::new();
    let mut job = fixture.job("update");
    job.describe("Manager update download", "Manager");
    let (tx, rx) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (finish_tx, finish_rx) = mpsc::channel();
    job.spawn_background(move |job| {
        tx.send(Ok(())).unwrap();
        ready_tx.send(job.operation_id().unwrap()).unwrap();
        finish_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        Ok(())
    });
    let id = ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(job.snapshot().busy);
    assert_eq!(poll_result(&job, &rx), None);
    assert!(!job.operations.read(&id).unwrap().lifecycle.terminal());
    finish_tx.send(()).unwrap();
    wait_until_finished(&job);
    assert_eq!(
        job.operations.read(&id).unwrap().lifecycle,
        craft_apps_manager::operations::Lifecycle::Completed
    );
    assert_eq!(poll_result(&job, &rx), Some(Ok(())));
}
