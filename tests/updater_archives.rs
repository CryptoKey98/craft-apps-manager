#![cfg(feature = "service-updater")]
mod common_updater;
use common_updater::*;
use craft_apps_manager::{
    jobs::Job, manager_updates::handlers::archive, model::BuilderPreferences,
};
use std::io::Write;
fn zip(root: &std::path::Path, names: &[&str]) -> std::path::PathBuf {
    let path = root.join("a.zip");
    let mut w = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    for name in names {
        w.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(b"payload").unwrap();
    }
    w.finish().unwrap();
    path
}
#[test]
fn renamed_nested_payload_installs() {
    let temp = Temp::new();
    let path = zip(&temp.0, &["New/Manager/manager"]);
    let mut package = catalog().packages[1].clone();
    package.handler = "archive.zip.v1".into();
    let result = archive::stage(
        &package,
        &path,
        &temp.0.join("stage"),
        &Job::new(temp.0.join("a.log"), &BuilderPreferences::default()),
    )
    .unwrap();
    assert_eq!(std::fs::read(result.entrypoint).unwrap(), b"payload");
}
#[test]
fn rejects_traversal_absolute_links_devices() {
    for name in [
        "../escape",
        "/absolute",
        "C:/escape",
        "CON",
        "dir/name:stream",
        "a/../escape",
    ] {
        let temp = Temp::new();
        let path = zip(&temp.0, &[name]);
        let package = catalog().packages[0].clone();
        assert!(
            archive::stage(
                &package,
                &path,
                &temp.0.join("stage"),
                &Job::new(temp.0.join("a.log"), &BuilderPreferences::default())
            )
            .is_err(),
            "{name}"
        );
        assert!(!temp.0.join("escape").exists());
    }
}
#[test]
fn rejects_conflicting_case_paths() {
    let temp = Temp::new();
    let path = zip(&temp.0, &["Manager/A", "Manager/a"]);
    assert!(archive::stage(
        &catalog().packages[0],
        &path,
        &temp.0.join("stage"),
        &Job::new(temp.0.join("a.log"), &BuilderPreferences::default())
    )
    .is_err());
}
#[test]
fn tar_preserves_executable_mode() {
    let temp = Temp::new();
    let path = temp.0.join("a.tar.gz");
    let gzip = flate2::write::GzEncoder::new(
        std::fs::File::create(&path).unwrap(),
        flate2::Compression::default(),
    );
    let mut tar = tar::Builder::new(gzip);
    let mut header = tar::Header::new_ustar();
    header.set_size(7);
    header.set_mode(0o755);
    header.set_cksum();
    tar.append_data(&mut header, "New/Manager/manager", b"payload".as_slice())
        .unwrap();
    tar.into_inner().unwrap().finish().unwrap();
    let result = archive::stage(
        &catalog().packages[1],
        &path,
        &temp.0.join("stage"),
        &Job::new(temp.0.join("a.log"), &BuilderPreferences::default()),
    )
    .unwrap();
    assert_eq!(std::fs::read(&result.entrypoint).unwrap(), b"payload");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(result.entrypoint)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
    }
}
#[test]
fn partial_stage_never_commits() {
    let temp = Temp::new();
    let path = zip(&temp.0, &["Manager/file", "../escape"]);
    assert!(archive::stage(
        &catalog().packages[0],
        &path,
        &temp.0.join("stage"),
        &Job::new(temp.0.join("a.log"), &BuilderPreferences::default())
    )
    .is_err());
    assert!(!temp.0.join("stage").exists());
}
#[test]
fn extraction_limits() {
    let temp = Temp::new();
    let path = temp.0.join("large.tar.gz");
    let mut header = tar::Header::new_ustar();
    header.set_path("Manager/huge").unwrap();
    header.set_size(archive::MAX_FILE + 1);
    header.set_mode(0o644);
    header.set_cksum();
    let mut gzip = flate2::write::GzEncoder::new(
        std::fs::File::create(&path).unwrap(),
        flate2::Compression::default(),
    );
    gzip.write_all(header.as_bytes()).unwrap();
    gzip.write_all(&[0u8; 1024]).unwrap();
    gzip.finish().unwrap();
    let mut p = catalog().packages[0].clone();
    p.handler = "archive.tar-gzip.v1".into();
    assert!(archive::stage(
        &p,
        &path,
        &temp.0.join("stage"),
        &Job::new(temp.0.join("a.log"), &BuilderPreferences::default())
    )
    .is_err());
}
#[test]
fn tar_symlinks_rejected() {
    let temp = Temp::new();
    let path = temp.0.join("link.tar.gz");
    let mut header = tar::Header::new_ustar();
    header.set_path("Manager/link").unwrap();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_link_name("../../escape").unwrap();
    header.set_size(0);
    header.set_mode(0o777);
    header.set_cksum();
    let gzip = flate2::write::GzEncoder::new(
        std::fs::File::create(&path).unwrap(),
        flate2::Compression::default(),
    );
    let mut tar = tar::Builder::new(gzip);
    tar.append(&header, std::io::empty()).unwrap();
    tar.into_inner().unwrap().finish().unwrap();
    let mut p = catalog().packages[0].clone();
    p.handler = "archive.tar-gzip.v1".into();
    assert!(archive::stage(
        &p,
        &path,
        &temp.0.join("stage"),
        &Job::new(temp.0.join("a.log"), &BuilderPreferences::default())
    )
    .is_err());
}
