use anyhow::{bail, Result};
use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
#[derive(Debug)]
pub struct Cancelled;
impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Cancelled")
    }
}
impl std::error::Error for Cancelled {}
pub fn is_cancelled(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| cause.is::<Cancelled>())
}
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct State {
    pub busy: bool,
    pub stage: String,
    pub detail: String,
    pub progress: Option<f32>,
    pub log: String,
    pub output: Option<PathBuf>,
    pub outcome: String,
    pub executable: Option<PathBuf>,
    pub units: usize,
}
#[derive(Clone)]
pub struct IdentityChoice {
    pub title: String,
    pub candidates: Vec<String>,
    answer: std::sync::mpsc::Sender<Option<usize>>,
}
impl IdentityChoice {
    pub fn answer(&self, selected: Option<usize>) {
        let _ = self.answer.send(selected);
    }
}
#[derive(Clone)]
pub struct Job {
    interactive: bool,
    choice: Arc<Mutex<Option<IdentityChoice>>>,
    pub state: Arc<Mutex<State>>,
    pub cancel: Arc<AtomicBool>,
    pub log_path: PathBuf,
    pub max_bytes: u64,
    pub archives: usize,
    pub operations: crate::operations::Manager,
    active: Arc<Mutex<Option<crate::operations::Handle>>>,
    descriptor: (String, String),
    parent: Option<String>,
    children: Arc<Mutex<std::collections::BTreeMap<String, Job>>>,
    relay: Option<Arc<Mutex<State>>>,
    executing_thread: Arc<Mutex<Option<std::thread::ThreadId>>>,
}
impl Job {
    pub fn new(log_path: PathBuf, prefs: &crate::model::BuilderPreferences) -> Self {
        let parent = log_path.parent().unwrap_or(Path::new("."));
        let root = if parent
            .file_name()
            .is_some_and(|name| name == "logs" || name == "runtime")
        {
            parent.parent().unwrap_or(parent)
        } else {
            parent
        };
        Self {
            interactive: false,
            choice: Arc::new(Mutex::new(None)),
            operations: crate::operations::Manager::new(root),
            active: Arc::new(Mutex::new(None)),
            descriptor: ("Operation".into(), String::new()),
            parent: None,
            children: Arc::new(Mutex::new(Default::default())),
            relay: None,
            executing_thread: Arc::new(Mutex::new(None)),
            state: Arc::new(Mutex::new(State::default())),
            cancel: Arc::new(AtomicBool::new(false)),
            log_path,
            max_bytes: prefs.log_size_mb * 1024 * 1024,
            archives: prefs.log_archives,
        }
    }
    pub fn interactive(log_path: PathBuf, prefs: &crate::model::BuilderPreferences) -> Self {
        let mut job = Self::new(log_path, prefs);
        job.enable_identity_choices();
        job
    }
    pub fn identity_choices_available(&self) -> bool {
        self.interactive
    }
    pub fn enable_identity_choices(&mut self) {
        self.interactive = true;
    }
    pub fn identity_choice(&self) -> Option<IdentityChoice> {
        self.choice.lock().unwrap().clone()
    }
    pub fn choose_identity(&self, title: &str, candidates: Vec<String>) -> Result<usize> {
        anyhow::ensure!(
            self.interactive,
            "This package needs an executable choice. Install it individually from the manager."
        );
        anyhow::ensure!(
            !candidates.is_empty(),
            "No compatible desktop executable found"
        );
        let count = candidates.len();
        let (tx, rx) = std::sync::mpsc::channel();
        *self.choice.lock().unwrap() = Some(IdentityChoice {
            title: title.into(),
            candidates,
            answer: tx,
        });
        self.stage("Choose application", None, title);
        let result = (|| loop {
            self.check()?;
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Some(index)) if index < count => return Ok(index),
                Ok(_) => return Err(Cancelled.into()),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    bail!("Application choice window closed")
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            }
        })();
        *self.choice.lock().unwrap() = None;
        result
    }
    pub fn describe(&mut self, kind: &str, resource: &str) {
        self.descriptor = (
            match kind {
                "install-app" => "Install / update",
                "uninstall-app" => "Uninstall",
                "source-app" | "sources" => "Source download",
                "build" => "Build",
                "setup" => "Set up tools",
                "remove-source" => "Remove source",
                "clean-build" => "Delete cache",
                "restore" => "Restore backup",
                other => other,
            }
            .into(),
            resource.into(),
        );
    }
    pub fn report(&self, key: &str, value: impl AsRef<str>) {
        if let Some(handle) = self.active.lock().unwrap().as_ref() {
            handle.report(key, value.as_ref());
        }
    }
    pub fn operation_id(&self) -> Option<String> {
        self.active
            .lock()
            .unwrap()
            .as_ref()
            .map(|handle| handle.id())
    }
    pub fn snapshot(&self) -> State {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| {
                self.state.clear_poison();
                poison.into_inner()
            })
            .clone();
        if let Some(handle) = self.active.lock().unwrap().as_ref() {
            let operation = handle.snapshot();
            // A terminal record may still be publishing its final result. Keep the
            // local worker busy until complete() has finished all finalization.
            state.busy |= !operation.lifecycle.terminal();
            if operation.lifecycle.terminal() {
                state.stage = if operation.lifecycle == crate::operations::Lifecycle::Completed {
                    "Complete".into()
                } else {
                    format!("{:?}", operation.lifecycle)
                };
                state.outcome = operation.result.unwrap_or_default();
            }
        }
        state
    }
    pub fn cancellation_available(&self) -> bool {
        self.active.lock().unwrap().as_ref().is_none_or(|handle| {
            let operation = handle.snapshot();
            !operation.lifecycle.terminal() && operation.cancellation_available
        })
    }
    pub fn builder_operation(operation: &crate::operations::Operation, app: &str) -> bool {
        operation.resource == app
            && operation.parent.is_none()
            && !operation.lifecycle.terminal()
            && matches!(
                operation.kind.as_str(),
                "Build" | "Source download" | "Set up tools"
            )
    }
    pub fn latest_builder_operation<'a>(
        operations: &'a [crate::operations::Operation],
        app: &str,
    ) -> Option<&'a crate::operations::Operation> {
        operations
            .iter()
            .filter(|op| {
                op.resource == app
                    && op.parent.is_none()
                    && matches!(
                        op.kind.as_str(),
                        "Build" | "Source download" | "Set up tools"
                    )
            })
            .max_by_key(|op| (!op.lifecycle.terminal(), op.started.unwrap_or(op.heartbeat)))
    }
    /// Observe the selected app, preserving local worker ownership and terminal outcomes.
    pub fn observed_builder_state(
        local: &State,
        local_app: &str,
        operations: &[crate::operations::Operation],
        app: &str,
    ) -> State {
        let mut state = if local_app == app {
            local.clone()
        } else {
            State::default()
        };
        if state.busy {
            return state;
        }
        if let Some(operation) = Self::latest_builder_operation(operations, app) {
            // Own jobs already contain their complete state, including the full log.
            if operation.owner == std::process::id() && local_app == app {
                return state;
            }
            use crate::operations::Lifecycle;
            state.busy = !operation.lifecycle.terminal();
            state.stage = match operation.lifecycle {
                Lifecycle::Completed => "Complete".into(),
                Lifecycle::CancellationRequested => "Cancellation requested".into(),
                _ => operation.stage.clone(),
            };
            state.detail = if operation.lifecycle.terminal() {
                operation
                    .result
                    .clone()
                    .unwrap_or_else(|| operation.detail.clone())
            } else {
                operation.detail.clone()
            };
            state.outcome = operation.result.clone().unwrap_or_default();
            state.progress = operation.progress;
            state.output = operation.facts.get("Output location").map(PathBuf::from);
            state.executable = None;
            state.log.clear();
        }
        state
    }
    pub fn request_cancel(&self) -> bool {
        if let Some(id) = self.operation_id() {
            if !self.operations.request_cancel(&id).unwrap_or(false) {
                return false;
            }
        } else if !self.state.lock().unwrap().busy {
            return false;
        }
        self.cancel.store(true, Ordering::Relaxed);
        true
    }
    fn begin(&self) -> Option<crate::operations::Handle> {
        let mut state = self.state.lock().unwrap();
        if state.busy {
            return None;
        }
        if self
            .active
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|handle| handle.snapshot().lifecycle.terminal())
        {
            self.children.lock().unwrap().clear();
        }
        let handle = self
            .active
            .lock()
            .unwrap()
            .as_ref()
            .filter(|handle| !handle.snapshot().lifecycle.terminal())
            .cloned()
            .unwrap_or_else(|| {
                self.operations.register(
                    &self.descriptor.0,
                    &self.descriptor.1,
                    self.parent.clone(),
                )
            });
        handle.report("Log file", &self.log_path.display().to_string());
        if !handle.start() {
            return None;
        }
        if self.parent.is_none() {
            self.cancel.store(false, Ordering::Relaxed);
        }
        state.busy = true;
        *self.executing_thread.lock().unwrap() = Some(std::thread::current().id());
        state.outcome.clear();
        state.stage = "Preparing".into();
        state.detail.clear();
        state.progress = None;
        state.units = 0;
        if self.descriptor.0.eq_ignore_ascii_case("build") {
            state.output = None;
        }
        state.executable = None;
        *self.active.lock().unwrap() = Some(handle.clone());
        drop(state);
        let job = self.clone();
        let observe = handle.clone();
        std::thread::spawn(move || {
            while !observe.snapshot().lifecycle.terminal() {
                if observe.cancel_requested() {
                    job.cancel.store(true, Ordering::Relaxed);
                }
                let current = job
                    .state
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                let mut state = State {
                    stage: current.stage.clone(),
                    detail: current.detail.clone(),
                    progress: current.progress,
                    ..Default::default()
                };
                drop(current);
                if !job.children.lock().unwrap().is_empty() {
                    state.progress = None;
                }
                observe.update(&state, job.cancel.load(Ordering::Relaxed));
                std::thread::sleep(Duration::from_millis(250));
            }
        });
        Some(handle)
    }
    fn complete(&self, handle: &crate::operations::Handle, result: &Result<impl Sized>) {
        use crate::operations::Lifecycle;
        let (lifecycle, stage, outcome) = match result {
            Ok(_) => (Lifecycle::Completed, "Complete", "Success".to_string()),
            Err(error) if is_cancelled(error) => {
                (Lifecycle::Cancelled, "Cancelled", format!("{error:#}"))
            }
            Err(error) => (Lifecycle::Failed, "Failed", format!("{error:#}")),
        };
        let last = self.snapshot();
        handle.report("Log file", &self.log_path.display().to_string());
        if lifecycle == Lifecycle::Completed {
            if let Some(output) = last.output {
                handle.report("Output location", &output.display().to_string());
            }
        }
        if lifecycle != Lifecycle::Completed && !last.stage.is_empty() {
            handle.report("Last stage", &last.stage);
        }
        handle.finish(lifecycle, outcome.clone());
        for child in self.children.lock().unwrap().values() {
            if let Some(id) = child.operation_id() {
                if let Ok(record) = self.operations.read(&id) {
                    if !record.lifecycle.terminal() {
                        if let Ok(pending) = self.operations.adopt(&id) {
                            let cancelled = lifecycle == Lifecycle::Cancelled
                                && record.lifecycle == Lifecycle::Queued;
                            pending.finish(
                                if cancelled {
                                    Lifecycle::Cancelled
                                } else {
                                    Lifecycle::Interrupted
                                },
                                "Not completed by the batch. Inspect its status before retrying."
                                    .into(),
                            );
                        }
                    }
                }
            }
        }
        let mut state = self.state.lock().unwrap();
        state.busy = false;
        state.outcome = outcome.clone();
        state.stage = stage.into();
        state.detail = outcome;
        state.progress = if lifecycle == Lifecycle::Completed {
            Some(1.0)
        } else {
            None
        };
    }
    /// Synchronous/headless work has the same lifecycle as spawned GUI work.
    pub fn tracked<T>(
        &self,
        kind: &str,
        resource: &str,
        work: impl FnOnce(&Job) -> Result<T>,
    ) -> Result<T> {
        if self.state.lock().unwrap().busy {
            anyhow::ensure!(
                *self.executing_thread.lock().unwrap() == Some(std::thread::current().id()),
                "Operation is already running in another worker"
            );
            return work(self);
        }
        let mut job = self.clone();
        job.describe(kind, resource);
        let handle = job
            .begin()
            .ok_or_else(|| anyhow::anyhow!("Operation is already running"))?;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&job)))
            .unwrap_or_else(|_| Err(anyhow::anyhow!("Worker failed unexpectedly")));
        job.state.clear_poison();
        job.complete(&handle, &result);
        result
    }
    pub fn prepare_child(&self, app: &str, kind: &str, format: &str, architecture: &str) -> Job {
        let mut children = self.children.lock().unwrap();
        if children.get(app).is_some_and(|child| {
            child
                .active
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|handle| handle.snapshot().lifecycle.terminal())
        }) {
            children.remove(app);
        }
        children
            .entry(app.into())
            .or_insert_with(|| {
                let handle = self.operations.register(
                    kind,
                    app,
                    self.operation_id().or_else(|| self.parent.clone()),
                );
                handle.options(format, architecture);
                let mut child = Job::new(
                    self.log_path.clone(),
                    &crate::model::BuilderPreferences::default(),
                );
                child.max_bytes = self.max_bytes;
                child.archives = self.archives;
                child.operations = self.operations.clone();
                child.descriptor = (kind.into(), app.into());
                child.parent = self.operation_id().or_else(|| self.parent.clone());
                child.active = Arc::new(Mutex::new(Some(handle)));
                child.cancel = self.cancel.clone();
                child.interactive = self.interactive;
                child.choice = self.choice.clone();
                child.relay = Some(self.state.clone());
                child
            })
            .clone()
    }
    pub fn child_ids(&self) -> std::collections::BTreeMap<String, String> {
        self.children
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(app, child)| Some((app.clone(), child.operation_id()?)))
            .collect()
    }
    pub fn attach_children(
        &mut self,
        root: &Path,
        parent: Option<String>,
        ids: &std::collections::BTreeMap<String, String>,
    ) -> Result<()> {
        self.operations = crate::operations::Manager::new(root);
        self.parent = parent;
        for (app, id) in ids {
            let handle = self.operations.adopt(id)?;
            anyhow::ensure!(
                handle.snapshot().resource == *app && handle.snapshot().parent == self.parent,
                "Batch operation identity changed"
            );
            let mut child = Job::new(
                self.log_path.clone(),
                &crate::model::BuilderPreferences::default(),
            );
            child.operations = self.operations.clone();
            child.parent = self.parent.clone();
            child.cancel = self.cancel.clone();
            child.relay = Some(self.state.clone());
            child.descriptor = (handle.snapshot().kind, app.clone());
            child.active = Arc::new(Mutex::new(Some(handle)));
            self.children.lock().unwrap().insert(app.clone(), child);
        }
        Ok(())
    }
    pub fn check(&self) -> Result<()> {
        if self
            .active
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|handle| handle.cancel_requested())
        {
            self.cancel.store(true, Ordering::Relaxed);
        }
        if self.cancel.load(Ordering::Relaxed) {
            return Err(Cancelled.into());
        }
        Ok(())
    }
    pub fn stage(&self, stage: &str, progress: Option<f32>, detail: impl Into<String>) {
        if self
            .active
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|handle| handle.snapshot().lifecycle.terminal())
        {
            return;
        }
        let detail = detail.into();
        let mut s = self.state.lock().unwrap();
        s.stage = stage.into();
        s.detail = detail.clone();
        s.progress = progress;
        drop(s);
        if let Some(parent) = &self.relay {
            let mut state = parent.lock().unwrap();
            state.stage = stage.into();
            state.detail = format!("{} · {detail}", self.descriptor.1);
            state.progress = None; // A child's percentage is not overall batch progress.
        }
    }
    pub fn log(&self, line: &str) {
        static ANSI: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let regex = ANSI.get_or_init(|| {
            regex::Regex::new(r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b\[[0-?]*[ -/]*[@-~]").unwrap()
        });
        let line = regex.replace_all(line, "");
        let line = line.replace('\x07', "");
        {
            let mut s = self.state.lock().unwrap();
            s.log.push_str(&line);
            s.log.push('\n');
            if s.log.len() > 200_000 {
                let mut cut = s.log.len() - 200_000;
                while !s.log.is_char_boundary(cut) {
                    cut += 1;
                }
                s.log.drain(..cut);
            }
        }
        if let Err(e) = self.write_log(&line) {
            let mut s = self.state.lock().unwrap();
            s.detail = format!("Log write warning: {e}");
        }
    }
    fn write_log(&self, line: &str) -> Result<()> {
        fs::create_dir_all(self.log_path.parent().unwrap())?;
        if fs::metadata(&self.log_path).map(|m| m.len()).unwrap_or(0) + line.len() as u64 + 2
            > self.max_bytes
        {
            for i in (1..=self.archives).rev() {
                let dest = self.log_path.with_extension(format!("log.{i}"));
                if dest.exists() {
                    fs::remove_file(&dest)?;
                }
                let from = if i == 1 {
                    self.log_path.clone()
                } else {
                    self.log_path.with_extension(format!("log.{}", i - 1))
                };
                if from.exists() {
                    fs::copy(&from, &dest)?;
                }
            }
            OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&self.log_path)?;
        }
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)?;
        writeln!(f, "{line}")?;
        Ok(())
    }
    fn line(&self, line: &str, cargo: bool) {
        if cargo && line.starts_with('{') {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                match v["reason"].as_str() {
                    Some("compiler-artifact") => {
                        let mut s = self.state.lock().unwrap();
                        s.units += 1;
                        s.detail = format!("{} compilation steps completed", s.units);
                        if let Some(exe) = v["executable"].as_str() {
                            s.executable = Some(exe.into());
                        }
                        drop(s);
                        self.log(&format!(
                            "{} {}",
                            if v["fresh"].as_bool() == Some(true) {
                                "Cached"
                            } else {
                                "Compiled"
                            },
                            v["target"]["name"].as_str().unwrap_or("crate")
                        ));
                        return;
                    }
                    Some("compiler-message") => {
                        if let Some(rendered) = v["message"]["rendered"].as_str() {
                            self.log(rendered);
                        }
                        return;
                    }
                    Some("build-script-executed") => {
                        self.log(&format!(
                            "Build script completed: {}",
                            v["package_id"].as_str().unwrap_or("crate")
                        ));
                        return;
                    }
                    Some("build-finished") => {
                        self.log(&format!("Cargo finished; success={}", v["success"]));
                        return;
                    }
                    _ => {}
                }
            }
        }
        self.log(line);
    }
    pub fn run(&self, cmd: &mut Command, cargo: bool) -> Result<()> {
        self.check()?;
        self.log(&format!(
            "Running: {} {}",
            cmd.get_program().to_string_lossy(),
            cmd.get_args()
                .map(|s| s.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
        ));
        crate::platform::hidden(cmd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn()?;
        let group = match crate::platform::ProcessGroup::attach(&child) {
            Ok(g) => g,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let a = tx.clone();
        let t1 = std::thread::spawn(move || {
            for line in BufReader::new(stdout).split(b'\n') {
                match line {
                    Ok(b) => {
                        if a.send(
                            String::from_utf8_lossy(&b)
                                .trim_end_matches('\r')
                                .to_string(),
                        )
                        .is_err()
                        {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        let t2 = std::thread::spawn(move || {
            for line in BufReader::new(stderr).split(b'\n') {
                match line {
                    Ok(b) => {
                        if tx
                            .send(
                                String::from_utf8_lossy(&b)
                                    .trim_end_matches('\r')
                                    .to_string(),
                            )
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        let code = loop {
            for line in rx.try_iter() {
                self.line(&line, cargo);
            }
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if self.cancel.load(Ordering::Relaxed) {
                drop(group);
                let _ = child.wait();
                let _ = t1.join();
                let _ = t2.join();
                for line in rx.try_iter() {
                    self.line(&line, cargo);
                }
                return Err(Cancelled.into());
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        drop(group);
        let _ = t1.join();
        let _ = t2.join();
        for line in rx.try_iter() {
            self.line(&line, cargo);
        }
        if !code.success() {
            bail!("Command failed ({code}). See the log.");
        }
        Ok(())
    }
    pub fn spawn(&self, work: impl FnOnce(Job) -> Result<()> + Send + 'static) {
        let Some(handle) = self.begin() else {
            return;
        };
        let mut job = self.clone();
        job.active = Arc::new(Mutex::new(Some(handle.clone())));
        std::thread::spawn(move || {
            *job.executing_thread.lock().unwrap() = Some(std::thread::current().id());
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(job.clone())));
            let result =
                result.unwrap_or_else(|_| Err(anyhow::anyhow!("Worker failed unexpectedly")));
            job.choice
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .take();
            job.state.clear_poison();
            if let Err(error) = &result {
                job.log(&format!(
                    "{}: {error:#}",
                    if is_cancelled(error) {
                        "Cancelled"
                    } else {
                        "ERROR"
                    }
                ));
            }
            job.complete(&handle, &result);
        });
    }
    pub fn history(&self, path: &Path) {
        if let Ok(bytes) = fs::read(path) {
            let start = bytes.len().saturating_sub(200_000);
            self.state.lock().unwrap().log = String::from_utf8_lossy(&bytes[start..]).into_owned();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restarted_manager_recovers_builder_progress_log_and_cancellation() {
        let root =
            std::env::temp_dir().join(format!("craft-build-restart-{}", uuid::Uuid::new_v4()));
        let producer = crate::operations::Manager::new(&root);
        let operation = producer.register("Build", "filmcraft", None);
        operation.start();
        let log = root.join("logs/filmcraft.log");
        crate::files::write_bytes(&log, b"Compiling filmcraft\n").unwrap();
        operation.report("Log file", &log.display().to_string());
        operation.update(
            &State {
                stage: "Compiling".into(),
                detail: "Building dependencies".into(),
                progress: Some(0.6),
                ..Default::default()
            },
            false,
        );
        // Persist a separate process owner; the reopened manager has no worker state.
        let mut record = operation.snapshot();
        record.owner = u32::MAX;
        crate::files::write_json(
            &root.join(format!("runtime/operations/{}.json", record.id)),
            &record,
        )
        .unwrap();
        for _ in 0..2 {
            let reopened = crate::operations::Manager::new(&root);
            let records = reopened.list();
            let active = records
                .iter()
                .find(|op| Job::builder_operation(op, &op.resource))
                .unwrap();
            assert_eq!(active.resource, "filmcraft");
            let observed =
                Job::observed_builder_state(&State::default(), "photocraft", &records, "filmcraft");
            assert!(observed.busy);
            assert_eq!(observed.stage, "Compiling");
            assert_eq!(observed.progress, Some(0.6));
            assert_eq!(observed.detail, "Building dependencies");
            assert!(fs::read_to_string(active.facts.get("Log file").unwrap())
                .unwrap()
                .contains("Compiling filmcraft"));
            assert!(reopened.request_cancel(&active.id).unwrap());
            assert!(operation.cancel_requested());
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shared_build_observer_tracks_parent_after_source_child_completes() {
        use crate::operations::Lifecycle;
        let root =
            std::env::temp_dir().join(format!("craft-parent-progress-{}", uuid::Uuid::new_v4()));
        let manager = crate::operations::Manager::new(&root);
        let parent = manager.register("Build", "filmcraft", None);
        parent.start();
        parent.update(
            &State {
                stage: "Compiling".into(),
                progress: Some(0.3),
                ..Default::default()
            },
            false,
        );
        let child = manager.register("Source download", "filmcraft", Some(parent.id()));
        child.start();
        child.finish(Lifecycle::Completed, "Success".into());
        let mut parent_record = parent.snapshot();
        parent_record.owner = u32::MAX;
        let mut child_record = child.snapshot();
        child_record.started = Some(parent_record.started.unwrap() + 1);
        let records = vec![child_record, parent_record.clone()];
        assert_eq!(
            Job::latest_builder_operation(&records, "filmcraft")
                .unwrap()
                .id,
            parent.id()
        );
        let observed =
            Job::observed_builder_state(&State::default(), "filmcraft", &records, "filmcraft");
        assert!(observed.busy);
        assert_eq!(observed.stage, "Compiling");
        assert_eq!(observed.progress, Some(0.3));
        let cancellable = records
            .iter()
            .find(|op| Job::builder_operation(op, "filmcraft"))
            .unwrap();
        assert_eq!(cancellable.id, parent.id());
        assert!(manager.request_cancel(&cancellable.id).unwrap());
        assert!(parent.cancel_requested());
        parent_record.lifecycle = Lifecycle::Completed;
        parent_record.stage = "Completed".into();
        parent_record.result = Some("Build succeeded".into());
        let finished = Job::observed_builder_state(
            &State::default(),
            "filmcraft",
            &[records[0].clone(), parent_record],
            "filmcraft",
        );
        assert!(!finished.busy);
        assert_eq!(finished.outcome, "Build succeeded");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shared_build_terminal_outcomes_replace_stale_state_and_isolate_apps() {
        use crate::operations::Lifecycle;
        let root =
            std::env::temp_dir().join(format!("craft-build-outcomes-{}", uuid::Uuid::new_v4()));
        let manager = crate::operations::Manager::new(&root);
        let operation = manager.register("Build", "filmcraft", None);
        operation.start();
        operation.update(
            &State {
                stage: "Compiling".into(),
                progress: Some(0.4),
                ..Default::default()
            },
            false,
        );
        let mut record = operation.snapshot();
        record.owner = u32::MAX;
        let unrelated = State {
            busy: true,
            stage: "Other app".into(),
            log: "Unrelated output".into(),
            ..Default::default()
        };
        let observed =
            Job::observed_builder_state(&unrelated, "photocraft", &[record.clone()], "filmcraft");
        assert_eq!(observed.stage, "Compiling");
        assert_eq!(observed.progress, Some(0.4));
        assert!(observed.log.is_empty());
        for (lifecycle, stage) in [
            (Lifecycle::Completed, "Complete"),
            (Lifecycle::Failed, "Failed"),
            (Lifecycle::Cancelled, "Cancelled"),
            (Lifecycle::Interrupted, "Interrupted"),
        ] {
            record.lifecycle = lifecycle;
            record.stage = format!("{lifecycle:?}");
            record.result = Some(format!("Result: {stage}"));
            record.progress = if lifecycle == Lifecycle::Completed {
                Some(1.0)
            } else {
                None
            };
            let stale = State {
                stage: "Old result".into(),
                log: "Old output".into(),
                output: Some(root.join("old")),
                ..Default::default()
            };
            let observed =
                Job::observed_builder_state(&stale, "filmcraft", &[record.clone()], "filmcraft");
            assert!(!observed.busy);
            assert_eq!(observed.stage, stage);
            assert_eq!(observed.outcome, format!("Result: {stage}"));
            assert_eq!(observed.detail, observed.outcome);
            assert!(observed.output.is_none());
            assert!(observed.log.is_empty());
        }
        assert_eq!(
            Job::observed_builder_state(&unrelated, "filmcraft", &[record], "filmcraft").stage,
            "Other app"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn running_build_publishes_existing_log_before_more_output() {
        let root = std::env::temp_dir().join(format!("craft-live-log-{}", uuid::Uuid::new_v4()));
        let path = root.join("logs/build.log");
        let job = Job::new(path.clone(), &Default::default());
        job.tracked("Build", "filmcraft", |worker| {
            worker.log("Compiling existing dependency");
            // Opening the other window requires no further log writes or completion.
            let consumer = crate::operations::Manager::new(&root);
            let operation = consumer
                .list()
                .into_iter()
                .find(|op| op.resource == "filmcraft")
                .unwrap();
            assert!(!operation.lifecycle.terminal());
            let published = operation.facts.get("Log file").unwrap();
            assert_eq!(Path::new(published), path);
            assert!(fs::read_to_string(published)
                .unwrap()
                .contains("Compiling existing dependency"));
            Ok(())
        })
        .unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shared_builder_state_is_observed_without_taking_worker_ownership() {
        let root =
            std::env::temp_dir().join(format!("craft-build-observer-{}", uuid::Uuid::new_v4()));
        let producer = crate::operations::Manager::new(&root);
        let operation = producer.register("Build", "filmcraft", None);
        operation.start();
        operation.update(
            &State {
                busy: true,
                stage: "Compiling".into(),
                detail: "12 compilation steps".into(),
                progress: Some(0.25),
                ..Default::default()
            },
            false,
        );
        // A separate manager instance reads the shared files, like either window.
        let consumer = crate::operations::Manager::new(&root);
        let mut records = consumer.list();
        for record in &mut records {
            record.owner = u32::MAX;
        }
        let observed =
            Job::observed_builder_state(&State::default(), "filmcraft", &records, "filmcraft");
        assert!(observed.busy);
        assert_eq!(observed.stage, "Compiling");
        assert_eq!(observed.progress, Some(0.25));
        assert!(
            !Job::observed_builder_state(&State::default(), "filmcraft", &records, "photocraft")
                .busy
        );
        let local = State {
            busy: true,
            stage: "Local worker".into(),
            ..Default::default()
        };
        assert_eq!(
            Job::observed_builder_state(&local, "filmcraft", &records, "filmcraft").stage,
            "Local worker"
        );
        assert!(consumer.request_cancel(&operation.id()).unwrap());
        assert!(operation.cancel_requested());
        operation.update(&State::default(), true);
        let mut cancelled_records = consumer.list();
        for record in &mut cancelled_records {
            record.owner = u32::MAX;
        }
        assert_eq!(
            Job::observed_builder_state(
                &State::default(),
                "filmcraft",
                &cancelled_records,
                "filmcraft"
            )
            .stage,
            "Cancellation requested"
        );
        operation.finish(crate::operations::Lifecycle::Cancelled, "Cancelled".into());
        assert!(
            !Job::observed_builder_state(
                &State::default(),
                "filmcraft",
                &consumer.list(),
                "filmcraft"
            )
            .busy
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reused_job_clears_cancellation_and_temporary_executable() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let job = Job::new(root.join("job.log"), &Default::default());
        job.tracked("Build", "first", |worker| {
            worker.state.lock().unwrap().output = Some(root.join("old-build"));
            worker.state.lock().unwrap().executable = Some(root.join("old.exe"));
            worker.cancel.store(true, Ordering::Relaxed);
            Err::<(), _>(Cancelled.into())
        })
        .unwrap_err();
        job.tracked("Download", "second", |worker| {
            worker.check()?;
            assert_eq!(worker.snapshot().output, Some(root.join("old-build")));
            assert!(worker.snapshot().executable.is_none());
            Ok(())
        })
        .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn tool_setup_preserves_previous_build_but_new_build_clears_it() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let job = Job::new(root.join("job.log"), &Default::default());
        job.state.lock().unwrap().output = Some(root.join("previous-build"));
        job.tracked("setup", "fixture", |worker| {
            assert_eq!(worker.snapshot().output, Some(root.join("previous-build")));
            Ok(())
        })
        .unwrap();
        job.tracked("build", "fixture", |worker| {
            assert!(worker.snapshot().output.is_none());
            Ok(())
        })
        .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn attached_batch_child_preserves_shared_cancellation_on_start() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let manager = crate::operations::Manager::new(&root);
        let parent = manager.register("Bulk update", "fixture", None);
        parent.start();
        let handle = manager.register("Install", "fixture", Some(parent.id()));
        let ids = std::collections::BTreeMap::from([("fixture".into(), handle.id())]);
        let mut job = Job::new(root.join("job.log"), &Default::default());
        job.attach_children(&root, Some(parent.id()), &ids).unwrap();
        let child = job.prepare_child("fixture", "Install", "installer", "x64");
        // Simulate cancellation arriving after the batch check but before child startup.
        job.cancel.store(true, Ordering::Relaxed);
        let error = child
            .tracked("Install", "fixture", |worker| worker.check())
            .unwrap_err();
        assert!(is_cancelled(&error));
        assert!(job.cancel.load(Ordering::Relaxed));
        assert_eq!(
            manager.read(&handle.id()).unwrap().lifecycle,
            crate::operations::Lifecycle::Cancelled
        );
        parent.finish(crate::operations::Lifecycle::Cancelled, "Cancelled".into());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn terminal_record_does_not_release_a_worker_before_finalization() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let job = Job::new(root.join("job.log"), &Default::default());
        let handle = job.operations.register("Build", "fixture", None);
        handle.start();
        *job.active.lock().unwrap() = Some(handle.clone());
        job.state.lock().unwrap().busy = true;
        handle.finish(crate::operations::Lifecycle::Completed, "Success".into());
        assert!(job.snapshot().busy);
        job.state.lock().unwrap().busy = false;
        assert!(!job.snapshot().busy);
        std::fs::remove_dir_all(root).unwrap();
    }
    fn finish(work: impl FnOnce(Job) -> Result<()> + Send + 'static) -> State {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let job = Job::new(
            root.join("job.log"),
            &crate::model::BuilderPreferences::default(),
        );
        job.spawn(work);
        for _ in 0..100 {
            if !job.state.lock().unwrap().busy {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let state = job
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        assert!(!state.busy);
        let _ = fs::remove_dir_all(root);
        state
    }
    #[test]
    fn cancellation_during_transfer_removes_partial_and_preserves_typed_marker() {
        use std::{io::Read, net::TcpListener};
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let job = Job::new(
            root.join("job.log"),
            &crate::model::BuilderPreferences::default(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/package", listener.local_addr().unwrap());
        let cancel = job.cancel.clone();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = [0; 4096];
            let _ = socket.read(&mut request).unwrap();
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 131072\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
            let _ = socket.write_all(&[1; 65536]);
            cancel.store(true, Ordering::Relaxed);
        });
        let destination = root.join("downloads/package");
        let error = crate::network::Network::new(&root)
            .unwrap()
            .download(&url, &destination, &job)
            .unwrap_err();
        server.join().unwrap();
        assert!(is_cancelled(&error));
        assert!(!destination.exists());
        assert_eq!(
            fs::read_dir(destination.parent().unwrap()).unwrap().count(),
            0
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cancellation_is_typed_through_context_and_does_not_log_error() {
        let state = finish(|job| {
            job.cancel.store(true, Ordering::Relaxed);
            job.check().map_err(|e| e.context("Download stopped"))
        });
        assert_eq!(state.stage, "Cancelled");
        assert!(!state.log.contains("ERROR"));
    }
    #[test]
    fn late_cancel_preserves_success_failure_and_panic() {
        let success = finish(|job| {
            job.cancel.store(true, Ordering::Relaxed);
            Ok(())
        });
        assert_eq!(success.stage, "Complete");
        let failure = finish(|job| {
            job.cancel.store(true, Ordering::Relaxed);
            bail!("Real disk error")
        });
        assert_eq!(failure.stage, "Failed");
        assert!(failure.log.contains("Real disk error"));
        let panic = finish(|job| {
            job.cancel.store(true, Ordering::Relaxed);
            panic!("worker panic");
        });
        assert_eq!(panic.stage, "Failed");
    }
}

#[cfg(test)]
mod identity_choice_tests {
    use super::*;
    #[test]
    fn headless_identity_choice_never_waits_for_a_window() {
        let job = Job::new(
            std::env::temp_dir().join("identity-choice-unused.log"),
            &Default::default(),
        );
        assert!(job
            .choose_identity("Choose", vec!["app.exe".into()])
            .is_err());
        assert!(job.identity_choice().is_none());
    }
    #[test]
    fn identity_choice_handles_selection_and_cancellation() {
        let root = std::env::temp_dir().join(format!("craft-choice-{}", uuid::Uuid::new_v4()));
        let mut job = Job::new(root.join("logs/test.log"), &Default::default());
        job.enable_identity_choices();
        for answer in [Some(1), None] {
            let worker = job.clone();
            let thread = std::thread::spawn(move || {
                worker.choose_identity("Choose", vec!["first.exe".into(), "second.exe".into()])
            });
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(choice) = job.identity_choice() {
                    choice.answer(answer);
                    break;
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            let result = thread.join().unwrap();
            match answer {
                Some(index) => assert_eq!(result.unwrap(), index),
                None => assert!(is_cancelled(&result.unwrap_err())),
            }
            assert!(job.identity_choice().is_none());
        }
        if root.exists() {
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}
