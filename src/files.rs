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
    Ok(serde_json::from_slice(bytes)?)
}
pub fn read_or_default<T: DeserializeOwned + Default>(p: &Path) -> Result<T> {
    if p.exists() {
        read_json(p)
    } else {
        Ok(T::default())
    }
}
pub fn write_json<T: Serialize>(p: &Path, v: &T) -> Result<()> {
    write_bytes(p, &serde_json::to_vec_pretty(v)?)
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
            bail!("Archive contains a linked path: {}", e.path().display());
        }
    }
    Ok(())
}
pub fn extract_zip(archive: &Path, dest: &Path, job: &crate::jobs::Job) -> Result<()> {
    if dest.exists() {
        bail!("Extraction destination already exists");
    }
    let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
    for i in 0..zip.len() {
        let e = zip.by_index(i)?;
        safe_relative(e.name().trim_end_matches('/'))?;
        if e.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            bail!("Linked archive entry");
        }
    }
    fs::create_dir_all(dest)?;
    let n = zip.len();
    for i in 0..n {
        job.check()?;
        let mut e = zip.by_index(i)?;
        let target = dest.join(e.name());
        if e.is_dir() {
            fs::create_dir_all(&target)?
        } else {
            fs::create_dir_all(target.parent().unwrap())?;
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)?;
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
pub fn verify_source(p: &Path, name: &str, sha: &str) -> Result<()> {
    if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("Invalid source commit");
    }
    let mut zip = zip::ZipArchive::new(File::open(p)?)?;
    let prefix = format!("{name}-{sha}/");
    let renamed_prefix = format!("{}-{sha}/", crate::model::repository(name));
    let mut cargo = false;
    for i in 0..zip.len() {
        let mut e = zip.by_index(i)?;
        if !e.name().starts_with(&prefix) && !e.name().starts_with(&renamed_prefix) {
            bail!("Source has an unexpected root");
        }
        safe_relative(e.name().trim_end_matches('/'))?;
        if e.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            bail!("Linked source entry");
        }
        if e.name() == format!("{prefix}Cargo.toml")
            || e.name() == format!("{renamed_prefix}Cargo.toml")
        {
            cargo = true;
        }
        std::io::copy(&mut e, &mut std::io::sink())?;
    }
    if !cargo {
        bail!("Source ZIP has no expected Cargo.toml");
    }
    Ok(())
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
