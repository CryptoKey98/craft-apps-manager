//! Persistent presentation state for automatically discovered applications.
use crate::{catalog::Entry, files};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

#[derive(Clone, Default, Serialize, Deserialize)]
struct State {
    known: BTreeSet<String>,
    unread: BTreeSet<String>,
    #[serde(default)]
    promoted: Vec<String>,
}

pub struct NewApps {
    root: PathBuf,
    state: State,
    preview: Option<String>,
}

fn eligible(entries: &[Entry]) -> BTreeSet<String> {
    entries
        .iter()
        .filter(|entry| entry.release && !entry.repository.contains('/'))
        .map(|entry| entry.repository.to_ascii_lowercase())
        .collect()
}

impl NewApps {
    pub fn load(root: &Path, entries: &[Entry], preview: Option<&str>) -> Result<Self> {
        let path = root.join("runtime/new-apps.json");
        // On upgrade, baseline existing apps rather than calling the entire library new.
        let state = if path.exists() {
            files::read_json(&path)?
        } else {
            State {
                known: eligible(entries),
                unread: BTreeSet::new(),
                promoted: Vec::new(),
            }
        };
        let mut tracker = Self {
            root: root.into(),
            state,
            preview: preview.map(str::to_owned),
        };
        tracker.refresh(root, entries)?;
        if let Some(key) = preview {
            if let Some(entry) = entries.iter().find(|entry| entry.key == key) {
                let identity = entry.repository.to_ascii_lowercase();
                tracker.state.promoted.retain(|app| app != &identity);
                tracker.state.promoted.insert(0, identity.clone());
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
        // Preserve pending discoveries when upgrading the older badge state.
        for identity in &next.unread {
            if !next.promoted.contains(identity) {
                next.promoted.push(identity.clone());
            }
        }
        for identity in eligible(entries) {
            if next.known.insert(identity.clone()) {
                next.promoted.insert(0, identity.clone());
                next.unread.insert(identity);
            }
        }
        self.commit(next)
    }

    pub fn is_new(&self, entry: &Entry) -> bool {
        self.state
            .unread
            .contains(&entry.repository.to_ascii_lowercase())
    }

    pub fn discovery_rank(&self, entry: &Entry) -> usize {
        let identity = entry.repository.to_ascii_lowercase();
        self.state
            .promoted
            .iter()
            .position(|app| app == &identity)
            .unwrap_or(usize::MAX)
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
    fn entries() -> Vec<Entry> {
        crate::catalog::builtin()
    }

    #[test]
    fn new_apps_persist_until_opened_and_do_not_reappear_after_refresh() {
        let root = std::env::temp_dir().join(format!("craft-new-apps-{}", uuid::Uuid::new_v4()));
        let mut apps = entries();
        let index = apps.iter().position(|app| app.release).unwrap();
        let discovered = apps.remove(index);
        let mut tracker = NewApps::load(&root, &apps, None).unwrap();
        assert!(apps.iter().all(|app| !tracker.is_new(app)));
        apps.push(discovered.clone());
        tracker.refresh(&root, &apps).unwrap();
        assert!(tracker.is_new(&discovered));
        assert_eq!(tracker.discovery_rank(&discovered), 0);
        tracker = NewApps::load(&root, &apps, None).unwrap();
        assert!(tracker.is_new(&discovered));
        tracker.mark_seen(&discovered).unwrap();
        assert_eq!(tracker.discovery_rank(&discovered), 0);
        tracker.refresh(&root, &apps).unwrap();
        let restarted = NewApps::load(&root, &apps, None).unwrap();
        assert!(!restarted.is_new(&discovered));
        assert_eq!(restarted.discovery_rank(&discovered), 0);
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
