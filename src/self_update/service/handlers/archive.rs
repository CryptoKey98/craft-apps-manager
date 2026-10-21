use super::StagedPayload;
use crate::{files, jobs::Job, manager_updates::model::*};
use anyhow::{ensure, Context, Result};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const MAX_ENTRIES: usize = 100_000;
pub const MAX_FILE: u64 = 1024 * 1024 * 1024;
pub const MAX_TOTAL: u64 = 4 * MAX_FILE;
struct Extractor<'a> {
    root: &'a Path,
    job: &'a Job,
    seen: BTreeSet<String>,
    count: usize,
    total: u64,
}
impl Extractor<'_> {
    fn entry(
        &mut self,
        name: &str,
        directory: bool,
        mode: u32,
        reader: &mut dyn Read,
    ) -> Result<()> {
        self.job.check()?;
        let name = name.trim_end_matches('/');
        relative(name)?;
        self.count += 1;
        ensure!(self.count <= MAX_ENTRIES, "Too many archive entries");
        ensure!(
            self.seen.insert(name.to_lowercase()),
            "Duplicate or conflicting archive path"
        );
        let dest = self.root.join(name);
        files::inside(&dest, self.root)?;
        if directory {
            fs::create_dir_all(&dest)?;
            return Ok(());
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        files::inside(&dest, self.root)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&dest)?;
        let mut size = 0;
        let mut buffer = [0; 65536];
        loop {
            self.job.check()?;
            let n = reader.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            size += n as u64;
            self.total += n as u64;
            ensure!(
                size <= MAX_FILE && self.total <= MAX_TOTAL,
                "Archive extraction limit exceeded"
            );
            file.write_all(&buffer[..n])?;
        }
        file.sync_all()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dest, fs::Permissions::from_mode(mode & 0o777))?;
        }
        #[cfg(not(unix))]
        let _ = mode;
        Ok(())
    }
}
pub fn stage(
    package: &PackageRecord,
    archive: &Path,
    stage: &Path,
    job: &Job,
) -> Result<StagedPayload> {
    ensure!(!stage.exists(), "Staging folder already exists");
    ensure!(package.layout.schema == 1, "Unsupported payload layout");
    relative(&package.layout.root)?;
    relative(&package.layout.entrypoint)?;
    fs::create_dir(stage)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(stage, fs::Permissions::from_mode(0o700))?;
    }
    let result = (|| {
        let mut extractor = Extractor {
            root: stage,
            job,
            seen: BTreeSet::new(),
            count: 0,
            total: 0,
        };
        match package.handler.as_str() {
            ZIP => {
                let mut zip = zip::ZipArchive::new(fs::File::open(archive)?)?;
                ensure!(zip.len() <= MAX_ENTRIES, "Too many archive entries");
                for i in 0..zip.len() {
                    let mut entry = zip.by_index(i)?;
                    ensure!(entry.size() <= MAX_FILE, "Archive file too large");
                    let mode = entry.unix_mode().unwrap_or(0o100644);
                    ensure!(
                        matches!(mode & 0o170000, 0 | 0o100000 | 0o040000),
                        "Archive contains link or special file"
                    );
                    let name = entry.name().to_owned();
                    let directory = entry.is_dir();
                    extractor.entry(&name, directory, mode, &mut entry)?;
                }
            }
            TAR => {
                let gzip = flate2::read::GzDecoder::new(fs::File::open(archive)?);
                let mut tar = tar::Archive::new(gzip);
                for entry in tar.entries()?.raw(true) {
                    let mut entry = entry?;
                    let kind = entry.header().entry_type();
                    ensure!(
                        kind.is_file() || kind.is_dir(),
                        "Archive contains link or special file"
                    );
                    ensure!(entry.size() <= MAX_FILE, "Archive file too large");
                    let name = String::from_utf8(entry.path_bytes().to_vec())
                        .context("Non-UTF8 archive path")?;
                    let mode = entry.header().mode()?;
                    extractor.entry(&name, kind.is_dir(), mode, &mut entry)?;
                }
                // Read to EOF to verify the gzip trailer even if tar ended earlier.
                let gzip = tar.into_inner();
                let remaining = std::io::copy(&mut gzip.take(MAX_TOTAL + 1), &mut std::io::sink())?;
                ensure!(remaining <= MAX_TOTAL, "Excessive archive trailer");
            }
            _ => anyhow::bail!("Unsupported archive handler"),
        }
        let root = stage.join(&package.layout.root);
        let entrypoint = root.join(&package.layout.entrypoint);
        files::inside(&entrypoint, stage)?;
        files::no_links(&root)?;
        ensure!(
            entrypoint.is_file(),
            "Declared manager entrypoint is missing"
        );
        Ok(StagedPayload {
            root,
            hash: files::hash(&entrypoint)?,
            entrypoint,
        })
    })();
    if result.is_err() {
        let parent = stage.parent().context("Missing stage parent")?;
        files::remove_managed(stage, parent)?;
    }
    result
}
pub fn declared_paths(package: &PackageRecord, stage: &Path) -> Result<(PathBuf, PathBuf)> {
    relative(&package.layout.root)?;
    relative(&package.layout.entrypoint)?;
    let root = stage.join(&package.layout.root);
    let entry = root.join(&package.layout.entrypoint);
    files::inside(&entry, stage)?;
    Ok((root, entry))
}
