//! One PolicyKit approval, scoped to immutable copies of the approved packages.
//! Only package-manager commands run as root; configuration and jobs remain user-owned.
use crate::{
    files,
    jobs::Job,
    model::Paths,
    updates::{self, ReleasePlan},
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    io::{Read, Write},
    os::unix::{
        fs::{DirBuilderExt, OpenOptionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};

#[derive(Serialize, Deserialize)]
struct Package {
    path: PathBuf,
    checksum: String,
}
#[derive(Serialize, Deserialize)]
struct Request {
    uid: u32,
    socket: PathBuf,
    packages: Vec<Package>,
}
#[derive(Serialize, Deserialize)]
struct Reply {
    code: Option<i32>,
    error: Option<String>,
}
struct Client {
    stream: UnixStream,
    packages: Vec<Package>,
    consumed: std::collections::HashSet<usize>,
}
thread_local! { static CLIENT: RefCell<Option<Client>> = const { RefCell::new(None) }; }
pub(crate) fn active() -> bool {
    CLIENT.with(|cell| cell.borrow().is_some())
}

pub fn eligible(plan: &ReleasePlan) -> bool {
    plan.preferences.batch_admin
        && plan.requested_tag.is_none()
        && plan
            .entries
            .iter()
            .filter(|e| {
                e.preferences.release_format == "installer"
                    && matches!(e.action.as_str(), "Install" | "Update")
                    && e.error.is_none()
            })
            .count()
            > 1
}
fn peer_uid(stream: &UnixStream) -> Result<u32> {
    use std::os::fd::AsRawFd;
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut length,
        )
    };
    ensure!(result == 0, "Cannot authenticate the batch connection");
    Ok(credentials.uid)
}
fn receive<T: serde::de::DeserializeOwned>(
    stream: &mut UnixStream,
    job: Option<&Job>,
) -> Result<T> {
    let mut bytes = Vec::new();
    loop {
        let mut byte = [0];
        match stream.read(&mut byte) {
            Ok(0) => anyhow::bail!("Administrator batch helper disconnected; review installed applications before retrying"),
            Ok(_) if byte[0] == b'\n' => return Ok(serde_json::from_slice(&bytes)?),
            Ok(_) => { bytes.push(byte[0]); ensure!(bytes.len() <= 4096, "Invalid batch response"); }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) => {
                // Do not kill a package transaction. Cancellation takes effect between packages.
                if let Some(job) = job {
                    if job.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                        job.stage("Finishing package transaction", None, "Cancellation requested. Remaining packages will not run.");
                    }
                }
            }
            Err(e) => return Err(e.into()),
        }
    }
}
fn send<T: Serialize>(stream: &mut UnixStream, value: &T) -> Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    stream.write_all(&bytes)?;
    Ok(())
}
pub(crate) fn install_package(
    file: &Path,
    downgrade: bool,
    job: Option<&Job>,
) -> Option<Result<i32>> {
    CLIENT.with(|cell| {
        let mut client = cell.borrow_mut();
        let client = client.as_mut()?;
        Some((|| {
            ensure!(
                !downgrade,
                "An approved update batch cannot downgrade packages"
            );
            if let Some(job) = job {
                job.check()?;
            }
            let index = client
                .packages
                .iter()
                .position(|p| p.path == file)
                .context("Package is outside the approved batch")?;
            ensure!(
                client.consumed.insert(index),
                "Package was already executed in this batch"
            );
            send(&mut client.stream, &index)?;
            let reply: Reply = receive(&mut client.stream, job)?;
            ensure!(reply.error.is_none(), "{}", reply.error.unwrap_or_default());
            reply
                .code
                .context("Package manager exited without a result")
        })())
    })
}
struct Scope;
impl Drop for Scope {
    fn drop(&mut self) {
        CLIENT.with(|cell| {
            cell.borrow_mut().take();
        });
    }
}
fn await_helper(listener: &UnixListener, child: &mut Child, job: &Job) -> Result<UnixStream> {
    listener.set_nonblocking(true)?;
    loop {
        if job.check().is_err() {
            // No package request has been sent, so terminating this approval process is safe.
            let _ = child.kill();
            let _ = child.wait();
            return Err(crate::jobs::Cancelled.into());
        }
        match listener.accept() {
            Ok((stream, _)) => {
                ensure!(peer_uid(&stream)? == 0, "Batch helper is not root");
                return Ok(stream);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e.into()),
        }
        if let Some(status) = child.try_wait()? {
            anyhow::bail!("Administrator approval was declined or unavailable ({status}). Open the manager from your Linux desktop, not an SSH session. See the batch authorization log.");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
pub fn execute(paths: &Paths, plan: &ReleasePlan, job: &Job) -> Result<()> {
    ensure!(
        eligible(plan),
        "Batch approval requires multiple installer operations"
    );
    let network = crate::network::Network::new(&paths.root)?;
    let mut packages = Vec::new();
    for entry in plan.entries.iter().filter(|e| {
        e.preferences.release_format == "installer"
            && matches!(e.action.as_str(), "Install" | "Update")
            && e.error.is_none()
    }) {
        job.check()?;
        let asset = entry.asset.as_ref().context("Missing approved package")?;
        files::safe_relative(&asset.name)?;
        crate::model::valid_app(&entry.app)?;
        let path = paths.at(format!("releases/installers/{}/{}", entry.app, asset.name));
        if !path.exists() {
            network.asset(asset, &path, job)?;
        }
        crate::network::verify_asset(&path, asset)?;
        packages.push(Package {
            path: path.canonicalize()?,
            checksum: files::hash(&path)?,
        });
    }
    // Keep the socket path short and private; long library paths exceed Unix socket limits.
    let directory = std::env::temp_dir().join(format!("craft-batch-{}", uuid::Uuid::new_v4()));
    std::fs::DirBuilder::new().mode(0o700).create(&directory)?;
    let manifest = directory.join("approved.json");
    let request = Request {
        uid: unsafe { libc::getuid() },
        socket: directory.join("worker.sock"),
        packages,
    };
    let bytes = serde_json::to_vec(&request)?;
    std::fs::write(&manifest, &bytes)?;
    let checksum = format!("{:x}", Sha256::digest(&bytes));
    let log_dir = paths.at(format!("runtime/admin-batches/{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&log_dir)?;
    let log = std::fs::File::create(log_dir.join("authorization.log"))?;
    let listener = UnixListener::bind(&request.socket)?;
    job.stage(
        "Awaiting administrator approval",
        None,
        "Approve access once for this installer batch",
    );
    let mut child = Command::new("pkexec")
        .arg("--disable-internal-agent")
        .arg(std::env::current_exe()?)
        .arg("--approved-admin-batch")
        .arg(&manifest)
        .arg(checksum)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()?;
    let result = (|| {
        let mut stream = await_helper(&listener, &mut child, job)?;
        stream.set_read_timeout(Some(Duration::from_millis(150)))?;
        let ready: Reply = receive(&mut stream, Some(job))?;
        ensure!(ready.error.is_none(), "{}", ready.error.unwrap_or_default());
        CLIENT.with(|cell| {
            *cell.borrow_mut() = Some(Client {
                stream,
                packages: request.packages,
                consumed: Default::default(),
            });
        });
        let _scope = Scope;
        // Existing execution retains locks, configuration, detection and child-operation results.
        updates::execute_plan_direct(paths, plan, job)
    })();
    // The socket closes after the last transaction; never terminate a live package manager.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    drop(listener);
    let _ = std::fs::remove_file(&request.socket);
    let _ = std::fs::remove_file(&manifest);
    let _ = std::fs::remove_dir(&directory);
    result
}
struct PrivatePackages(PathBuf);
impl Drop for PrivatePackages {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn snapshot_packages(request: &Request, destination: &Path) -> Result<Vec<PathBuf>> {
    ensure!(
        (2..=100).contains(&request.packages.len()),
        "Invalid approved package count"
    );
    let extension = crate::installers::installer_extension()?;
    let mut copies = Vec::new();
    for (index, package) in request.packages.iter().enumerate() {
        ensure!(
            package.path.is_absolute() && package.path.to_string_lossy().ends_with(extension),
            "Invalid package path or format"
        );
        let mut source = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&package.path)?;
        ensure!(
            source.metadata()?.is_file(),
            "Approved package is not a regular file"
        );
        let copy = destination.join(format!("package-{index}{extension}"));
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&copy)?;
        std::io::copy(&mut source, &mut output)?;
        output.sync_all()?;
        ensure!(
            files::hash(&copy)? == package.checksum,
            "Approved package changed; review the batch again"
        );
        copies.push(copy);
    }
    Ok(copies)
}
pub fn worker(manifest: &Path, checksum: &str) -> Result<()> {
    let mut bytes = Vec::new();
    std::fs::File::open(manifest)?
        .take(128 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 128 * 1024 && format!("{:x}", Sha256::digest(&bytes)) == checksum,
        "The approved batch changed; review it again"
    );
    ensure!(
        unsafe { libc::geteuid() } == 0,
        "The batch helper requires administrator approval"
    );
    let request: Request = serde_json::from_slice(&bytes)?;
    let caller: u32 = std::env::var("PKEXEC_UID")
        .context("Missing PolicyKit caller")?
        .parse()?;
    ensure!(
        caller != 0 && caller == request.uid,
        "Batch caller identity mismatch"
    );
    let mut stream = UnixStream::connect(&request.socket)?;
    ensure!(
        peer_uid(&stream)? == caller,
        "Batch connection belongs to another user"
    );
    let directory = PathBuf::from("/tmp").join(format!("craft-approved-{}", uuid::Uuid::new_v4()));
    std::fs::DirBuilder::new().mode(0o700).create(&directory)?;
    let _private = PrivatePackages(directory.clone());
    let copies = match snapshot_packages(&request, &directory) {
        Ok(copies) => copies,
        Err(e) => {
            send(
                &mut stream,
                &Reply {
                    code: None,
                    error: Some(format!("{e:#}")),
                },
            )?;
            return Err(e);
        }
    };
    send(
        &mut stream,
        &Reply {
            code: Some(0),
            error: None,
        },
    )?;
    let kind = crate::installers::package_kind()?;
    let mut used = std::collections::HashSet::new();
    loop {
        let index: usize = match receive(&mut stream, None) {
            Ok(index) => index,
            Err(_) => break,
        };
        ensure!(
            index < copies.len() && used.insert(index),
            "Unapproved or repeated package request"
        );
        let status = crate::installers::approved_package_command(kind, &copies[index]).status();
        let reply = match status {
            Ok(status) => Reply {
                code: status.code(),
                error: None,
            },
            Err(e) => Reply {
                code: None,
                error: Some(e.to_string()),
            },
        };
        send(&mut stream, &reply)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn batch_eligibility_preserves_opt_out_and_requires_multiple_installers() {
        let preferences = crate::model::Preferences::default();
        let mut plan: ReleasePlan = serde_json::from_value(serde_json::json!({
            "entries": (["photocraft", "filmcraft"].map(|app| {
                let mut prefs = preferences.clone();
                prefs.release_format = "installer".into();
                crate::updates::ReleaseEntry { app: app.into(), action: "Update".into(), version: "1.0".into(), asset: None, destination: None, error: None, preferences: prefs }
            })),
            "preferences": preferences, "requested_tag": null, "snapshot": [],
            "desktop_shortcut": false, "installer_destination": null
        })).unwrap();
        assert!(eligible(&plan));
        plan.preferences.batch_admin = false;
        assert!(!eligible(&plan));
        plan.preferences.batch_admin = true;
        plan.entries[0].preferences.release_format = "portable".into();
        assert!(!eligible(&plan));
        plan.entries[0].preferences.release_format = "installer".into();
        plan.requested_tag = Some("1.0".into());
        assert!(!eligible(&plan));
    }
    fn fixture() -> (PrivatePackages, Request) {
        let directory =
            std::env::temp_dir().join(format!("craft-batch-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let packages = (0..2)
            .map(|index| {
                let path = directory.join(format!(
                    "input-{index}{}",
                    crate::installers::installer_extension().unwrap()
                ));
                std::fs::write(&path, format!("mock package {index}")).unwrap();
                Package {
                    checksum: files::hash(&path).unwrap(),
                    path,
                }
            })
            .collect();
        (
            PrivatePackages(directory.clone()),
            Request {
                uid: unsafe { libc::getuid() },
                socket: directory.join("socket"),
                packages,
            },
        )
    }
    #[test]
    fn snapshots_are_independent_and_changed_packages_are_rejected() {
        let (directory, request) = fixture();
        let copies = snapshot_packages(&request, &directory.0).unwrap();
        std::fs::write(&request.packages[0].path, b"changed").unwrap();
        assert_eq!(std::fs::read(&copies[0]).unwrap(), b"mock package 0");
        let next = directory.0.join("next");
        std::fs::create_dir(&next).unwrap();
        assert!(snapshot_packages(&request, &next)
            .unwrap_err()
            .to_string()
            .contains("changed"));
    }
    #[test]
    fn snapshots_reject_symlinks_and_wrong_package_formats() {
        let (directory, mut request) = fixture();
        let link = directory.0.join(format!(
            "linked{}",
            crate::installers::installer_extension().unwrap()
        ));
        std::os::unix::fs::symlink(&request.packages[0].path, &link).unwrap();
        request.packages[0].path = link;
        assert!(snapshot_packages(&request, &directory.0).is_err());
        request.packages[0].path = directory.0.join("bad.exe");
        assert!(snapshot_packages(&request, &directory.0)
            .unwrap_err()
            .to_string()
            .contains("format"));
    }
    #[test]
    fn unchanged_manifest_is_required_before_any_privileged_work() {
        let (directory, _) = fixture();
        let manifest = directory.0.join("manifest");
        std::fs::write(&manifest, b"changed").unwrap();
        assert!(worker(&manifest, "wrong")
            .unwrap_err()
            .to_string()
            .contains("approved batch changed"));
    }
    #[test]
    fn scoped_client_preserves_package_results_and_rejects_unapproved_work() {
        let (_directory, request) = fixture();
        let approved = request.packages[0].path.clone();
        let (stream, mut helper) = UnixStream::pair().unwrap();
        assert_eq!(peer_uid(&stream).unwrap(), unsafe { libc::getuid() });
        let thread = std::thread::spawn(move || {
            assert_eq!(receive::<usize>(&mut helper, None).unwrap(), 0);
            send(
                &mut helper,
                &Reply {
                    code: Some(100),
                    error: None,
                },
            )
            .unwrap();
        });
        CLIENT.with(|cell| {
            *cell.borrow_mut() = Some(Client {
                stream,
                packages: request.packages,
                consumed: Default::default(),
            });
        });
        let _scope = Scope;
        assert_eq!(
            install_package(&approved, false, None).unwrap().unwrap(),
            100
        );
        assert!(install_package(&approved, false, None).unwrap().is_err());
        assert!(install_package(Path::new("/unapproved.deb"), false, None)
            .unwrap()
            .is_err());
        assert!(install_package(&approved, true, None).unwrap().is_err());
        thread.join().unwrap();
    }
    #[test]
    fn cancelled_jobs_do_not_dispatch_another_package() {
        let (directory, request) = fixture();
        let approved = request.packages[0].path.clone();
        let (stream, _helper) = UnixStream::pair().unwrap();
        CLIENT.with(|cell| {
            *cell.borrow_mut() = Some(Client {
                stream,
                packages: request.packages,
                consumed: Default::default(),
            });
        });
        let _scope = Scope;
        let job = Job::new(directory.0.join("test.log"), &Default::default());
        job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        let error = install_package(&approved, false, Some(&job))
            .unwrap()
            .unwrap_err();
        assert!(crate::jobs::is_cancelled(&error));
        CLIENT.with(|cell| assert!(cell.borrow().as_ref().unwrap().consumed.is_empty()));
    }
}
