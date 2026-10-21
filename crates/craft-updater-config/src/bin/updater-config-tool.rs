use anyhow::{bail, Context, Result};
use craft_updater_config::{resolve, ConfigOverrides, View};
use std::{collections::BTreeMap, path::PathBuf};
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    anyhow::ensure!(
        args.next().as_deref() == Some("export"),
        "Usage: updater-config-tool export --project-root PATH --view VIEW --target-os OS"
    );
    let mut values = BTreeMap::new();
    while let Some(key) = args.next() {
        anyhow::ensure!(
            [
                "--project-root",
                "--view",
                "--target-os",
                "--config",
                "--out",
                "--repository",
                "--channel",
                "--revision",
                "--catalog-url",
                "--catalog-mirror",
                "--check-url"
            ]
            .contains(&key.as_str()),
            "Unknown export argument"
        );
        let value = args.next().context("Missing export argument value")?;
        anyhow::ensure!(
            values.insert(key, value).is_none(),
            "Duplicate export argument"
        );
    }
    let get = |name: &str| values.get(name).cloned();
    let project = PathBuf::from(get("--project-root").context("Missing --project-root")?);
    let config = get("--config").map(PathBuf::from);
    let target = get("--target-os").context("Missing --target-os")?;
    let view = match get("--view").as_deref() {
        Some("build") => View::Build,
        Some("packaging") => View::Packaging,
        Some("publisher") => View::Publisher,
        Some("worker") => View::Worker,
        _ => bail!("Invalid --view"),
    };
    let overrides = ConfigOverrides {
        repository: get("--repository"),
        channel: get("--channel"),
        revision: get("--revision").map(|v| v.parse()).transpose()?,
        catalog_url: get("--catalog-url"),
        mirror_url: get("--catalog-mirror"),
        check_url: get("--check-url"),
    };
    let resolved = resolve(
        &project,
        config.as_deref(),
        &std::env::vars()
            .filter(|(k, _)| k.starts_with("CRAFT_"))
            .collect(),
        &target,
        &overrides,
    )?;
    let bytes = serde_json::to_vec_pretty(&resolved.view(view)?)?;
    if let Some(out) = get("--out") {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(out)?;
        file.write_all(&bytes)?;
    } else {
        println!("{}", String::from_utf8(bytes)?);
    }
    Ok(())
}
