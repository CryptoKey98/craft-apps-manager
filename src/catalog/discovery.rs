//! Persistent presentation state for automatically discovered applications.
use crate::{catalog::Entry, files};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Clone, Default, Serialize, Deserialize)]
struct State {
    known: BTreeSet<String>,
    unread: BTreeSet<String>,
    #[serde(default)]
    announced: BTreeMap<String, i64>,
}

pub struct NewApps {
    root: PathBuf,
    state: State,
    preview: Option<String>,
}

#[cfg(test)]
fn eligible(entries: &[Entry]) -> BTreeSet<String> {
    entries
        .iter()
        .filter(|entry| entry.release && !entry.repository.contains('/'))
        .map(|entry| entry.repository.to_ascii_lowercase())
        .collect()
}

const NEW_APP_SECONDS: i64 = 14 * 24 * 3600;

/// Shared offline admission dates. Null entries are established apps whose
/// historical admission date is unknown; upgrading must not announce them anew.
pub(crate) fn bundled_admissions() -> &'static BTreeMap<String, Option<i64>> {
    static DATES: std::sync::OnceLock<BTreeMap<String, Option<i64>>> = std::sync::OnceLock::new();
    DATES.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../assets/app-release-catalog/admissions.json"
        ))
        .expect("Invalid bundled app admission dates")
    })
}
pub(crate) fn bundled_added_at(repository: &str) -> Option<i64> {
    bundled_admissions().get(repository).copied().flatten()
}
fn added_at(entry: &Entry) -> Option<i64> {
    entry
        .discovery_added_at
        .or_else(|| bundled_added_at(&crate::catalog::repository_slug(&entry.repository)))
}
pub(crate) fn recently_added(entry: &Entry) -> bool {
    let now = chrono::Utc::now().timestamp();
    added_at(entry).is_some_and(|added| added > 0 && now >= added && now - added < NEW_APP_SECONDS)
}

impl NewApps {
    pub fn load(root: &Path, entries: &[Entry], preview: Option<&str>) -> Result<Self> {
        let path = root.join("runtime/new-apps.json");
        let state = if path.exists() {
            files::read_json(&path)?
        } else {
            State::default()
        };
        let mut tracker = Self {
            root: root.into(),
            state,
            preview: preview.map(str::to_owned),
        };
        tracker.refresh(root, entries)?;
        if let Some(key) = preview {
            if let Some(entry) = entries.iter().find(|entry| entry.key == key) {
                tracker
                    .state
                    .unread
                    .insert(entry.repository.to_ascii_lowercase());
            }
        }
        Ok(tracker)
    }

    pub fn refresh(&mut self, root: &Path, entries: &[Entry]) -> Result<()> {
        if self.root != root {
            *self = Self::load(root, entries, self.preview.as_deref())?;
            return Ok(());
        }
        let mut next = self.state.clone();
        let recent: BTreeSet<_> = entries
            .iter()
            .filter(|entry| recently_added(entry))
            .map(|entry| entry.repository.to_ascii_lowercase())
            .collect();
        // Keep legacy unread records while official metadata is unavailable.
        next.unread.retain(|identity| {
            entries
                .iter()
                .find(|entry| entry.repository.eq_ignore_ascii_case(identity))
                .is_none_or(|entry| added_at(entry).is_none() || recent.contains(identity))
        });
        let now = chrono::Utc::now().timestamp();
        for entry in entries
            .iter()
            .filter(|entry| entry.release && !entry.repository.contains('/'))
        {
            let identity = entry.repository.to_ascii_lowercase();
            next.known.insert(identity.clone());
            if let Some(added) = added_at(entry).filter(|added| *added > 0 && *added <= now) {
                // Remember the shared announcement once. Reads, refreshes and
                // dismissals must not restart its window or resurrect a badge.
                if !next.announced.contains_key(&identity) {
                    next.announced.insert(identity.clone(), added);
                    if recent.contains(&identity) {
                        next.unread.insert(identity);
                    }
                }
            }
        }
        self.commit(next)
    }

    pub fn is_new(&self, entry: &Entry) -> bool {
        (self.preview.as_deref() == Some(entry.key.as_str()) || recently_added(entry))
            && self
                .state
                .unread
                .contains(&entry.repository.to_ascii_lowercase())
    }

    pub fn mark_seen(&mut self, entry: &Entry) -> Result<()> {
        if !self.is_new(entry) {
            return Ok(());
        }
        let mut next = self.state.clone();
        next.unread.remove(&entry.repository.to_ascii_lowercase());
        self.commit(next)
    }

    fn commit(&mut self, next: State) -> Result<()> {
        if self.preview.is_none() {
            files::write_json(&self.root.join("runtime/new-apps.json"), &next)?;
        }
        self.state = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn with_added(entry: &Entry, date: Option<i64>) -> Entry {
        let mut value = serde_json::to_value(entry).unwrap();
        value["discoveryAddedAt"] = serde_json::json!(date);
        serde_json::from_value(value).unwrap()
    }

    fn entries() -> Vec<Entry> {
        crate::catalog::builtin()
    }

    #[test]
    fn official_admission_controls_badges_for_fresh_and_existing_users() {
        let root = std::env::temp_dir().join(format!("craft-new-age-{}", uuid::Uuid::new_v4()));
        let now = chrono::Utc::now().timestamp();
        let mut apps = entries();
        for app in &mut apps {
            *app = with_added(app, Some(now - 15 * 86400));
        }
        apps[0] = with_added(&apps[0], Some(now - 86400));
        let recent = apps[0].clone();
        let mut tracker = NewApps::load(&root, &apps, None).unwrap();
        assert!(
            tracker.is_new(&recent),
            "fresh installs should highlight recent official additions"
        );
        assert!(!tracker.is_new(&apps[1]));
        tracker.mark_seen(&recent).unwrap();
        tracker.refresh(&root, &apps).unwrap();
        assert!(!tracker.is_new(&recent));
        let mut discovered = apps[1].clone();
        discovered.repository = "newcraft".into();
        discovered = with_added(&discovered, Some(now - 86400));
        apps.push(discovered.clone());
        tracker.refresh(&root, &apps).unwrap();
        assert!(tracker.is_new(&discovered));
        discovered = with_added(&discovered, Some(now - 14 * 86400));
        assert!(
            !tracker.is_new(&discovered),
            "expiry must apply without a refresh or restart"
        );
        *apps.last_mut().unwrap() = discovered.clone();
        tracker.refresh(&root, &apps).unwrap();
        assert!(!NewApps::load(&root, &apps, None)
            .unwrap()
            .is_new(&discovered));
        discovered = with_added(&discovered, None);
        assert!(!tracker.is_new(&discovered));
        discovered = with_added(&discovered, Some(now + 86400));
        assert!(!tracker.is_new(&discovered));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn new_apps_persist_until_opened_and_do_not_reappear_after_refresh() {
        let root = std::env::temp_dir().join(format!("craft-new-apps-{}", uuid::Uuid::new_v4()));
        let mut apps = entries();
        let index = apps.iter().position(|app| app.release).unwrap();
        let discovered = with_added(
            &apps.remove(index),
            Some(chrono::Utc::now().timestamp() - 86400),
        );
        let mut tracker = NewApps::load(&root, &apps, None).unwrap();
        assert!(apps.iter().all(|app| !tracker.is_new(app)));
        apps.push(discovered.clone());
        tracker.refresh(&root, &apps).unwrap();
        assert!(tracker.is_new(&discovered));
        tracker = NewApps::load(&root, &apps, None).unwrap();
        assert!(tracker.is_new(&discovered));
        tracker.mark_seen(&discovered).unwrap();
        tracker.refresh(&root, &apps).unwrap();
        let restarted = NewApps::load(&root, &apps, None).unwrap();
        assert!(!restarted.is_new(&discovered));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_unread_state_waits_for_official_dates_and_expires() {
        let root = std::env::temp_dir().join(format!("craft-legacy-age-{}", uuid::Uuid::new_v4()));
        let mut apps = entries();
        let identity = apps[0].repository.to_ascii_lowercase();
        files::write_json(
            &root.join("runtime/new-apps.json"),
            &serde_json::json!({
                "known": eligible(&apps), "unread": [identity], "promoted": [identity]
            }),
        )
        .unwrap();
        let mut tracker = NewApps::load(&root, &apps, None).unwrap();
        assert!(!tracker.is_new(&apps[0]));
        let now = chrono::Utc::now().timestamp();
        apps[0] = with_added(&apps[0], Some(now - 86400));
        tracker.refresh(&root, &apps).unwrap();
        assert!(tracker.is_new(&apps[0]));
        apps[0] = with_added(&apps[0], Some(now - 15 * 86400));
        tracker.refresh(&root, &apps).unwrap();
        assert!(!tracker.is_new(&apps[0]));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preview_never_changes_real_discovery_state() {
        let root = std::env::temp_dir().join(format!("craft-new-apps-{}", uuid::Uuid::new_v4()));
        let apps = entries();
        let app = &apps[0];
        NewApps::load(&root, &apps, None).unwrap();
        let path = root.join("runtime/new-apps.json");
        let before = std::fs::read(&path).unwrap();
        let mut tracker = NewApps::load(&root, &apps, Some(&app.key)).unwrap();
        assert!(tracker.is_new(app));
        tracker.mark_seen(app).unwrap();
        assert!(!tracker.is_new(app));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(!NewApps::load(&root, &apps, None).unwrap().is_new(app));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preview_does_not_create_a_state_file_and_explicit_custom_apps_are_not_new() {
        let root = std::env::temp_dir().join(format!("craft-new-apps-{}", uuid::Uuid::new_v4()));
        let mut apps = entries();
        let mut tracker = NewApps::load(&root, &apps, Some(&apps[0].key)).unwrap();
        let mut custom = apps[0].clone();
        custom.repository = "someone/custom-app".into();
        apps.push(custom.clone());
        tracker.refresh(&root, &apps).unwrap();
        assert!(!tracker.is_new(&custom));
        assert!(!root.exists());
    }
}

#[cfg(test)]
mod admission_regressions {
    use super::*;
    #[test]
    fn badge_window_starts_at_official_admission_even_for_an_old_repository() {
        let root =
            std::env::temp_dir().join(format!("craft-admission-badge-{}", uuid::Uuid::new_v4()));
        let now = chrono::Utc::now().timestamp();
        let mut value = serde_json::to_value(crate::catalog::builtin().remove(0)).unwrap();
        value["createdAt"] = serde_json::json!(now - 90 * 86400);
        value["discoveryAddedAt"] = serde_json::json!(now - 86400);
        let entry: Entry = serde_json::from_value(value.clone()).unwrap();
        let mut tracker = NewApps::load(&root, std::slice::from_ref(&entry), None).unwrap();
        assert!(
            tracker.is_new(&entry),
            "An old repository can be newly admitted to discovery"
        );
        tracker.mark_seen(&entry).unwrap();
        assert!(!NewApps::load(&root, std::slice::from_ref(&entry), None)
            .unwrap()
            .is_new(&entry));
        value["createdAt"] = serde_json::json!(now - 86400);
        value["discoveryAddedAt"] = serde_json::json!(now - 15 * 86400);
        let old: Entry = serde_json::from_value(value).unwrap();
        assert!(
            !NewApps::load(&root.join("fresh"), std::slice::from_ref(&old), None)
                .unwrap()
                .is_new(&old),
            "A new installation must not restart an expired shared window"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod migration_regressions {
    use super::*;
    #[test]
    fn legacy_discovery_records_gain_one_announcement_without_losing_dismissal() {
        let root =
            std::env::temp_dir().join(format!("craft-admission-upgrade-{}", uuid::Uuid::new_v4()));
        let mut entry = crate::catalog::builtin().remove(0);
        entry.discovery_added_at = Some(chrono::Utc::now().timestamp() - 3600);
        let identity = entry.repository.clone();
        files::write_json(
            &root.join("runtime/new-apps.json"),
            &serde_json::json!({"known":[identity],"unread":[],"promoted":[identity]}),
        )
        .unwrap();
        let mut tracker = NewApps::load(&root, std::slice::from_ref(&entry), None).unwrap();
        assert!(tracker.is_new(&entry));
        tracker.mark_seen(&entry).unwrap();
        tracker
            .refresh(&root, std::slice::from_ref(&entry))
            .unwrap();
        let tracker = NewApps::load(&root, std::slice::from_ref(&entry), None).unwrap();
        assert!(!tracker.is_new(&entry));
        let saved: serde_json::Value =
            files::read_json(&root.join("runtime/new-apps.json")).unwrap();
        assert!(
            saved.get("promoted").is_none(),
            "Obsolete permanent priority must be removed"
        );
        assert_eq!(
            saved["announced"][&identity],
            entry.discovery_added_at.unwrap()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
