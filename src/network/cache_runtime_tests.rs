use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

// Fixture cleanup must wait for asynchronous work, including work enqueued by a
// warming callback. This instrumentation is compiled only into the test binary.
static BACKGROUND: OnceLock<(Mutex<usize>, Condvar)> = OnceLock::new();
struct Pending;
impl Drop for Pending {
    fn drop(&mut self) {
        let (pending, ready) = BACKGROUND.get().unwrap();
        *pending.lock().unwrap() -= 1;
        ready.notify_all();
    }
}
pub(super) fn tracked(work: Work) -> Work {
    let (pending, _) = BACKGROUND.get_or_init(|| (Mutex::new(0), Condvar::new()));
    *pending.lock().unwrap() += 1;
    let pending = Pending;
    Box::new(move || {
        let _pending = pending;
        work();
    })
}
pub(super) fn wait_for_background() {
    let (pending, ready) = BACKGROUND.get_or_init(|| (Mutex::new(0), Condvar::new()));
    let (pending, result) = ready
        .wait_timeout_while(
            pending.lock().unwrap(),
            Duration::from_secs(10),
            |pending| *pending != 0,
        )
        .unwrap();
    let unfinished = *pending;
    drop(pending);
    assert!(
        !result.timed_out() || unfinished == 0,
        "Cache background work did not finish"
    );
}

fn root() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/api-cache-fixtures")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn history_reuse_does_not_publish_tag_data_or_bypass_execution_refresh() {
    let root = root();
    let history = key(&root, "fixture:history", None);
    let tag = key(&root, "fixture:tag", None);
    let at = chrono::Utc::now().timestamp() - 120;
    let release =
        serde_json::json!({"tag_name":"v1.1.0","draft":false,"prerelease":false,"assets":[]});
    disk(&root)
        .write(
            &history.hash,
            at,
            Some("history-only-etag".into()),
            &serde_json::json!([release.clone()]),
        )
        .unwrap();
    assert_eq!(
        release_from_history(&tag, &history, "v1.1.0", 900),
        Some((release.clone(), at))
    );
    assert!(
        read(&tag).is_none(),
        "History reuse must not publish a derived tag record"
    );
    assert_eq!(
        read(&history).unwrap().at,
        at,
        "Reusing a list must not freshen it"
    );
    let mut fetched = false;
    request_with_age(tag, false, 60, |_| {
        fetched = true;
        Ok(Response {
            value: Some(release),
            etag: None,
            cacheable: true,
        })
    })
    .unwrap();
    assert!(
        fetched,
        "Installation must still validate metadata older than60seconds"
    );
    clear(&root).unwrap();
    wait_for_background();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn history_reuse_respects_expiry_scope_exact_tags_and_authoritative_details() {
    let root = root();
    let history = key(&root, "fixture:history", Some("fixture-A"));
    let tag = key(&root, "fixture:tag", Some("fixture-A"));
    let release =
        serde_json::json!({"tag_name":"v1.1.0","draft":false,"prerelease":false,"assets":[]});
    let now = chrono::Utc::now().timestamp();
    disk(&root)
        .write(
            &history.hash,
            now - 901,
            None,
            &serde_json::json!([release.clone()]),
        )
        .unwrap();
    assert!(release_from_history(&tag, &history, "v1.1.0", 900).is_none());
    assert!(read(&tag).is_none());
    disk(&root)
        .write(
            &history.hash,
            now - 120,
            None,
            &serde_json::json!([release.clone()]),
        )
        .unwrap();
    assert!(release_from_history(&tag, &history, "v9.9.9", 900).is_none());
    assert!(
        read(&tag).is_none(),
        "Missing tags must use the real tag endpoint"
    );
    let other_history = key(&root, "fixture:history", Some("fixture-B"));
    let other_tag = key(&root, "fixture:tag", Some("fixture-B"));
    assert!(release_from_history(&other_tag, &other_history, "v1.1.0", 900).is_none());
    assert!(read(&other_tag).is_none());
    let mut newer = release;
    newer["name"] = serde_json::json!("Newer verified tag details");
    disk(&root)
        .write(&tag.hash, now - 120, None, &newer)
        .unwrap();
    assert!(release_from_history(&tag, &history, "v1.1.0", 900).is_none());
    assert_eq!(read(&tag).unwrap().value, newer);
    clear(&root).unwrap();
    wait_for_background();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_release_expires_and_is_scoped_to_credentials_and_library() {
    let root = root();
    let item = key(&root, "fixture:missing", None);
    assert!(request(item.clone(), false, |_| Err(missing_response())).is_err());
    let error = request(item.clone(), false, |_| {
        panic!("Repeated missing-release HTTP")
    })
    .unwrap_err();
    assert_eq!(status(&error), Some(reqwest::StatusCode::NOT_FOUND));
    let response = || {
        Ok(Response {
            value: Some(serde_json::json!({"ok":true})),
            etag: None,
            cacheable: true,
        })
    };
    // A credential switch cannot inherit an unauthenticated/private-resource 404.
    request(
        key(&root, "fixture:missing", Some("fixture-credential")),
        false,
        |_| response(),
    )
    .unwrap();
    assert!(!MISSING.get().unwrap().lock().unwrap().contains_key(&key(
        &root.join("other"),
        "fixture:missing",
        None
    )));
    MISSING
        .get()
        .unwrap()
        .lock()
        .unwrap()
        .get_mut(&item)
        .unwrap()
        .0 = Instant::now() - Duration::from_secs(61);
    assert_eq!(
        request(item, false, |_| response()).unwrap().value["ok"],
        true
    );
    clear(&root).unwrap();
    wait_for_background();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn force_and_clear_bypass_missing_release_and_withdrawn_data_cannot_revive() {
    let root = root();
    let item = key(&root, "fixture:withdrawn", None);
    let success = || {
        Ok(Response {
            value: Some(serde_json::json!({"ok":true})),
            etag: None,
            cacheable: true,
        })
    };
    request(item.clone(), false, |_| success()).unwrap();
    assert!(request(item.clone(), true, |_| Err(missing_response())).is_err());
    assert!(
        disk(&root)
            .read(&item.hash, item.legacy.as_deref())
            .unwrap()
            .is_none(),
        "Another process must not load the withdrawn response from disk"
    );
    assert!(
        read(&item).is_none(),
        "Withdrawn data must not remain available to previews"
    );
    MISSING
        .get()
        .unwrap()
        .lock()
        .unwrap()
        .get_mut(&item)
        .unwrap()
        .0 = Instant::now() - Duration::from_secs(61);
    let mut fetched = false;
    request_with_age(item.clone(), false, 86400, |_| {
        fetched = true;
        success()
    })
    .unwrap();
    assert!(
        fetched,
        "An expired 404 must not resurrect the older disk success"
    );
    assert!(request(item.clone(), true, |_| Err(missing_response())).is_err());
    request(item.clone(), true, |_| success()).unwrap();
    assert!(!MISSING.get().unwrap().lock().unwrap().contains_key(&item));
    assert!(request(item.clone(), true, |_| Err(missing_response())).is_err());
    clear(&root).unwrap();
    request(item, false, |_| success()).unwrap();
    clear(&root).unwrap();
    wait_for_background();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn request_purpose_controls_freshness_without_duplicate_cache_keys() {
    for (age, max_age, expected_fetch) in [
        (120, 60, true),
        (120, 900, false),
        (7200, 86400, false),
        (86401, 86400, true),
        (-60, 86400, true),
    ] {
        let root = root();
        let key = key(&root, "fixture:policy", None);
        disk(&root)
            .write(
                &key.hash,
                chrono::Utc::now().timestamp() - age,
                None,
                &serde_json::json!({"old":true}),
            )
            .unwrap();
        let mut fetched = false;
        let record = request_with_age(key.clone(), false, max_age, |_| {
            fetched = true;
            Ok(Response {
                value: Some(serde_json::json!({"new":true})),
                etag: None,
                cacheable: true,
            })
        })
        .unwrap();
        assert_eq!(fetched, expected_fetch, "age={age}, max_age={max_age}");
        assert_eq!(record.value[if fetched { "new" } else { "old" }], true);
        assert_eq!(
            disk(&root).read(&key.hash, None).unwrap().unwrap().value,
            record.value
        );
        forget(&root);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn cache_waiters_cannot_bypass_dependent_invalidation_through_hot_memory() {
    let root = root();
    let key = key(&root, "fixture:dependent-finalization", None);
    let owner_key = key.clone();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let owner = std::thread::spawn(move || {
        request_finishing(
            owner_key,
            true,
            60,
            |_| {
                Ok(Response {
                    value: Some(serde_json::json!({"new":true})),
                    etag: None,
                    cacheable: true,
                })
            },
            || {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            },
        )
        .unwrap()
    });
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let waiter_key = key.clone();
    let (returned_tx, returned_rx) = mpsc::channel();
    let waiter = std::thread::spawn(move || {
        let result = request_with_age(waiter_key, false, 900, |_| {
            panic!("Must share the completed response")
        });
        returned_tx.send(result).unwrap();
    });
    let early = returned_rx.recv_timeout(Duration::from_millis(100));
    release_tx.send(()).unwrap();
    owner.join().unwrap();
    waiter.join().unwrap();
    assert!(
        early.is_err(),
        "Returned new release before dependent invalidation completed"
    );
    assert_eq!(returned_rx.recv().unwrap().unwrap().value["new"], true);
    forget(&root);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_update_revalidation_does_not_make_an_old_response_fresh() {
    let root = root();
    let key = key(&root, "fixture:failed-revalidation", None);
    let at = chrono::Utc::now().timestamp() - 120;
    disk(&root)
        .write(&key.hash, at, None, &serde_json::json!({"old":true}))
        .unwrap();
    assert!(request_with_age(key.clone(), false, 60, |_| anyhow::bail!("offline")).is_err());
    assert_eq!(disk(&root).read(&key.hash, None).unwrap().unwrap().at, at);
    let mut retried = false;
    request_with_age(key, false, 60, |_| {
        retried = true;
        Ok(Response {
            value: None,
            etag: None,
            cacheable: true,
        })
    })
    .unwrap();
    assert!(retried);
    forget(&root);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn overlapping_forced_requests_share_one_result() {
    let root = root();
    let key = key(&root, "fixture:coalesce", None);
    let barrier = Arc::new(std::sync::Barrier::new(9));
    let calls = Arc::new(AtomicUsize::new(0));
    let (release_tx, release_rx) = mpsc::channel();
    let release = Arc::new(Mutex::new(release_rx));
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let (key, barrier, calls, release) =
                (key.clone(), barrier.clone(), calls.clone(), release.clone());
            std::thread::spawn(move || {
                barrier.wait();
                request(key, true, |_| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    release
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(10))
                        .unwrap();
                    Ok(Response {
                        value: Some(serde_json::json!({"shared":true})),
                        etag: None,
                        cacheable: true,
                    })
                })
                .unwrap()
                .value
                .clone()
            })
        })
        .collect();
    barrier.wait();
    let deadline = Instant::now() + Duration::from_secs(5);
    // Owner holds two Arcs; seven waiters and this observer make ten.
    let observed = loop {
        let observed = flight(&key);
        if Arc::strong_count(&observed) >= 10 {
            break observed;
        }
        assert!(
            Instant::now() < deadline,
            "Not all requests joined the flight"
        );
        drop(observed);
        std::thread::sleep(Duration::from_millis(1));
    };
    release_tx.send(()).unwrap();
    for worker in workers {
        assert_eq!(worker.join().unwrap()["shared"], true);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    drop(observed);
    forget(&root);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn memory_budget_evicts_old_response_and_preview_never_waits_for_disk_lock() {
    let root = root();
    let key = key(&root, "fixture:preview", None);
    let disk = disk(&key.root);
    let guard = disk.lock(&key.hash).unwrap();
    disk.write(
        &key.hash,
        chrono::Utc::now().timestamp(),
        None,
        &serde_json::json!({"ready":true}),
    )
    .unwrap();
    let start = Instant::now();
    assert!(peek::<Value>(key.clone(), || {}).is_none());
    assert!(start.elapsed() < Duration::from_millis(100));
    // A held stripe must not block the preview or consume its caller's thread.
    drop(guard);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(value) = peek::<Value>(key.clone(), || {}) {
            assert_eq!(value["ready"], true);
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let value = serde_json::json!({"large":"x".repeat(9*1024*1024)});
    let older = key.clone();
    let newer = super::key(&root, "fixture:newer", None);
    for key in [&older, &newer] {
        remember_record(
            key,
            Arc::new(Record {
                at: 0,
                accessed: 0,
                etag: None,
                bytes: serde_json::to_vec(&value).unwrap().len(),
                value: value.clone(),
                generation: uuid::Uuid::new_v4().to_string(),
            }),
        );
    }
    assert!(hot::<Record>(&MemoryKey::Raw(older), false).is_none());
    assert!(hot::<Record>(&MemoryKey::Raw(newer), false).is_some());
    assert!(memory().lock().unwrap().bytes <= MEMORY_BUDGET);
    forget(&root);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn oversized_replacement_cannot_revive_previous_response() {
    let root = root();
    let key = key(&root, "fixture:oversized", None);
    request(key.clone(), true, |_| {
        Ok(Response {
            value: Some(serde_json::json!({"old":true})),
            etag: None,
            cacheable: true,
        })
    })
    .unwrap();
    let large = Value::String("x".repeat(MEMORY_BUDGET + 1));
    request(key.clone(), true, |_| {
        Ok(Response {
            value: Some(large),
            etag: None,
            cacheable: true,
        })
    })
    .unwrap();
    let mut fetched = false;
    let result = request(key, false, |_| {
        fetched = true;
        Ok(Response {
            value: Some(serde_json::json!({"latest":true})),
            etag: None,
            cacheable: true,
        })
    })
    .unwrap();
    assert!(fetched);
    assert_eq!(result.value["latest"], true);
    forget(&root);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn stricter_waiters_do_not_accept_longer_lived_disk_hits() {
    for force in [true, false] {
        let root = root();
        let key = key(&root, "fixture:forced-waiter", None);
        let disk = disk(&root);
        let guard = disk.lock(&key.hash).unwrap();
        disk.write(
            &key.hash,
            chrono::Utc::now().timestamp() - 120,
            None,
            &serde_json::json!({"old":true}),
        )
        .unwrap();
        let normal_key = key.clone();
        let normal = std::thread::spawn(move || {
            request(normal_key, false, |_| {
                panic!("Fresh disk entry should not fetch")
            })
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        let observed = loop {
            let observed = flight(&key);
            if observed.state.lock().unwrap().running {
                break observed;
            }
            assert!(Instant::now() < deadline);
            drop(observed);
            std::thread::sleep(Duration::from_millis(1));
        };
        let forced_key = key.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let forced_calls = calls.clone();
        let forced = std::thread::spawn(move || {
            request_with_age(forced_key, force, 60, |_| {
                forced_calls.fetch_add(1, Ordering::SeqCst);
                Ok(Response {
                    value: Some(serde_json::json!({"latest":true})),
                    etag: None,
                    cacheable: true,
                })
            })
        });
        while Arc::strong_count(&observed) < 4 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        drop(guard);
        normal.join().unwrap().unwrap();
        assert_eq!(forced.join().unwrap().unwrap().value["latest"], true);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        drop(observed);
        forget(&root);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn typed_decode_failure_is_throttled_until_response_generation_changes() {
    let failed = WarmState {
        running: false,
        at: Instant::now(),
        generation: Some("original".into()),
    };
    assert!(warming_blocks(&failed, &Some("original".into())));
    assert!(!warming_blocks(&failed, &Some("replacement".into())));
    let pending = WarmState {
        running: true,
        ..failed
    };
    assert!(warming_blocks(&pending, &Some("replacement".into())));
}

#[test]
fn failed_disk_invalidation_does_not_expire_into_a_stale_response() {
    let root = root();
    let key = key(&root, "fixture:invalidation", None);
    let disk = disk(&key.root);
    {
        let _guard = disk.lock(&key.hash).unwrap();
        disk.write(&key.hash, 0, None, &serde_json::json!({"stale":true}))
            .unwrap();
    }
    // Simulate a failed removal: the invalidated bytes remain on disk.
    invalidate_disk(&key);
    assert!(
        read(&key).is_none(),
        "Elapsed time cannot validate unchanged disk bytes"
    );
    assert!(preview_read(&key).is_none());
    let result = request(key.clone(), true, |_| {
        Ok(Response {
            value: Some(serde_json::json!({"latest":true})),
            etag: None,
            cacheable: true,
        })
    })
    .unwrap();
    assert_eq!(result.value["latest"], true);
    assert!(
        disk_allowed(&key),
        "A successful replacement permits disk reuse"
    );
    forget_key(&key);
    assert_eq!(read(&key).unwrap().value["latest"], true);
    forget(&root);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalidation_overflow_never_forgets_rejected_responses() {
    let root = root();
    let mut invalid = InvalidDisk::default();
    let first = key(&root, "fixture:first", None);
    invalid.reject(&first);
    for i in 0..512 {
        invalid.reject(&key(&root, &format!("fixture:{i}"), None));
    }
    assert!(!invalid.allows(&first));
    assert!(!invalid.allows(&key(&root, "fixture:unknown", None)));
    assert!(invalid.keys.len() <= 512);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn overlapping_failure_is_shared_and_a_later_request_can_recover() {
    let root = root();
    let key = key(&root, "fixture:shared-failure", None);
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let owner_key = key.clone();
    let owner = std::thread::spawn(move || {
        request(owner_key, true, |_| {
            entered_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            anyhow::bail!("fixture HTTP failure")
        })
    });
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let observed = flight(&key);
    let waiter_key = key.clone();
    let waiter = std::thread::spawn(move || {
        request(waiter_key, true, |_| {
            panic!("Waiter must share the owner's failure")
        })
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while Arc::strong_count(&observed) < 4 {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    release_tx.send(()).unwrap();
    for result in [owner.join().unwrap(), waiter.join().unwrap()] {
        assert_eq!(result.unwrap_err().to_string(), "fixture HTTP failure");
    }
    let recovered = request(key, true, |_| {
        Ok(Response {
            value: Some(serde_json::json!({"recovered":true})),
            etag: None,
            cacheable: true,
        })
    })
    .unwrap();
    assert_eq!(recovered.value["recovered"], true);
    drop(observed);
    forget(&root);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn decoded_preview_cannot_publish_over_a_replacement() {
    type Gate = (mpsc::Sender<()>, mpsc::Receiver<()>);
    static GATE: OnceLock<Mutex<Option<Gate>>> = OnceLock::new();
    #[derive(Debug)]
    struct Preview(Value);
    impl<'de> serde::Deserialize<'de> for Preview {
        fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
            let value = Value::deserialize(d)?;
            let gate = GATE.get().unwrap().lock().unwrap().take();
            if let Some((entered, release)) = gate {
                entered.send(()).unwrap();
                release.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            Ok(Self(value))
        }
    }
    let root = root();
    let key = key(&root, "fixture:typed-race", None);
    let make = |generation: &str| {
        Arc::new(Record {
            at: 0,
            accessed: chrono::Utc::now().timestamp(),
            etag: None,
            value: serde_json::json!({"generation":generation}),
            generation: generation.into(),
            bytes: 32,
        })
    };
    remember_record(&key, make("old"));
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    *GATE.get_or_init(Mutex::default).lock().unwrap() = Some((entered_tx, release_rx));
    assert!(peek::<Preview>(key.clone(), || {}).is_none());
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    remember_record(&key, make("new"));
    release_tx.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(value) = peek::<Preview>(key.clone(), || {}) {
            assert_eq!(value.0["generation"], "new");
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    forget(&root);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn fresh_memory_response_survives_a_rejected_unwritable_disk_copy() {
    let root = root();
    let key = key(&root, "fixture:memory-only", None);
    let disk = disk(&key.root);
    {
        let _guard = disk.lock(&key.hash).unwrap();
        disk.write(&key.hash, 0, None, &serde_json::json!({"old":true}))
            .unwrap();
    }
    invalidate_disk(&key);
    remember_record(
        &key,
        Arc::new(Record {
            at: chrono::Utc::now().timestamp(),
            accessed: chrono::Utc::now().timestamp(),
            value: serde_json::json!({"new":true}),
            bytes: 12,
            etag: None,
            generation: "memory-only".into(),
        }),
    );
    assert_eq!(
        request(key.clone(), false, |_| {
            panic!("Fresh memory response must survive unavailable disk storage")
        })
        .unwrap()
        .value["new"],
        true
    );
    assert_eq!(read(&key).unwrap().value["new"], true);
    forget(&root);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn clear_waits_for_inflight_response_and_forgets_memory_and_disk() {
    let root = root();
    let key = key(&root, "fixture:clear-inflight", None);
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let worker_key = key.clone();
    let worker = std::thread::spawn(move || {
        request(worker_key, true, |_| {
            entered_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            Ok(Response {
                value: Some(serde_json::json!({"old":true})),
                etag: None,
                cacheable: true,
            })
        })
        .unwrap()
    });
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let clear_root = root.clone();
    let (done_tx, done_rx) = mpsc::channel();
    let cleaner = std::thread::spawn(move || {
        let result = clear(&clear_root);
        done_tx.send(result).unwrap();
    });
    assert!(done_rx.recv_timeout(Duration::from_millis(50)).is_err());
    release_tx.send(()).unwrap();
    worker.join().unwrap();
    done_rx
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap();
    cleaner.join().unwrap();
    assert!(hot::<Record>(&MemoryKey::Raw(key.clone()), false).is_none());
    assert!(read(&key).is_none());
    forget(&root);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn clear_forgets_typed_entries_and_preview_does_not_wait_for_cleanup() {
    let root = root();
    let key = key(&root, "fixture:clear-typed", None);
    request(key.clone(), true, |_| {
        Ok(Response {
            value: Some(serde_json::json!({"ok":true})),
            etag: None,
            cacheable: true,
        })
    })
    .unwrap();
    peek::<Value>(key.clone(), || {});
    wait_for_background();
    assert!(hot::<Value>(&MemoryKey::Typed(key.clone(), TypeId::of::<Value>()), false).is_some());
    let gate = root_gate(&root);
    let exclusive = gate.write().unwrap();
    let started = Instant::now();
    assert!(peek::<Value>(key.clone(), || {}).is_none());
    assert!(started.elapsed() < Duration::from_millis(100));
    drop(exclusive);
    clear(&root).unwrap();
    assert!(hot::<Value>(&MemoryKey::Typed(key.clone(), TypeId::of::<Value>()), false).is_none());
    assert!(read(&key).is_none());
    forget(&root);
    std::fs::remove_dir_all(root).unwrap();
}
