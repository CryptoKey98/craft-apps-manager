//! Durable observation and cooperative cancellation; execution locks remain with jobs.
#[cfg(any(target_os = "windows", target_os = "linux"))]
pub mod elevation;
pub mod jobs;

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
// Only an exited process is proof that a stale worker cannot publish again.
// PID reuse, permission failures, and unsupported/invalid IDs all retain history.
fn owner_may_publish(owner: u32) -> bool {
    if owner == 0 || owner == std::process::id() {
        return true;
    }
    #[cfg(target_os = "windows")]
    unsafe {
        use windows::Win32::{
            Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, WAIT_OBJECT_0},
            System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_ACCESS_RIGHTS},
        };
        match OpenProcess(PROCESS_ACCESS_RIGHTS(0x00100000), false, owner) {
            Ok(handle) => {
                let alive = WaitForSingleObject(handle, 0) != WAIT_OBJECT_0;
                let _ = CloseHandle(handle);
                alive
            }
            Err(error) => {
                error.code() != windows::core::HRESULT::from_win32(ERROR_INVALID_PARAMETER.0)
            }
        }
    }
    #[cfg(unix)]
    {
        let Ok(owner) = i32::try_from(owner) else {
            return true;
        };
        if unsafe { libc::kill(owner, 0) } == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }
    #[cfg(not(any(target_os = "windows", unix)))]
    {
        true
    }
}
#[derive(Clone)]
pub struct Manager {
    directory: PathBuf,
}
#[derive(Clone)]
pub struct Handle {
    manager: Manager,
    record: Arc<Mutex<Operation>>,
    // Serialize writes and stop touching disk after the terminal snapshot is saved.
    publication: Arc<Mutex<bool>>,
}
impl Manager {
    fn activity_lock(&self, wait: bool) -> Result<crate::platform::Lock> {
        std::fs::create_dir_all(&self.directory)?;
        anyhow::ensure!(
            !files::linked(&self.directory)?,
            "Linked operation directory"
        );
        let directory = self.directory.canonicalize()?;
        use sha2::Digest;
        let name = format!(
            "Local\\CraftAppsManagerActivity-{:x}",
            sha2::Sha256::digest(directory.to_string_lossy().as_bytes())
        );
        if wait {
            crate::platform::Lock::wait(&name)
        } else {
            crate::platform::Lock::take(&name)
        }
    }
    pub fn new(root: &Path) -> Self {
        Self {
            directory: root.join("runtime/operations"),
        }
    }
    fn path(&self, id: &str) -> Result<PathBuf> {
        uuid::Uuid::parse_str(id)?;
        Ok(self.directory.join(format!("{id}.json")))
    }
    /// Separate phase markers avoid lost updates when two processes acknowledge
    /// different activities, and let completion notify after an active item was read.
    pub fn acknowledge_activity(&self, id: &str, terminal: bool) -> Result<()> {
        self.write_marker(
            id,
            if terminal {
                "read-terminal"
            } else {
                "read-active"
            },
            b"read",
        )
    }
    fn write_marker(&self, id: &str, extension: &str, bytes: &[u8]) -> Result<()> {
        let record = self.path(id)?;
        // A stale window cannot recreate markers after another window clears history.
        if !std::fs::symlink_metadata(&record).is_ok_and(|meta| meta.file_type().is_file()) {
            return Ok(());
        }
        let marker = record.with_extension(extension);
        files::write_bytes(&marker, bytes)?;
        // Marker writes run on the UI thread: never wait for a publisher/clear.
        // If deletion won the race after our initial check, remove the late marker.
        if std::fs::symlink_metadata(&record)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        {
            Self::delete_regular_file(&marker)?;
        }
        Ok(())
    }
    pub fn announce_activity(&self, id: &str) -> Result<()> {
        self.write_marker(id, "notified", b"new")
    }
    pub fn activity_was_announced(&self, id: &str) -> bool {
        self.path(id)
            .is_ok_and(|path| path.with_extension("notified").is_file())
    }
    pub fn activity_is_read(&self, id: &str, terminal: bool) -> bool {
        self.path(id).is_ok_and(|path| {
            path.with_extension(if terminal {
                "read-terminal"
            } else {
                "read-active"
            })
            .is_file()
        })
    }
    pub fn clear_activity(&self, records: &[Operation]) -> Result<()> {
        // Serialize with publication, registration and other windows' clears.
        // Only phases present at click time are cleared; new completions remain unread.
        let _lock = self.activity_lock(true)?;
        let current = self.list_records();
        let protected = Self::protected_records(&current);
        for record in records {
            let Some(latest) = current.iter().find(|item| item.id == record.id) else {
                continue;
            };
            if record.lifecycle.terminal()
                && !protected.contains(&record.id)
                && serde_json::to_value(latest)? == serde_json::to_value(record)?
            {
                self.delete_record_locked(&record.id)?;
            } else {
                // An active/stale observation must not acknowledge a later completion.
                self.write_marker(
                    &record.id,
                    if record.lifecycle.terminal() && record.lifecycle != Lifecycle::Interrupted {
                        "read-terminal"
                    } else {
                        "read-active"
                    },
                    b"read",
                )?;
            }
        }
        self.remove_orphan_markers()?;
        Self::delete_regular_file(&self.directory.join("activity-cleared.json"))
    }
    pub fn register(&self, kind: &str, resource: &str, parent: Option<String>) -> Handle {
        let handle = Handle {
            manager: self.clone(),
            publication: Arc::new(Mutex::new(false)),
            record: Arc::new(Mutex::new(Operation {
                id: uuid::Uuid::new_v4().to_string(),
                parent,
                resource: resource.chars().take(512).collect(),
                kind: kind.chars().take(128).collect(),
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
        let _ = self.publish_record(&handle.snapshot(), true);
        self.list();
        handle
    }
    pub fn adopt(&self, id: &str) -> Result<Handle> {
        let _lock = self.activity_lock(true)?;
        let mut record: Operation = files::read_json(&self.path(id)?)?;
        anyhow::ensure!(
            record.id == id && !record.lifecycle.terminal(),
            "Operation already finished"
        );
        record.owner = std::process::id();
        // Persist live ownership before a concurrent clear can observe dead work.
        files::write_json(&self.path(id)?, &record)?;
        Ok(Handle {
            manager: self.clone(),
            record: Arc::new(Mutex::new(record)),
            publication: Arc::new(Mutex::new(false)),
        })
    }
    pub fn read(&self, id: &str) -> Result<Operation> {
        files::read_json(&self.path(id)?)
    }
    pub fn list(&self) -> Vec<Operation> {
        let mut records = self.list_records();
        self.prune(&mut records);
        records
    }
    fn list_records(&self) -> Vec<Operation> {
        let mut records: Vec<Operation> = std::fs::read_dir(&self.directory).into_iter().flatten().flatten()
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .filter_map(|entry| {
                // Bounded records, not logs; malformed or partial external data is ignored.
                // Unicode and JSON escaping expand bounded character fields into bytes.
                if !entry.file_type().ok()?.is_file() || entry.metadata().ok()?.len() > 262_144 { return None; }
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
        self.write_marker(id, "cancel", b"cancel")?;
        Ok(true)
    }
    fn delete_regular_file(path: &Path) -> Result<()> {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) => {
                anyhow::ensure!(
                    metadata.file_type().is_file(),
                    "Not a regular activity file: {}",
                    path.display()
                );
                match std::fs::remove_file(path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }
    fn delete_record_locked(&self, id: &str) -> Result<()> {
        let path = self.path(id)?;
        Self::delete_regular_file(&path)?;
        for extension in ["notified", "cancel", "read-active", "read-terminal"] {
            Self::delete_regular_file(&path.with_extension(extension))?;
        }
        Ok(())
    }
    fn publish_record(&self, record: &Operation, create: bool) -> Result<()> {
        let _lock = self.activity_lock(true)?;
        let path = self.path(&record.id)?;
        if !create && !std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_file())
        {
            return Ok(());
        }
        files::write_json(&path, record)
    }
    fn protected_records(records: &[Operation]) -> std::collections::HashSet<String> {
        // Keep a whole batch while any part can still run or resume. Completed
        // children can be needed for the parent's final result and recovery.
        let mut owners = std::collections::HashMap::new();
        let mut protected: std::collections::HashSet<String> = records
            .iter()
            .filter(|record| {
                !record.lifecycle.terminal()
                    || record.lifecycle == Lifecycle::Interrupted
                        && record.completed.is_none()
                        && *owners
                            .entry(record.owner)
                            .or_insert_with(|| owner_may_publish(record.owner))
            })
            .map(|record| record.id.clone())
            .collect();
        loop {
            let before = protected.len();
            for record in records
                .iter()
                .filter(|record| protected.contains(&record.id))
                .cloned()
                .collect::<Vec<_>>()
            {
                if let Some(parent) = record.parent {
                    protected.insert(parent);
                }
            }
            for record in records {
                if record
                    .parent
                    .as_ref()
                    .is_some_and(|parent| protected.contains(parent))
                {
                    protected.insert(record.id.clone());
                }
            }
            if protected.len() == before {
                break;
            }
        }
        protected
    }
    fn prune(&self, records: &mut Vec<Operation>) {
        if !self.directory.exists() {
            return;
        }
        // Listing must never wait for a background clear or publisher on a UI thread.
        let Ok(_lock) = self.activity_lock(false) else {
            return;
        };
        *records = self.list_records();
        let protected = Self::protected_records(records);
        let mut finished: Vec<_> = records
            .iter()
            .filter(|record| record.lifecycle.terminal() && !protected.contains(&record.id))
            .collect();
        finished.sort_by_key(|record| {
            std::cmp::Reverse((
                record.completed.unwrap_or(record.heartbeat),
                record.id.clone(),
            ))
        });
        let mut removed = std::collections::HashSet::new();
        for record in finished.into_iter().skip(100) {
            // A worker may resume after an Interrupted observation. Never prune
            // a record whose persisted heartbeat or terminal phase changed.
            if self.read(&record.id).is_ok_and(|current| {
                current.heartbeat == record.heartbeat
                    && current.completed == record.completed
                    && (current.lifecycle == record.lifecycle
                        || record.lifecycle == Lifecycle::Interrupted
                            && !current.lifecycle.terminal())
            }) && self.delete_record_locked(&record.id).is_ok()
            {
                removed.insert(record.id.clone());
            }
        }
        records.retain(|record| !removed.contains(&record.id));
        let _ = self.remove_orphan_markers();
    }
    fn remove_orphan_markers(&self) -> Result<()> {
        // Old windows can recreate acknowledgements after pruning. Only recognized
        // UUID sidecars with no record may be removed; unrelated files stay intact.
        for entry in std::fs::read_dir(&self.directory)
            .into_iter()
            .flatten()
            .flatten()
        {
            let path = entry.path();
            if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
                continue;
            }
            if !path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    matches!(ext, "notified" | "cancel" | "read-active" | "read-terminal")
                })
            {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if let Ok(record_path) = self.path(id) {
                if std::fs::symlink_metadata(record_path)
                    .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
                {
                    Self::delete_regular_file(&path)?;
                }
            }
        }
        Ok(())
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
        // Serialize before taking the snapshot so an older write cannot follow
        // completion. Disk/OS-lock waits must not hold the UI's snapshot mutex.
        let mut terminal_published = self.publication.lock().unwrap();
        if *terminal_published {
            return;
        }
        let record = self.snapshot();
        if self.manager.publish_record(&record, false).is_ok() {
            *terminal_published = record.lifecycle.terminal();
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
        record.stage = state.stage.chars().take(128).collect();
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
        // Publication serializes with finish and skips disk writes once terminal.
        drop(record);
        self.publish();
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
        self.manager.list();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn exited_owner() -> u32 {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--list")
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let owner = child.id();
        assert!(child.wait().unwrap().success());
        owner
    }

    #[test]
    fn bounded_unicode_details_remain_visible_in_history() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let manager = Manager::new(&root);
        let handle = manager.register("Install", "fixture", None);
        handle.start();
        for key in 0..16 {
            handle.report(&key.to_string(), &"🦀".repeat(1024));
        }
        handle.update(
            &State {
                detail: "🦀".repeat(4096),
                ..Default::default()
            },
            false,
        );
        handle.finish(Lifecycle::Failed, "🦀".repeat(4096));
        assert_eq!(manager.list().len(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn finished_history_bounds_interrupted_records_and_cleans_markers() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let manager = Manager::new(&root);
        let stale = manager.register("Build", "stale", None);
        {
            let mut record = stale.record.lock().unwrap();
            record.heartbeat = now() - 60_000;
            record.owner = exited_owner();
        }
        stale.publish();
        manager.announce_activity(&stale.id()).unwrap();
        manager.acknowledge_activity(&stale.id(), false).unwrap();
        for _ in 0..102 {
            manager
                .register("Download", "completed", None)
                .finish(Lifecycle::Completed, "Success".into());
        }
        assert_eq!(
            manager
                .list()
                .iter()
                .filter(|op| op.lifecycle.terminal())
                .count(),
            100
        );
        assert!(!manager.path(&stale.id()).unwrap().exists());
        assert!(!manager.activity_was_announced(&stale.id()));
        assert!(!manager.activity_is_read(&stale.id(), false));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn clearing_activity_deletes_finished_records_and_markers_but_keeps_running_work() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let manager = Manager::new(&root);
        let done = manager.register("Install", "done", None);
        done.finish(Lifecycle::Completed, "ok".into());
        let active = manager.register("Build", "active", None);
        active.start();
        let stale = manager.register("Build", "stale", None);
        stale.record.lock().unwrap().heartbeat = now() - 60_000;
        stale.publish();
        for extension in ["notified", "cancel", "read-active", "read-terminal"] {
            files::write_bytes(
                &manager.path(&done.id()).unwrap().with_extension(extension),
                b"marker",
            )
            .unwrap();
        }
        files::write_json(
            &manager.directory.join("activity-cleared.json"),
            &serde_json::json!({"finished": {}}),
        )
        .unwrap();
        manager.clear_activity(&manager.list()).unwrap();
        let reopened = Manager::new(&root);
        let records = reopened.list();
        assert!(records.iter().all(|op| op.id != done.id()));
        assert!(!reopened.path(&done.id()).unwrap().exists());
        for extension in ["notified", "cancel", "read-active", "read-terminal"] {
            assert!(!reopened
                .path(&done.id())
                .unwrap()
                .with_extension(extension)
                .exists());
        }
        assert!(!reopened.directory.join("activity-cleared.json").exists());
        assert!(records.iter().any(|op| op.id == stale.id()));
        assert!(records.iter().any(|op| op.id == active.id()));
        assert!(reopened.activity_is_read(&active.id(), false));
        assert!(reopened.request_cancel(&active.id()).unwrap());
        active.finish(Lifecycle::Completed, "ok".into());
        active.record.lock().unwrap().completed = done.snapshot().completed;
        active.publish();
        assert!(!reopened.activity_is_read(&active.id(), true));
        assert!(reopened.read(&active.id()).is_ok());
        // Late publishers and stale windows cannot recreate a deleted activity.
        done.publish();
        assert!(reopened.read(&done.id()).is_err());
        reopened.announce_activity(&done.id()).unwrap();
        reopened.acknowledge_activity(&done.id(), true).unwrap();
        assert!(!reopened.activity_was_announced(&done.id()));
        assert!(!reopened.activity_is_read(&done.id(), true));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn clearing_activity_protects_active_batches_and_completions_newer_than_the_snapshot() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let manager = Manager::new(&root);
        let parent = manager.register("Batch", "parent", None);
        parent.start();
        let child = manager.register("Install", "child", Some(parent.id()));
        child.finish(Lifecycle::Completed, "ok".into());
        let grandchild = manager.register("Download", "grandchild", Some(child.id()));
        grandchild.finish(Lifecycle::Completed, "ok".into());
        let late = manager.register("Build", "late", None);
        late.start();
        let snapshot = manager.list();
        late.finish(Lifecycle::Completed, "ok".into());
        manager.clear_activity(&snapshot).unwrap();
        for handle in [&parent, &child, &grandchild, &late] {
            assert!(manager.read(&handle.id()).is_ok());
        }
        assert!(!manager.activity_is_read(&late.id(), true));
        parent.finish(Lifecycle::Completed, "ok".into());
        manager.clear_activity(&manager.list()).unwrap();
        assert!(manager.list().is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn clearing_activity_removes_dead_interrupted_work_and_preserves_unrelated_files() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let manager = Manager::new(&root);
        let stopped = manager.register("Build", "stopped", None);
        {
            let mut record = stopped.record.lock().unwrap();
            record.heartbeat = now() - 60_000;
            record.owner = exited_owner();
        }
        stopped.publish();
        let unknown = manager.directory.join("keep.json");
        files::write_bytes(&unknown, b"unrelated").unwrap();
        let snapshot = manager.list();
        assert_eq!(snapshot[0].lifecycle, Lifecycle::Interrupted);
        manager.clear_activity(&snapshot).unwrap();
        assert!(manager.list().is_empty());
        assert!(unknown.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn activity_markers_and_cancellation_do_not_wait_for_the_history_lock() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let manager = Manager::new(&root);
        let active = manager.register("Build", "active", None);
        active.start();
        let lock = manager.activity_lock(true).unwrap();
        let other = manager.clone();
        let id = active.id();
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let result = (|| -> Result<bool> {
                other.announce_activity(&id)?;
                other.acknowledge_activity(&id, false)?;
                other.request_cancel(&id)
            })();
            sender.send(result).unwrap();
        });
        let result = receiver.recv_timeout(std::time::Duration::from_secs(2));
        drop(lock);
        worker.join().unwrap();
        assert!(result.unwrap().unwrap());
        assert!(manager.activity_was_announced(&active.id()));
        assert!(manager.activity_is_read(&active.id(), false));
        assert!(active.cancel_requested());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn operation_snapshots_do_not_wait_for_blocked_activity_publication() {
        use std::sync::{mpsc, TryLockError};
        use std::time::{Duration, Instant};

        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let manager = Manager::new(&root);
        let active = manager.register("Uninstall", "fixture", None);
        active.start();
        let id = active.id();
        // Block durable writes without slowing the filesystem or running an installer.
        let history_lock = manager.activity_lock(true).unwrap();
        let progress = active.clone();
        let writer = std::thread::spawn(move || {
            progress.update(
                &State {
                    stage: "Removing package".into(),
                    ..Default::default()
                },
                false,
            );
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        let writer_started = loop {
            let started = match active.record.try_lock() {
                Ok(record) => record.stage == "Removing package",
                Err(TryLockError::WouldBlock) => true,
                Err(TryLockError::Poisoned(_)) => false,
            };
            if started || Instant::now() >= deadline {
                break started;
            }
            std::thread::yield_now();
        };
        let observe = active.clone();
        let (sender, receiver) = mpsc::channel();
        let reader = std::thread::spawn(move || sender.send(observe.snapshot()).unwrap());
        let snapshot = receiver.recv_timeout(Duration::from_secs(1));
        let finish = active.clone();
        let (finished_sender, finished_receiver) = mpsc::channel();
        let finisher = std::thread::spawn(move || {
            finished_sender
                .send(finish.finish(Lifecycle::Completed, "Success".into()))
                .unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        let finisher_started = loop {
            let started = active
                .record
                .try_lock()
                .is_ok_and(|record| record.lifecycle == Lifecycle::Completed);
            if started || Instant::now() >= deadline {
                break started;
            }
            std::thread::yield_now();
        };
        let premature_finish = finished_receiver.recv_timeout(Duration::from_millis(100));
        // Always release and join before assertions, including on the failing old code.
        drop(history_lock);
        writer.join().unwrap();
        reader.join().unwrap();
        finisher.join().unwrap();
        let completed = manager.read(&id).unwrap();
        std::fs::remove_dir_all(root).unwrap();

        assert!(writer_started, "Progress worker did not reach publication");
        assert!(
            snapshot.is_ok(),
            "Reading UI progress waited for durable activity publication"
        );
        assert_eq!(snapshot.unwrap().stage, "Removing package");
        assert!(finisher_started, "Completion did not reach publication");
        assert!(
            premature_finish.is_err(),
            "Completion returned before its durable write"
        );
        assert!(finished_receiver.recv().unwrap());
        assert_eq!(completed.lifecycle, Lifecycle::Completed);
        assert_eq!(completed.result.as_deref(), Some("Success"));
    }

    #[test]
    fn late_terminal_publication_does_not_recreate_removed_operation_folders() {
        for lifecycle in [
            Lifecycle::Completed,
            Lifecycle::Failed,
            Lifecycle::Cancelled,
        ] {
            let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
            let manager = Manager::new(&root);
            let handle = manager.register("Build", "fixture", None);
            let late_progress = handle.clone();
            assert!(handle.finish(lifecycle, "Finished".into()));
            assert_eq!(manager.read(&handle.id()).unwrap().lifecycle, lifecycle);
            std::fs::remove_dir_all(&root).unwrap();
            // A heartbeat can reach publication after finish has returned.
            late_progress.publish();
            let recreated = root.exists();
            if recreated {
                std::fs::remove_dir_all(&root).unwrap();
            }
            assert!(
                !recreated,
                "Late {lifecycle:?} publication recreated its folder"
            );
        }
    }

    #[test]
    fn failed_terminal_publication_can_retry_after_storage_recovers() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let manager = Manager::new(&root);
        let handle = manager.register("Build", "fixture", None);
        let path = manager.path(&handle.id()).unwrap();
        let original = std::fs::read(&path).unwrap();
        std::fs::remove_dir_all(&manager.directory).unwrap();
        std::fs::write(&manager.directory, b"Blocked directory").unwrap();
        assert!(handle.finish(Lifecycle::Completed, "Success".into()));
        std::fs::remove_file(&manager.directory).unwrap();
        std::fs::create_dir_all(&manager.directory).unwrap();
        std::fs::write(&path, original).unwrap();
        handle.publish();
        assert_eq!(
            manager.read(&handle.id()).unwrap().lifecycle,
            Lifecycle::Completed
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn delayed_terminal_publication_cannot_restore_a_deleted_record() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let manager = Manager::new(&root);
        let done = manager.register("Build", "done", None);
        done.finish(Lifecycle::Completed, "ok".into());
        let id = done.id();
        let lock = manager.activity_lock(true).unwrap();
        let delayed = done.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let publisher = std::thread::spawn(move || {
            sender.send(()).unwrap();
            delayed.publish();
        });
        receiver.recv().unwrap();
        manager.delete_record_locked(&id).unwrap();
        drop(lock);
        publisher.join().unwrap();
        assert!(manager.read(&id).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn clear_reports_a_locked_marker_and_retry_removes_the_orphan() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let manager = Manager::new(&root);
        let done = manager.register("Install", "done", None);
        done.finish(Lifecycle::Completed, "ok".into());
        manager.announce_activity(&done.id()).unwrap();
        let active = manager.register("Build", "active", None);
        active.start();
        let marker = manager.path(&done.id()).unwrap().with_extension("notified");
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&marker)
            .unwrap();
        assert!(manager.clear_activity(&manager.list()).is_err());
        assert!(manager.read(&active.id()).is_ok());
        drop(held);
        manager.clear_activity(&manager.list()).unwrap();
        assert!(!marker.exists());
        assert!(manager.read(&done.id()).is_err());
        assert!(manager.request_cancel(&active.id()).unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn opening_legacy_history_prunes_old_records_but_preserves_active_ancestry_and_unknown_files() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let manager = Manager::new(&root);
        let parent = manager.register("Install", "parent", None);
        parent.finish(Lifecycle::Completed, "ok".into());
        let child = manager.register("Install", "child", Some(parent.id()));
        // Model a long history setup without depending on the runner's disk speed.
        child.record.lock().unwrap().heartbeat = now() - 60_000;
        child.publish();
        let original = parent.snapshot();
        let exited_owner = exited_owner();
        for index in 0..105 {
            let mut legacy = original.clone();
            legacy.id = uuid::Uuid::new_v4().to_string();
            legacy.lifecycle = Lifecycle::Running;
            legacy.completed = None;
            legacy.owner = exited_owner;
            legacy.heartbeat = now() - 60_000 - index;
            let mut json = serde_json::to_value(&legacy).unwrap();
            json.as_object_mut().unwrap().remove("facts");
            files::write_json(&manager.path(&legacy.id).unwrap(), &json).unwrap();
            for extension in ["notified", "cancel", "read-active", "read-terminal"] {
                files::write_bytes(
                    &manager.path(&legacy.id).unwrap().with_extension(extension),
                    b"marker",
                )
                .unwrap();
            }
        }
        let orphan = manager
            .path(&uuid::Uuid::new_v4().to_string())
            .unwrap()
            .with_extension("notified");
        files::write_bytes(&orphan, b"orphan").unwrap();
        let unrelated = manager.directory.join("keep.notified");
        files::write_bytes(&unrelated, b"user file").unwrap();
        // This is an active-operation retention test. Start its simulated worker
        // after fixture writes, which can exceed the real heartbeat timeout.
        assert!(child.start());
        let reopened = Manager::new(&root);
        let records = reopened.list();
        assert_eq!(records.len(), 102); // 100 finished, plus the protected parent and active child.
        assert!(records.iter().any(|op| op.id == parent.id()));
        assert!(records
            .iter()
            .any(|op| op.id == child.id() && !op.lifecycle.terminal()));
        assert!(reopened.request_cancel(&parent.id()).is_ok());
        assert!(!orphan.exists());
        assert!(unrelated.exists());
        assert_eq!(
            std::fs::read_dir(&manager.directory)
                .unwrap()
                .flatten()
                .filter(|entry| entry
                    .path()
                    .extension()
                    .is_some_and(|ext| ext == "notified"))
                .count(),
            101
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn stale_live_owner_and_its_ancestors_survive_retention_and_can_finish() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let manager = Manager::new(&root);
        let parent = manager.register("Install", "parent", None);
        parent.finish(Lifecycle::Completed, "ok".into());
        let worker = manager.register("Build", "worker", Some(parent.id()));
        worker.start();
        worker.record.lock().unwrap().heartbeat = now() - 60_000;
        worker.publish();
        for index in 0..105 {
            let mut old = parent.snapshot();
            old.id = uuid::Uuid::new_v4().to_string();
            old.completed = Some(now() + index);
            files::write_json(&manager.path(&old.id).unwrap(), &old).unwrap();
        }
        let listed = manager.list();
        assert!(listed.iter().any(|op| op.id == worker.id()));
        assert!(manager.read(&worker.id()).is_ok());
        assert!(manager.read(&parent.id()).is_ok());
        worker.update(&State::default(), false);
        assert!(worker.finish(Lifecycle::Completed, "Recovered".into()));
        assert_eq!(
            manager.read(&worker.id()).unwrap().lifecycle,
            Lifecycle::Completed
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn older_window_clear_cannot_resurrect_newer_cleared_completions() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let older_window = Manager::new(&root);
        let newer_window = Manager::new(&root);
        let active = older_window.register("Build", "active", None);
        active.start();
        let interrupted = older_window.register("Build", "interrupted", None);
        interrupted.start();
        interrupted.record.lock().unwrap().heartbeat = now() - 60_000;
        interrupted.publish();
        let older_snapshot = older_window.list();
        assert!(older_snapshot
            .iter()
            .any(|op| op.lifecycle == Lifecycle::Interrupted));
        active.finish(Lifecycle::Completed, "ok".into());
        interrupted.finish(Lifecycle::Completed, "ok".into());
        newer_window.clear_activity(&newer_window.list()).unwrap();
        older_window.clear_activity(&older_snapshot).unwrap();
        assert!(newer_window.list().is_empty());
        assert!(!newer_window.path(&active.id()).unwrap().exists());
        assert!(!newer_window.path(&interrupted.id()).unwrap().exists());
        older_window.clear_activity(&[]).unwrap();
        assert!(!older_window
            .directory
            .join("activity-cleared.json")
            .exists());
        std::fs::remove_dir_all(root).unwrap();
    }
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
