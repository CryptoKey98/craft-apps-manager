//! Local release preparation; never contacts GitHub or publishes packages.
use anyhow::{bail, Context, Result};
use std::{fs, path::Path};
use toml_edit::{value, DocumentMut};

fn version(text: &str) -> Result<semver::Version> {
    let v = semver::Version::parse(text)?;
    if !v.pre.is_empty() || !v.build.is_empty() || v.major > 255 || v.minor > 255 || v.patch > 65535
    {
        bail!("Use a stable major.minor.patch version within Windows MSI limits (255.255.65535)");
    }
    Ok(v)
}

fn prepare(files: &[String], next: Option<&str>) -> Result<Vec<String>> {
    let mut manifest = files[0].parse::<DocumentMut>()?;
    if manifest["package"]["name"].as_str() != Some("craft-apps-manager") {
        bail!("Run this tool from the Craft Apps Manager source directory");
    }
    let current = manifest["package"]["version"]
        .as_str()
        .context("Missing package version")?
        .to_owned();
    let current_version = version(&current)?;
    let mut lock = files[1].parse::<DocumentMut>()?;
    let packages = lock["package"]
        .as_array_of_tables_mut()
        .context("Missing lockfile packages")?;
    let matches: Vec<_> = packages
        .iter_mut()
        .filter(|p| p["name"].as_str() == Some("craft-apps-manager"))
        .collect();
    if matches.len() != 1 {
        bail!("Expected exactly one manager package in Cargo.lock");
    }
    let package = matches.into_iter().next().unwrap();
    if package["version"].as_str() != Some(current.as_str()) {
        bail!("Cargo.lock version does not match Cargo.toml; regenerate the lockfile first");
    }
    let caption = format!("Version **{current}** includes");
    if files[2].matches(&caption).count() != 1 {
        bail!("README current-version caption does not match Cargo.toml");
    }
    let heading = files[3]
        .lines()
        .find(|line| line.starts_with("## "))
        .context("Missing changelog release heading")?;
    if heading != format!("## {current}") {
        bail!("Latest changelog heading does not match Cargo.toml");
    }
    let Some(next) = next else {
        return Ok(files.to_vec());
    };
    if version(next)? <= current_version {
        bail!("New version must be newer than {current}");
    }
    manifest["package"]["version"] = value(next);
    package["version"] = value(next);
    let readme = files[2].replacen(&caption, &format!("Version **{next}** includes"), 1);
    let changelog = files[3].replacen("# Changelog", &format!("# Changelog\n\n## {next}\n\n- Describe the changes before merging this release preparation."), 1);
    Ok(vec![
        manifest.to_string(),
        lock.to_string(),
        readme,
        changelog,
    ])
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let (next, dry_run) = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["check"] => (None, true),
        ["bump", v] => (Some(*v), false),
        ["bump", v, "--dry-run"] => (Some(*v), true),
        _ => bail!("Usage: release-tool check | bump VERSION [--dry-run]"),
    };
    let paths = ["Cargo.toml", "Cargo.lock", "README.md", "CHANGELOG.md"];
    let original = paths
        .iter()
        .map(fs::read_to_string)
        .collect::<std::io::Result<Vec<_>>>()?;
    let updated = prepare(&original, next)?;
    if !dry_run {
        for (i, path) in paths.iter().enumerate() {
            if let Err(error) = fs::write(Path::new(path), &updated[i]) {
                let mut failures = Vec::new();
                for j in 0..=i {
                    if let Err(e) = fs::write(paths[j], &original[j]) {
                        failures.push(format!("{}: {e}", paths[j]));
                    }
                }
                bail!("Could not write {path}: {error}. Rollback errors: {failures:?}");
            }
        }
    }
    println!("{}", match next { Some(v) if dry_run => format!("Validated preparation for {v}; no files changed"), Some(v) => format!("Prepared {v}. Edit CHANGELOG.md, review the diff, and open a PR. Nothing published."), None => "Version references agree".to_owned() });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Vec<String> {
        vec!["[package]\nname = \"craft-apps-manager\"\nversion = \"0.4.1\"\n".into(), "[[package]]\nname = \"craft-apps-manager\"\nversion = \"0.4.1\"\n[[package]]\nname = \"another\"\nversion = \"0.4.1\"\n".into(), "Version **0.4.1** includes packages".into(), "# Changelog\n\n## 0.4.1\n\n- Previous changes\n".into()]
    }
    #[test]
    fn bump_preserves_dependencies_and_history() {
        let output = prepare(&fixture(), Some("0.4.2")).unwrap();
        assert!(output[1].contains("name = \"another\"\nversion = \"0.4.1\""));
        assert!(output[3].contains("## 0.4.1"));
        prepare(&output, None).unwrap();
    }
    #[test]
    fn stale_references_are_rejected() {
        for index in 1..4 {
            let mut files = fixture();
            files[index] = files[index].replace("0.4.1", "0.4.0");
            assert!(prepare(&files, None).is_err());
        }
    }
    #[test]
    fn invalid_or_older_versions_are_rejected() {
        for v in [
            "0.4.1",
            "0.3.0",
            "1.2",
            "1.2.3-beta",
            "256.0.0",
            "0.0.65536",
        ] {
            assert!(prepare(&fixture(), Some(v)).is_err());
        }
    }
}
