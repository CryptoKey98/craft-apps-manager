use anyhow::{bail, Context, Result};
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Component, Path},
};
use walkdir::WalkDir;

pub fn read_json<T: DeserializeOwned>(p: &Path) -> Result<T> {
    let bytes = fs::read(p).with_context(|| format!("Read {}", p.display()))?;
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
    if !crate::portable_mode::applies(p) {
        return Ok(serde_json::from_slice(bytes)?);
    }
    let mut value = serde_json::from_slice(bytes)?;
    crate::portable_mode::convert(p, &mut value, false)?;
    Ok(serde_json::from_value(value)?)
}
pub fn read_or_default<T: DeserializeOwned + Default>(p: &Path) -> Result<T> {
    if p.exists() {
        read_json(p)
    } else {
        Ok(T::default())
    }
}
pub fn write_json<T: Serialize>(p: &Path, v: &T) -> Result<()> {
    write_bytes(p, &json_bytes(p, v)?)
}
pub fn json_bytes<T: Serialize + ?Sized>(p: &Path, v: &T) -> Result<Vec<u8>> {
    if !crate::portable_mode::applies(p) {
        return Ok(serde_json::to_vec_pretty(v)?);
    }
    let mut value = serde_json::to_value(v)?;
    crate::portable_mode::convert(p, &mut value, true)?;
    Ok(serde_json::to_vec_pretty(&value)?)
}
/// Replaces a file atomically so readers never see a partial write.
pub fn write_bytes(p: &Path, bytes: &[u8]) -> Result<()> {
    fs::create_dir_all(p.parent().context("No parent directory")?)?;
    let tmp = p.with_extension(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
    {
        let mut f = File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    let result = crate::platform::atomic_replace(&tmp, p);
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}
pub fn hash(p: &Path) -> Result<String> {
    let mut f = File::open(p)?;
    let mut sha = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        };
        sha.update(&buf[..n]);
    }
    Ok(format!("{:x}", sha.finalize()))
}
pub fn safe_relative(name: &str) -> Result<()> {
    if name.is_empty()
        || name.contains('\\')
        || name.contains(':')
        || name.starts_with('/')
        || name.split('/').any(|part| {
            part == ".." || part.trim_end_matches([' ', '.']) != part && !part.is_empty()
        })
    {
        bail!("Unsafe archive path: {name}");
    }
    for c in Path::new(name).components() {
        if !matches!(c, Component::Normal(_)) {
            bail!("Unsafe archive path: {name}");
        }
    }
    for part in name.split('/').filter(|s| !s.is_empty()) {
        let base = part.split('.').next().unwrap().to_uppercase();
        if ["CON", "PRN", "AUX", "NUL"].contains(&base.as_str())
            || base.starts_with("COM")
                && base[3..].parse::<u8>().is_ok_and(|n| (1..=9).contains(&n))
            || base.starts_with("LPT")
                && base[3..].parse::<u8>().is_ok_and(|n| (1..=9).contains(&n))
        {
            bail!("Reserved Windows archive name");
        }
    }
    Ok(())
}
pub fn linked(p: &Path) -> Result<bool> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt;
        Ok(fs::symlink_metadata(p)?.file_attributes() & 0x400 != 0)
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(fs::symlink_metadata(p)?.file_type().is_symlink())
    }
}
pub fn inside(p: &Path, root: &Path) -> Result<()> {
    let root = std::path::absolute(root)?;
    let p = std::path::absolute(p)?;
    if p == root
        || !p.starts_with(&root)
        || p.components().any(|c| matches!(c, Component::ParentDir))
    {
        bail!("Path is outside managed folder: {}", p.display());
    }
    let mut cursor = p.clone();
    while cursor.starts_with(&root) {
        if cursor.exists() && linked(&cursor)? {
            bail!("Linked managed path: {}", cursor.display());
        }
        if !cursor.pop() {
            break;
        }
    }
    Ok(())
}
pub fn remove_managed(p: &Path, root: &Path) -> Result<()> {
    inside(p, root)?;
    if !p.exists() {
        return Ok(());
    }
    // std remove_dir_all unlinks junctions and symlinks without traversing their targets.
    if p.is_dir() {
        fs::remove_dir_all(p)?
    } else {
        fs::remove_file(p)?
    }
    Ok(())
}
pub fn no_links(root: &Path) -> Result<()> {
    for e in WalkDir::new(root).follow_links(false) {
        let e = e?;
        if linked(e.path())? {
            bail!("Linked path is not allowed: {}", e.path().display());
        }
    }
    Ok(())
}
pub fn extract_zip(archive: &Path, dest: &Path, job: &crate::jobs::Job) -> Result<()> {
    extract_zip_entries(archive, dest, job, false)
}

pub(crate) fn extract_source_zip(
    archive: &Path,
    dest: &Path,
    job: &crate::jobs::Job,
) -> Result<()> {
    extract_zip_entries(archive, dest, job, true)
}

fn extract_zip_entries(
    archive: &Path,
    dest: &Path,
    job: &crate::jobs::Job,
    source: bool,
) -> Result<()> {
    if dest.exists() {
        bail!("Extraction destination already exists");
    }
    let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
    let aliases = if source {
        source_documentation_aliases(&mut zip)?
    } else {
        Default::default()
    };
    let mut archive_root = None;
    for i in 0..zip.len() {
        let e = zip.by_index(i)?;
        safe_relative(e.name().trim_end_matches('/'))?;
        if is_zip_link(&e) && !aliases.contains(&i) {
            bail!("Linked archive entry");
        }
        if source {
            source_relative(e.name(), &mut archive_root)?;
        }
    }
    fs::create_dir_all(dest)?;
    let n = zip.len();
    for i in 0..n {
        job.check()?;
        if aliases.contains(&i) {
            continue;
        }
        let mut e = zip.by_index(i)?;
        // Source ZIPs have already been checked against their pinned commit root.
        // Extract straight into the build directory to avoid the long GitHub wrapper path.
        let relative = if source {
            let (_, relative) = e.name().split_once('/').context("Missing source root")?;
            if relative.is_empty() {
                continue;
            }
            relative
        } else {
            e.name()
        };
        let target = dest.join(relative);
        if e.is_dir() {
            fs::create_dir_all(&target).with_context(|| format!("Extract {}", target.display()))?;
        } else {
            fs::create_dir_all(target.parent().unwrap())
                .with_context(|| format!("Create parent of {}", target.display()))?;
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
                .with_context(|| format!("Extract {}", target.display()))?;
            std::io::copy(&mut e, &mut f)?;
            #[cfg(unix)]
            if let Some(mode) = e.unix_mode() {
                use std::os::unix::fs::PermissionsExt;
                f.set_permissions(fs::Permissions::from_mode(mode & 0o777))?;
            }
        }
        if i % 100 == 0 || i + 1 == n {
            job.stage(
                "Extracting",
                Some((i + 1) as f32 / n as f32),
                format!("{} of {n} files", i + 1),
            );
        }
    }
    Ok(())
}

fn source_relative<'a>(name: &'a str, archive_root: &mut Option<String>) -> Result<&'a str> {
    let (root, relative) = name.split_once('/').context("Missing source root")?;
    anyhow::ensure!(
        archive_root.get_or_insert_with(|| root.to_owned()) == root,
        "Source ZIP contains multiple roots"
    );
    if !relative.is_empty() {
        safe_relative(relative.trim_end_matches('/'))?;
    }
    Ok(relative)
}

fn is_zip_link(entry: &zip::read::ZipFile<'_>) -> bool {
    entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)
}

// Omit redundant documentation aliases only when their original is a regular file
// in the same archive root. Never materialize or follow links on disk.
fn source_documentation_aliases(
    zip: &mut zip::ZipArchive<File>,
) -> Result<std::collections::BTreeSet<usize>> {
    let mut aliases = std::collections::BTreeSet::new();
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        if !is_zip_link(&entry) {
            continue;
        }
        let name = entry.name().to_owned();
        let relative = source_relative(&name, &mut None)?;
        let (parent, file) = relative.rsplit_once('/').unwrap_or(("", relative));
        let mut target = String::new();
        (&mut entry).take(1025).read_to_string(&mut target)?;
        drop(entry);
        let mut parts: Vec<_> = parent.split('/').filter(|part| !part.is_empty()).collect();
        let (target_parent, target_file) = target.rsplit_once('/').unwrap_or(("", target.as_str()));
        anyhow::ensure!(
            (file.ends_with(".md") && target_file.ends_with(".md"))
                || (matches!(file, "LICENSE" | "NOTICE") && target_file == file),
            "Linked source entry: {name}"
        );
        safe_relative(target_file)?;
        if !target_parent.is_empty() {
            for part in target_parent.split('/') {
                if part == ".." {
                    anyhow::ensure!(parts.pop().is_some(), "Linked source entry: {name}");
                } else {
                    safe_relative(part)?;
                    parts.push(part);
                }
            }
        } else if target.starts_with('/') {
            bail!("Linked source entry: {name}");
        }
        parts.push(target_file);
        let root = name.split_once('/').unwrap().0;
        let original_path = format!("{root}/{}", parts.join("/"));
        let original_entry = zip
            .by_name(&original_path)
            .with_context(|| format!("Missing original for source documentation alias: {name}"))?;
        anyhow::ensure!(
            original_entry.is_file() && !is_zip_link(&original_entry),
            "Linked source entry: {name}"
        );
        aliases.insert(index);
    }
    Ok(aliases)
}

pub fn verify_source(p: &Path, name: &str, sha: &str) -> Result<()> {
    if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("Invalid source commit");
    }
    let mut zip = zip::ZipArchive::new(File::open(p)?)?;
    let aliases = source_documentation_aliases(&mut zip)?;
    let prefix = format!("{name}-{sha}/");
    let renamed_prefix = format!("{}-{sha}/", crate::model::repository_name(name));
    let mut cargo = false;
    let mut root = None;
    for i in 0..zip.len() {
        let mut e = zip.by_index(i)?;
        if !e.name().starts_with(&prefix) && !e.name().starts_with(&renamed_prefix) {
            bail!("Source has an unexpected root");
        }
        safe_relative(e.name().trim_end_matches('/'))?;
        let relative = source_relative(e.name(), &mut root)?;
        if aliases.contains(&i) {
            continue;
        }
        if relative == "Cargo.toml" && e.is_file() {
            cargo = true;
        }
        std::io::copy(&mut e, &mut std::io::sink())?;
    }
    if !cargo {
        bail!("Source ZIP has no expected Cargo.toml");
    }
    Ok(())
}

#[cfg(test)]
#[test]
fn source_archive_skips_only_contained_documentation_aliases() {
    let root = std::env::temp_dir().join(format!("craft-source-alias-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    let sha = "a".repeat(40);
    let prefix = format!("mailcraft-{sha}");
    let archive = root.join("source.zip");
    let options = zip::write::SimpleFileOptions::default();
    let make_zip = |link: &str, target: &str| {
        let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
        zip.start_file(format!("{prefix}/Cargo.toml"), options)
            .unwrap();
        zip.write_all(b"[workspace]\n").unwrap();
        zip.start_file(format!("{prefix}/README.md"), options)
            .unwrap();
        zip.write_all(b"Documentation\n").unwrap();
        zip.start_file(format!("{prefix}/docs/LICENSE"), options)
            .unwrap();
        zip.write_all(b"License\n").unwrap();
        zip.add_symlink(format!("{prefix}/{link}"), target, options)
            .unwrap();
        zip.finish().unwrap();
    };
    make_zip("GUIDE.md", "README.md");
    let project = root.join("src");
    let job = crate::jobs::Job::new(root.join("extract.log"), &Default::default());
    let valid = verify_source(&archive, "mailcraft", &sha);
    let extracted = extract_source_zip(&archive, &project, &job);
    let source_files = (
        project.join("Cargo.toml").is_file(),
        project.join("README.md").is_file(),
        project.join("GUIDE.md").exists(),
        project.join(&prefix).exists(),
    );
    let generic_result = extract_zip(&archive, &root.join("generic"), &job);
    make_zip("crates/core/LICENSE", "../../docs/LICENSE");
    let nested_license = verify_source(&archive, "mailcraft", &sha);
    make_zip("GUIDE.md", "../README.md");
    let unsafe_target = verify_source(&archive, "mailcraft", &sha);
    make_zip("src/GUIDE.md", "missing/README.md");
    let missing_target = verify_source(&archive, "mailcraft", &sha);
    make_zip("src/main.rs", "../README.md");
    let source_link = verify_source(&archive, "mailcraft", &sha);
    fs::remove_dir_all(&root).unwrap();
    valid.unwrap();
    extracted.unwrap();
    assert_eq!(source_files, (true, true, false, false));
    assert!(generic_result.is_err());
    nested_license.unwrap();
    assert!(unsafe_target.is_err());
    assert!(missing_target.is_err());
    assert!(source_link.is_err());
}

#[cfg(test)]
#[test]
#[ignore = "Explicit check of externally downloaded source ZIPs"]
fn verifies_and_extracts_pinned_source_fixture() {
    let path = std::path::PathBuf::from(std::env::var_os("CRAFT_SOURCE_ZIP_FIXTURE").unwrap());
    let name = std::env::var("CRAFT_SOURCE_ZIP_APP").unwrap();
    let sha = std::env::var("CRAFT_SOURCE_ZIP_COMMIT").unwrap();
    let root = std::env::temp_dir().join(format!("craft-source-{}", uuid::Uuid::new_v4()));
    let project = root.join("src");
    let job = crate::jobs::Job::new(root.join("extract.log"), &Default::default());
    verify_source(&path, &name, &sha).unwrap();
    let extracted = extract_source_zip(&path, &project, &job);
    let manifest = project.join("Cargo.toml").is_file();
    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }
    extracted.unwrap();
    assert!(manifest);
}
pub fn verify_trees(a: &Path, b: &Path) -> Result<()> {
    let af: Vec<_> = WalkDir::new(a)
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|e| e.file_type().is_file())
        .collect();
    let bf = WalkDir::new(b)
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|e| e.file_type().is_file())
        .count();
    if af.len() != bf {
        bail!("Archive file count mismatch");
    }
    for f in af {
        if hash(f.path())? != hash(&b.join(f.path().strip_prefix(a)?))? {
            bail!("Archive content mismatch");
        }
    }
    Ok(())
}

/// Moves across volumes only after a complete, hash-verified copy is ready.
/// The source is untouched if copying fails; activation is a local rename.
pub fn move_verified(source: &Path, destination: &Path) -> Result<()> {
    move_verified_with(source, destination, |a, b| fs::rename(a, b))
}
fn move_verified_with(
    source: &Path,
    destination: &Path,
    rename: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
) -> Result<()> {
    anyhow::ensure!(!destination.exists(), "Move destination already exists");
    fs::create_dir_all(destination.parent().context("Missing move parent")?)?;
    match rename(source, destination) {
        Ok(()) => return Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {}
        Err(e) => return Err(e.into()),
    }
    let temporary = destination.with_extension(format!("{}.moving", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        if source.is_dir() {
            copy_directory_verified(source, &temporary)?;
        } else {
            anyhow::ensure!(!linked(source)?, "Cannot move a linked file");
            fs::copy(source, &temporary)?;
            anyhow::ensure!(
                hash(source)? == hash(&temporary)?,
                "Copy verification failed"
            );
        }
        // Retire the source by a rename on its own volume. Cleanup failure
        // must not turn a completed move into a failed transaction.
        let retired = source.with_extension(format!("{}.retired", uuid::Uuid::new_v4()));
        fs::rename(source, &retired)?;
        if let Err(error) = fs::rename(&temporary, destination) {
            fs::rename(&retired, source)
                .context("Could not restore the source after failed activation")?;
            return Err(error.into());
        }
        let cleanup = if retired.is_dir() {
            fs::remove_dir_all(&retired)
        } else {
            fs::remove_file(&retired)
        };
        if let Err(error) = cleanup {
            eprintln!("A retired copy was kept at {}: {error}", retired.display());
        }
        Ok(())
    })();
    if temporary.exists() {
        let _ = remove_managed(&temporary, destination.parent().unwrap());
    }
    result
}

pub fn copy_directory_verified(source: &Path, destination: &Path) -> Result<()> {
    anyhow::ensure!(!destination.exists(), "Copy destination already exists");
    no_links(source)?;
    fs::create_dir_all(destination)?;
    for entry in WalkDir::new(source).min_depth(1) {
        let entry = entry?;
        let target = destination.join(entry.path().strip_prefix(source)?);
        if entry.file_type().is_dir() {
            fs::create_dir(&target)?;
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    verify_trees(source, destination)?;
    // Apply directory permissions after children have been written.
    for entry in WalkDir::new(source).contents_first(true) {
        let entry = entry?;
        if entry.file_type().is_dir() {
            fs::set_permissions(
                destination.join(entry.path().strip_prefix(source)?),
                entry.metadata()?.permissions(),
            )?;
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub(crate) fn canonical_directory(path: &Path) -> Result<()> {
    anyhow::ensure!(
        path.is_absolute() && path.canonicalize()? == path,
        "Noncanonical directory"
    );
    let mut cursor = std::path::PathBuf::from("/");
    for part in path.components().skip(1) {
        cursor.push(part);
        let meta = fs::symlink_metadata(&cursor)?;
        anyhow::ensure!(
            meta.is_dir() && !meta.file_type().is_symlink(),
            "Linked approval parent"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cross_volume_move_verifies_contents_and_preserves_unrelated_files() {
        let root = std::env::temp_dir().join(format!("craft-move-{}", uuid::Uuid::new_v4()));
        let source = root.join("source");
        let destination = root.join("other-drive/app");
        fs::create_dir_all(source.join("data")).unwrap();
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(source.join("data/project.pcraft"), b"project").unwrap();
        fs::write(
            destination.parent().unwrap().join("unrelated.png"),
            b"image",
        )
        .unwrap();
        move_verified_with(&source, &destination, |_, _| {
            Err(std::io::ErrorKind::CrossesDevices.into())
        })
        .unwrap();
        assert!(!source.exists());
        assert_eq!(
            fs::read(destination.join("data/project.pcraft")).unwrap(),
            b"project"
        );
        assert_eq!(
            fs::read(destination.parent().unwrap().join("unrelated.png")).unwrap(),
            b"image"
        );
        assert!(
            move_verified_with(&destination, &destination, |_, _| panic!(
                "must reject existing destination"
            ))
            .is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn hostile_paths() {
        for s in [
            "../a",
            "a/../b",
            "C:/x",
            "/x",
            "a\\x",
            "file:stream",
            "CON.txt",
            "a. /x",
        ] {
            assert!(safe_relative(s).is_err(), "{s}");
        }
        assert!(safe_relative("project/src/main.rs").is_ok());
    }
    #[test]
    fn bom() {
        let p = std::env::temp_dir().join(format!("{}.json", uuid::Uuid::new_v4()));
        fs::write(&p, b"\xef\xbb\xbf{\"a\":1}").unwrap();
        assert_eq!(read_json::<serde_json::Value>(&p).unwrap()["a"], 1);
        fs::remove_file(p).unwrap();
    }
}
