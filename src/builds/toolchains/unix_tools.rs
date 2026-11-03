use crate::{jobs::Job, model::Paths, network::Network};
use anyhow::{bail, ensure, Context, Result};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{symlink, PermissionsExt},
    path::{Component, Path, PathBuf},
    process::Command,
};

const NODE_VERSION: &str = "v24.22.1";
const MAX_BYTES: u64 = 32 * 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 200_000;
const MAX_TRAILING_BYTES: u64 = 1024 * 1024;

fn node_is_supported(path: &Path) -> bool {
    Command::new(path)
        .arg("--version")
        .output()
        .is_ok_and(|out| {
            out.status.success()
                && String::from_utf8_lossy(&out.stdout)
                    .trim()
                    .strip_prefix('v')
                    .and_then(|version| version.split('.').next())
                    .and_then(|major| major.parse::<u32>().ok())
                    .is_some_and(|major| major >= 20)
        })
}

fn node_pair(paths: &Paths) -> Option<(PathBuf, PathBuf)> {
    let managed = paths.tools.join("node");
    let node = managed.join("bin/node");
    let npm = managed.join("lib/node_modules/npm/bin/npm-cli.js");
    if npm.is_file() && node_is_supported(&node) {
        return Some((node, npm));
    }
    let node = super::system("node")?;
    let npm = fs::canonicalize(super::system("npm")?).ok()?;
    (npm.is_file() && node_is_supported(&node)).then_some((node, npm))
}

pub(super) fn node(paths: &Paths) -> Option<PathBuf> {
    node_pair(paths).map(|(node, _)| node)
}

pub(super) fn npm(paths: &Paths) -> Option<PathBuf> {
    node_pair(paths).map(|(_, npm)| npm)
}

pub(super) fn tool_bins(paths: &Paths) -> Vec<PathBuf> {
    let mut bins = Vec::new();
    if let Some(node) = node(paths) {
        if let Some(bin) = node.parent() {
            bins.push(bin.to_path_buf());
        }
    }
    for (folder, command) in [("cmake", "cmake"), ("pkgconf", "pkgconf")] {
        if let Some(tool) = super::find_in(&paths.tools.join(folder), command) {
            if let Some(bin) = tool.parent() {
                if !bins.iter().any(|path| path == bin) {
                    bins.push(bin.to_path_buf());
                }
            }
        }
    }
    bins
}

pub(super) fn ensure_node(paths: &Paths, job: &Job) -> Result<()> {
    job.check()?;
    if node_pair(paths).is_some() {
        return Ok(());
    }
    let platform = match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "darwin",
        _ => bail!("Managed Node.js is only available on Linux and macOS"),
    };
    let architecture = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        _ => bail!(
            "Managed Node.js supports only x64 and ARM64. Install Node.js 20+ with npm manually."
        ),
    };
    let name = format!("node-{NODE_VERSION}-{platform}-{architecture}.tar.gz");
    let base = format!("https://nodejs.org/dist/{NODE_VERSION}/");
    let network = Network::new(&paths.root)?;
    let checksums = network.text(&format!("{base}SHASUMS256.txt"))?;
    job.check()?;
    let checksum = checksums
        .lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            (parts.next()? == name).then_some(hash)
        })
        .context("Official Node.js archive checksum is missing")?;
    job.stage("Installing Node.js", None, "Setting up Node.js LTS and npm");
    install_tar_gz(
        paths,
        &network,
        job,
        "node",
        &format!("{base}{name}"),
        checksum,
    )?;
    node_pair(paths).context("Installed Node.js could not run with npm")?;
    Ok(())
}

/// This guard owns only a newly created UUID staging directory.
struct Stage {
    path: PathBuf,
    preserve: bool,
}

impl Stage {
    fn create(path: PathBuf) -> Result<Self> {
        fs::create_dir(&path)?;
        Ok(Self {
            path,
            preserve: false,
        })
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        if !self.preserve {
            // Archives can contain read-only directories. Do not follow links
            // while restoring owner permissions needed to discard our stage.
            let mut pending = vec![self.path.clone()];
            while let Some(directory) = pending.pop() {
                if let Ok(metadata) = fs::symlink_metadata(&directory) {
                    if !metadata.file_type().is_dir() {
                        continue;
                    }
                    let _ = fs::set_permissions(
                        &directory,
                        fs::Permissions::from_mode(metadata.permissions().mode() | 0o700),
                    );
                    if let Ok(entries) = fs::read_dir(directory) {
                        for entry in entries.flatten() {
                            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                                pending.push(entry.path());
                            }
                        }
                    }
                }
            }
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

pub(super) fn install_tar_gz(
    paths: &Paths,
    network: &Network,
    job: &Job,
    folder: &str,
    url: &str,
    sha256: &str,
) -> Result<()> {
    crate::files::safe_relative(folder)?;
    ensure!(
        Path::new(folder).components().count() == 1,
        "Invalid tool folder"
    );
    ensure!(
        sha256.len() == 64 && sha256.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Invalid tool archive checksum"
    );
    job.check()?;
    fs::create_dir_all(&paths.tools)?;
    let destination = paths.tools.join(folder);
    crate::files::inside(&destination, &paths.tools)?;
    let mut stage = Stage::create(
        paths
            .tools
            .join(format!(".{folder}-stage-{}", uuid::Uuid::new_v4().simple())),
    )?;
    let archive = stage.path.join("download.tar.gz");
    network.download(url, &archive, job)?;
    job.check()?;
    ensure!(
        crate::files::hash(&archive)?.eq_ignore_ascii_case(sha256),
        "Tool archive checksum mismatch"
    );
    let unpacked = stage.path.join("unpacked");
    extract_tar_gz(&archive, &unpacked, job)?;
    let root = single_top_directory(&unpacked)?;
    job.check()?;
    // Keep the previous installation intact until extraction and validation
    // finish. A failed activation restores it before returning the error.
    let backup = stage.path.join("previous");
    let had_previous = fs::symlink_metadata(&destination).is_ok();
    if had_previous {
        fs::rename(&destination, &backup).context("Could not preserve previous build tool")?;
    }
    if let Err(error) = fs::rename(&root, &destination) {
        if had_previous {
            if let Err(restore) = fs::rename(&backup, &destination) {
                stage.preserve = true;
                bail!("Tool activation failed: {error}. Previous installation is preserved at {} because restoration failed: {restore}", backup.display());
            }
        }
        return Err(error).context("Could not activate extracted build tool");
    }
    Ok(())
}

fn single_top_directory(folder: &Path) -> Result<PathBuf> {
    let mut entries = fs::read_dir(folder)?;
    let entry = entries.next().context("Tool archive is empty")??;
    ensure!(
        entries.next().is_none() && entry.file_type()?.is_dir(),
        "Tool archive must contain exactly one top-level directory"
    );
    Ok(entry.path())
}

fn relative_link_target(link: &Path, target: &Path) -> Result<PathBuf> {
    ensure!(
        !target.as_os_str().is_empty() && !target.is_absolute(),
        "Unsafe archive link target"
    );
    let mut resolved = link
        .parent()
        .context("Archive link has no parent")?
        .to_path_buf();
    for component in target.components() {
        match component {
            Component::Normal(part) => resolved.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                ensure!(
                    resolved.components().count() > 1,
                    "Archive link escapes tool directory"
                );
                resolved.pop();
            }
            _ => bail!("Unsafe archive link target"),
        }
    }
    ensure!(
        !resolved.as_os_str().is_empty(),
        "Archive link escapes tool directory"
    );
    Ok(resolved)
}

fn extract_tar_gz(archive: &Path, destination: &Path, job: &Job) -> Result<()> {
    job.check()?;
    let mut stage = Stage::create(destination.to_path_buf())?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(File::open(archive)?));
    let mut seen = BTreeSet::new();
    let mut links = Vec::new();
    let mut directories = Vec::new();
    let mut bytes = 0u64;
    for (index, entry) in tar.entries()?.enumerate() {
        job.check()?;
        ensure!(index < MAX_ENTRIES, "Too many tool archive entries");
        let mut entry = entry?;
        let raw_path = entry.path_bytes();
        let name = std::str::from_utf8(&raw_path).context("Tool archive path is not UTF-8")?;
        crate::files::safe_relative(name.trim_end_matches('/'))?;
        let relative = PathBuf::from(name);
        ensure!(seen.insert(relative.clone()), "Duplicate tool archive path");
        let output = destination.join(&relative);
        bytes = bytes
            .checked_add(entry.size())
            .context("Tool archive size overflow")?;
        ensure!(bytes <= MAX_BYTES, "Tool archive is too large");
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            fs::create_dir_all(&output)?;
            directories.push((output, entry.header().mode()? & 0o777));
        } else if kind.is_file() {
            fs::create_dir_all(output.parent().context("Archive entry has no parent")?)?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&output)?;
            let mut buffer = [0u8; 65536];
            loop {
                job.check()?;
                let length = entry.read(&mut buffer)?;
                if length == 0 {
                    break;
                }
                file.write_all(&buffer[..length])?;
            }
            file.set_permissions(fs::Permissions::from_mode(entry.header().mode()? & 0o777))?;
        } else if kind.is_symlink() {
            let target = entry
                .link_name()?
                .context("Archive link target missing")?
                .into_owned();
            let resolved = relative_link_target(&relative, &target)?;
            links.push((relative, target, resolved));
        } else {
            bail!("Hard links and special files are not allowed in tool archives");
        }
    }
    // tar stops at its zero blocks. Read the rest of the gzip stream so CRC
    // and truncation failures cannot reach activation, while bounding padding.
    let mut gzip = tar.into_inner();
    let mut tail_bytes = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        job.check()?;
        let length = gzip
            .read(&mut buffer)
            .context("Invalid gzip archive trailer")?;
        if length == 0 {
            break;
        }
        tail_bytes += length as u64;
        ensure!(
            tail_bytes <= MAX_TRAILING_BYTES,
            "Too much trailing data in tool archive"
        );
    }
    let top = single_top_directory(destination)?;
    let canonical_top = fs::canonicalize(&top)?;
    // Resolve only existing internal targets. Deferring links prevents archive
    // entries from writing through a symlink created by an earlier entry.
    for _ in 0..64 {
        if links.is_empty() {
            break;
        }
        let previous = links.len();
        let mut pending = Vec::new();
        for (relative, target, resolved) in links {
            job.check()?;
            // Resolve the original target through any preceding links; lexical
            // removal of `..` would be incorrect when an ancestor is a link.
            let candidate = destination
                .join(relative.parent().context("Archive link has no parent")?)
                .join(&target);
            match fs::canonicalize(&candidate) {
                Ok(canonical) => {
                    ensure!(
                        canonical.starts_with(&canonical_top),
                        "Archive link escapes tool directory"
                    );
                    let output = destination.join(relative);
                    fs::create_dir_all(output.parent().context("Archive link has no parent")?)?;
                    symlink(target, output)?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    pending.push((relative, target, resolved))
                }
                Err(error) => return Err(error.into()),
            }
        }
        links = pending;
        ensure!(
            links.is_empty() || links.len() < previous,
            "Tool archive has dangling or cyclic links"
        );
    }
    ensure!(links.is_empty(), "Tool archive link nesting exceeds limit");
    for (directory, mode) in directories.into_iter().rev() {
        job.check()?;
        fs::set_permissions(directory, fs::Permissions::from_mode(mode))?;
    }
    job.check()?;
    stage.preserve = true;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: PathBuf,
        job: Job,
    }
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("craft-unix-tools-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            let job = Job::new(root.join("job.log"), &Default::default());
            Self { root, job }
        }
        fn archive(&self, entries: &[(&str, tar::EntryType, &str)]) -> PathBuf {
            let archive = self.root.join("tools.tar.gz");
            let gzip = flate2::write::GzEncoder::new(
                File::create(&archive).unwrap(),
                flate2::Compression::default(),
            );
            let mut builder = tar::Builder::new(gzip);
            for (name, kind, content) in entries {
                let mut header = tar::Header::new_gnu();
                header.set_entry_type(*kind);
                header.set_mode(0o755);
                header.set_size(if kind.is_file() {
                    content.len() as u64
                } else {
                    0
                });
                // Raw names let the fixture represent malicious traversal that
                // tar::Builder's convenient append_path API correctly rejects.
                header.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
                if kind.is_symlink() || kind.is_hard_link() {
                    header.set_link_name(content).unwrap();
                }
                header.set_cksum();
                builder
                    .append(
                        &header,
                        if kind.is_file() {
                            content.as_bytes()
                        } else {
                            &[]
                        },
                    )
                    .unwrap();
            }
            builder.into_inner().unwrap().finish().unwrap();
            archive
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn node_tarball_keeps_executable_and_internal_npm_symlink() {
        let f = Fixture::new();
        let archive = f.archive(&[
            (
                "node/bin/npm",
                tar::EntryType::Symlink,
                "../lib/node_modules/npm/bin/npm-cli.js",
            ),
            (
                "node/bin/node",
                tar::EntryType::Regular,
                "#!/bin/sh\necho v24.22.1\n",
            ),
            (
                "node/lib/node_modules/npm/bin/npm-cli.js",
                tar::EntryType::Regular,
                "npm code",
            ),
        ]);
        let output = f.root.join("output");
        extract_tar_gz(&archive, &output, &f.job).unwrap();
        assert_eq!(
            fs::read_to_string(output.join("node/bin/npm")).unwrap(),
            "npm code"
        );
        assert_ne!(
            fs::metadata(output.join("node/bin/node"))
                .unwrap()
                .permissions()
                .mode()
                & 0o111,
            0
        );
    }

    #[test]
    fn unsafe_archive_entries_are_rejected_and_partial_output_removed() {
        for entries in [
            vec![("node/../escape", tar::EntryType::Regular, "bad")],
            vec![("node/bin/npm", tar::EntryType::Symlink, "../../../escape")],
            vec![
                ("node/bin/npm", tar::EntryType::Symlink, "../../node/target"),
                ("node/target", tar::EntryType::Regular, "a"),
            ],
            vec![("node/bin/npm", tar::EntryType::Symlink, "/etc/passwd")],
            vec![("node/file", tar::EntryType::Link, "node/other")],
            vec![
                ("node/file", tar::EntryType::Regular, "a"),
                ("node/file", tar::EntryType::Regular, "b"),
            ],
            vec![
                ("node/link", tar::EntryType::Symlink, "target"),
                ("node/link", tar::EntryType::Symlink, "target"),
                ("node/target", tar::EntryType::Regular, "a"),
            ],
            vec![("node/link", tar::EntryType::Symlink, "missing")],
            vec![
                ("node/file", tar::EntryType::Regular, "a"),
                ("other/file", tar::EntryType::Regular, "b"),
            ],
        ] {
            let f = Fixture::new();
            let archive = f.archive(&entries);
            let output = f.root.join("output");
            assert!(
                extract_tar_gz(&archive, &output, &f.job).is_err(),
                "{entries:?}"
            );
            assert!(
                !output.exists(),
                "Failed extraction retained partial output: {entries:?}"
            );
            assert!(!f.root.join("escape").exists());
        }
    }

    #[test]
    fn cancellation_leaves_no_extraction_directory() {
        let f = Fixture::new();
        let archive = f.archive(&[("node/file", tar::EntryType::Regular, "a")]);
        f.job
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let output = f.root.join("output");
        let error = extract_tar_gz(&archive, &output, &f.job).unwrap_err();
        assert!(crate::jobs::is_cancelled(&error));
        assert!(!output.exists());
    }

    #[test]
    fn corrupt_or_truncated_gzip_trailer_rejects_valid_tar_and_cleans_output() {
        for truncated in [false, true] {
            let f = Fixture::new();
            let archive =
                f.archive(&[("node/file", tar::EntryType::Regular, "valid tar contents")]);
            let mut bytes = fs::read(&archive).unwrap();
            if truncated {
                bytes.truncate(bytes.len() - 4);
            } else {
                let checksum = bytes.len() - 8;
                bytes[checksum] ^= 1;
            }
            fs::write(&archive, bytes).unwrap();
            let output = f.root.join("output");
            assert!(extract_tar_gz(&archive, &output, &f.job).is_err());
            assert!(!output.exists());
        }
    }

    #[test]
    fn owned_stage_cleanup_handles_unreadable_directories_without_following_links() {
        let f = Fixture::new();
        let outside = f.root.join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep"), "keep").unwrap();
        let stage = Stage::create(f.root.join("stage")).unwrap();
        let nested = stage.path.join("nested");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("remove"), "remove").unwrap();
        symlink(&outside, stage.path.join("link")).unwrap();
        fs::set_permissions(&nested, fs::Permissions::from_mode(0o0)).unwrap();
        fs::set_permissions(&stage.path, fs::Permissions::from_mode(0o0)).unwrap();
        let stage_path = stage.path.clone();
        drop(stage);
        assert!(!stage_path.exists());
        assert_eq!(fs::read_to_string(outside.join("keep")).unwrap(), "keep");
    }

    #[test]
    fn node_version_probe_rejects_old_and_failed_commands() {
        let f = Fixture::new();
        let script = f.root.join("node");
        for (body, supported) in [
            ("#!/bin/sh\necho v18.20.8\n", false),
            ("#!/bin/sh\necho v24.22.1\n", true),
            ("#!/bin/sh\necho v24.22.1\nexit 1\n", false),
        ] {
            fs::write(&script, body).unwrap();
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
            assert_eq!(node_is_supported(&script), supported);
        }
    }

    #[test]
    fn installation_verifies_checksum_before_replacing_existing_tool_and_cleans_stage() {
        use std::net::TcpListener;

        for valid_checksum in [false, true] {
            let f = Fixture::new();
            let paths = Paths::new(f.root.clone(), None);
            let destination = paths.tools.join("node");
            fs::create_dir_all(&destination).unwrap();
            fs::write(destination.join("old-tool"), "previous installation").unwrap();
            let archive = f.archive(&[(
                "node-package/bin/node",
                tar::EntryType::Regular,
                "new installation",
            )]);
            let bytes = fs::read(&archive).unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/tools.tar.gz", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let (mut connection, _) = listener.accept().unwrap();
                let mut request = [0u8; 8192];
                assert!(connection.read(&mut request).unwrap() > 0);
                write!(
                    connection,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len()
                )
                .unwrap();
                connection.write_all(&bytes).unwrap();
            });
            let checksum = if valid_checksum {
                crate::files::hash(&archive).unwrap()
            } else {
                "0".repeat(64)
            };
            let result = install_tar_gz(
                &paths,
                &Network::new(&f.root).unwrap(),
                &f.job,
                "node",
                &url,
                &checksum,
            );
            server.join().unwrap();
            if valid_checksum {
                result.unwrap();
                assert_eq!(
                    fs::read_to_string(destination.join("bin/node")).unwrap(),
                    "new installation"
                );
                assert!(!destination.join("old-tool").exists());
            } else {
                assert!(result
                    .unwrap_err()
                    .to_string()
                    .contains("checksum mismatch"));
                assert_eq!(
                    fs::read_to_string(destination.join("old-tool")).unwrap(),
                    "previous installation"
                );
            }
            assert_eq!(
                fs::read_dir(&paths.tools).unwrap().count(),
                1,
                "Owned download, stage or backup was retained"
            );
        }
    }
}
