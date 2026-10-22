use super::*;
#[test]
fn restarted_manager_recovers_builder_progress_log_and_cancellation() {
    let root = std::env::temp_dir().join(format!("craft-build-restart-{}", uuid::Uuid::new_v4()));
    let producer = crate::operations::Manager::new(&root);
    let operation = producer.register("Build", "filmcraft", None);
    operation.start();
    let log = root.join("logs/filmcraft.log");
    crate::files::write_bytes(&log, b"Compiling filmcraft\n").unwrap();
    operation.report("Log file", &log.display().to_string());
    operation.update(
        &State {
            stage: "Compiling".into(),
            detail: "Building dependencies".into(),
            progress: Some(0.6),
            ..Default::default()
        },
        false,
    );
    // Persist a separate process owner; the reopened manager has no worker state.
    let mut record = operation.snapshot();
    record.owner = u32::MAX;
    crate::files::write_json(
        &root.join(format!("runtime/operations/{}.json", record.id)),
        &record,
    )
    .unwrap();
    for _ in 0..2 {
        let reopened = crate::operations::Manager::new(&root);
        let records = reopened.list();
        let active = records
            .iter()
            .find(|op| Job::builder_operation(op, &op.resource))
            .unwrap();
        assert_eq!(active.resource, "filmcraft");
        let observed =
            Job::observed_builder_state(&State::default(), "photocraft", &records, "filmcraft");
        assert!(observed.busy);
        assert_eq!(observed.stage, "Compiling");
        assert_eq!(observed.progress, Some(0.6));
        assert_eq!(observed.detail, "Building dependencies");
        assert!(fs::read_to_string(active.facts.get("Log file").unwrap())
            .unwrap()
            .contains("Compiling filmcraft"));
        assert!(reopened.request_cancel(&active.id).unwrap());
        assert!(operation.cancel_requested());
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn shared_build_observer_tracks_parent_after_source_child_completes() {
    use crate::operations::Lifecycle;
    let root = std::env::temp_dir().join(format!("craft-parent-progress-{}", uuid::Uuid::new_v4()));
    let manager = crate::operations::Manager::new(&root);
    let parent = manager.register("Build", "filmcraft", None);
    parent.start();
    parent.update(
        &State {
            stage: "Compiling".into(),
            progress: Some(0.3),
            ..Default::default()
        },
        false,
    );
    let child = manager.register("Source download", "filmcraft", Some(parent.id()));
    child.start();
    child.finish(Lifecycle::Completed, "Success".into());
    let mut parent_record = parent.snapshot();
    parent_record.owner = u32::MAX;
    let mut child_record = child.snapshot();
    child_record.started = Some(parent_record.started.unwrap() + 1);
    let records = vec![child_record, parent_record.clone()];
    assert_eq!(
        Job::latest_builder_operation(&records, "filmcraft")
            .unwrap()
            .id,
        parent.id()
    );
    let observed =
        Job::observed_builder_state(&State::default(), "filmcraft", &records, "filmcraft");
    assert!(observed.busy);
    assert_eq!(observed.stage, "Compiling");
    assert_eq!(observed.progress, Some(0.3));
    let cancellable = records
        .iter()
        .find(|op| Job::builder_operation(op, "filmcraft"))
        .unwrap();
    assert_eq!(cancellable.id, parent.id());
    assert!(manager.request_cancel(&cancellable.id).unwrap());
    assert!(parent.cancel_requested());
    parent_record.lifecycle = Lifecycle::Completed;
    parent_record.stage = "Completed".into();
    parent_record.result = Some("Build succeeded".into());
    let finished = Job::observed_builder_state(
        &State::default(),
        "filmcraft",
        &[records[0].clone(), parent_record],
        "filmcraft",
    );
    assert!(!finished.busy);
    assert_eq!(finished.outcome, "Build succeeded");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn shared_build_terminal_outcomes_replace_stale_state_and_isolate_apps() {
    use crate::operations::Lifecycle;
    let root = std::env::temp_dir().join(format!("craft-build-outcomes-{}", uuid::Uuid::new_v4()));
    let manager = crate::operations::Manager::new(&root);
    let operation = manager.register("Build", "filmcraft", None);
    operation.start();
    operation.update(
        &State {
            stage: "Compiling".into(),
            progress: Some(0.4),
            ..Default::default()
        },
        false,
    );
    let mut record = operation.snapshot();
    record.owner = u32::MAX;
    let unrelated = State {
        busy: true,
        stage: "Other app".into(),
        log: "Unrelated output".into(),
        ..Default::default()
    };
    let observed =
        Job::observed_builder_state(&unrelated, "photocraft", &[record.clone()], "filmcraft");
    assert_eq!(observed.stage, "Compiling");
    assert_eq!(observed.progress, Some(0.4));
    assert!(observed.log.is_empty());
    for (lifecycle, stage) in [
        (Lifecycle::Completed, "Complete"),
        (Lifecycle::Failed, "Failed"),
        (Lifecycle::Cancelled, "Cancelled"),
        (Lifecycle::Interrupted, "Interrupted"),
    ] {
        record.lifecycle = lifecycle;
        record.stage = format!("{lifecycle:?}");
        record.result = Some(format!("Result: {stage}"));
        record.progress = if lifecycle == Lifecycle::Completed {
            Some(1.0)
        } else {
            None
        };
        let stale = State {
            stage: "Old result".into(),
            log: "Old output".into(),
            output: Some(root.join("old")),
            ..Default::default()
        };
        let observed =
            Job::observed_builder_state(&stale, "filmcraft", &[record.clone()], "filmcraft");
        assert!(!observed.busy);
        assert_eq!(observed.stage, stage);
        assert_eq!(observed.outcome, format!("Result: {stage}"));
        assert_eq!(observed.detail, observed.outcome);
        assert!(observed.output.is_none());
        assert!(observed.log.is_empty());
    }
    assert_eq!(
        Job::observed_builder_state(&unrelated, "filmcraft", &[record], "filmcraft").stage,
        "Other app"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn running_build_publishes_existing_log_before_more_output() {
    let root = std::env::temp_dir().join(format!("craft-live-log-{}", uuid::Uuid::new_v4()));
    let path = root.join("logs/build.log");
    let job = Job::new(path.clone(), &Default::default());
    job.tracked("Build", "filmcraft", |worker| {
        worker.log("Compiling existing dependency");
        // Opening the other window requires no further log writes or completion.
        let consumer = crate::operations::Manager::new(&root);
        let operation = consumer
            .list()
            .into_iter()
            .find(|op| op.resource == "filmcraft")
            .unwrap();
        assert!(!operation.lifecycle.terminal());
        let published = operation.facts.get("Log file").unwrap();
        assert_eq!(Path::new(published), path);
        assert!(fs::read_to_string(published)
            .unwrap()
            .contains("Compiling existing dependency"));
        Ok(())
    })
    .unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn shared_builder_state_is_observed_without_taking_worker_ownership() {
    let root = std::env::temp_dir().join(format!("craft-build-observer-{}", uuid::Uuid::new_v4()));
    let producer = crate::operations::Manager::new(&root);
    let operation = producer.register("Build", "filmcraft", None);
    operation.start();
    operation.update(
        &State {
            busy: true,
            stage: "Compiling".into(),
            detail: "12 compilation steps".into(),
            progress: Some(0.25),
            ..Default::default()
        },
        false,
    );
    // A separate manager instance reads the shared files, like either window.
    let consumer = crate::operations::Manager::new(&root);
    let mut records = consumer.list();
    for record in &mut records {
        record.owner = u32::MAX;
    }
    let observed =
        Job::observed_builder_state(&State::default(), "filmcraft", &records, "filmcraft");
    assert!(observed.busy);
    assert_eq!(observed.stage, "Compiling");
    assert_eq!(observed.progress, Some(0.25));
    assert!(
        !Job::observed_builder_state(&State::default(), "filmcraft", &records, "photocraft").busy
    );
    let local = State {
        busy: true,
        stage: "Local worker".into(),
        ..Default::default()
    };
    assert_eq!(
        Job::observed_builder_state(&local, "filmcraft", &records, "filmcraft").stage,
        "Local worker"
    );
    assert!(consumer.request_cancel(&operation.id()).unwrap());
    assert!(operation.cancel_requested());
    operation.update(&State::default(), true);
    let mut cancelled_records = consumer.list();
    for record in &mut cancelled_records {
        record.owner = u32::MAX;
    }
    assert_eq!(
        Job::observed_builder_state(
            &State::default(),
            "filmcraft",
            &cancelled_records,
            "filmcraft"
        )
        .stage,
        "Cancellation requested"
    );
    operation.finish(crate::operations::Lifecycle::Cancelled, "Cancelled".into());
    assert!(
        !Job::observed_builder_state(
            &State::default(),
            "filmcraft",
            &consumer.list(),
            "filmcraft"
        )
        .busy
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reused_job_clears_cancellation_and_temporary_executable() {
    let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let job = Job::new(root.join("job.log"), &Default::default());
    job.tracked("Build", "first", |worker| {
        worker.state.lock().unwrap().output = Some(root.join("old-build"));
        worker.state.lock().unwrap().executable = Some(root.join("old.exe"));
        worker.cancel.store(true, Ordering::Relaxed);
        Err::<(), _>(Cancelled.into())
    })
    .unwrap_err();
    job.tracked("Download", "second", |worker| {
        worker.check()?;
        assert_eq!(worker.snapshot().output, Some(root.join("old-build")));
        assert!(worker.snapshot().executable.is_none());
        Ok(())
    })
    .unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn tool_setup_preserves_previous_build_but_new_build_clears_it() {
    let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let job = Job::new(root.join("job.log"), &Default::default());
    job.state.lock().unwrap().output = Some(root.join("previous-build"));
    job.tracked("setup", "fixture", |worker| {
        assert_eq!(worker.snapshot().output, Some(root.join("previous-build")));
        Ok(())
    })
    .unwrap();
    job.tracked("build", "fixture", |worker| {
        assert!(worker.snapshot().output.is_none());
        Ok(())
    })
    .unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn attached_batch_child_preserves_shared_cancellation_on_start() {
    let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let manager = crate::operations::Manager::new(&root);
    let parent = manager.register("Bulk update", "fixture", None);
    parent.start();
    let handle = manager.register("Install", "fixture", Some(parent.id()));
    let ids = std::collections::BTreeMap::from([("fixture".into(), handle.id())]);
    let mut job = Job::new(root.join("job.log"), &Default::default());
    job.attach_children(&root, Some(parent.id()), &ids).unwrap();
    let child = job.prepare_child("fixture", "Install", "installer", "x64");
    // Simulate cancellation arriving after the batch check but before child startup.
    job.cancel.store(true, Ordering::Relaxed);
    let error = child
        .tracked("Install", "fixture", |worker| worker.check())
        .unwrap_err();
    assert!(is_cancelled(&error));
    assert!(job.cancel.load(Ordering::Relaxed));
    assert_eq!(
        manager.read(&handle.id()).unwrap().lifecycle,
        crate::operations::Lifecycle::Cancelled
    );
    parent.finish(crate::operations::Lifecycle::Cancelled, "Cancelled".into());
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn terminal_record_does_not_release_a_worker_before_finalization() {
    let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let job = Job::new(root.join("job.log"), &Default::default());
    let handle = job.operations.register("Build", "fixture", None);
    handle.start();
    *job.active.lock().unwrap() = Some(handle.clone());
    job.state.lock().unwrap().busy = true;
    handle.finish(crate::operations::Lifecycle::Completed, "Success".into());
    assert!(job.snapshot().busy);
    job.state.lock().unwrap().busy = false;
    assert!(!job.snapshot().busy);
    std::fs::remove_dir_all(root).unwrap();
}
fn finish(work: impl FnOnce(Job) -> Result<()> + Send + 'static) -> State {
    let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let job = Job::new(
        root.join("job.log"),
        &crate::model::BuilderPreferences::default(),
    );
    job.spawn_worker(work)
        .expect("worker should start")
        .join()
        .expect("worker finalization should not panic");
    let state = job
        .state
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone();
    assert!(!state.busy);
    let _ = fs::remove_dir_all(root);
    state
}
#[test]
fn worker_completion_is_not_limited_to_one_second() {
    let state = finish(|_| {
        std::thread::sleep(Duration::from_millis(1100));
        Ok(())
    });
    assert_eq!(state.stage, "Complete");
}
#[test]
fn cancellation_during_transfer_removes_partial_and_preserves_typed_marker() {
    use std::{io::Read, net::TcpListener};
    let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let job = Job::new(
        root.join("job.log"),
        &crate::model::BuilderPreferences::default(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/package", listener.local_addr().unwrap());
    let cancel = job.cancel.clone();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = [0; 4096];
        let _ = socket.read(&mut request).unwrap();
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 131072\r\nConnection: close\r\n\r\n")
            .unwrap();
        let _ = socket.write_all(&[1; 65536]);
        cancel.store(true, Ordering::Relaxed);
    });
    let destination = root.join("downloads/package");
    let error = crate::network::Network::new(&root)
        .unwrap()
        .download(&url, &destination, &job)
        .unwrap_err();
    server.join().unwrap();
    assert!(is_cancelled(&error));
    assert!(!destination.exists());
    assert_eq!(
        fs::read_dir(destination.parent().unwrap()).unwrap().count(),
        0
    );
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn cancellation_is_typed_through_context_and_does_not_log_error() {
    let state = finish(|job| {
        job.cancel.store(true, Ordering::Relaxed);
        job.check().map_err(|e| e.context("Download stopped"))
    });
    assert_eq!(state.stage, "Cancelled");
    assert!(!state.log.contains("ERROR"));
}
#[test]
fn late_cancel_preserves_success_failure_and_panic() {
    let success = finish(|job| {
        job.cancel.store(true, Ordering::Relaxed);
        Ok(())
    });
    assert_eq!(success.stage, "Complete");
    let failure = finish(|job| {
        job.cancel.store(true, Ordering::Relaxed);
        bail!("Real disk error")
    });
    assert_eq!(failure.stage, "Failed");
    assert!(failure.log.contains("Real disk error"));
    let panic = finish(|job| {
        job.cancel.store(true, Ordering::Relaxed);
        panic!("worker panic");
    });
    assert_eq!(panic.stage, "Failed");
}
