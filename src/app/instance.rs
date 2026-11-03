//! GUI ownership and activation. Headless commands never claim a window slot.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

#[derive(Serialize, Deserialize)]
struct Endpoint {
    port: u16,
    pid: u32,
    nonce: String,
}

pub(super) fn is_gui_launch(args: &[String]) -> bool {
    !args.iter().any(|arg| {
        matches!(
            arg.as_str(),
            "--build-identity"
                | "--native-bridge-admin"
                | "--approved-admin-batch"
                | "--apply-self-update"
                | "--check-app-updates"
                | "--check-source-updates"
                | "--install-shortcuts"
                | "--list-apps"
                | "--install-app"
                | "--uninstall-app"
                | "--update"
                | "--update-source"
                | "--diagnostics"
                | "--release-format"
                | "--architecture"
        ) || (cfg!(target_os = "windows") && arg == "--restore-relocation-files")
    })
}

#[derive(Default)]
struct Activation {
    pending: AtomicBool,
    wake: Mutex<Option<Box<dyn Fn() + Send>>>,
}

pub(crate) struct Instance {
    // Never unlink a lock file: another process may already have it open.
    _lock: File,
    endpoint: std::path::PathBuf,
    address: SocketAddr,
    activation: Arc<Activation>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Instance {
    pub(crate) fn claim(root: &Path, builder: bool) -> Result<Option<Self>> {
        fs::create_dir_all(root)?;
        let directory = fs::canonicalize(root)?.join("runtime/windows");
        fs::create_dir_all(&directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        }
        let name = if builder { "builder" } else { "manager" };
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let lock = options.open(directory.join(format!("{name}.lock")))?;
        let endpoint = directory.join(format!("{name}.json"));
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match lock.try_lock() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock) => {
                    if activate(&endpoint).is_ok() {
                        return Ok(None);
                    }
                    // The owner may still be publishing its endpoint, or exiting.
                    // Retry ownership too, so a dead process cannot block startup.
                    if Instant::now() >= deadline {
                        bail!("The existing {name} window is not responding. Close it before reopening this data folder.");
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
            }
        }
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let address = listener.local_addr()?;
        let nonce = uuid::Uuid::new_v4().to_string();
        let activation = Arc::new(Activation::default());
        let stop = Arc::new(AtomicBool::new(false));
        let state = activation.clone();
        let stopping = stop.clone();
        let expected = nonce.clone();
        let worker = std::thread::Builder::new()
            .name("window-activation".into())
            .spawn(move || {
                for connection in listener.incoming() {
                    if stopping.load(Ordering::Acquire) {
                        break;
                    }
                    let Ok(mut stream) = connection else { break };
                    let timeout = Some(Duration::from_millis(300));
                    if stream.set_read_timeout(timeout).is_err()
                        || stream.set_write_timeout(timeout).is_err()
                    {
                        continue;
                    }
                    let mut request = [0; 36];
                    if stream.read_exact(&mut request).is_ok() && request == expected.as_bytes() {
                        state.pending.store(true, Ordering::Release);
                        if let Some(wake) = state.wake.lock().unwrap().as_ref() {
                            wake();
                        }
                        let _ = stream.write_all(b"active\n");
                    }
                }
            })?;
        let owner = Self {
            _lock: lock,
            endpoint,
            address,
            activation,
            stop,
            worker: Some(worker),
        };
        craft_apps_manager::files::write_json(
            &owner.endpoint,
            &Endpoint {
                port: address.port(),
                pid: std::process::id(),
                nonce,
            },
        )?;
        Ok(Some(owner))
    }

    pub(crate) fn set_wake(&self, wake: impl Fn() + Send + 'static) {
        *self.activation.wake.lock().unwrap() = Some(Box::new(wake));
    }

    pub(crate) fn take_activation(&self) -> bool {
        self.activation.pending.swap(false, Ordering::AcqRel)
    }
}

fn activate(path: &Path) -> Result<()> {
    let mut bytes = Vec::new();
    File::open(path)?.take(1024).read_to_end(&mut bytes)?;
    let endpoint: Endpoint = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(endpoint.nonce.len() == 36, "Invalid window endpoint");
    let mut stream = TcpStream::connect_timeout(
        &(Ipv4Addr::LOCALHOST, endpoint.port).into(),
        Duration::from_millis(300),
    )?;
    let timeout = Some(Duration::from_millis(300));
    stream.set_write_timeout(timeout)?;
    stream.set_read_timeout(timeout)?;
    #[cfg(target_os = "windows")]
    {
        // A newly launched process can grant its foreground permission to the
        // existing window; otherwise Windows may refuse that window's focus call.
        let _ = unsafe {
            windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(endpoint.pid)
        };
    }
    stream.write_all(endpoint.nonce.as_bytes())?;
    let mut response = [0; 7];
    stream
        .read_exact(&mut response)
        .context("Activate existing window")?;
    anyhow::ensure!(&response == b"active\n", "Invalid window response");
    Ok(())
}

impl Drop for Instance {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Wake blocking accept; no polling or persistent filesystem reads.
        let _ = TcpStream::connect_timeout(&self.address, Duration::from_millis(300));
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let _ = fs::remove_file(&self.endpoint);
    }
}

#[cfg(test)]
#[path = "instance_tests.rs"]
mod tests;
