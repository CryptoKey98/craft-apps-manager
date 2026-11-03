use super::ConfigPatch;
use anyhow::{bail, ensure, Result};
use serde_json::{Map, Value};

pub(super) fn apply(latest: &Value, patch: ConfigPatch) -> Result<Value> {
    for (index, field) in patch.fields.iter().enumerate() {
        ensure!(
            !field.path.is_empty(),
            "Configuration patch cannot replace the document root"
        );
        ensure!(
            field.path[0] != "schema_version",
            "Configuration patches cannot change schema version"
        );
        for other in &patch.fields[..index] {
            ensure!(
                !field.path.starts_with(&other.path) && !other.path.starts_with(&field.path),
                "Configuration patch contains overlapping fields"
            );
        }
    }
    let mut merged = latest.clone();
    for field in patch.fields {
        let mut current = latest;
        for part in &field.path[..field.path.len() - 1] {
            match current.get(part) {
                Some(next) if next.is_object() => current = next,
                None if current.is_object() => {
                    current = &Value::Null;
                    break;
                }
                _ => bail!("Configuration conflict at {}", field.path.join("/")),
            }
        }
        let key = field.path.last().expect("nonempty path");
        let existing = current.get(key).cloned();
        if existing == field.after {
            continue;
        }
        ensure!(
            existing == field.before,
            "Configuration conflict at {}",
            field.path.join("/")
        );
        let mut parent = &mut merged;
        for part in &field.path[..field.path.len() - 1] {
            let map = parent.as_object_mut().ok_or_else(|| {
                anyhow::anyhow!("Configuration conflict at {}", field.path.join("/"))
            })?;
            parent = map
                .entry(part.clone())
                .or_insert_with(|| Value::Object(Map::new()));
        }
        let map = parent
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("Configuration conflict at {}", field.path.join("/")))?;
        match field.after {
            Some(value) => {
                let value = if field.path.len() == 1 && key == "custom_apps" {
                    super::document::preserve_custom_apps(
                        existing.as_ref().unwrap_or(&Value::Array(Vec::new())),
                        &value,
                    )?
                } else {
                    value
                };
                map.insert(key.clone(), value);
            }
            None => {
                map.remove(key);
            }
        }
    }
    super::document::validate(&merged)?;
    Ok(merged)
}
