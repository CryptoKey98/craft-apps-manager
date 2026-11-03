use super::*;
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        // Resolve the existing temporary base (macOS /var is a system symlink),
        // but keep this fixture's own path absent for linked/non-directory tests.
        Self(
            std::env::temp_dir()
                .canonicalize()
                .unwrap()
                .join(format!("craft-disk-cache-{}", uuid::Uuid::new_v4())),
        )
    }
    fn cache(&self) -> DiskCache {
        DiskCache::new(&self.0)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn key(n: u8) -> String {
    format!("{n:064x}")
}
#[test]
fn large_repetitive_payload_is_compressed() {
    let f = Fixture::new();
    let c = f.cache();
    let k = key(1);
    let value = serde_json::json!({"text":"abcdef".repeat(10000)});
    c.write(&k, 10, None, &value).unwrap();
    assert!(fs::metadata(f.0.join(format!("{k}.body"))).unwrap().len() < 1000);
}
#[test]
fn refresh_and_touch_leave_body_unchanged() {
    let f = Fixture::new();
    let c = f.cache();
    let k = key(2);
    let value = serde_json::json!({"text":"abcdef".repeat(10000)});
    let r = c.write(&k, 10, Some("etag".into()), &value).unwrap();
    let path = f.0.join(format!("{k}.body"));
    let before = fs::read(&path).unwrap();
    let refreshed = c.refresh(&k, &r, 20).unwrap();
    assert_ne!(r.generation, refreshed.generation);
    c.touch(&k, 1000).unwrap();
    let read = c.read(&k, None).unwrap().unwrap();
    assert_eq!(read.value, value);
    assert_eq!(read.at, 20);
    assert_eq!(read.accessed, 1000);
    assert_eq!(read.etag.as_deref(), Some("etag"));
    assert_eq!(fs::read(path).unwrap(), before);
}
#[test]
fn corrupt_or_interrupted_pairs_are_misses() {
    let f = Fixture::new();
    let c = f.cache();
    let k = key(3);
    c.write(&k, 10, None, &serde_json::json!({"a":1})).unwrap();
    fs::write(f.0.join(format!("{k}.body")), b"{\"a\":2}").unwrap();
    assert!(c.read(&k, None).unwrap().is_none());
    fs::remove_file(f.0.join(format!("{k}.meta.json"))).unwrap();
    assert!(c.read(&k, None).unwrap().is_none());
}
#[test]
fn legacy_is_promoted_only_after_verified_roundtrip() {
    let f = Fixture::new();
    let c = f.cache();
    let old = key(4);
    let new = key(5);
    fs::create_dir_all(&f.0).unwrap();
    let path = f.0.join(format!("{old}.json"));
    fs::write(&path, br#"{"at":7,"etag":"old","value":{"hello":"world"}}"#).unwrap();
    let r = c.read(&new, Some(&old)).unwrap().unwrap();
    assert_eq!(r.at, 7);
    assert_eq!(r.value["hello"], "world");
    assert!(!path.exists());
    assert!(c.read(&new, None).unwrap().is_some());
}
#[test]
fn decoded_size_and_traversal_are_rejected() {
    let f = Fixture::new();
    let c = f.cache();
    assert!(c
        .write("../escape", 0, None, &serde_json::json!(1))
        .is_err());
    assert!(c
        .write(
            &key(1),
            0,
            None,
            &serde_json::json!("x".repeat(16 * 1024 * 1024))
        )
        .is_err());
}
#[test]
fn prune_respects_age_lru_locks_and_unknown_files() {
    let f = Fixture::new();
    let c = f.cache();
    for (n, at) in [(1, 10), (2, 20), (3, 30)] {
        c.write(&key(n), at, None, &serde_json::json!({"n":n}))
            .unwrap();
    }
    fs::write(f.0.join("keep.txt"), b"unknown").unwrap();
    let held = c.lock(&key(1)).unwrap();
    c.prune(100, 0, 1000).unwrap();
    assert!(c.read(&key(1), None).unwrap().is_some());
    assert!(c.read(&key(2), None).unwrap().is_none());
    assert!(c.read(&key(3), None).unwrap().is_none());
    assert_eq!(fs::read(f.0.join("keep.txt")).unwrap(), b"unknown");
    drop(held);
    c.prune(100, 0, 1000).unwrap();
    assert!(c.read(&key(1), None).unwrap().is_none());
    assert!(f.0.join(".locks/01.lock").exists());
}
#[test]
fn prune_evicts_oldest_access_and_accounts_orphans_and_legacy() {
    let f = Fixture::new();
    let c = f.cache();
    c.write(&key(1), 10, None, &serde_json::json!(1)).unwrap();
    c.write(&key(2), 20, None, &serde_json::json!(2)).unwrap();
    c.touch(&key(1), 1000).unwrap();
    let second_bytes = fs::metadata(f.0.join(format!("{}.body", key(2))))
        .unwrap()
        .len()
        + fs::metadata(f.0.join(format!("{}.meta.json", key(2))))
            .unwrap()
            .len();
    let total: u64 = fs::read_dir(&f.0)
        .unwrap()
        .filter_map(|e| {
            let e = e.unwrap();
            e.file_type()
                .unwrap()
                .is_file()
                .then(|| e.metadata().unwrap().len())
        })
        .sum();
    c.prune(1000, total - second_bytes, 2000).unwrap();
    assert!(c.read(&key(1), None).unwrap().is_some());
    assert!(c.read(&key(2), None).unwrap().is_none());
    fs::write(
        f.0.join(format!("{}.json", key(3))),
        br#"{"at":1,"etag":null,"value":1}"#,
    )
    .unwrap();
    fs::write(f.0.join(format!("{}.body", key(4))), b"orphan").unwrap();
    c.prune(1000, 0, 2000).unwrap();
    assert!(!f.0.join(format!("{}.json", key(3))).exists());
    assert!(!f.0.join(format!("{}.body", key(4))).exists());
}
#[test]
fn budget_eviction_removes_complete_entry() {
    let f = Fixture::new();
    let c = f.cache();
    let k = key(7);
    c.write(&k, 10, None, &serde_json::json!("x".repeat(10000)))
        .unwrap();
    let total = fs::metadata(f.0.join(format!("{k}.body"))).unwrap().len()
        + fs::metadata(f.0.join(format!("{k}.meta.json")))
            .unwrap()
            .len();
    c.prune(10, total - 1, 1000).unwrap();
    assert!(!f.0.join(format!("{k}.body")).exists());
    assert!(!f.0.join(format!("{k}.meta.json")).exists());
}
#[test]
fn gzip_expansion_beyond_cap_is_a_miss() {
    let f = Fixture::new();
    let c = f.cache();
    let k = key(8);
    c.write(&k, 0, None, &serde_json::json!(1)).unwrap();
    let mut gzip = GzEncoder::new(Vec::new(), Compression::fast());
    gzip.write_all(&vec![b'x'; MAX_BODY + 1]).unwrap();
    let body = gzip.finish().unwrap();
    fs::write(f.0.join(format!("{k}.body")), &body).unwrap();
    let mut m = c.metadata(&k).unwrap();
    m.body_hash = hash(&body);
    m.codec = "gzip".into();
    m.decoded_len = MAX_BODY;
    c.write_metadata(&k, &m).unwrap();
    assert!(c.read(&k, None).unwrap().is_none());
}
#[test]
fn cross_process_lock_child() {
    let Some(root) = std::env::var_os("CRAFT_DISK_CACHE_LOCK_CHILD") else {
        return;
    };
    let cache = DiskCache::new(Path::new(&root));
    assert!(cache.try_lock(&key(9)).unwrap().is_none());
    cache.prune(1000, 0, 0).unwrap();
    assert!(cache.read(&key(9), None).unwrap().is_some());
}
#[test]
fn stripe_lock_coordinates_another_process() {
    let f = Fixture::new();
    let c = f.cache();
    c.write(&key(9), 0, None, &serde_json::json!(9)).unwrap();
    let _held = c.lock(&key(9)).unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "network::disk_cache::tests::cross_process_lock_child",
            "--test-threads=1",
        ])
        .env("CRAFT_DISK_CACHE_LOCK_CHILD", &f.0)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
}
#[test]
fn lock_file_count_is_bounded() {
    let f = Fixture::new();
    let c = f.cache();
    for n in 0..256 {
        drop(c.lock(&format!("{n:064x}")).unwrap());
    }
    assert_eq!(fs::read_dir(f.0.join(".locks")).unwrap().count(), 64);
}
#[test]
fn old_unused_entries_expire_even_under_budget() {
    let f = Fixture::new();
    let c = f.cache();
    c.write(&key(1), 1, None, &serde_json::json!(1)).unwrap();
    c.write(&key(2), 100, None, &serde_json::json!(2)).unwrap();
    let stats = c.prune(110, u64::MAX, 100).unwrap();
    assert!(c.read(&key(1), None).unwrap().is_none());
    assert!(c.read(&key(2), None).unwrap().is_some());
    assert_eq!(stats.files, 2);
    assert!(stats.bytes > 0);
}
#[test]
fn linked_root_is_rejected_without_touching_target() {
    let f = Fixture::new();
    let target = Fixture::new();
    fs::create_dir_all(&target.0).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target.0, &f.0).unwrap();
    #[cfg(windows)]
    {
        // Junctions require no Developer Mode or administrator privileges.
        let status = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&f.0)
            .arg(&target.0)
            .output()
            .unwrap();
        assert!(status.status.success());
    }
    let c = f.cache();
    assert!(c.write(&key(1), 0, None, &serde_json::json!(1)).is_err());
    assert!(c.prune(0, 0, 0).is_err());
    assert!(c.clear().is_err());
    assert_eq!(fs::read_dir(&target.0).unwrap().count(), 0);
    #[cfg(unix)]
    fs::remove_file(&f.0).unwrap();
    #[cfg(windows)]
    fs::remove_dir(&f.0).unwrap();
}
#[test]
fn malformed_legacy_is_preserved_as_a_miss() {
    let f = Fixture::new();
    let c = f.cache();
    fs::create_dir_all(&f.0).unwrap();
    let path = f.0.join(format!("{}.json", key(1)));
    let bytes = br#"{"at":"not a timestamp","value":1}"#;
    fs::write(&path, bytes).unwrap();
    assert!(c.read(&key(2), Some(&key(1))).unwrap().is_none());
    assert_eq!(fs::read(path).unwrap(), bytes);
}
#[test]
fn remove_only_deletes_key_owned_files() {
    let f = Fixture::new();
    let c = f.cache();
    let k = key(1);
    c.write(&k, 0, None, &serde_json::json!(1)).unwrap();
    fs::write(f.0.join(format!("{k}.json")), b"legacy").unwrap();
    fs::write(f.0.join(format!("{k}.unknown")), b"keep").unwrap();
    let guard = c.lock(&k).unwrap();
    c.remove(&k).unwrap();
    drop(guard);
    assert!(c.read(&k, None).unwrap().is_none());
    assert!(!f.0.join(format!("{k}.json")).exists());
    assert_eq!(fs::read(f.0.join(format!("{k}.unknown"))).unwrap(), b"keep");
    assert!(f.0.join(".locks/01.lock").exists());
}
#[test]
fn small_payload_uses_identity_and_preserves_json_types() {
    let f = Fixture::new();
    let c = f.cache();
    let k = key(1);
    let value = serde_json::json!({"null":null,"bool":true,"array":[1,"2",false]});
    let r = c.write(&k, 0, None, &value).unwrap();
    assert_eq!(c.metadata(&k).unwrap().codec, "identity");
    assert_eq!(c.read(&k, None).unwrap().unwrap().value, value);
    assert_eq!(r.bytes, serde_json::to_vec(&value).unwrap().len());
}
#[test]
fn same_second_refresh_survives_an_older_prune_snapshot() {
    let f = Fixture::new();
    let c = f.cache();
    let k = key(1);
    let record = c.write(&k, 10, None, &serde_json::json!("old")).unwrap();
    let paths = c.groups().unwrap().remove(&k).unwrap();
    let candidate = c.candidate(k.clone(), paths);
    let refreshed = c.refresh(&k, &record, 10).unwrap();
    assert_ne!(refreshed.generation, record.generation);
    let mut total = u64::MAX;
    c.prune_candidate(&candidate, 100, 0, &mut total, 1000)
        .unwrap();
    assert_eq!(
        c.read(&k, None).unwrap().unwrap().generation,
        refreshed.generation
    );
}
#[test]
fn same_second_replacement_survives_an_older_prune_snapshot() {
    let f = Fixture::new();
    let c = f.cache();
    let k = key(1);
    c.write(&k, 10, None, &serde_json::json!("old")).unwrap();
    let candidate = c.candidate(k.clone(), c.groups().unwrap().remove(&k).unwrap());
    c.write(&k, 10, None, &serde_json::json!("replacement"))
        .unwrap();
    let mut total = u64::MAX;
    c.prune_candidate(&candidate, 100, 0, &mut total, 1000)
        .unwrap();
    assert_eq!(c.read(&k, None).unwrap().unwrap().value, "replacement");
}
#[test]
fn changed_orphan_survives_an_older_prune_snapshot() {
    let f = Fixture::new();
    let c = f.cache();
    let k = key(1);
    fs::create_dir_all(&f.0).unwrap();
    let body = f.0.join(format!("{k}.body"));
    fs::write(&body, b"old").unwrap();
    let candidate = c.candidate(k.clone(), c.groups().unwrap().remove(&k).unwrap());
    fs::write(&body, b"replacement").unwrap();
    let mut total = u64::MAX;
    c.prune_candidate(&candidate, 0, 0, &mut total, 1000)
        .unwrap();
    assert_eq!(fs::read(body).unwrap(), b"replacement");
}
#[test]
#[ignore = "isolated release-mode cold-path instrumentation"]
fn cold_disk_path_components() {
    use std::{hint::black_box, time::Instant};
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/api-cache-benchmark");
    let f = Fixture(base.join(format!("components-{}", uuid::Uuid::new_v4())));
    let c = f.cache();
    let k = key(1);
    c.write(
        &k,
        1,
        None,
        &serde_json::json!({"payload":"small".repeat(1000)}),
    )
    .unwrap();
    let body = f.0.join(format!("{k}.body"));
    fn measure(mut action: impl FnMut()) -> f64 {
        action();
        let start = Instant::now();
        for _ in 0..101 {
            action();
        }
        start.elapsed().as_secs_f64() * 1000.0 / 101.0
    }
    let mut report = serde_json::json!({});
    report["safe_ms"] = serde_json::json!(measure(|| {
        c.safe(&body).unwrap();
    }));
    report["single_metadata_walk_ms"] = serde_json::json!(measure(|| {
        for path in body.ancestors() {
            if let Ok(meta) = fs::symlink_metadata(path) {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    assert_eq!(meta.file_attributes() & 0x400, 0);
                }
                #[cfg(not(windows))]
                assert!(!meta.file_type().is_symlink());
            }
        }
    }));
    report["lock_ms"] = serde_json::json!(measure(|| {
        drop(c.lock(&k).unwrap());
    }));
    report["metadata_ms"] = serde_json::json!(measure(|| {
        black_box(c.metadata(&k).unwrap());
    }));
    report["bounded_body_ms"] = serde_json::json!(measure(|| {
        black_box(c.bounded(&body, MAX_BODY).unwrap());
    }));
    report["read_ms"] = serde_json::json!(measure(|| {
        black_box(c.read(&k, None).unwrap().unwrap());
    }));
    report["lock_and_read_ms"] = serde_json::json!(measure(|| {
        let _g = c.lock(&k).unwrap();
        black_box(c.read(&k, None).unwrap().unwrap());
    }));
    let file = std::env::var_os("CRAFT_COMPONENT_REPORT")
        .unwrap_or_else(|| "components-report.json".into());
    fs::write(base.join(file), serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    println!("{report}");
}
#[test]
fn linked_lock_directory_is_rejected_before_creating_lock_targets() {
    let f = Fixture::new();
    let target = Fixture::new();
    let c = f.cache();
    fs::create_dir_all(&f.0).unwrap();
    fs::create_dir_all(&target.0).unwrap();
    let lockdir = f.0.join(".locks");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target.0, &lockdir).unwrap();
    #[cfg(windows)]
    assert!(std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&lockdir)
        .arg(&target.0)
        .output()
        .unwrap()
        .status
        .success());
    assert!(c.lock(&key(1)).is_err());
    assert!(c.try_lock(&key(1)).is_err());
    assert!(c.clear().is_err());
    assert_eq!(fs::read_dir(&target.0).unwrap().count(), 0);
    #[cfg(unix)]
    fs::remove_file(&lockdir).unwrap();
    #[cfg(windows)]
    fs::remove_dir(&lockdir).unwrap();
}
#[test]
fn existing_non_directory_cache_root_is_rejected() {
    let f = Fixture::new();
    fs::write(&f.0, b"keep").unwrap();
    let c = f.cache();
    assert!(c.lock(&key(1)).is_err());
    assert_eq!(fs::read(&f.0).unwrap(), b"keep");
    fs::remove_file(&f.0).unwrap();
}

#[test]
fn clear_removes_owned_responses_and_preserves_unknown_files() {
    let f = Fixture::new();
    let c = f.cache();
    let k = key(50);
    c.write(&k, 10, None, &serde_json::json!({"ok":true}))
        .unwrap();
    fs::write(f.0.join(format!("{k}.json")), b"legacy").unwrap();
    fs::write(
        f.0.join(format!("{k}.0123456789abcdef0123456789abcdef.tmp")),
        b"temp",
    )
    .unwrap();
    fs::write(f.0.join("settings.json"), b"keep").unwrap();
    fs::create_dir(f.0.join(format!("{}.body", key(51)))).unwrap();
    c.clear().unwrap();
    assert!(c.read(&k, None).unwrap().is_none());
    assert_eq!(fs::read(f.0.join("settings.json")).unwrap(), b"keep");
    assert!(f.0.join(format!("{}.body", key(51))).is_dir());
    assert!(f.0.join(".locks").is_dir());
    assert!(!f.0.join(format!("{k}.json")).exists());
    assert!(!f
        .0
        .join(format!("{k}.0123456789abcdef0123456789abcdef.tmp"))
        .exists());
}
