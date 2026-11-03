use super::*;
use serde::Deserialize;
use std::hint::black_box;

#[derive(Deserialize)]
struct Legacy {
    value: Value,
}
fn timing(mut action: impl FnMut(), count: usize) -> Value {
    action();
    let mut times = Vec::new();
    for _ in 0..count {
        let start = Instant::now();
        action();
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(f64::total_cmp);
    serde_json::json!({"p50_ms":times[times.len()/2],"p95_ms":times[(times.len()*95).div_ceil(100).saturating_sub(1)]})
}

#[test]
#[ignore = "requires CRAFT_CACHE_BENCH_INPUT pointing to a copied benchmark corpus"]
fn actual_cache_paths() {
    let project = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let allowed = std::fs::canonicalize(project.join("target/api-cache-benchmark")).unwrap();
    let input = std::fs::canonicalize(
        std::env::var_os("CRAFT_CACHE_BENCH_INPUT").expect("Select copied corpus"),
    )
    .unwrap();
    assert!(
        input.starts_with(&allowed),
        "Benchmark must use the isolated corpus"
    );
    let work = allowed.join(format!("rust-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&work).unwrap();
    let disk = disk(&work);
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&input).unwrap() {
        let path = entry.unwrap().path();
        if path
            .extension()
            .is_none_or(|extension| extension != "json" && extension != "bin")
        {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let legacy: Legacy = serde_json::from_slice(&bytes).unwrap();
        let hash = format!(
            "{:x}",
            Sha256::digest(path.file_name().unwrap().as_encoded_bytes())
        );
        let guard = disk.lock(&hash).unwrap();
        let saved = disk
            .write(&hash, chrono::Utc::now().timestamp(), None, &legacy.value)
            .unwrap();
        assert_eq!(disk.read(&hash, None).unwrap().unwrap().value, legacy.value);
        drop(guard);
        entries.push((
            bytes.len(),
            path,
            Key {
                root: work.clone(),
                hash,
                legacy: None,
            },
            saved.bytes,
        ));
    }
    entries.sort_by_key(|entry| entry.0);
    assert!(!entries.is_empty());
    let original_bytes: usize = entries.iter().map(|entry| entry.0).sum();
    let legacy = timing(
        || {
            for (_, path, _, _) in &entries {
                let legacy: Legacy = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
                black_box(legacy.value);
            }
        },
        21,
    );
    let compressed = timing(
        || {
            for (_, _, key, _) in &entries {
                let guard = disk.lock(&key.hash).unwrap();
                black_box(disk.read(&key.hash, None).unwrap().unwrap());
                drop(guard);
            }
        },
        21,
    );
    let stats = disk
        .prune(chrono::Utc::now().timestamp(), DISK_BUDGET, MAX_AGE)
        .unwrap();
    let mut representatives = Vec::new();
    for index in [0, entries.len() / 2, entries.len() - 1] {
        let (_, path, key, _) = &entries[index];
        forget(&work);
        let deadline = Instant::now() + Duration::from_secs(10);
        while peek::<Value>(key.clone(), || {}).is_none() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let old = timing(
            || {
                let legacy: Legacy = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
                black_box(legacy.value);
            },
            101,
        );
        let compressed = timing(
            || {
                let guard = disk.lock(&key.hash).unwrap();
                black_box(disk.read(&key.hash, None).unwrap().unwrap());
                drop(guard);
            },
            101,
        );
        let hot = timing(
            || {
                black_box(peek::<Value>(key.clone(), || {}).expect("Warm preview"));
            },
            1001,
        );
        representatives.push(serde_json::json!({"input_bytes":entries[index].0,"legacy":old,"compressed_disk":compressed,"memory_preview":hot}));
    }
    let report = serde_json::json!({"profile":if cfg!(debug_assertions){"debug"}else{"release"},"entries":entries.len(),"original_bytes":original_bytes,"compressed_body_and_metadata_bytes":stats.bytes,"legacy_corpus":legacy,"compressed_corpus":compressed,"representatives":representatives,"notes":"Warm OS cache; Rust serde_json/flate2 and real path safety/OS locks; no network or actual UI click timing. Memory timing is the nonblocking preview core. Original corpus read only."});
    std::fs::write(
        allowed.join("rust-report.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!("{report}");
    forget(&work);
    // Leave only the report; work is a unique directory strictly under benchmark root.
    std::fs::remove_dir_all(work).unwrap();
}
