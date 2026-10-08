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
#[derive(Clone, Default)]
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
pub struct Job {
    pub state: Arc<Mutex<State>>,
    pub cancel: Arc<AtomicBool>,
    pub log_path: PathBuf,
    pub max_bytes: u64,
    pub archives: usize,
}
impl Job {
    pub fn new(log_path: PathBuf, prefs: &crate::model::BuilderPreferences) -> Self {
        Self {
            state: Arc::new(Mutex::new(State::default())),
            cancel: Arc::new(AtomicBool::new(false)),
            log_path,
            max_bytes: prefs.log_size_mb * 1024 * 1024,
            archives: prefs.log_archives,
        }
    }
    pub fn check(&self) -> Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(Cancelled.into());
        }
        Ok(())
    }
    pub fn stage(&self, stage: &str, progress: Option<f32>, detail: impl Into<String>) {
        let mut s = self.state.lock().unwrap();
        s.stage = stage.into();
        s.detail = detail.into();
        s.progress = progress;
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
        self.cancel.store(false, Ordering::Relaxed);
        {
            let mut s = self.state.lock().unwrap();
            s.busy = true;
            s.outcome.clear();
            s.progress = None;
        }
        let job = self.clone();
        std::thread::spawn(move || {
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(job.clone())));
            let (stage, outcome) = match result {
                Ok(Ok(())) => ("Complete", "Success".to_string()),
                Ok(Err(e)) if is_cancelled(&e) => {
                    let message = format!("{e:#}");
                    job.log(&message);
                    ("Cancelled", message)
                }
                Ok(Err(e)) => {
                    let error = format!("{e:#}");
                    job.log(&format!("ERROR: {error}"));
                    ("Failed", error)
                }
                Err(_) => {
                    job.log("ERROR: worker panicked");
                    ("Failed", "Worker failed unexpectedly".into())
                }
            };
            let mut s = job.state.lock().unwrap();
            s.busy = false;
            s.outcome = outcome.clone();
            s.stage = stage.into();
            s.detail = outcome;
            s.progress = if s.stage == "Complete" {
                Some(1.0)
            } else {
                None
            };
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
        let state = job.state.lock().unwrap().clone();
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
