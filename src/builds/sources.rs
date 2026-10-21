//! Source archive downloads and removal; shares release transaction/backup logic.
use crate::installations::{backups::backup_path, releases::replace_transaction};
use crate::{
    backups, files,
    jobs::Job,
    model::{Paths, Source},
    network::Network,
    platform,
};
use anyhow::{bail, Context, Result};
use std::{collections::BTreeMap, fs};

/// Remove one managed source archive, keeping builds, workspaces and backups.
pub fn remove_source(paths: &Paths, app: &str) -> Result<()> {
    crate::model::valid_app(app)?;
    // Same lock order as a build which downloads its source first.
    let _build_lock = platform::Lock::take(r"Local\CraftAppsSourceBuilder")?;
    let _lock = platform::Lock::take(r"Local\CraftAppsManager")?;
    let root = paths.at("sources");
    let archive = root.join(format!("{app}-source.zip"));
    let index_path = root.join("source-index.json");
    files::inside(&archive, &root)?;
    files::inside(&index_path, &root)?;
    let mut index: BTreeMap<String, Source> = files::read_or_default(&index_path)?;
    anyhow::ensure!(
        index.remove(app).is_some(),
        "No managed source for this app"
    );
    let staged = root.join(format!(".{app}-removing-{}.zip", uuid::Uuid::new_v4()));
    files::inside(&staged, &root)?;
    let existed = archive.is_file();
    anyhow::ensure!(!archive.exists() || existed, "Source archive is not a file");
    if existed {
        fs::rename(&archive, &staged)?;
    }
    if let Err(error) = files::write_json(&index_path, &index) {
        if existed {
            fs::rename(&staged, &archive)
                .context("Could not restore source archive after index save failed")?;
        }
        return Err(error);
    }
    files::remove_managed(&staged, &root)
}

pub fn sources(paths: &Paths, names: &[String], job: &Job) -> Result<()> {
    job.tracked(
        "Source download",
        if names.len() == 1 {
            &names[0]
        } else {
            "Source library"
        },
        |job| sources_owned(paths, names, job),
    )
}

fn sources_owned(paths: &Paths, names: &[String], job: &Job) -> Result<()> {
    let _lock = platform::Lock::take("Local\\CraftAppsManager")?;
    job.log(&format!(
        "\nSource updates — {}",
        chrono::Utc::now().to_rfc3339()
    ));
    let prefs = paths.preferences()?;
    let network = Network::new(&paths.root)?;
    let mut index: BTreeMap<String, Source> =
        files::read_or_default(&paths.at("sources/source-index.json"))?;
    let mut errors = 0;
    for name in names {
        job.prepare_child(name, "Source download", "source", "source");
    }
    for name in names {
        crate::model::valid_app(name)?;
        if let Err(error) = job.check() {
            if errors > 0 {
                bail!("{errors} source update(s) failed before cancellation; see the log");
            }
            return Err(error);
        }
        job.stage("Checking source", None, crate::model::title(name));
        let child = job.prepare_child(name, "Source download", "source", "source");
        let result = child.tracked("Source download", name, |job| {
            let repository = crate::model::repository(name);
            let repo: serde_json::Value =
                network.json(&format!("https://api.github.com/repos/{repository}"))?;
            let branch = repo["default_branch"]
                .as_str()
                .context("No default branch")?;
            let mut endpoint = reqwest::Url::parse(&format!(
                "https://api.github.com/repos/{repository}/commits/"
            ))?;
            endpoint
                .path_segments_mut()
                .map_err(|_| anyhow::anyhow!("Invalid API endpoint"))?
                .pop_if_empty()
                .push(branch);
            let commit: serde_json::Value = network.json(endpoint.as_str())?;
            let sha = commit["sha"].as_str().context("No commit")?;
            if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
                bail!("Invalid source commit")
            }
            let destination = paths.at(format!("sources/{name}-source.zip"));
            files::inside(&destination, &paths.at("sources"))?;
            let old = index.get(name).cloned();
            if destination.exists() {
                if files::linked(&destination)? {
                    bail!("Linked source archive")
                };
                let old = old
                    .as_ref()
                    .context("Existing ZIP is unmanaged; leaving it intact")?;
                if old.sha.len() != 40 || !old.sha.bytes().all(|b| b.is_ascii_hexdigit()) {
                    bail!("Invalid recorded source commit; leaving the existing ZIP intact");
                }
                if old.sha == sha
                    && files::hash(&destination)?.eq_ignore_ascii_case(&old.archive_sha256)
                {
                    job.report(
                        "Result",
                        "The existing source archive is already up to date and was verified.",
                    );
                    job.report("Source location", destination.display().to_string());
                    job.report("Commit", sha);
                    job.log(&format!("{name}: source up to date"));
                    return Ok(());
                }
            }
            let archive = paths.at(format!("runtime/downloads/{name}-{sha}.zip"));
            network.download(
                &format!("https://codeload.github.com/{repository}/zip/{sha}"),
                &archive,
                job,
            )?;
            files::verify_source(&archive, name, sha)?;
            let hash = files::hash(&archive)?;
            let backup = old
                .as_ref()
                .filter(|_| destination.exists())
                .map(|o| backup_path(paths, name, &o.sha, true));
            index.insert(
                name.clone(),
                Source {
                    sha: sha.into(),
                    branch: branch.into(),
                    repository: repository.clone(),
                    archive_sha256: hash,
                    downloaded_at: chrono::Utc::now().to_rfc3339(),
                },
            );
            if let Err(e) = replace_transaction(&archive, &destination, backup.as_deref(), || {
                files::write_json(&paths.at("sources/source-index.json"), &index)
            }) {
                if let Some(old) = old {
                    index.insert(name.clone(), old);
                } else {
                    index.remove(name);
                }
                return Err(e);
            }
            if let Err(e) = backups::finish(paths, &prefs, backup.as_deref(), name, true, job) {
                job.log(&format!("Source backup warning: {e}"))
            }
            job.report("Source location", destination.display().to_string());
            job.report("Commit", sha);
            job.report(
                "Result",
                "Source archive downloaded, verified and saved successfully.",
            );
            job.log(&format!("{name}: source updated ({})", &sha[..7]));
            Ok(())
        });
        if let Err(e) = result {
            if crate::jobs::is_cancelled(&e) {
                if errors == 0 {
                    return Err(e);
                }
                job.log(&format!(
                    "{name}: cancelled; {errors} earlier source failure(s) retained"
                ));
                break;
            }
            errors += 1;
            job.log(&format!("{name}: {e:#}"));
            if e.to_string().contains("API limit")
                || job.cancel.load(std::sync::atomic::Ordering::Relaxed)
            {
                break;
            }
        }
    }
    if errors > 0 {
        bail!("{errors} source update(s) failed; see the log");
    }
    Ok(())
}

#[cfg(test)]
mod source_removal_tests {
    use super::*;
    #[test]
    fn removes_only_selected_managed_source_and_rejects_unmanaged_files() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let paths = Paths::new(root.clone(), None);
        let source = Source {
            sha: "a".repeat(40),
            branch: "main".into(),
            repository: "storytold/filmcraft".into(),
            archive_sha256: String::new(),
            downloaded_at: String::new(),
        };
        files::write_json(
            &paths.at("sources/source-index.json"),
            &BTreeMap::from([("filmcraft", source.clone()), ("soundcraft", source)]),
        )
        .unwrap();
        for name in [
            "sources/filmcraft-source.zip",
            "sources/soundcraft-source.zip",
            "sources/photocraft-source.zip",
            "builds/filmcraft/keep",
            "workspace/filmcraft/keep",
            "backups/sources/filmcraft-keep.zip",
        ] {
            let path = paths.at(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"keep").unwrap();
        }
        assert!(remove_source(&paths, "photocraft").is_err());
        remove_source(&paths, "filmcraft").unwrap();
        assert!(!paths.at("sources/filmcraft-source.zip").exists());
        let index: BTreeMap<String, Source> =
            files::read_json(&paths.at("sources/source-index.json")).unwrap();
        assert!(!index.contains_key("filmcraft"));
        assert!(index.contains_key("soundcraft"));
        for name in [
            "sources/soundcraft-source.zip",
            "sources/photocraft-source.zip",
            "builds/filmcraft/keep",
            "workspace/filmcraft/keep",
            "backups/sources/filmcraft-keep.zip",
        ] {
            assert_eq!(fs::read(paths.at(name)).unwrap(), b"keep");
        }
        assert!(remove_source(&paths, "../filmcraft").is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
