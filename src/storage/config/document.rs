//! Bounded YAML adapter; no includes, properties or resource-unsafe features.
use anyhow::{bail, ensure, Context, Result};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
pub(crate) const MAX_BYTES: usize = 16 * 1024 * 1024;
pub(crate) fn defaults() -> Value {
    json!({"schema_version":1,"manager":{"rollbackEnabled":true},"builder":{},"custom_apps":[],"launch":{},"build_launch":{},"legacy_extensions":{}})
}
/// Normalize known historical names without replacing unknown configuration.
/// Ambiguous documents keep both names so typed validation still rejects them.
pub(super) fn normalize_manager_alias(document: &mut Value) {
    if let Some(manager) = document.get_mut("manager").and_then(Value::as_object_mut) {
        if !manager.contains_key("checkManagerOnStartup") {
            if let Some(value) = manager.remove("checkUpdaterOnStartup") {
                manager.insert("checkManagerOnStartup".into(), value);
            }
        }
    }
}
pub(crate) fn parse(bytes: &[u8]) -> Result<Value> {
    ensure!(
        bytes.len() <= MAX_BYTES,
        "Configuration exceeds the 16 MiB limit"
    );
    let options = serde_saphyr::options! {
        budget: serde_saphyr::budget! {
            max_reader_input_bytes: Some(MAX_BYTES),
            max_events: 100_000,
            max_nodes: 100_000,
            max_depth: 64,
            max_anchors: 0,
            max_aliases: 0,
            max_documents: 1,
            max_total_scalar_bytes: MAX_BYTES,
        },
        duplicate_keys: serde_saphyr::options::DuplicateKeyPolicy::Error,
        merge_keys: serde_saphyr::options::MergeKeyPolicy::Error,
        strict_booleans: true,
        reject_unsupported_tags: true,
        non_finite_float_policy: serde_saphyr::options::NonFiniteFloatPolicy::Reject,
        with_snippet: false,
    };
    let mut value: Value = serde_saphyr::from_slice_with_options(bytes, options)
        .map_err(|_| anyhow::anyhow!("Invalid or unsupported YAML configuration"))?;
    normalize_manager_alias(&mut value);
    validate(&value)?;
    Ok(value)
}
pub(crate) fn section<T: DeserializeOwned>(document: &Value, name: &str) -> Result<T> {
    let fallback = if name == "custom_apps" {
        json!([])
    } else {
        json!({})
    };
    serde_json::from_value(document.get(name).unwrap_or(&fallback).clone())
        .with_context(|| format!("Invalid configuration section {name}"))
}
/// Retained registrations carry raw fields the current manager may not understand.
/// Only overlay properties whose typed values actually changed.
pub(super) fn preserve_custom_apps(raw: &Value, desired: &Value) -> Result<Value> {
    let old = raw
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Invalid custom apps list"))?;
    let new = desired
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Invalid custom apps list"))?;
    fn preserve_rules(raw: &Value, edited: &Value) -> Result<Value> {
        let raw = raw
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Invalid download rules"))?;
        let edited = edited
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Invalid download rules"))?;
        let old_rules = raw
            .iter()
            .cloned()
            .map(serde_json::from_value::<crate::downloads::Rule>)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let new_rules = edited
            .iter()
            .cloned()
            .map(serde_json::from_value::<crate::downloads::Rule>)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut matched = vec![None; new_rules.len()];
        let mut used = vec![false; old_rules.len()];
        // Reserve unchanged rules before matching edited rules by target, so
        // reordering cannot attach a sibling's future metadata to another rule.
        for exact in [true, false] {
            for (index, rule) in new_rules.iter().enumerate() {
                if matched[index].is_some() {
                    continue;
                }
                let candidates = old_rules
                    .iter()
                    .enumerate()
                    .filter(|(old_index, old)| {
                        !used[*old_index]
                            && if exact {
                                *old == rule
                            } else {
                                old.same_target(rule)
                            }
                    })
                    .map(|(old_index, _)| old_index)
                    .collect::<Vec<_>>();
                ensure!(
                    candidates.len() <= 1,
                    "Ambiguous retained custom download rule"
                );
                if let Some(&old_index) = candidates.first() {
                    matched[index] = Some(old_index);
                    used[old_index] = true;
                }
            }
        }
        edited
            .iter()
            .enumerate()
            .map(|(index, desired)| {
                let Some(old_index) = matched[index] else {
                    return Ok(desired.clone());
                };
                let mut retained = raw[old_index].clone();
                overlay(
                    &mut retained,
                    &serde_json::to_value(&old_rules[old_index])?,
                    desired,
                )?;
                Ok(retained)
            })
            .collect::<Result<Vec<_>>>()
            .map(Value::Array)
    }
    fn overlay(raw: &mut Value, base: &Value, edited: &Value) -> Result<()> {
        if base == edited {
            return Ok(());
        }
        if let (Some(target), Some(before), Some(after)) =
            (raw.as_object_mut(), base.as_object(), edited.as_object())
        {
            for (key, value) in after {
                if let Some(previous) = before.get(key) {
                    if previous != value {
                        let retained = target.entry(key.clone()).or_insert(Value::Null);
                        if key == "downloadRules" && retained.is_array() && value.is_array() {
                            *retained = preserve_rules(retained, value)?;
                        } else {
                            overlay(retained, previous, value)?;
                        }
                    }
                } else {
                    target.insert(key.clone(), value.clone());
                }
            }
            for key in before.keys().filter(|key| !after.contains_key(*key)) {
                target.remove(key);
            }
        } else {
            *raw = edited.clone();
        }
        Ok(())
    }
    new.iter()
        .map(|entry| {
            let Some(previous) = old.iter().find(|old| old.get("key") == entry.get("key")) else {
                return Ok(entry.clone());
            };
            let typed: crate::catalog::Entry = serde_json::from_value(previous.clone())?;
            let mut retained = previous.clone();
            overlay(&mut retained, &serde_json::to_value(typed)?, entry)?;
            Ok(retained)
        })
        .collect::<Result<Vec<_>>>()
        .map(Value::Array)
}
pub(crate) fn validate(document: &Value) -> Result<()> {
    ensure!(document.is_object(), "Configuration must be a mapping");
    ensure!(
        document.get("schema_version").and_then(Value::as_u64) == Some(1),
        "Unsupported configuration schema version"
    );
    for key in [
        "manager",
        "builder",
        "launch",
        "build_launch",
        "legacy_extensions",
    ] {
        ensure!(
            document.get(key).is_none_or(Value::is_object),
            "Configuration section {key} must be a mapping"
        );
    }
    let manager: crate::storage::preferences::Preferences = section(document, "manager")?;
    ensure!(
        (10..=60).contains(&manager.app_check_interval_minutes),
        "Update interval must be between 10 and 60 minutes"
    );
    for options in manager.app_install_options.values() {
        options.validate()?;
    }
    let builder: crate::builds::preferences::BuilderPreferences = section(document, "builder")?;
    ensure!(
        (1..=100).contains(&builder.log_size_mb) && builder.log_archives <= 5,
        "Unsupported log size or archive count"
    );
    let entries: Vec<crate::catalog::Entry> = section(document, "custom_apps")?;
    crate::catalog::validate_config_custom_entries(&entries)?;
    let _: std::collections::BTreeMap<String, crate::installations::launch::LaunchSettings> =
        section(document, "launch")?;
    let _: std::collections::BTreeMap<String, crate::builds::BuildLaunchOptions> =
        section(document, "build_launch")?;
    // Validate document-side budgets too, before serializing a programmatic patch.
    fn visit(value: &Value, depth: usize, nodes: &mut usize) -> Result<()> {
        *nodes += 1;
        if depth > 64 || *nodes > 100_000 {
            bail!("Configuration exceeds document limits");
        }
        match value {
            Value::Object(map) => {
                for value in map.values() {
                    visit(value, depth + 1, nodes)?;
                }
            }
            Value::Array(array) => {
                for value in array {
                    visit(value, depth + 1, nodes)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    visit(document, 1, &mut 0)
}
pub(crate) fn encode(document: &Value) -> Result<Vec<u8>> {
    validate(document)?;
    let bytes = serde_saphyr::to_string(document)?.into_bytes();
    // Prove output obeys exactly the parser contract before touching disk.
    let decoded = parse(&bytes)?;
    ensure!(
        decoded == *document,
        "Configuration serialization changed document values"
    );
    Ok(bytes)
}
