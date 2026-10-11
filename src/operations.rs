//! Durable observation and cooperative cancellation; execution locks remain with jobs.
use crate::{files, jobs::State};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lifecycle {
    Queued,
    Running,
    CancellationRequested,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}
impl Lifecycle {
    pub fn terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Operation {
    pub id: String,
    pub parent: Option<String>,
    pub resource: String,
    pub kind: String,
    pub lifecycle: Lifecycle,
    pub stage: String,
    pub detail: String,
    pub progress: Option<f32>,
    pub started: Option<i64>,
    pub completed: Option<i64>,
    pub heartbeat: i64,
    pub cancellation_available: bool,
    pub result: Option<String>,
    pub owner: u32,
    pub format: Option<String>,
    pub architecture: Option<String>,
    #[serde(default)]
    pub facts: std::collections::BTreeMap<String, String>,
}
fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
#[derive(Clone)]
pub struct Manager {
    directory: PathBuf,
}
#[derive(Clone)]
pub struct Handle {
    manager: Manager,
    record: Arc<Mutex<Operation>>,
}
impl Manager {
    pub fn new(root: &Path) -> Self {
        Self {
            directory: root.join("runtime/operations"),
        }
    }
    fn path(&self, id: &str) -> Result<PathBuf> {
        uuid::Uuid::parse_str(id)?;
        Ok(self.directory.join(format!("{id}.json")))
    }
    pub fn register(&self, kind: &str, resource: &str, parent: Option<String>) -> Handle {
        let handle = Handle {
            manager: self.clone(),
            record: Arc::new(Mutex::new(Operation {
                id: uuid::Uuid::new_v4().to_string(),
                parent,
                resource: resource.into(),
                kind: kind.into(),
                lifecycle: Lifecycle::Queued,
                stage: "Queued".into(),
                detail: String::new(),
                progress: None,
                started: None,
                completed: None,
                heartbeat: now(),
                cancellation_available: true,
                result: None,
                owner: std::process::id(),
                format: None,
                architecture: None,
                facts: Default::default(),
            })),
        };
        handle.publish();
        self.prune();
        handle
    }
    pub fn adopt(&self, id: &str) -> Result<Handle> {
        let mut record: Operation = files::read_json(&self.path(id)?)?;
        anyhow::ensure!(
            record.id == id && !record.lifecycle.terminal(),
            "Operation already finished"
        );
        record.owner = std::process::id();
        Ok(Handle {
            manager: self.clone(),
            record: Arc::new(Mutex::new(record)),
        })
    }
    pub fn read(&self, id: &str) -> Result<Operation> {
        files::read_json(&self.path(id)?)
    }
    pub fn list(&self) -> Vec<Operation> {
        let mut records: Vec<Operation> = std::fs::read_dir(&self.directory).into_iter().flatten().flatten()
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .filter_map(|entry| {
                // Bounded records, not logs; malformed or partial external data is ignored.
                if entry.metadata().ok()?.len() > 32_768 { return None; }
                let mut record: Operation = files::read_json(&entry.path()).ok()?;
                if self.path(&record.id).ok()? != entry.path() { return None; }
                let parent_alive = record.parent.as_ref().and_then(|id| self.read(id).ok()).is_some_and(|parent| !parent.lifecycle.terminal() && now() - parent.heartbeat <= 30_000);
                if !record.lifecycle.terminal() && now() - record.heartbeat > 30_000 && !(record.lifecycle == Lifecycle::Queued && parent_alive) {
                    record.lifecycle = Lifecycle::Interrupted;
                    record.cancellation_available = false;
                    record.result = Some("Worker stopped reporting. The outcome is unknown; inspect the installation before retrying.".into());
                }
                Some(record)
            }).collect();
        records.sort_by_key(|op| std::cmp::Reverse(op.started.unwrap_or(op.heartbeat)));
        records
    }
    pub fn request_cancel(&self, id: &str) -> Result<bool> {
        let record = self.read(id)?;
        if record.lifecycle.terminal() || !record.cancellation_available || record.parent.is_some()
        {
            return Ok(false);
        }
        files::write_bytes(&self.path(id)?.with_extension("cancel"), b"cancel")?;
        Ok(true)
    }
    fn prune(&self) {
        let records = self.list();
        for record in records
            .iter()
            .filter(|record| {
                record.lifecycle.terminal()
                    && !records.iter().any(|child| {
                        child.parent.as_ref() == Some(&record.id) && !child.lifecycle.terminal()
                    })
            })
            .skip(100)
        {
            if let Ok(path) = self.path(&record.id) {
                let _ = std::fs::remove_file(path.with_extension("cancel"));
                let _ = std::fs::remove_file(path);
            }
        }
    }
}
impl Handle {
    pub fn id(&self) -> String {
        self.record.lock().unwrap().id.clone()
    }
    pub fn snapshot(&self) -> Operation {
        self.record.lock().unwrap().clone()
    }
    /// Record execution-confirmed details, keeping history bounded and immutable after completion.
    pub fn report(&self, key: &str, value: &str) {
        let mut record = self.record.lock().unwrap();
        if record.lifecycle.terminal()
            || (record.facts.len() >= 16 && !record.facts.contains_key(key))
        {
            return;
        }
        record.facts.insert(
            key.chars().take(64).collect(),
            value.chars().take(1024).collect(),
        );
        drop(record);
        self.publish();
    }
    fn publish(&self) {
        let record = self.record.lock().unwrap();
        if let Ok(path) = self.manager.path(&record.id) {
            let _ = files::write_json(&path, &*record);
        }
    }
    pub fn options(&self, format: &str, architecture: &str) {
        let mut record = self.record.lock().unwrap();
        if record.lifecycle.terminal() {
            return;
        }
        record.format = Some(format.into());
        record.architecture = Some(architecture.into());
        drop(record);
        self.publish();
    }
    pub fn start(&self) -> bool {
        let mut record = self.record.lock().unwrap();
        if record.lifecycle.terminal() {
            return false;
        }
        if record.lifecycle == Lifecycle::Queued {
            record.lifecycle = Lifecycle::Running;
            record.started = Some(now());
        }
        record.heartbeat = now();
        drop(record);
        self.publish();
        true
    }
    pub fn update(&self, state: &State, requested: bool) {
        let mut record = self.record.lock().unwrap();
        if record.lifecycle.terminal() {
            return;
        }
        if requested {
            record.lifecycle = Lifecycle::CancellationRequested;
        }
        record.stage = state.stage.clone();
        record.detail = state.detail.chars().take(4096).collect();
        record.progress = state
            .progress
            .filter(|p| p.is_finite())
            .map(|p| p.clamp(0.0, 1.0));
        record.cancellation_available = record.parent.is_none()
            && !matches!(
                record.kind.as_str(),
                "Uninstall"
                    | "Remove source"
                    | "Delete cache"
                    | "Restore backup"
                    | "uninstall-app"
                    | "remove-source"
                    | "clear"
                    | "clear-app"
                    | "clean"
                    | "clean-build"
                    | "delete-backups"
                    | "restore"
            )
            && !(cfg!(target_os = "linux") && state.stage == "Installing package");
        record.heartbeat = now();
        // Finish must not return while a delayed progress publication is still pending.
        if let Ok(path) = self.manager.path(&record.id) {
            let _ = files::write_json(&path, &*record);
        }
    }
    pub fn cancel_requested(&self) -> bool {
        self.manager
            .path(&self.id())
            .is_ok_and(|path| path.with_extension("cancel").exists())
    }
    pub fn finish(&self, lifecycle: Lifecycle, message: String) -> bool {
        if !lifecycle.terminal() {
            return false;
        }
        let mut record = self.record.lock().unwrap();
        if record.lifecycle.terminal() {
            return false;
        }
        record.lifecycle = lifecycle;
        record.completed = Some(now());
        record.heartbeat = now();
        record.cancellation_available = false;
        record.progress = if lifecycle == Lifecycle::Completed {
            Some(1.0)
        } else {
            None
        };
        record.stage = format!("{lifecycle:?}");
        record.result = Some(message.chars().take(4096).collect());
        drop(record);
        self.publish();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lifecycle_is_independent_terminal_and_durable() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let manager = Manager::new(&root);
        let first = manager.register("Install", "first", None);
        let second = manager.register("Build", "second", None);
        assert_eq!(first.snapshot().lifecycle, Lifecycle::Queued);
        first.start();
        second.start();
        first.update(
            &State {
                stage: "Downloading".into(),
                progress: Some(0.4),
                ..Default::default()
            },
            false,
        );
        assert_eq!(second.snapshot().progress, None);
        assert!(Manager::new(&root).request_cancel(&first.id()).unwrap());
        first.update(&State::default(), true);
        // A cancellation request racing with verified success is still success.
        assert!(first.finish(Lifecycle::Completed, "Success".into()));
        assert!(!first.finish(Lifecycle::Cancelled, "late".into()));
        first.update(
            &State {
                stage: "Installing".into(),
                ..Default::default()
            },
            false,
        );
        assert_eq!(
            manager.read(&first.id()).unwrap().lifecycle,
            Lifecycle::Completed
        );
        assert!(!manager.request_cancel(&first.id()).unwrap());
        assert_eq!(manager.list().len(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn execution_details_survive_restart_and_cannot_change_after_completion() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let manager = Manager::new(&root);
        let handle = manager.register("Install", "photocraft", None);
        handle.start();
        handle.report("Installation location", "C:/Apps/PhotoCraft");
        handle.report("Desktop shortcut", "Could not be created: access denied");
        handle.finish(Lifecycle::Completed, "Success".into());
        handle.report("Desktop shortcut", "Created successfully");
        let record = Manager::new(&root).read(&handle.id()).unwrap();
        assert_eq!(record.facts["Installation location"], "C:/Apps/PhotoCraft");
        assert!(record.facts["Desktop shortcut"].contains("access denied"));
        let mut legacy = serde_json::to_value(&record).unwrap();
        legacy.as_object_mut().unwrap().remove("facts");
        assert!(serde_json::from_value::<Operation>(legacy)
            .unwrap()
            .facts
            .is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn stale_heartbeat_is_unknown_not_success_and_can_recover() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let manager = Manager::new(&root);
        let handle = manager.register("Build", "app", None);
        handle.start();
        handle.record.lock().unwrap().heartbeat = now() - 60_000;
        handle.publish();
        assert_eq!(manager.list()[0].lifecycle, Lifecycle::Interrupted);
        handle.update(&State::default(), false);
        assert_eq!(manager.list()[0].lifecycle, Lifecycle::Running);
        assert!(manager.request_cancel("../escape").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
