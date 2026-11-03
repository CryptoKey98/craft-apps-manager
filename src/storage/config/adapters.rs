//! Typed consumer adapters preserve raw unknown properties and merge field edits.
use super::{ConfigPatch, ConfigStore, FieldPatch};
use anyhow::Result;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;

impl ConfigStore {
    pub fn patch_section_typed<T: Serialize + DeserializeOwned + Default>(
        &self,
        section: &str,
        base: &T,
        edited: &T,
    ) -> Result<super::ConfigSnapshot> {
        let _lock = crate::platform::Lock::take("Local\\CraftAppsManager")?;
        self.patch_section_typed_locked(section, base, edited)
    }
    pub fn patch_section_typed_locked<T: Serialize + DeserializeOwned + Default>(
        &self,
        section: &str,
        base: &T,
        edited: &T,
    ) -> Result<super::ConfigSnapshot> {
        let _config_lock = super::write_lock()?;
        let snapshot = self.load_or_migrate_locked()?;
        let current: T = super::document::section(&snapshot.document, section)?;
        let logical = serde_json::to_value(current)?;
        let base = serde_json::to_value(base)?;
        let edited = serde_json::to_value(edited)?;
        let mut fields = Vec::new();
        diff(
            &base,
            &edited,
            vec![section.into()],
            &mut |path, old, new| {
                let tail = &path[1..];
                let raw = at(&snapshot.document, &path).cloned();
                let before = if at(&logical, tail) == Some(old)
                    || (old.is_null() && at(&logical, tail).is_none())
                {
                    raw
                } else {
                    Some(old.clone())
                };
                fields.push(FieldPatch {
                    path,
                    before,
                    after: (!new.is_null()).then(|| new.clone()),
                });
            },
        );
        let document = super::patch::apply(&snapshot.document, ConfigPatch { fields })?;
        if document == snapshot.document {
            return Ok(snapshot);
        }
        self.publish_locked(document)
    }
    /// Caller owns Local\\CraftAppsManager, including migration ownership.
    pub(crate) fn mutate_section_locked<T: Serialize + DeserializeOwned>(
        &self,
        section: &str,
        change: impl FnOnce(&mut T) -> Result<()>,
    ) -> Result<super::ConfigSnapshot> {
        let _config_lock = super::write_lock()?;
        let snapshot = self.load_or_migrate_locked()?;
        let mut typed: T = super::document::section(&snapshot.document, section)?;
        let before = serde_json::to_value(&typed)?;
        change(&mut typed)?;
        let mut after = serde_json::to_value(&typed)?;
        if section == "custom_apps" {
            after = super::document::preserve_custom_apps(
                snapshot
                    .document
                    .get(section)
                    .unwrap_or(&Value::Array(Vec::new())),
                &after,
            )?;
        }
        let mut document = snapshot.document.clone();
        diff(
            &before,
            &after,
            vec![section.into()],
            &mut |path, _, new| {
                put(&mut document, &path, new.clone());
            },
        );
        if document == snapshot.document {
            return Ok(snapshot);
        }
        self.publish_locked(document)
    }
    pub fn mutate_section<T: Serialize + DeserializeOwned>(
        &self,
        section: &str,
        change: impl FnOnce(&mut T) -> Result<()>,
    ) -> Result<super::ConfigSnapshot> {
        let _lock = crate::platform::Lock::take("Local\\CraftAppsManager")?;
        self.mutate_section_locked(section, change)
    }
}
fn at<'a>(value: &'a Value, path: &[String]) -> Option<&'a Value> {
    path.iter().try_fold(value, |v, k| v.get(k))
}
fn put(value: &mut Value, path: &[String], new: Value) {
    let mut cursor = value;
    for key in &path[..path.len() - 1] {
        if cursor.get(key).is_none() {
            cursor[key] = serde_json::json!({});
        }
        cursor = &mut cursor[key];
    }
    if new.is_null() {
        if let Some(map) = cursor.as_object_mut() {
            map.remove(&path[path.len() - 1]);
        }
    } else {
        cursor[&path[path.len() - 1]] = new;
    }
}
fn diff(
    base: &Value,
    edited: &Value,
    path: Vec<String>,
    emit: &mut impl FnMut(Vec<String>, &Value, &Value),
) {
    if base == edited {
        return;
    }
    if let (Some(old), Some(new)) = (base.as_object(), edited.as_object()) {
        // A removed typed property becomes null; typed consumers normally serialize every field.
        for key in old
            .keys()
            .chain(new.keys())
            .collect::<std::collections::BTreeSet<_>>()
        {
            let mut next = path.clone();
            next.push(key.clone());
            diff(
                old.get(key).unwrap_or(&Value::Null),
                new.get(key).unwrap_or(&Value::Null),
                next,
                emit,
            );
        }
    } else {
        emit(path, base, edited);
    }
}

pub(crate) fn merge_typed_into<T: Serialize>(
    document: &mut Value,
    section: &str,
    base: &T,
    edited: &T,
) -> Result<()> {
    let before = serde_json::to_value(base)?;
    let mut after = serde_json::to_value(edited)?;
    if section == "custom_apps" {
        after = super::document::preserve_custom_apps(
            document.get(section).unwrap_or(&Value::Array(Vec::new())),
            &after,
        )?;
    }
    diff(
        &before,
        &after,
        vec![section.into()],
        &mut |path, _, new| put(document, &path, new.clone()),
    );
    Ok(())
}

#[cfg(test)]
mod consumer_tests {
    use super::*;
    use crate::storage::preferences::Preferences;
    use serde_json::json;
    #[test]
    fn typed_patch_merges_unrelated_fields_preserves_unknowns_and_rejects_conflicts() {
        let root = std::env::temp_dir().join(format!("craft-adapter-{}", uuid::Uuid::new_v4()));
        let store = ConfigStore::new(root.clone());
        let mut initial = super::super::document::defaults();
        initial["manager"]["future"] = json!({"keep":true});
        store.publish_locked(initial).unwrap();
        let base = store.load().unwrap().manager().unwrap();
        let mut first = base.clone();
        first.app_check_interval_minutes = 30;
        store.patch_section_typed("manager", &base, &first).unwrap();
        let mut second = base.clone();
        second.theme = crate::storage::preferences::Theme::Dark;
        let result = store
            .patch_section_typed("manager", &base, &second)
            .unwrap();
        assert_eq!(result.manager().unwrap().app_check_interval_minutes, 30);
        assert_eq!(result.document["manager"]["future"]["keep"], true);
        let mut conflict = base.clone();
        conflict.app_check_interval_minutes = 40;
        assert!(store
            .patch_section_typed("manager", &base, &conflict)
            .is_err());
        assert!(!root.join("manager-settings.json").exists());
        let restored = ConfigStore::new(root.clone())
            .load()
            .unwrap()
            .manager()
            .unwrap();
        assert_eq!(restored.theme, crate::storage::preferences::Theme::Dark);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn independent_libraries_and_noop_do_not_create_json_or_config() {
        let root = std::env::temp_dir().join(format!("craft-independent-{}", uuid::Uuid::new_v4()));
        let first = ConfigStore::new(root.join("first"));
        let second = ConfigStore::new(root.join("second"));
        let base = Preferences::default();
        first.patch_section_typed("manager", &base, &base).unwrap();
        assert!(!first.path().exists());
        let mut edited = base.clone();
        edited.app_check_interval_minutes = 50;
        first
            .patch_section_typed("manager", &base, &edited)
            .unwrap();
        assert_eq!(
            second
                .load()
                .unwrap()
                .manager()
                .unwrap()
                .app_check_interval_minutes,
            base.app_check_interval_minutes
        );
        assert!(!second.path().exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
