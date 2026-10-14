//! Stable portable-copy letters live in the installation inventory itself.
use crate::model::Config;
use anyhow::{Context, Result};
use std::collections::{BTreeMap, BTreeSet};

pub fn valid_label(label: &str) -> bool {
    index(label).is_some()
}

pub fn summary(copy: &crate::model::Installed) -> String {
    display_summary(copy, false)
}

pub fn version_summary(copy: &crate::model::Installed) -> String {
    display_summary(copy, true)
}

fn display_summary(copy: &crate::model::Installed, include_version: bool) -> String {
    let kind = if copy.install_kind == "installer" {
        "Installer"
    } else {
        "Portable"
    };
    let mut text = format!(
        "{kind} · {}",
        crate::model::architecture_label(&copy.architecture)
    );
    if include_version && !copy.version.is_empty() {
        text.push_str(&format!(
            " · v{}",
            copy.version.trim_start_matches(['v', 'V'])
        ));
    }
    if kind == "Portable" && valid_label(&copy.installation_label) {
        text.push_str(&format!(" ({})", copy.installation_label));
    }
    text
}

fn index(label: &str) -> Option<u32> {
    if label.is_empty() || label.len() > 7 {
        return None;
    }
    let mut value = 0u64;
    for byte in label.bytes() {
        if !byte.is_ascii_uppercase() {
            return None;
        }
        value = value
            .checked_mul(26)?
            .checked_add(u64::from(byte - b'A') + 1)?;
    }
    u32::try_from(value - 1).ok()
}

fn label(index: u32) -> String {
    let mut value = u64::from(index) + 1;
    let mut letters = Vec::new();
    while value > 0 {
        value -= 1;
        letters.push(char::from(b'A' + (value % 26) as u8));
        value /= 26;
    }
    letters.into_iter().rev().collect()
}

fn identity(copy: &crate::model::Installed) -> (String, String) {
    (copy.name.clone(), path_identity(&copy.path))
}

/// Normalize separators and redundant current-directory components without
/// disk access. Preserve case on platforms with case-sensitive paths.
pub fn path_identity(path: &str) -> String {
    let normalized: std::path::PathBuf = std::path::Path::new(path).components().collect();
    let normalized = normalized.to_string_lossy();
    if cfg!(target_os = "windows") {
        normalized.to_lowercase()
    } else {
        normalized.into_owned()
    }
}

/// Migrate unlabeled records deterministically, sharing the selected record's
/// label with its inventory entry. New copies reuse the first unused letter.
pub fn assign(config: &mut Config) -> Result<()> {
    let mut assignments = BTreeMap::new();
    let mut used: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for copy in config.installations.iter().chain(&config.apps) {
        if copy.path.is_empty() || copy.install_kind == "installer" {
            continue;
        }
        let identity = identity(copy);
        if assignments.contains_key(&identity) {
            continue;
        }
        if let Some(index) = index(&copy.installation_label) {
            let next = index
                .checked_add(1)
                .context("Portable installation label limit reached")?;
            let counter = config
                .portable_label_counters
                .entry(copy.name.clone())
                .or_default();
            *counter = (*counter).max(next);
            if used
                .entry(copy.name.clone())
                .or_default()
                .insert(copy.installation_label.clone())
            {
                assignments.insert(identity, copy.installation_label.clone());
            }
        }
    }
    for copy in config.installations.iter_mut().chain(&mut config.apps) {
        if copy.path.is_empty() || copy.install_kind == "installer" {
            copy.installation_label.clear();
            continue;
        }
        let identity = identity(copy);
        let chosen = if let Some(chosen) = assignments.get(&identity) {
            chosen.clone()
        } else {
            let occupied = used.entry(copy.name.clone()).or_default();
            let mut candidate = 0u32;
            while occupied.contains(&label(candidate)) {
                candidate = candidate
                    .checked_add(1)
                    .context("Portable installation label limit reached")?;
            }
            let chosen = label(candidate);
            occupied.insert(chosen.clone());
            let next = candidate
                .checked_add(1)
                .context("Portable installation label limit reached")?;
            let counter = config
                .portable_label_counters
                .entry(copy.name.clone())
                .or_default();
            *counter = (*counter).max(next);
            assignments.insert(identity, chosen.clone());
            chosen
        };
        copy.installation_label = chosen;
    }
    Ok(())
}

/// Persist only label metadata in older settings, preserving unknown fields.
pub(crate) fn migrate(paths: &crate::model::Paths) -> Result<Option<Config>> {
    let file = paths.at("settings.json");
    if !file.exists() {
        return Ok(None);
    }
    let mut value: serde_json::Value = crate::files::read_json(&file)?;
    let mut config: Config = serde_json::from_value(value.clone())?;
    anyhow::ensure!(
        std::path::Path::new(&config.apps_root) == paths.root,
        "Installation settings belong to another library"
    );
    assign(&mut config)?;
    let original = value.clone();
    for (name, copies) in [
        ("apps", &config.apps),
        ("installations", &config.installations),
    ] {
        if let Some(records) = value
            .get_mut(name)
            .and_then(serde_json::Value::as_array_mut)
        {
            for (record, copy) in records.iter_mut().zip(copies) {
                record["installation_label"] =
                    serde_json::Value::String(copy.installation_label.clone());
            }
        }
    }
    value["portableLabelCounters"] = serde_json::to_value(&config.portable_label_counters)?;
    if value != original {
        crate::files::write_json(&file, &value)?;
    }
    Ok(Some(config))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Installed;

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_path_aliases_share_one_label() {
        let mut config: Config = serde_json::from_value(serde_json::json!({
            "appsRoot":"", "apps":[], "installations":[
                {"name":"filmcraft","version":"","path":"C:\\Apps\\FilmCraft","installation_label":"A"},
                {"name":"filmcraft","version":"","path":"c:/Apps/./filmcraft/"},
                {"name":"filmcraft","version":"","path":"C:\\Apps\\other"}
            ]
        })).unwrap();
        assign(&mut config).unwrap();
        assert_eq!(config.installations[0].installation_label, "A");
        assert_eq!(config.installations[1].installation_label, "A");
        assert_eq!(config.installations[2].installation_label, "B");
    }

    #[test]
    fn legacy_migration_preserves_unknown_fields_and_is_idempotent() {
        let root =
            std::env::temp_dir().join(format!("craft-legacy-labels-{}", uuid::Uuid::new_v4()));
        let paths = crate::model::Paths::new(root.clone(), None);
        crate::files::write_json(&paths.at("settings.json"), &serde_json::json!({"appsRoot":root, "futureSetting":"keep", "apps":[{"name":"filmcraft","path":root.join("releases/filmcraft"),"version":"0.1.0","install_kind":"portable","futureField":42}]})).unwrap();
        let config = migrate(&paths).unwrap().unwrap();
        assert_eq!(config.apps[0].installation_label, "A");
        let value: serde_json::Value = crate::files::read_json(&paths.at("settings.json")).unwrap();
        assert_eq!(value["futureSetting"], "keep");
        assert_eq!(value["apps"][0]["futureField"], 42);
        let before = std::fs::read(paths.at("settings.json")).unwrap();
        migrate(&paths).unwrap();
        assert_eq!(std::fs::read(paths.at("settings.json")).unwrap(), before);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn labels_survive_selection_updates_moves_removal_and_restart() {
        let root = std::env::temp_dir().join(format!("craft-labels-{}", uuid::Uuid::new_v4()));
        let make = |name: &str| Installed {
            name: "filmcraft".into(),
            path: root.join(name).display().to_string(),
            install_kind: "portable".into(),
            ..Default::default()
        };
        let mut config = Config {
            apps_root: root.display().to_string(),
            apps: vec![make("first")],
            installations: vec![make("first"), make("second")],
            portable_label_counters: Default::default(),
        };
        assign(&mut config).unwrap();
        assert_eq!(config.apps[0].installation_label, "A");
        assert_eq!(config.installations[1].installation_label, "B");
        config.apps[0] = config.installations[1].clone();
        config.apps[0].version = "0.8.2".into();
        config.installations[1].path = root.join("moved").display().to_string();
        config.apps[0]
            .path
            .clone_from(&config.installations[1].path);
        config.installations.remove(0);
        config.installations.push(make("third"));
        let mut restarted: Config =
            serde_json::from_slice(&serde_json::to_vec(&config).unwrap()).unwrap();
        assign(&mut restarted).unwrap();
        assert_eq!(restarted.apps[0].installation_label, "B");
        assert_eq!(restarted.installations[0].installation_label, "B");
        assert_eq!(restarted.installations[1].installation_label, "A");
        assert_eq!(label(25), "Z");
        assert_eq!(label(26), "AA");
    }

    #[test]
    fn removed_letters_are_reused_without_renaming_surviving_copies() {
        let mut config: Config = serde_json::from_value(serde_json::json!({
            "appsRoot":"", "apps":[], "portableLabelCounters":{"filmcraft":99},
            "installations":[
                {"name":"filmcraft","version":"","path":"first","install_kind":"portable","installation_label":"A"},
                {"name":"filmcraft","version":"","path":"third","install_kind":"portable","installation_label":"C"},
                {"name":"filmcraft","version":"","path":"new","install_kind":"portable"},
                {"name":"filmcraft","version":"","path":"another","install_kind":"portable"}
            ]
        })).unwrap();
        assign(&mut config).unwrap();
        let letters: Vec<_> = config
            .installations
            .iter()
            .map(|copy| copy.installation_label.as_str())
            .collect();
        assert_eq!(letters, ["A", "C", "B", "D"]);
        config.apps = vec![config.installations[2].clone()];
        let mut restarted: Config =
            serde_json::from_value(serde_json::to_value(&config).unwrap()).unwrap();
        assign(&mut restarted).unwrap();
        assert_eq!(restarted.apps[0].installation_label, "B");
        assert_eq!(restarted.installations[1].installation_label, "C");
        restarted.apps.clear();
        restarted.installations.clear();
        restarted.installations.push(Installed {
            name: "filmcraft".into(),
            path: "reinstalled".into(),
            install_kind: "portable".into(),
            ..Default::default()
        });
        assign(&mut restarted).unwrap();
        assert_eq!(restarted.installations[0].installation_label, "A");
    }

    #[test]
    fn duplicate_labels_are_repaired_and_installers_are_not_labeled() {
        let mut config: Config = serde_json::from_value(serde_json::json!({"appsRoot":"", "apps":[], "installations":[
            {"name":"filmcraft","version":"","path":"first","install_kind":"portable","installation_label":"A"},
            {"name":"filmcraft","version":"","path":"second","install_kind":"portable","installation_label":"A"},
            {"name":"filmcraft","version":"","path":"native","install_kind":"installer","installation_label":"C"}
        ]})).unwrap();
        assign(&mut config).unwrap();
        assert_eq!(config.installations[0].installation_label, "A");
        assert_eq!(config.installations[1].installation_label, "B");
        assert!(config.installations[2].installation_label.is_empty());
    }
}
