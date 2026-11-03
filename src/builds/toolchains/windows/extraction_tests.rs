use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

struct Fixture {
    root: PathBuf,
    job: Job,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("craft tool extraction {}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let job = Job::new(root.join("logs/test.log"), &Default::default());
        Self { root, job }
    }
    fn archive(&self) -> PathBuf {
        let path = self.root.join("LLVM test.tar.xz");
        fs::write(&path, PROBE_XZ).unwrap();
        path
    }
    fn dest(&self) -> PathBuf {
        self.root.join("output")
    }
    fn leftovers(&self) -> Vec<String> {
        fs::read_dir(&self.root)
            .unwrap()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.starts_with("tool-extract-").then_some(name)
            })
            .collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn native_xz_extraction_does_not_request_seven_zip() {
    let f = Fixture::new();
    let tar = system_tar().expect("Windows tar required for this test");
    extract_tar_xz(&f.archive(), &f.dest(), &f.job, Some(&tar), || {
        anyhow::bail!("Fallback must not run")
    })
    .unwrap();
    assert_eq!(
        fs::read(f.dest().join("probe.txt")).unwrap(),
        b"Craft tool archive probe\n"
    );
    assert!(f.leftovers().is_empty());
}

#[test]
fn native_extraction_does_not_rewrite_archive_or_emit_group_warning() {
    let f = Fixture::new();
    let tar = system_tar().expect("Windows tar required for this test");
    extract_tar_xz(&f.archive(), &f.dest(), &f.job, Some(&tar), || {
        anyhow::bail!("Fallback must not run")
    })
    .unwrap();
    let log = fs::read_to_string(f.root.join("logs/test.log")).unwrap_or_default();
    assert!(!log.contains("Can't translate gname"), "{log}");
    assert!(
        log.contains("x probe.txt"),
        "Missing extraction activity: {log}"
    );
    assert!(log.contains("Checking extracted build tools"), "{log}");
    assert!(log.contains("Checked 1 extracted entries"), "{log}");
    assert_eq!(
        fs::read(f.dest().join("probe.txt")).unwrap(),
        b"Craft tool archive probe\n"
    );
}

#[test]
fn native_extraction_keeps_first_duplicate_including_case_collision() {
    for name in ["tool.txt", "TOOL.txt"] {
        let f = Fixture::new();
        let archive = f.root.join("duplicates.tar");
        fs::write(
            &archive,
            tar_bytes(&[("tool.txt", b'0', b"first"), (name, b'0', b"second")]),
        )
        .unwrap();
        extract_tar_xz(
            &archive,
            &f.dest(),
            &f.job,
            Some(&system_tar().unwrap()),
            || panic!("Native extraction must not fall back"),
        )
        .unwrap();
        assert_eq!(fs::read(f.dest().join("tool.txt")).unwrap(), b"first");
        assert!(f.leftovers().is_empty());
    }
}

#[test]
fn native_extraction_rejects_hard_links_before_activation_and_cleans_stage() {
    let f = Fixture::new();
    let archive = f.root.join("links.tar");
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_ustar();
    header.set_mode(0o644);
    header.set_size(4);
    header.set_cksum();
    builder
        .append_data(&mut header, "original", &b"data"[..])
        .unwrap();
    let mut link = tar::Header::new_ustar();
    link.set_entry_type(tar::EntryType::Link);
    link.set_mode(0o644);
    link.set_size(0);
    builder.append_link(&mut link, "alias", "original").unwrap();
    fs::write(&archive, builder.into_inner().unwrap()).unwrap();
    let error = extract_tar_xz(
        &archive,
        &f.dest(),
        &f.job,
        Some(&system_tar().unwrap()),
        || panic!("Hard links must not trigger fallback"),
    )
    .unwrap_err();
    assert!(error.to_string().contains("hard link"), "{error:#}");
    assert!(!f.dest().exists());
    assert!(f.leftovers().is_empty());
}

#[test]
fn native_validation_rejects_links_to_files_outside_stage() {
    let f = Fixture::new();
    let output = f.root.join("staged");
    fs::create_dir(&output).unwrap();
    let original = f.root.join("preserved.txt");
    fs::write(&original, b"preserve").unwrap();
    fs::hard_link(&original, output.join("alias")).unwrap();
    let error = validate_native_payload(&output, &f.job).unwrap_err();
    assert!(error.to_string().contains("hard link"), "{error:#}");
    fs::remove_dir_all(&output).unwrap();
    assert_eq!(fs::read(original).unwrap(), b"preserve");
}

#[test]
fn existing_tool_destination_is_preserved() {
    let f = Fixture::new();
    fs::create_dir(f.dest()).unwrap();
    fs::write(f.dest().join("existing"), b"preserve").unwrap();
    assert!(extract_tar_xz(
        &f.archive(),
        &f.dest(),
        &f.job,
        Some(&system_tar().unwrap()),
        || { panic!("Existing tools must not trigger fallback") }
    )
    .is_err());
    assert_eq!(fs::read(f.dest().join("existing")).unwrap(), b"preserve");
    assert!(f.leftovers().is_empty());
}

#[test]
fn missing_native_extractor_requests_fallback_only_when_needed() {
    let f = Fixture::new();
    let called = AtomicBool::new(false);
    let error = extract_tar_xz(&f.archive(), &f.dest(), &f.job, None, || {
        called.store(true, Ordering::Relaxed);
        anyhow::bail!("Fallback requested")
    })
    .unwrap_err();
    assert!(called.load(Ordering::Relaxed));
    assert!(error.to_string().contains("Fallback requested"));
    assert!(!f.dest().exists());
    assert!(f.leftovers().is_empty());
}

#[test]
fn cancelled_extraction_never_requests_fallback_or_creates_output() {
    let f = Fixture::new();
    f.job.cancel.store(true, Ordering::Relaxed);
    let error = extract_tar_xz(&f.archive(), &f.dest(), &f.job, None, || {
        panic!("No fallback on cancellation")
    })
    .unwrap_err();
    assert!(crate::jobs::is_cancelled(&error));
    assert!(!f.dest().exists());
    assert!(f.leftovers().is_empty());
}

fn tar_bytes(entries: &[(&str, u8, &[u8])]) -> Vec<u8> {
    let mut result = Vec::new();
    for (path, kind, body) in entries {
        let mut h = tar::Header::new_ustar();
        h.set_entry_type(tar::EntryType::new(*kind));
        h.set_mode(0o644);
        h.set_size(body.len() as u64);
        h.as_mut_bytes()[..100].fill(0);
        h.as_mut_bytes()[..path.len()].copy_from_slice(path.as_bytes());
        h.set_cksum();
        result.extend_from_slice(h.as_bytes());
        result.extend_from_slice(body);
        result.resize(result.len().div_ceil(512) * 512, 0);
    }
    result.resize(result.len() + 1024, 0);
    result
}

#[test]
fn unsafe_tar_is_rejected_before_writing_any_member() {
    for (name, kind) in [
        ("../escape", b'0'),
        ("/absolute", b'0'),
        ("C:/escape", b'0'),
        ("safe:stream", b'0'),
        ("NUL.txt", b'0'),
        ("bad\nname", b'0'),
        ("bad\\name", b'0'),
        ("link", b'2'),
        ("hard", b'1'),
        ("device", b'3'),
        ("pipe", b'6'),
    ] {
        let f = Fixture::new();
        let archive = f.root.join("bad.tar");
        fs::write(
            &archive,
            tar_bytes(&[("first.txt", b'0', b"safe"), (name, kind, b"")]),
        )
        .unwrap();
        assert!(
            extract_tar(&archive, &f.dest(), &f.job).is_err(),
            "accepted {name}"
        );
        assert!(!f.dest().exists(), "created output for {name}");
    }
}

#[test]
fn duplicate_case_paths_and_truncated_tar_are_rejected() {
    let f = Fixture::new();
    let archive = f.root.join("bad.tar");
    fs::write(
        &archive,
        tar_bytes(&[("File", b'0', b"one"), ("file", b'0', b"two")]),
    )
    .unwrap();
    assert!(extract_tar(&archive, &f.dest(), &f.job).is_err());
    assert!(!f.dest().exists());
    let mut bytes = tar_bytes(&[("File", b'0', b"one")]);
    bytes.truncate(513);
    fs::write(&archive, bytes).unwrap();
    assert!(extract_tar(&archive, &f.dest(), &f.job).is_err());
    assert!(!f.dest().exists());
}

#[test]
fn corrupt_real_archive_does_not_trigger_fallback() {
    let f = Fixture::new();
    let archive = f.archive();
    fs::write(&archive, b"not xz").unwrap();
    let tar = system_tar().unwrap();
    assert!(
        extract_tar_xz(&archive, &f.dest(), &f.job, Some(&tar), || panic!(
            "No fallback on a corrupt download"
        ))
        .is_err()
    );
    assert!(!f.dest().exists());
    assert!(f.leftovers().is_empty());
}

#[test]
#[ignore = "Requires an existing 7-Zip installation; run explicitly for fallback smoke tests"]
fn existing_seven_zip_fallback_handles_xz_when_windows_tool_is_unusable() {
    let f = Fixture::new();
    let paths = crate::model::Paths::new(f.root.clone(), None);
    let seven = super::super::seven(&paths).expect("Install 7-Zip for this explicit test");
    let invalid_native = Path::new("C:/Windows/System32/where.exe");
    let called = AtomicBool::new(false);
    extract_tar_xz(
        &f.archive(),
        &f.dest(),
        &f.job,
        Some(invalid_native),
        || {
            called.store(true, Ordering::Relaxed);
            Ok(seven)
        },
    )
    .unwrap();
    assert!(called.load(Ordering::Relaxed));
    assert_eq!(
        fs::read(f.dest().join("probe.txt")).unwrap(),
        b"Craft tool archive probe\n"
    );
    assert!(f.leftovers().is_empty());
}

#[test]
#[ignore = "Downloads the official optional 7-Zip SDK into an isolated temporary library"]
fn downloaded_private_fallback_extracts_without_system_installation() {
    let f = Fixture::new();
    let paths = crate::model::Paths::new(f.root.clone(), None);
    let network = crate::network::Network::new(&f.root).unwrap();
    let downloaded = super::super::install_seven(&paths, &network, &f.job).unwrap();
    assert!(downloaded.starts_with(&paths.tools));
    assert!(downloaded.parent().unwrap().join("License.txt").is_file());
    extract_tar_xz(&f.archive(), &f.dest(), &f.job, None, || Ok(downloaded)).unwrap();
    assert_eq!(
        fs::read(f.dest().join("probe.txt")).unwrap(),
        b"Craft tool archive probe\n"
    );
    assert!(f.leftovers().is_empty());
}

#[test]
fn native_extraction_rejects_parent_traversal_and_cleans_staging() {
    let tar = system_tar().unwrap();
    let (name, kind) = ("../outside.txt", b'0');
    let f = Fixture::new();
    let plain = f.root.join("input.tar");
    let compressed = f.root.join("input.tar.xz");
    fs::write(
        &plain,
        tar_bytes(&[("first.txt", b'0', b"safe"), (name, kind, b"")]),
    )
    .unwrap();
    f.job
        .run(
            Command::new(&tar)
                .args(["-cJf"])
                .arg(&compressed)
                .args(["--format", "pax", "-P"])
                .arg(format!("@{}", plain.display())),
            false,
        )
        .unwrap();
    assert!(
        extract_tar_xz(&compressed, &f.dest(), &f.job, Some(&tar), || panic!(
            "Unsafe payload must not fall back"
        ))
        .is_err()
    );
    assert!(!f.dest().exists());
    assert!(!f.root.parent().unwrap().join("outside.txt").exists());
    assert!(f.leftovers().is_empty());
}

#[test]
fn cancellation_after_fallback_selection_cleans_staging_without_running_it() {
    let f = Fixture::new();
    let error = extract_tar_xz(&f.archive(), &f.dest(), &f.job, None, || {
        f.job.cancel.store(true, Ordering::Relaxed);
        Ok(PathBuf::from("must-not-run.exe"))
    })
    .unwrap_err();
    assert!(crate::jobs::is_cancelled(&error));
    assert!(!f.dest().exists());
    assert!(f.leftovers().is_empty());
}
