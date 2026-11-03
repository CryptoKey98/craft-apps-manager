//! Explicit local artifact tooling; never uploads or invokes an installer.
use anyhow::{ensure, Context, Result};
use craft_apps_manager::{
    files,
    installations::release_catalog::{
        self as catalog,
        publisher::{self, Discovery, InspectionBatch, InspectionInput, Publication},
    },
};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    io::Read,
    path::{Path, PathBuf},
};
fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    files::no_links(path)?;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(128 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 128 * 1024 * 1024, "Input artifact too large");
    Ok(serde_json::from_slice(&bytes)?)
}
fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let command = args.next().context(
        "Expected discover, inspect, pending-count, inspect-pending, aggregate, sign or verify",
    )?;
    let mut flags = BTreeMap::<String, OsString>::new();
    while let Some(key) = args.next() {
        let key = key.to_str().context("Invalid argument")?;
        ensure!(key.starts_with("--"), "Expected named option");
        let value = args.next().context("Missing option value")?;
        ensure!(
            flags.insert(key.into(), value).is_none(),
            "Duplicate option"
        );
    }
    let path = |name: &str| -> Result<PathBuf> {
        Ok(PathBuf::from(
            flags.get(name).with_context(|| format!("Missing {name}"))?,
        ))
    };
    let previous: Option<Publication> = flags
        .get("--previous")
        .map(|p| read(Path::new(p)))
        .transpose()?;
    match command.to_str().context("Invalid command")? {
        "enroll" => {
            let repositories: Vec<String> = read(&path("--repositories")?)?;
            let output = publisher::enroll(&repositories, &publisher::Github::new()?)?;
            files::write_json(&path("--out")?, &output)?;
        }
        "discover" => {
            let repos: Vec<String> = read(&path("--repositories")?)?;
            let prior: Option<Discovery> = flags
                .get("--checkpoint")
                .map(|p| read(Path::new(p)))
                .transpose()?;
            let output = publisher::discover(&repos, prior.as_ref(), &publisher::Github::new()?)?;
            files::write_json(&path("--out")?, &output)?;
        }
        "inspect" => {
            let identity: InspectionInput = read(&path("--identity")?)?;
            let evidence = catalog::inspection::inspect(
                &path("--package")?,
                &identity.repository,
                &identity.tag,
                &identity.asset,
                &identity.architecture,
            )?;
            files::write_json(
                &path("--out")?,
                &catalog::store::Observation {
                    repository: identity.repository,
                    evidence,
                    used: chrono::Utc::now().timestamp(),
                },
            )?;
        }
        "pending-count" => {
            let discovery: Discovery = read(&path("--discovery")?)?;
            let prior: InspectionBatch = flags
                .get("--checkpoint")
                .map(|p| read(Path::new(p)))
                .transpose()?
                .unwrap_or_default();
            let since = flags
                .get("--since")
                .map(|s| -> Result<i64> {
                    Ok(s.to_str().context("Invalid attempt cutoff")?.parse()?)
                })
                .transpose()?;
            println!(
                "{}",
                publisher::inspection_queue(&discovery, previous.as_ref(), &prior, since).len()
            );
        }
        "inspect-pending" => {
            let discovery: Discovery = read(&path("--discovery")?)?;
            let prior: Option<InspectionBatch> = flags
                .get("--checkpoint")
                .map(|p| read(Path::new(p)))
                .transpose()?;
            let budget = flags
                .get("--max-packages")
                .map(|s| {
                    s.to_str()
                        .context("Invalid budget")?
                        .parse::<usize>()
                        .map_err(anyhow::Error::from)
                })
                .transpose()?
                .unwrap_or(8);
            let report = publisher::inspect_pending(
                &discovery,
                previous.as_ref(),
                prior.as_ref(),
                &path("--staging")?,
                budget,
            )?;
            files::write_json(&path("--out")?, &report)?;
        }
        "aggregate" => {
            let discovery: Discovery = read(&path("--discovery")?)?;
            let evidence = flags
                .get("--evidence")
                .map(|p| read::<InspectionBatch>(Path::new(p)))
                .transpose()?
                .unwrap_or_default();
            let output =
                publisher::aggregate(previous.as_ref(), &discovery, &evidence.observations)?;
            files::write_json(&path("--out")?, &output)?;
        }
        "sign" => {
            let input: Publication = read(&path("--input")?)?;
            let key_file = path("--key-file")?;
            files::no_links(&key_file)?;
            ensure!(
                std::fs::metadata(&key_file)?.len() <= 256,
                "Invalid signing-key file size"
            );
            let secret = std::fs::read_to_string(key_file)?;
            let key = ed25519_dalek::SigningKey::from_bytes(&catalog::verification::decode::<32>(
                secret.trim(),
            )?);
            publisher::sign(
                &input,
                &key,
                flags
                    .get("--key-id")
                    .and_then(|s| s.to_str())
                    .context("Missing --key-id")?,
                &path("--out")?,
            )?;
        }
        "verify" => {
            let source: catalog::Source = read(&path("--source")?)?;
            let index = path("--index")?;
            ensure!(
                std::fs::metadata(&index)?.len() <= catalog::verification::MAX_ROOT as u64,
                "Oversized index"
            );
            let verified = catalog::verify_index(&std::fs::read(index)?, &source, 0)?;
            for (repo, reference) in &verified.index.repositories {
                let object = path("--objects")?.join(format!("{}.json", reference.sha256));
                files::no_links(&object)?;
                ensure!(
                    std::fs::metadata(&object)?.len() == reference.size,
                    "App object size mismatch"
                );
                catalog::verify_app(&std::fs::read(object)?, repo, reference)?;
            }
            println!(
                "Verified {} app catalogs",
                verified.index.repositories.len()
            );
        }
        _ => anyhow::bail!("Unknown command"),
    }
    Ok(())
}
