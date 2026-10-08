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
#[derive(Clone)]
pub struct Backup {
    pub path: PathBuf,
    pub source: bool,
}
pub fn list(paths: &Paths, app: &str) -> Result<Vec<Backup>> {
    crate::model::valid_app(app)?;
    let mut found = Vec::new();
    for source in [false, true] {
        let root = paths.at(if source {
            "backups/sources"
        } else {
            "backups/releases"
        });
        if !root.exists() {
            continue;
        }
        for e in fs::read_dir(root)? {
            let e = e?;
            if managed_name(&e.file_name().to_string_lossy(), source, Some(app)) {
                found.push(Backup {
                    path: e.path(),
                    source,
                });
            }
        }
    }
    found.sort_by_key(|b| std::cmp::Reverse(fs::metadata(&b.path).and_then(|m| m.modified()).ok()));
    Ok(found)
}
pub fn delete_selected(paths: &Paths, app: &str, selected: &[Backup]) -> Result<()> {
    let _lock = crate::platform::Lock::take("Local\\CraftAppsManager")?;
    crate::model::valid_app(app)?;
    for backup in selected {
        let root = paths.at(if backup.source {
            "backups/sources"
        } else {
            "backups/releases"
        });
        files::inside(&backup.path, &root)?;
        if !managed_name(
            &backup
                .path
                .file_name()
                .context("Missing backup name")?
                .to_string_lossy(),
            backup.source,
            Some(app),
        ) {
            bail!("Not a managed backup for this app");
        }
    }
    for backup in selected {
        files::remove_managed(
            &backup.path,
            &paths.at(if backup.source {
                "backups/sources"
            } else {
                "backups/releases"
            }),
        )?;
    }
    Ok(())
}
pub fn restore(paths: &Paths, app: &str, backup: &Backup, job: &Job) -> Result<()> {
    use crate::{model::Source, platform, updates};
    let _lock = platform::Lock::take("Local\\CraftAppsManager")?;
    crate::model::valid_app(app)?;
    let root = paths.at(if backup.source {
        "backups/sources"
    } else {
        "backups/releases"
    });
    files::inside(&backup.path, &root)?;
    let label = backup
        .path
        .file_name()
        .context("Missing backup name")?
        .to_string_lossy();
    if !managed_name(&label, backup.source, Some(app)) {
        bail!("Not a managed backup for this app");
    }
    files::no_links(&backup.path)?;
    if !backup.source && paths.preferences()?.release_format != "portable" {
        bail!("Select Portable ZIP in Settings to restore a portable release. MSI installations are not changed.");
    }
    if !backup.source && platform::running_app(app)? {
        bail!("Close the app before restoring its backup");
    }
    let cache = paths.at("runtime/downloads");
    fs::create_dir_all(&cache)?;
    let stage = cache.join(format!("restore-{}", uuid::Uuid::new_v4().simple()));
    fs::create_dir_all(&stage)?;
    let result = (|| -> Result<()> {
        let contents = stage.join("contents");
        if backup.path.is_dir() {
            fs::create_dir_all(&contents)?;
            for e in walkdir::WalkDir::new(&backup.path) {
                let e = e?;
                let dest = contents.join(e.path().strip_prefix(&backup.path)?);
                if e.file_type().is_dir() {
                    fs::create_dir_all(dest)?;
                } else {
                    fs::copy(e.path(), dest)?;
                }
            }
        } else if backup.path.extension().is_some_and(|s| s == "7z") {
            let seven = tools::seven(paths).context("7-Zip is required to restore this backup")?;
            job.run(
                Command::new(&seven).args(["t", "-t7z"]).arg(&backup.path),
                false,
            )?;
            // Validate every member before allowing 7-Zip to write paths.
            let output = Command::new(&seven)
                .args(["l", "-slt", "-ba"])
                .arg(&backup.path)
                .output()?;
            if !output.status.success() {
                bail!("Could not list backup archive");
            }
            for line in String::from_utf8(output.stdout)?.lines() {
                if let Some(path) = line.strip_prefix("Path = ") {
                    files::safe_relative(&path.replace('\\', "/"))?;
                }
            }
            job.run(
                Command::new(&seven)
                    .args(["x", "-t7z", "-y"])
                    .arg(&backup.path)
                    .arg(format!("-o{}", contents.display())),
                false,
            )?;
            files::no_links(&contents)?;
        } else {
            files::extract_zip(&backup.path, &contents, job)?;
        }
        job.check()?;
        let rollback = stage.join("rollback");
        if backup.source {
            let re = regex::Regex::new(r"-([a-f0-9]{40})$")?;
            let dirs: Vec<_> = fs::read_dir(&contents)?.collect::<std::io::Result<Vec<_>>>()?;
            if dirs.len() != 1 {
                bail!("Source backup must contain one repository folder");
            }
            let name = dirs[0].file_name().to_string_lossy().into_owned();
            let sha = re
                .captures(&name)
                .context("Source backup has no full commit ID")?[1]
                .to_owned();
            let zip = stage.join("source.zip");
            if backup.path.extension().is_some_and(|s| s == "zip") {
                fs::copy(&backup.path, &zip)?;
            } else {
                use std::io::Write;
                let mut writer = zip::ZipWriter::new(fs::File::create(&zip)?);
                for e in walkdir::WalkDir::new(&contents).min_depth(1) {
                    let e = e?;
                    if e.file_type().is_file() {
                        let name = e
                            .path()
                            .strip_prefix(&contents)?
                            .to_string_lossy()
                            .replace('\\', "/");
                        writer.start_file(name, zip::write::SimpleFileOptions::default())?;
                        writer.write_all(&fs::read(e.path())?)?;
                    }
                }
                writer.finish()?;
            }
            files::verify_source(&zip, app, &sha)?;
            let mut index: std::collections::BTreeMap<String, Source> =
                files::read_or_default(&paths.at("sources/source-index.json"))?;
            index.insert(
                app.into(),
                Source {
                    sha,
                    branch: index
                        .get(app)
                        .map(|s| s.branch.clone())
                        .unwrap_or_else(|| "main".into()),
                    repository: format!("storytold/{}", crate::model::repository(app)),
                    archive_sha256: files::hash(&zip)?,
                    downloaded_at: chrono::Utc::now().to_rfc3339(),
                },
            );
            updates::replace_transaction(
                &zip,
                &paths.at(format!("sources/{app}-source.zip")),
                Some(&rollback),
                || files::write_json(&paths.at("sources/source-index.json"), &index),
            )?;
        } else {
            let matches: Vec<_> = walkdir::WalkDir::new(&contents)
                .into_iter()
                .collect::<std::result::Result<Vec<_>, _>>()?
                .into_iter()
                .filter(|e| {
                    e.file_type().is_file()
                        && e.file_name().to_string_lossy() == crate::model::executable_name(app)
                })
                .collect();
            if matches.len() != 1 {
                bail!("Backup must contain exactly one app executable");
            }
            let mut config = paths.config()?;
            let record = config
                .apps
                .iter_mut()
                .find(|a| a.name == app)
                .context("Missing app configuration")?;
            record.version = label
                .strip_prefix(&format!("{app}-"))
                .context("Missing version")?
                .split('-')
                .next()
                .context("Missing version")?
                .into();
            updates::version(&record.version)?;
            let target = paths.at(format!("releases/{app}"));
            files::inside(&target, &paths.at("releases"))?;
            record.path = target.to_string_lossy().into_owned();
            record.install_kind = "portable".into();
            record.product_code.clear();
            if platform::running_app(app)? {
                bail!("App opened during restore; close it and retry");
            }
            updates::replace_transaction(
                matches[0].path().parent().unwrap(),
                &target,
                Some(&rollback),
                || paths.save_config(&config),
            )?;
        }
        job.log(&format!(
            "Restored {} backup: {label}",
            crate::model::title(app)
        ));
        Ok(())
    })();
    if result.is_err() && stage.join("rollback").exists() {
        job.log(&format!(
            "Rollback copy retained for recovery: {}",
            stage.display()
        ));
    } else {
        let _ = files::remove_managed(&stage, &cache);
    }
    result
}
pub fn clear(paths: &Paths) -> Result<()> {
    clear_selected(paths, None)
}
pub fn clear_app(paths: &Paths, app: &str) -> Result<()> {
    crate::model::valid_app(app)?;
    clear_selected(paths, Some(app))
}
fn clear_selected(paths: &Paths, app: Option<&str>) -> Result<()> {
    let _lock = crate::platform::Lock::take("Local\\CraftAppsManager")?;
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
            if managed_name(&e.file_name().to_string_lossy(), source, app) {
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
