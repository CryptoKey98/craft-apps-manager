//! Extract verified tool archives into private staging directories.
use crate::{files, jobs::Job};
use anyhow::{ensure, Context, Result};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::Command,
};

const PROBE_XZ: &[u8] = include_bytes!("../../../../assets/tool-extractor-probe.tar.xz");
const MAX_TAR_BYTES: u64 = 32 * 1024 * 1024 * 1024;

pub(super) fn system_tar() -> Option<PathBuf> {
    use windows::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{FOLDERID_Windows, SHGetKnownFolderPath, KF_FLAG_DEFAULT},
    };
    let root = unsafe {
        let value = SHGetKnownFolderPath(&FOLDERID_Windows, KF_FLAG_DEFAULT, None).ok()?;
        let path = value.to_string();
        CoTaskMemFree(Some(value.0.cast()));
        PathBuf::from(path.ok()?)
    };
    // Sysnative reaches the native tool from a 32-bit manager on 64-bit Windows.
    [
        root.join("Sysnative/tar.exe"),
        root.join("System32/tar.exe"),
    ]
    .into_iter()
    .find(|p| p.is_file())
}

pub(super) struct Stage(pub(super) PathBuf);
impl Stage {
    pub(super) fn new(cache: &Path) -> Result<Self> {
        fs::create_dir_all(cache)?;
        let path = cache.join(format!("tool-extract-{}", uuid::Uuid::new_v4().simple()));
        files::inside(&path, cache)?;
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        if let Some(parent) = self.0.parent() {
            let _ = files::remove_managed(&self.0, parent);
        }
    }
}

fn native_extract(tar: &Path, archive: &Path, output: &Path, job: &Job) -> Result<()> {
    fs::create_dir(output)?;
    job.log("Extracting build tools; extracted entries will appear below.");
    // Do not use -P: it disables tar's traversal and symlink protections.
    job.run(
        Command::new(tar)
            // Keep the first entry rather than overwrite it with a duplicate.
            .arg("-xkvf")
            .arg(std::path::absolute(archive)?)
            .arg("-C")
            .arg(output),
        false,
    )?;
    validate_native_payload(output, job)
}

fn validate_native_payload(root: &Path, job: &Job) -> Result<()> {
    use std::os::windows::{fs::MetadataExt, io::AsRawHandle};
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT,
        },
    };

    let mut count = 0usize;
    let mut total = 0u64;
    let mut last_update = std::time::Instant::now();
    job.stage("Checking extracted tools", None, "Checking files and links");
    job.log("Checking extracted build tools before activation.");
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        job.check()?;
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        ensure!(
            metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0,
            "Extracted tool archive contains a linked path"
        );
        ensure!(
            metadata.is_file() || metadata.is_dir(),
            "Unsupported extracted tool entry"
        );
        if entry.depth() == 0 {
            continue;
        }
        count += 1;
        if metadata.is_file() {
            let file = File::open(entry.path())?;
            let mut info = BY_HANDLE_FILE_INFORMATION::default();
            // The borrowed handle remains valid while `file` is alive.
            unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
                .context("Could not verify extracted tool file links")?;
            ensure!(
                info.nNumberOfLinks == 1,
                "Extracted tool archive contains a hard link"
            );
            total = total
                .checked_add(metadata.len())
                .context("Tool size overflow")?;
        }
        ensure!(
            count <= 200_000 && total <= MAX_TAR_BYTES,
            "Extracted tools exceed limits"
        );
        if last_update.elapsed() >= std::time::Duration::from_millis(250) {
            job.stage(
                "Checking extracted tools",
                None,
                format!("{count} entries checked"),
            );
            last_update = std::time::Instant::now();
        }
    }
    job.log(&format!(
        "Checked {count} extracted entries ({total} bytes)."
    ));
    Ok(())
}

fn native_supported(tar: &Path, stage: &Path, job: &Job) -> Result<bool> {
    let archive = stage.join("probe.tar.xz");
    let output = stage.join("probe");
    fs::write(&archive, PROBE_XZ)?;
    let result = native_extract(tar, &archive, &output, job);
    job.check()?;
    if let Err(error) = result {
        if crate::jobs::is_cancelled(&error) {
            return Err(error);
        }
        // Only a known-good capability probe can select the fallback.
        if error
            .downcast_ref::<std::io::Error>()
            .is_some_and(|e| e.kind() != std::io::ErrorKind::NotFound)
        {
            return Err(error);
        }
        return Ok(false);
    }
    Ok(fs::read(output.join("probe.txt"))? == b"Craft tool archive probe\n")
}

pub(super) fn extract_tar_xz(
    archive: &Path,
    dest: &Path,
    job: &Job,
    native: Option<&Path>,
    fallback: impl FnOnce() -> Result<PathBuf>,
) -> Result<()> {
    job.check()?;
    ensure!(!dest.exists(), "Extraction destination already exists");
    // Keep staging beside the destination so activation is atomic even when
    // the library download cache and build-tools folder are on different drives.
    let stage = Stage::new(dest.parent().context("Missing tool destination parent")?)?;
    let payload = stage.0.join("payload");
    if let Some(tar) = native.filter(|p| p.is_file()) {
        if native_supported(tar, &stage.0, job)? {
            job.stage("Extracting tools", None, "Using Windows archive extraction");
            native_extract(tar, archive, &payload, job)?;
        } else {
            let decoded = fallback_decode(archive, &stage.0, job, fallback)?;
            extract_tar(&decoded, &payload, job)?;
        }
    } else {
        let decoded = fallback_decode(archive, &stage.0, job, fallback)?;
        extract_tar(&decoded, &payload, job)?;
    }
    job.check()?;
    fs::rename(payload, dest)?;
    Ok(())
}

fn fallback_decode(
    archive: &Path,
    stage: &Path,
    job: &Job,
    fallback: impl FnOnce() -> Result<PathBuf>,
) -> Result<PathBuf> {
    job.check()?;
    job.log("Windows TAR.XZ extraction is unavailable; using the optional 7-Zip build tool.");
    let seven = fallback()?;
    let output = stage.join("fallback");
    fs::create_dir(&output)?;
    // XZ is a single stream with no stored pathname. Only decode the outer
    // compression here; never let 7-Zip extract the TAR members.
    job.run(
        Command::new(seven)
            .args(["x", "-txz", "-y"])
            .arg(archive)
            .arg(format!("-o{}", output.display())),
        false,
    )?;
    let name = archive.file_stem().context("Missing tool archive name")?;
    let decoded = output.join(name);
    files::inside(&decoded, &output)?;
    files::no_links(&output)?;
    ensure!(
        decoded.is_file(),
        "Fallback did not produce the expected tool TAR"
    );
    Ok(decoded)
}

fn tar_entries(
    archive: &Path,
    job: &Job,
    mut consume: impl FnMut(&str, bool, &mut dyn Read) -> Result<()>,
) -> Result<()> {
    let mut tar = tar::Archive::new(File::open(archive)?);
    let mut seen = BTreeSet::new();
    let mut total = 0u64;
    let mut count = 0usize;
    for entry in tar.entries()? {
        job.check()?;
        let mut entry = entry?;
        let kind = entry.header().entry_type();
        ensure!(
            kind.is_file() || kind.is_dir(),
            "Tool archive contains a link or special file"
        );
        let path = String::from_utf8(entry.path_bytes().into_owned())
            .context("Non-UTF8 tool archive path")?;
        let path = path.trim_end_matches('/');
        files::safe_relative(path)?;
        ensure!(
            !path
                .chars()
                .any(|c| c.is_control() || "<>\"|?*".contains(c)),
            "Unsafe Windows tool archive path"
        );
        ensure!(
            seen.insert(path.to_lowercase()),
            "Duplicate tool archive path"
        );
        count += 1;
        total = total
            .checked_add(entry.size())
            .context("Tool archive size overflow")?;
        ensure!(
            count <= 200_000 && total <= MAX_TAR_BYTES,
            "Tool archive exceeds extraction limits"
        );
        consume(path, kind.is_dir(), &mut entry)?;
    }
    Ok(())
}

fn copy_checked(reader: &mut dyn Read, writer: &mut dyn Write, job: &Job) -> Result<()> {
    let mut buffer = [0u8; 65536];
    loop {
        job.check()?;
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            return Ok(());
        }
        writer.write_all(&buffer[..n])?;
    }
}

fn extract_tar(archive: &Path, dest: &Path, job: &Job) -> Result<()> {
    ensure!(!dest.exists(), "Extraction destination already exists");
    let mut file = File::open(archive)?;
    let size = file.metadata()?.len();
    ensure!(
        (1024..=MAX_TAR_BYTES).contains(&size) && size % 512 == 0,
        "Invalid tool TAR length"
    );
    file.seek(SeekFrom::End(-1024))?;
    let mut end = [0u8; 1024];
    file.read_exact(&mut end)?;
    ensure!(end.iter().all(|b| *b == 0), "Missing tool TAR end marker");
    tar_entries(archive, job, |_, _, reader| {
        copy_checked(reader, &mut std::io::sink(), job)
    })?;
    job.check()?;
    fs::create_dir(dest)?;
    let result = tar_entries(archive, job, |name, directory, reader| {
        let path = dest.join(name);
        files::inside(&path, dest)?;
        if directory {
            fs::create_dir_all(path)?;
        } else {
            fs::create_dir_all(path.parent().context("Missing tool parent")?)?;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)?;
            copy_checked(reader, &mut file, job)?;
        }
        Ok(())
    });
    if result.is_err() {
        files::remove_managed(dest, dest.parent().context("Missing stage parent")?)?;
    }
    result
}

#[cfg(test)]
#[path = "extraction_tests.rs"]
mod tests;
