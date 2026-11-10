use super::*;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

const ARCHIVE: &str = "rollback.zip";

fn directories(source: &Path) -> Result<BTreeMap<PathBuf, u32>> {
    let mut result = BTreeMap::new();
    for entry in walkdir::WalkDir::new(source).min_depth(1) {
        let entry = entry?;
        if entry.file_type().is_dir() {
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt;
                entry.metadata()?.permissions().mode() & 0o777
            };
            #[cfg(not(unix))]
            let mode = 0o755;
            result.insert(entry.path().strip_prefix(source)?.to_path_buf(), mode);
        }
    }
    Ok(result)
}

pub(super) fn create(
    source: &Path,
    destination: &Path,
    point: &mut RollbackPoint,
    job: &Job,
) -> Result<()> {
    files::no_links(source)?;
    point.directories = directories(source)?;
    fs::create_dir(destination)?;
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination.join(ARCHIVE))?;
    let mut zip = zip::ZipWriter::new(file);
    for (relative, mode) in &point.directories {
        job.check()?;
        let name = relative.to_string_lossy().replace('\\', "/");
        files::safe_relative(&name)?;
        zip.add_directory(
            name,
            zip::write::SimpleFileOptions::default().unix_permissions(*mode),
        )?;
    }
    for (index, relative) in point.files.keys().enumerate() {
        job.check()?;
        let name = relative.to_string_lossy().replace('\\', "/");
        files::safe_relative(&name)?;
        let mut options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .compression_level(Some(6))
            .large_file(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            options = options.unix_permissions(
                fs::metadata(source.join(relative))?.permissions().mode() & 0o777,
            );
        }
        #[cfg(not(unix))]
        {
            options = options.unix_permissions(0o644);
        }
        zip.start_file(name, options)?;
        let mut input = fs::File::open(source.join(relative))?;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            job.check()?;
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            zip.write_all(&buffer[..count])?;
        }
        job.stage(
            "Compressing rollback",
            Some((index + 1) as f32 / point.files.len().max(1) as f32),
            format!("{} of {} files", index + 1, point.files.len()),
        );
    }
    zip.finish()?.sync_all()?;
    verify(destination, point, None, Some(job))?;
    ensure!(
        manifest(source)? == point.files && directories(source)? == point.directories,
        "Previous version changed during compression; recovery files were preserved"
    );
    Ok(())
}

/// Verify the complete original manifest while streaming. Extraction uses the
/// same bounded reads, so changed headers cannot expand beyond expected sizes.
pub(super) fn verify(
    folder: &Path,
    point: &RollbackPoint,
    destination: Option<&Path>,
    job: Option<&Job>,
) -> Result<()> {
    files::no_links(folder)?;
    let entries = fs::read_dir(folder)?.collect::<std::io::Result<Vec<_>>>()?;
    ensure!(
        entries.len() == 1
            && entries[0].file_name() == ARCHIVE
            && entries[0].file_type()?.is_file(),
        "Unexpected rollback archive files"
    );
    files::safe_relative(&point.executable.to_string_lossy().replace('\\', "/"))?;
    ensure!(
        point.files.contains_key(&point.executable)
            || point.files.keys().any(|p| p.starts_with(&point.executable)),
        "Previous executable is missing"
    );
    let mut zip = zip::ZipArchive::new(fs::File::open(folder.join(ARCHIVE))?)?;
    ensure!(
        zip.len() == point.files.len() + point.directories.len(),
        "Rollback archive file count changed"
    );
    if let Some(destination) = destination {
        ensure!(!destination.exists(), "Rollback destination already exists");
        fs::create_dir_all(destination)?;
    }
    let mut seen = std::collections::BTreeSet::new();
    for index in 0..zip.len() {
        if let Some(job) = job {
            job.check()?;
        }
        let mut entry = zip.by_index(index)?;
        let name = if entry.is_dir() {
            entry.name().trim_end_matches('/')
        } else {
            entry.name()
        };
        files::safe_relative(name)?;
        let relative = PathBuf::from(name);
        ensure!(seen.insert(relative.clone()), "Duplicate archive entry");
        if entry.is_dir() {
            let mode = point
                .directories
                .get(&relative)
                .context("Unexpected rollback directory")?;
            ensure!(
                entry.size() == 0
                    && entry
                        .unix_mode()
                        .is_none_or(|m| matches!(m & 0o170000, 0 | 0o040000) && m & 0o777 == *mode),
                "Rollback directory changed"
            );
            if let Some(destination) = destination {
                fs::create_dir_all(destination.join(&relative))?;
            }
            continue;
        }
        ensure!(
            !entry.is_dir()
                && entry
                    .unix_mode()
                    .is_none_or(|m| matches!(m & 0o170000, 0 | 0o100000)),
            "Special archive entry"
        );
        let (size, hash) = point
            .files
            .get(&relative)
            .context("Unexpected rollback archive entry")?;
        ensure!(entry.size() == *size, "Rollback archive size changed");
        let mut output = if let Some(destination) = destination {
            let target = destination.join(&relative);
            fs::create_dir_all(target.parent().context("Missing archive parent")?)?;
            Some(
                fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(target)?,
            )
        } else {
            None
        };
        let mut digest = Sha256::new();
        let mut read = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            if let Some(job) = job {
                job.check()?;
            }
            let count = entry.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            read = read
                .checked_add(count as u64)
                .context("Archive size overflow")?;
            ensure!(read <= *size, "Rollback archive exceeds verified size");
            digest.update(&buffer[..count]);
            if let Some(output) = &mut output {
                output.write_all(&buffer[..count])?;
            }
        }
        ensure!(
            read == *size && format!("{:x}", digest.finalize()) == *hash,
            "Rollback archive verification failed"
        );
        #[cfg(unix)]
        if let Some(output) = output {
            use std::os::unix::fs::PermissionsExt;
            output.set_permissions(fs::Permissions::from_mode(
                entry.unix_mode().unwrap_or(0o644) & 0o777,
            ))?;
        }
    }
    #[cfg(unix)]
    if let Some(destination) = destination {
        use std::os::unix::fs::PermissionsExt;
        // Apply restrictive directory permissions only after their children exist.
        for (relative, mode) in point.directories.iter().rev() {
            fs::set_permissions(
                destination.join(relative),
                fs::Permissions::from_mode(*mode),
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn fixture() -> (Fixture, PathBuf, RollbackPoint, Job) {
        let f =
            Fixture(std::env::temp_dir().join(format!("rollback-zip-{}", uuid::Uuid::new_v4())));
        let source = f.0.join("original");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("program"), b"original").unwrap();
        let point = RollbackPoint {
            id: uuid::Uuid::new_v4().to_string(),
            version: "1.0.0".into(),
            bytes: 8,
            architecture: "x64".into(),
            created_at: String::new(),
            installation_id: String::new(),
            app: "fixture".into(),
            package_type: "zip".into(),
            executable: "program".into(),
            files: manifest(&source).unwrap(),
            directories: BTreeMap::new(),
            native: None,
            compressed: true,
        };
        let job = Job::new(f.0.join("test.log"), &Default::default());
        (f, source, point, job)
    }
    #[test]
    fn empty_directories_survive_compressed_restore() {
        let (f, source, mut point, job) = fixture();
        fs::create_dir_all(source.join("profile/empty")).unwrap();
        let folder = f.0.join("archive");
        create(&source, &folder, &mut point, &job).unwrap();
        let destination = f.0.join("restore");
        verify(&folder, &point, Some(&destination), Some(&job)).unwrap();
        assert!(destination.join("profile/empty").is_dir());
    }
    #[test]
    fn added_source_files_cannot_be_silently_excluded_from_the_archive() {
        let (f, source, mut point, job) = fixture();
        fs::write(source.join("new-settings"), b"keep these settings").unwrap();
        assert!(create(&source, &f.0.join("archive"), &mut point, &job).is_err());
        assert!(source.join("new-settings").exists());
    }
    #[test]
    fn cancelled_or_changed_source_cannot_publish_a_verified_archive() {
        let (f, source, mut point, job) = fixture();
        job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(create(&source, &f.0.join("cancelled"), &mut point, &job).is_err());
        job.cancel
            .store(false, std::sync::atomic::Ordering::Relaxed);
        fs::write(source.join("program"), b"changed source").unwrap();
        assert!(create(&source, &f.0.join("changed"), &mut point, &job).is_err());
        assert!(source.join("program").exists());
    }
    #[test]
    fn traversal_and_duplicate_entries_are_rejected() {
        for names in [vec!["../escape"], vec!["program", "prograx"]] {
            let (f, _, mut point, _) = fixture();
            let folder = f.0.join("archive");
            fs::create_dir(&folder).unwrap();
            if names.len() == 2 {
                point
                    .files
                    .insert("second".into(), point.files[Path::new("program")].clone());
            }
            let mut zip = zip::ZipWriter::new(fs::File::create(folder.join(ARCHIVE)).unwrap());
            let duplicate = names.len() == 2;
            for name in names {
                zip.start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(b"original").unwrap();
            }
            zip.finish().unwrap();
            if duplicate {
                // ZipWriter prevents duplicate names; construct an adversarial
                // archive by changing both fixed-length filename headers.
                let path = folder.join(ARCHIVE);
                let mut bytes = fs::read(&path).unwrap();
                for i in 0..bytes.len().saturating_sub(6) {
                    if &bytes[i..i + 7] == b"prograx" {
                        bytes[i..i + 7].copy_from_slice(b"program");
                    }
                }
                fs::write(path, bytes).unwrap();
            }
            assert!(verify(&folder, &point, Some(&f.0.join("restore")), None).is_err());
            assert!(!f.0.join("escape").exists());
        }
    }
    #[cfg(unix)]
    #[test]
    fn executable_permissions_survive_compressed_restore() {
        use std::os::unix::fs::PermissionsExt;
        let (f, source, mut point, job) = fixture();
        fs::set_permissions(source.join("program"), fs::Permissions::from_mode(0o751)).unwrap();
        let folder = f.0.join("archive");
        create(&source, &folder, &mut point, &job).unwrap();
        let dest = f.0.join("restore");
        verify(&folder, &point, Some(&dest), Some(&job)).unwrap();
        assert_eq!(
            fs::metadata(dest.join("program"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o751
        );
    }
}
