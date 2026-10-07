use crate::{
    files,
    jobs::Job,
    model::{Paths, Preferences, SOURCES},
    tools,
};
use anyhow::{bail, Context, Result};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
pub fn finish(
    paths: &Paths,
    p: &Preferences,
    backup: Option<&Path>,
    name: &str,
    source: bool,
    job: &Job,
) -> Result<()> {
    let Some(backup) = backup else { return Ok(()) };
    let root = paths.at(if source {
        "backups/sources"
    } else {
        "backups/releases"
    });
    files::inside(backup, &root)?;
    if !(if source {
        p.keep_source_backups
    } else {
        p.keep_app_backups
    }) {
        files::remove_managed(backup, &root)?;
        return Ok(());
    }
    let compress = if source {
        p.compress_source_backups
    } else {
        p.compress_backups
    };
    let mut current = backup.to_path_buf();
    if compress {
        match compress_backup(paths, backup, source, job) {
            Ok(Some(archive)) => current = archive,
            Ok(None) => {}
            Err(e) => job.log(&format!("Backup retained uncompressed: {e:#}")),
        }
    }
    let mut items: Vec<_> = fs::read_dir(&root)?
        .filter_map(|e| e.ok())
        .filter(|e| managed_name(&e.file_name().to_string_lossy(), source, Some(name)))
        .collect();
    items.sort_by_key(|e| {
        (
            e.path() == current,
            e.metadata().and_then(|m| m.modified()).ok(),
        )
    });
    items.reverse();
    for e in items.into_iter().skip(p.backup_versions) {
        files::remove_managed(&e.path(), &root)?;
    }
    Ok(())
}
fn managed_name(s: &str, source: bool, app: Option<&str>) -> bool {
    let pattern = if source {
        r"^([a-z]+)-source-[a-f0-9]{7}-[a-f0-9]{32}\.zip(?:\.7z)?$"
    } else {
        r"^([a-z]+)-\d+\.\d+\.\d+-[a-f0-9]{32}(?:\.(?:zip|7z))?$"
    };
    let re = regex::Regex::new(pattern).unwrap();
    re.captures(s)
        .is_some_and(|c| SOURCES.contains(&&c[1]) && app.is_none_or(|n| n == &c[1]))
}
pub fn clear(paths: &Paths) -> Result<()> {
    let _lock = crate::platform::Lock::take("Local\\CraftAppsUpdater")?;
    for source in [false, true] {
        let root = paths.at(if source {
            "backups/sources"
        } else {
            "backups/releases"
        });
        if !root.exists() {
            continue;
        }
        for e in fs::read_dir(&root)? {
            let e = e?;
            if managed_name(&e.file_name().to_string_lossy(), source, None) {
                files::remove_managed(&e.path(), &root)?;
            }
        }
    }
    Ok(())
}
fn compress_backup(
    paths: &Paths,
    original: &Path,
    source: bool,
    job: &Job,
) -> Result<Option<PathBuf>> {
    let Some(seven) = tools::seven(paths) else {
        job.log("7-Zip unavailable; retaining original backup.");
        return Ok(None);
    };
    files::no_links(original)?;
    let downloads = paths.at("runtime/downloads");
    fs::create_dir_all(&downloads)?;
    let stage = downloads.join(format!("compress-{}", uuid::Uuid::new_v4().simple()));
    let verify = downloads.join(format!("verify-{}", uuid::Uuid::new_v4().simple()));
    let archive = PathBuf::from(format!("{}.7z", original.display()));
    let partial = PathBuf::from(format!("{}.partial", archive.display()));
    if archive.exists() {
        bail!("Archive already exists");
    }
    let result = (|| -> Result<Option<PathBuf>> {
        let input = if source {
            files::extract_zip(original, &stage, job)?;
            stage.clone()
        } else {
            original.to_path_buf()
        };
        job.stage("Compressing backup", None, "7-Zip Ultra / LZMA2");
        job.run(
            Command::new(&seven)
                .args([
                    "a",
                    "-t7z",
                    "-mx=9",
                    "-m0=LZMA2",
                    "-md=64m",
                    "-ms=on",
                    "-mmt=2",
                    "-y",
                ])
                .arg(&partial)
                .arg(input.join("*")),
            false,
        )?;
        job.run(
            Command::new(&seven).args(["t", "-t7z"]).arg(&partial),
            false,
        )?;
        job.run(
            Command::new(&seven)
                .args(["x", "-t7z", "-y"])
                .arg(&partial)
                .arg(format!("-o{}", verify.display())),
            false,
        )?;
        files::verify_trees(&input, &verify).context("Backup content verification failed")?;
        if source && fs::metadata(&partial)?.len() >= fs::metadata(original)?.len() {
            return Ok(None);
        }
        fs::rename(&partial, &archive)?;
        files::remove_managed(
            original,
            &paths.at(if source {
                "backups/sources"
            } else {
                "backups/releases"
            }),
        )?;
        job.log(&format!(
            "Backup compressed and verified: {}",
            archive.display()
        ));
        Ok(Some(archive.clone()))
    })();
    for p in [&stage, &verify, &partial] {
        if p.exists() {
            let _ = files::remove_managed(p, &downloads);
        }
    }
    result
}
