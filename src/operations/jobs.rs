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
#[path = "jobs/tests.rs"]
mod tests;

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
