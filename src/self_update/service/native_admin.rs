//! Short-lived, exact-package administrator approval for native bridge updates.
use anyhow::{ensure, Context, Result};

struct Gate {
    token: String,
    expires: i64,
    count: usize,
    next: usize,
    pending: Option<usize>,
}
impl Gate {
    #[cfg(any(target_os = "macos", test))]
    fn close(&self, token: &str, time: i64) -> Result<()> {
        ensure!(
            token == self.token && time < self.expires,
            "Approval close expired or belongs to another session"
        );
        ensure!(
            self.next == self.count && self.pending.is_none(),
            "Approval still has unverified work"
        );
        Ok(())
    }
    fn claim(&mut self, token: &str, index: usize, now: i64) -> Result<()> {
        ensure!(token == self.token, "Wrong native approval session");
        ensure!(now < self.expires, "Native administrator approval expired");
        ensure!(
            self.pending.is_none(),
            "Previous native package needs verified acknowledgement"
        );
        ensure!(
            index == self.next && index < self.count,
            "Repeated or out-of-order native package"
        );
        self.next += 1;
        self.pending = Some(index);
        Ok(())
    }
    fn acknowledge(&mut self, token: &str, index: usize, now: i64) -> Result<bool> {
        ensure!(
            token == self.token && now < self.expires,
            "Native acknowledgement expired or belongs to another session"
        );
        ensure!(
            self.pending == Some(index),
            "Unverified or repeated native acknowledgement"
        );
        self.pending = None;
        Ok(self.next == self.count)
    }
}

use super::{
    bridge::Journey,
    catalog::{verify_catalog, TrustContext, VerifiedCatalog},
    model::{ClientProfile, InstallationKind, PackageRecord, UpdateOffer},
    transaction::SignedSelection,
};
use crate::{files, jobs::Job, model::Paths};
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
#[cfg(target_os = "macos")]
pub(crate) mod macos;
#[cfg(any(target_os = "macos", test))]
mod macos_bootstrap;
#[cfg(target_os = "macos")]
mod macos_bundle;
#[cfg(any(target_os = "macos", test))]
mod macos_policy;
#[cfg(target_os = "macos")]
mod macos_worker;
#[path = "native_admin/platform.rs"]
mod platform;

const MAX_MESSAGE: usize = 12 * 1024 * 1024;
const IDLE: Duration = Duration::from_secs(180);

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum GrantSpan {
    #[default]
    Journey,
    Step,
}
impl GrantSpan {
    fn is_journey(&self) -> bool {
        *self == Self::Journey
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    #[serde(default, skip_serializing_if = "GrantSpan::is_journey")]
    grant_span: GrantSpan,
    envelope: Vec<u8>,
    source: ClientProfile,
    offer: UpdateOffer,
    expires: i64,
    target: PathBuf,
    root: PathBuf,
    source_hash: String,
    account: String,
    callback: u16,
    secret: [u8; 32],
    packages: Vec<PathBuf>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Session {
    manifest: Manifest,
    port: u16,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedWire {
    payload: String,
    signature: Vec<u8>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Hello {
    role: String,
    port: u16,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    role: String,
    id: String,
    action: String,
    index: usize,
    #[serde(default)]
    envelope: Vec<u8>,
    profile: Option<ClientProfile>,
    offer: Option<UpdateOffer>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    role: String,
    id: String,
    code: Option<i32>,
    error: Option<String>,
    log: String,
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
fn marker(root: &Path) -> PathBuf {
    root.join("runtime/manager-updates/native-admin-session.json")
}
pub(crate) fn exists(root: &Path) -> bool {
    marker(root).exists()
}
fn read_bounded<T: DeserializeOwned>(path: &Path) -> Result<T> {
    files::no_links(path)?;
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_MESSAGE as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_MESSAGE,
        "Native approval record is too large"
    );
    Ok(serde_json::from_slice(&bytes)?)
}
fn wire_send<T: Serialize>(stream: &mut TcpStream, secret: &[u8; 32], value: &T) -> Result<()> {
    let payload = serde_json::to_string(value)?;
    let signature = SigningKey::from_bytes(secret)
        .sign(payload.as_bytes())
        .to_bytes()
        .to_vec();
    let bytes = serde_json::to_vec(&SignedWire { payload, signature })?;
    ensure!(
        bytes.len() <= MAX_MESSAGE,
        "Native approval message too large"
    );
    stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
    stream.write_all(&bytes)?;
    Ok(())
}
fn wire_read<T: DeserializeOwned>(stream: &mut TcpStream, secret: &[u8; 32]) -> Result<T> {
    let mut length = [0; 4];
    stream.read_exact(&mut length)?;
    let size = u32::from_be_bytes(length) as usize;
    ensure!(size <= MAX_MESSAGE, "Native approval message too large");
    let mut bytes = vec![0; size];
    stream.read_exact(&mut bytes)?;
    let wire: SignedWire = serde_json::from_slice(&bytes)?;
    let key: VerifyingKey = SigningKey::from_bytes(secret).verifying_key();
    key.verify(
        wire.payload.as_bytes(),
        &ed25519_dalek::Signature::from_slice(&wire.signature)?,
    )?;
    Ok(serde_json::from_str(&wire.payload)?)
}
fn connection(port: u16) -> Result<TcpStream> {
    let stream = TcpStream::connect_timeout(
        &SocketAddrV4::new(Ipv4Addr::LOCALHOST, port).into(),
        Duration::from_secs(2),
    )?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    Ok(stream)
}
fn accepted_connection(listener: &TcpListener) -> std::io::Result<TcpStream> {
    let (stream, _) = listener.accept()?;
    // Windows inherits the listener's nonblocking mode; a framed message may
    // arrive in multiple reads. Keep accept pollable, but bound blocking I/O.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    Ok(stream)
}
fn native_packages<'a>(
    catalog: &'a VerifiedCatalog,
    profile: &ClientProfile,
    offer: &UpdateOffer,
) -> Result<Vec<&'a PackageRecord>> {
    #[cfg(any(target_os = "macos", test))]
    if profile.installation == InstallationKind::Bundle {
        return macos_policy::packages(catalog, profile, offer);
    }
    super::transaction::validate_offer(catalog, profile, offer)?;
    ensure!(
        (2..=8).contains(&offer.route_ids.len()),
        "Not a bounded native bridge"
    );
    ensure!(
        matches!(
            profile.installation,
            InstallationKind::Msi
                | InstallationKind::Deb
                | InstallationKind::Rpm
                | InstallationKind::Arch
        ),
        "Not a native installation"
    );
    offer
        .route_ids
        .iter()
        .map(|id| {
            let route = catalog
                .catalog()
                .routes
                .iter()
                .find(|r| &r.id == id)
                .context("Missing native route")?;
            let p = catalog
                .catalog()
                .packages
                .iter()
                .find(|p| p.id == route.target)
                .context("Missing native package")?;
            ensure!(
                p.installation == profile.installation
                    && p.handler.starts_with("native.")
                    && profile.capabilities.contains(&p.handler)
                    && p.required_capabilities.is_subset(&profile.capabilities),
                "Native bridge needs a different installer; separate approval is required"
            );
            Ok(p)
        })
        .collect()
}
fn grant_packages<'a>(
    catalog: &'a VerifiedCatalog,
    profile: &ClientProfile,
    offer: &UpdateOffer,
    span: GrantSpan,
) -> Result<Vec<&'a PackageRecord>> {
    let mut packages = native_packages(catalog, profile, offer)?;
    if span == GrantSpan::Step {
        ensure!(
            profile.platform == "macos" && profile.installation == InstallationKind::Bundle,
            "Single-step grants are only valid for macOS bundles"
        );
        packages.truncate(1);
    }
    Ok(packages)
}
fn approved(
    manifest: &Manifest,
    trust: &TrustContext,
    time: i64,
) -> Result<(VerifiedCatalog, Option<Journey>)> {
    ensure!(
        manifest.schema == 1
            && time < manifest.expires
            && manifest.expires.saturating_sub(time) <= 3600,
        "Native approval expired or invalid"
    );
    let catalog = verify_catalog(&manifest.envelope, trust, time, None)?;
    ensure!(
        manifest.expires <= catalog.catalog().expires,
        "Native approval outlives signed metadata"
    );
    let selection = SignedSelection {
        catalog: catalog.clone(),
        offer: manifest.offer.clone(),
    };
    let journey = if manifest.offer.route_ids.len() > 1 {
        Some(Journey::approve(&selection, &manifest.source, trust, time)?)
    } else {
        None
    };
    ensure!(
        grant_packages(
            &catalog,
            &manifest.source,
            &manifest.offer,
            manifest.grant_span
        )?
        .len()
            == manifest.packages.len(),
        "Native package scope changed"
    );
    Ok((catalog, journey))
}

/// Starts an exact, signed native session before any native transaction runs.
pub(crate) fn prepare(
    paths: &Paths,
    selection: &SignedSelection,
    profile: &ClientProfile,
    continuing: bool,
    consent_expires: Option<i64>,
    job: &Job,
) -> Result<()> {
    let mac = cfg!(target_os = "macos") && profile.installation == InstallationKind::Bundle;
    #[cfg(target_os = "macos")]
    if mac {
        let bundle = crate::self_update::app_bundle(&std::env::current_exe()?.canonicalize()?)?;
        if matches!(
            macos::update_access(&bundle)?,
            macos::UpdateAccess::Writable
        ) {
            return Ok(());
        }
        ensure!(
            job.identity_choices_available(),
            "Protected manager updates require interactive administrator approval"
        );
    }
    if !matches!(
        profile.installation,
        InstallationKind::Msi
            | InstallationKind::Deb
            | InstallationKind::Rpm
            | InstallationKind::Arch
    ) && !mac
    {
        return Ok(());
    }
    let retain = paths.preferences()?.batch_admin;
    if !retain {
        revoke(&paths.root);
        if !mac {
            return Ok(());
        }
    }
    if continuing && retain {
        let session = session_for(&paths.root)?
            .context("Native administrator approval was lost; check again to approve the update")?;
        ensure!(
            session.manifest.root == paths.root.canonicalize()?
                && session_target_matches(
                    &session.manifest.target,
                    &std::env::current_exe()?.canonicalize()?
                )
                && session.manifest.expires > now(),
            "Native approval no longer matches this installation"
        );
        return Ok(());
    }
    if !mac && selection.offer.route_ids.len() < 2 {
        return Ok(());
    }
    // A bridge introducing a capability absent from this broker keeps the
    // existing per-hop approval path; never block that compatibility upgrade.
    let span = if mac && (!retain || selection.offer.route_ids.len() == 1) {
        GrantSpan::Step
    } else {
        GrantSpan::Journey
    };
    let packages = match grant_packages(&selection.catalog, profile, &selection.offer, span) {
        Ok(packages) => packages,
        Err(e) if mac => return Err(e),
        Err(_) => return Ok(()),
    };
    let home = paths.at(format!(
        "runtime/native-admin/{}",
        uuid::Uuid::new_v4().simple()
    ));
    platform::user_directory(&home)?;
    let mut preparation = PreparationGuard {
        home: home.clone(),
        keep: false,
    };
    let mut downloads = Vec::new();
    for (index, p) in packages.iter().enumerate() {
        job.check()?;
        let file = home.join(format!(
            "package-{index}{}",
            platform::extension(&profile.installation)?
        ));
        super::transport::download_asset(p, &file, job)?;
        platform::validate(&file, p, &selection.catalog)?;
        if !platform::batch_supported(&file)? {
            job.log("This installer uses the existing separate approval path because its native registration lacks the location needed for a shared approval.");
            return Ok(());
        }
        downloads.push(file);
    }
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    listener.set_nonblocking(true)?;
    let mut secret = [0; 32];
    secret[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    secret[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    let manifest = Manifest {
        schema: 1,
        grant_span: span,
        envelope: selection.catalog.envelope().to_vec(),
        source: profile.clone(),
        offer: selection.offer.clone(),
        expires: selection
            .catalog
            .catalog()
            .expires
            .min(if mac {
                consent_expires.unwrap_or_else(|| now().saturating_add(3600))
            } else {
                consent_expires.context("Missing signed bridge consent expiry")?
            })
            .min(now().saturating_add(3600)),
        target: std::env::current_exe()?.canonicalize()?,
        root: paths.root.canonicalize()?,
        source_hash: files::hash(&std::env::current_exe()?)?,
        account: platform::account()?,
        callback: listener.local_addr()?.port(),
        secret,
        packages: downloads,
    };
    let path = home.join("approved.json");
    files::write_json(&path, &manifest)?;
    let helper = home.join(if cfg!(windows) {
        "admin-helper.exe"
    } else {
        "admin-helper"
    });
    fs::copy(std::env::current_exe()?, &helper)?;
    let checksum = files::hash(&path)?;
    job.stage(
        "Awaiting administrator approval",
        None,
        "Approve once for all steps of this manager bridge update.",
    );
    job.check()?;
    let mut worker = platform::elevate(&helper, &path, &checksum)?;
    let deadline = Instant::now() + Duration::from_secs(300);
    let session = loop {
        job.check()?;
        ensure!(
            Instant::now() < deadline && now() < manifest.expires,
            "Native approval did not start; approval may have been cancelled"
        );
        ensure!(worker.running()?,"Native administrator worker stopped before completing its authenticated connection; see the authorization log");
        match accepted_connection(&listener) {
            Ok(mut stream) => {
                // Both sides prove the secret without sending it over the connection.
                wire_send(
                    &mut stream,
                    &secret,
                    &Hello {
                        role: "native-parent".into(),
                        port: 0,
                    },
                )?;
                if let Ok(hello) = wire_read::<Hello>(&mut stream, &secret) {
                    if hello.role == "native-worker" && hello.port > 0 {
                        break Session {
                            manifest,
                            port: hello.port,
                        };
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(e) => return Err(e.into()),
        }
    };
    // The manifest and descriptor contain an ephemeral session key, not OS credentials.
    // Keep both inside a user-only runtime directory, never settings or diagnostic output.
    let stored = (|| -> Result<()> {
        let descriptor = home.join("session.json");
        files::write_json(&descriptor, &session)?;
        files::write_json(&marker(&paths.root), &descriptor)?;
        job.check()?;
        Ok(())
    })();
    if let Err(error) = stored {
        let _ = exchange(&session, request("abort", 0), false, None);
        let _ = clear(&paths.root, &session);
        return Err(error);
    }
    preparation.keep = true;
    Ok(())
}

fn session_target_matches(approved: &Path, running: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        approved.ancestors().nth(3) == running.ancestors().nth(3)
    }
    #[cfg(not(target_os = "macos"))]
    {
        approved == running
    }
}

#[cfg(target_os = "macos")]
fn macos_plan_target_matches(approved: &Path, bundle: &Path) -> bool {
    bundle.extension().is_some_and(|s| s == "app") && approved.ancestors().nth(3) == Some(bundle)
}

struct PreparationGuard {
    home: PathBuf,
    keep: bool,
}
impl Drop for PreparationGuard {
    fn drop(&mut self) {
        if !self.keep && files::no_links(&self.home).is_ok() {
            for name in ["approved.json", "session.json"] {
                let _ = fs::remove_file(self.home.join(name));
            }
        }
    }
}

fn session_for(root: &Path) -> Result<Option<Session>> {
    let session = session_record(root)?;
    if let Some(session) = &session {
        ensure!(
            now() < session.manifest.expires,
            "Native administrator approval expired"
        );
    }
    Ok(session)
}
// Completion/revocation may read an expired descriptor, but cannot start an install.
fn session_record(root: &Path) -> Result<Option<Session>> {
    let path = marker(root);
    if !path.exists() {
        return Ok(None);
    }
    let descriptor: PathBuf = read_bounded(&path)?;
    files::inside(&descriptor, &root.join("runtime/native-admin"))?;
    let session: Session = read_bounded(&descriptor)?;
    ensure!(
        session.manifest.root == root.canonicalize()?,
        "Native administrator approval belongs to another data folder"
    );
    Ok(Some(session))
}
pub(crate) fn stage(paths: &Paths, package: &PackageRecord, dest: &Path) -> Result<bool> {
    let Some(session) = session_for(&paths.root)? else {
        return Ok(false);
    };
    let (catalog, _) = approved(
        &session.manifest,
        &super::transaction::production_trust()?,
        now(),
    )?;
    let packages = grant_packages(
        &catalog,
        &session.manifest.source,
        &session.manifest.offer,
        session.manifest.grant_span,
    )?;
    let index = packages
        .iter()
        .position(|p| p.sha256 == package.sha256 && p.id == package.id)
        .context("Package is outside the approved native bridge")?;
    copy_package(&session.manifest.packages[index], dest, package)?;
    Ok(true)
}
fn copy_package(source: &Path, dest: &Path, p: &PackageRecord) -> Result<()> {
    copy_into(source, dest, p, false)
}
fn copy_into(source: &Path, dest: &Path, p: &PackageRecord, private: bool) -> Result<()> {
    let mut input = fs::File::open(source)?.take(p.size + 1);
    let mut output = if private {
        platform::snapshot_file(dest)?
    } else {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dest)?
    };
    let size = std::io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    drop(output);
    ensure!(
        size == p.size && files::hash(dest)? == p.sha256,
        "Approved native package changed"
    );
    Ok(())
}
fn exchange(
    session: &Session,
    mut request: Request,
    installing: bool,
    job: Option<&Job>,
) -> Result<Reply> {
    request.role = "native-client".into();
    request.id = uuid::Uuid::new_v4().simple().to_string();
    let mut stream = connection(session.port)?;
    wire_send(&mut stream, &session.manifest.secret, &request)?;
    // Package managers must finish; cancellation only stops subsequent transactions.
    // Keep partial frames intact. A cancelled active package manager must finish;
    // the caller's next job.check() prevents subsequent steps.
    stream.set_read_timeout(Some(Duration::from_secs(if installing { 3600 } else { 5 })))?;
    let reply: Reply = wire_read(&mut stream, &session.manifest.secret)?;
    ensure!(
        reply.role == "native-result" && reply.id == request.id,
        "Native approval response mismatch"
    );
    if let Some(job) = job {
        for line in reply.log.lines() {
            job.log(line);
        }
    }
    ensure!(
        reply.error.is_none(),
        "{}",
        reply.error.as_deref().unwrap_or_default()
    );
    Ok(reply)
}
fn request(action: &str, index: usize) -> Request {
    Request {
        role: String::new(),
        id: String::new(),
        action: action.into(),
        index,
        envelope: Vec::new(),
        profile: None,
        offer: None,
    }
}
pub(crate) fn install(
    paths: &Paths,
    selection: &SignedSelection,
    profile: &ClientProfile,
    job: Option<&Job>,
) -> Result<Option<(i32, String)>> {
    let Some(session) = session_for(&paths.root)? else {
        return Ok(None);
    };
    ensure!(
        session.manifest.target == std::env::current_exe()?.canonicalize()?
            || std::env::args().any(|a| a == "--apply-self-update"),
        "Native session executable mismatch"
    );
    let (original, _) = approved(
        &session.manifest,
        &super::transaction::production_trust()?,
        now(),
    )?;
    let packages = grant_packages(
        &original,
        &session.manifest.source,
        &session.manifest.offer,
        session.manifest.grant_span,
    )?;
    let index = packages
        .iter()
        .position(|p| p.id == selection.offer.immediate_package_id)
        .context("Unapproved native package")?;
    if let Some(job) = job {
        job.check()?;
        job.stage(
            "Installing package",
            None,
            "Using the administrator approval for this bridge update.",
        );
    }
    let mut req = request("install", index);
    req.envelope = selection.catalog.envelope().to_vec();
    req.profile = Some(profile.clone());
    req.offer = Some(selection.offer.clone());
    let reply = exchange(&session, req, true, job)?;
    Ok(Some((
        reply
            .code
            .context("Native package manager returned no exit status")?,
        reply.log,
    )))
}
pub(crate) fn acknowledge(root: &Path, package: &PackageRecord) -> Result<()> {
    let Some(session) = session_record(root)? else {
        #[cfg(target_os = "macos")]
        anyhow::bail!("Protected bundle acknowledgement lost its administrator session");
        #[cfg(not(target_os = "macos"))]
        return Ok(());
    };
    if now() >= session.manifest.expires {
        #[cfg(target_os = "macos")]
        {
            revoke(root);
            anyhow::bail!("Administrator approval expired before bundle acknowledgement; check recovery before retrying");
        }
        #[cfg(not(target_os = "macos"))]
        {
            // The unelevated caller has already verified the completed installation.
            // Expiry revokes further work; it must not prevent its ordinary restart.
            revoke(root);
            super::bridge::revoke(root)?;
            return Ok(());
        }
    }
    let (catalog, _) = approved(
        &session.manifest,
        &super::transaction::production_trust()?,
        now(),
    )?;
    let packages = grant_packages(
        &catalog,
        &session.manifest.source,
        &session.manifest.offer,
        session.manifest.grant_span,
    )?;
    let index = packages
        .iter()
        .position(|p| p.id == package.id && p.sha256 == package.sha256)
        .context("Unapproved native completion")?;
    let ack = exchange(&session, request("ack", index), false, None);
    #[cfg(target_os = "macos")]
    if let Err(error) = ack {
        // Never retry publication. A consumed acknowledgement can be confirmed by status.
        let status = exchange(&session, request("status", 0), false, None)
            .and_then(|r| Ok(serde_json::from_str::<macos_worker::Status>(&r.log)?));
        if !status.is_ok_and(|s| s.pending.is_none() && s.next > index) {
            revoke(root);
            return Err(error);
        }
    }
    #[cfg(not(target_os = "macos"))]
    ack?;
    if index + 1 == packages.len() {
        #[cfg(target_os = "macos")]
        let _ = exchange(&session, request("close", index), false, None);
        clear(root, &session)?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub(crate) fn verify_macos_session(plan: &super::transaction::TransactionPlanV2) -> Result<()> {
    let session = session_for(&plan.legacy.root)?.context("Missing live administrator approval")?;
    ensure!(
        session.manifest.root == plan.legacy.root.canonicalize()?
            && macos_plan_target_matches(&session.manifest.target, &plan.legacy.target)
            && session.manifest.source.repository == plan.profile.repository
            && session.manifest.source.channel == plan.profile.channel
            && session.manifest.source.architecture == plan.profile.architecture,
        "Administrator approval scope differs from plan"
    );
    let (catalog, _) = approved(
        &session.manifest,
        &super::transaction::production_trust()?,
        now(),
    )?;
    let packages = grant_packages(
        &catalog,
        &session.manifest.source,
        &session.manifest.offer,
        session.manifest.grant_span,
    )?;
    let selected = plan.offer.immediate_package_id.as_str();
    let index = packages
        .iter()
        .position(|p| p.id == selected)
        .context("Unapproved bundle package")?;
    let status: macos_worker::Status =
        serde_json::from_str(&exchange(&session, request("status", 0), false, None)?.log)?;
    ensure!(
        status.pending.is_none()
            && status.next == index
            && status.installed_hash == plan.source_hash,
        "Administrator worker source or pending step changed"
    );
    Ok(())
}
fn clear(root: &Path, session: &Session) -> Result<()> {
    let path = marker(root);
    if path.exists() {
        fs::remove_file(path)?;
    }
    if let Some(home) = session.manifest.packages.first().and_then(|p| p.parent()) {
        let approved = home.join("approved.json");
        if approved.exists() {
            fs::remove_file(approved)?;
        }
        let descriptor = home.join("session.json");
        if descriptor.exists() {
            fs::remove_file(descriptor)?;
        }
    }
    Ok(())
}
pub(crate) fn revoke(root: &Path) {
    if let Ok(Some(session)) = session_record(root) {
        let _ = exchange(&session, request("abort", 0), false, None);
        let _ = clear(root, &session);
    } else {
        let _ = fs::remove_file(marker(root));
    }
}
pub fn cancel(root: PathBuf) {
    std::thread::spawn(move || revoke(&root));
}

/// Entry point for the explicitly elevated, short-lived native-only worker.
pub fn worker(path: &Path, checksum: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let _bootstrap = macos::bootstrap_guard()?;
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_MESSAGE as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_MESSAGE,
        "Native approval manifest too large"
    );
    use sha2::{Digest, Sha256};
    ensure!(
        format!("{:x}", Sha256::digest(&bytes)) == checksum,
        "Native approval manifest changed"
    );
    let manifest: Manifest = serde_json::from_slice(&bytes)?;
    platform::require_admin(&manifest.account)?;
    #[cfg(target_os = "macos")]
    {
        ensure!(
            path == std::env::current_exe()?
                .parent()
                .context("Missing bootstrap parent")?
                .join("approved.json"),
            "Manifest is outside private bootstrap"
        );
        macos::validate_downloads(&manifest)?;
        ensure!(
            files::hash(&std::env::current_exe()?)? == manifest.source_hash,
            "Private administrator helper differs from approved source"
        );
    }
    platform::target(&manifest.target, &manifest.source)?;
    ensure!(
        files::hash(&manifest.target)? == manifest.source_hash,
        "Installed manager changed before administrator approval"
    );
    let trust = super::transaction::production_trust()?;
    let (catalog, journey) = approved(&manifest, &trust, now())?;
    let packages = grant_packages(
        &catalog,
        &manifest.source,
        &manifest.offer,
        manifest.grant_span,
    )?;
    let private = platform::private_directory()?;
    let mut copies = Vec::new();
    let mut _locks = Vec::new();
    for (index, p) in packages.iter().enumerate() {
        let copy = private.0.join(format!(
            "package-{index}{}",
            platform::extension(&manifest.source.installation)?
        ));
        copy_into(&manifest.packages[index], &copy, p, true)?;
        _locks.push(platform::lock_snapshot(&copy)?);
        platform::validate(&copy, p, &catalog)?;
        ensure!(
            platform::batch_supported(&copy)?,
            "Native package registration cannot be bound to this approval"
        );
        copies.push(copy);
    }
    #[cfg(target_os = "macos")]
    return macos_worker::run(manifest, catalog, journey, copies, &trust);
    #[cfg(not(target_os = "macos"))]
    {
        let mut journey = journey;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let mut parent = connection(manifest.callback)?;
        let hello: Hello = wire_read(&mut parent, &manifest.secret)?;
        ensure!(
            hello.role == "native-parent",
            "Wrong native approval parent"
        );
        wire_send(
            &mut parent,
            &manifest.secret,
            &Hello {
                role: "native-worker".into(),
                port: listener.local_addr()?.port(),
            },
        )?;
        drop(parent);
        let mut gate = Gate {
            token: "session".into(),
            expires: manifest.expires,
            count: copies.len(),
            next: 0,
            pending: None,
        };
        let mut idle = Instant::now();
        let mut installed_hash = manifest.source_hash.clone();
        let mut prior = super::trust::TrustRecord::from_catalog(&catalog);
        while now() < manifest.expires && idle.elapsed() < IDLE {
            let mut stream = match accepted_connection(&listener) {
                Ok(value) => value,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                }
                Err(e) => return Err(e.into()),
            };
            let Ok(req) = wire_read::<Request>(&mut stream, &manifest.secret) else {
                continue;
            };
            if req.role != "native-client" {
                continue;
            }
            let mut reply = Reply {
                role: "native-result".into(),
                id: req.id.clone(),
                code: None,
                error: None,
                log: String::new(),
            };
            let operation = (|| -> Result<bool> {
                if req.action == "abort" {
                    return Ok(true);
                }
                if req.action == "ack" {
                    ensure!(
                        gate.pending == Some(req.index),
                        "Unverified or repeated native acknowledgement"
                    );
                    ensure!(
                        files::hash(&manifest.target)? == installed_hash,
                        "Installed manager changed before verification acknowledgement"
                    );
                    platform::verify_registered(
                        &manifest.target,
                        &copies[req.index],
                        packages[req.index],
                        &catalog,
                    )?;
                    reply.code = Some(0);
                    return gate.acknowledge("session", req.index, now());
                }
                ensure!(
                    req.action == "install" && gate.pending.is_none(),
                    "Native package has not been verified"
                );
                let profile = req.profile.context("Missing native source")?;
                let fresh = verify_catalog(&req.envelope, &trust, now(), Some(&prior))?;
                let selection = SignedSelection {
                    catalog: fresh.clone(),
                    offer: req.offer.context("Missing native offer")?,
                };
                let index = req.index;
                ensure!(index < packages.len(), "Unapproved native package");
                if index == 0 {
                    if let Some(journey) = &journey {
                        journey.validate_selection(&selection, &profile, &trust, now())?;
                    } else {
                        super::transaction::validate_offer(&fresh, &profile, &selection.offer)?;
                        ensure!(
                            serde_json::to_value(&profile)?
                                == serde_json::to_value(&manifest.source)?
                                && serde_json::to_value(&selection.offer)?
                                    == serde_json::to_value(&manifest.offer)?
                                && serde_json::to_value(
                                    fresh
                                        .catalog()
                                        .packages
                                        .iter()
                                        .find(|p| p.id == selection.offer.immediate_package_id)
                                        .context("Missing direct package")?
                                )? == serde_json::to_value(packages[index])?
                                && serde_json::to_value(fresh.catalog().release(packages[index]))?
                                    == serde_json::to_value(
                                        catalog.catalog().release(packages[index])
                                    )?,
                            "Direct bundle approval scope changed"
                        );
                    }
                } else {
                    journey = Some(
                        journey
                            .as_ref()
                            .context("No approved bridge journey")?
                            .next(&selection, &profile, &trust, now())?,
                    );
                }
                ensure!(
                    selection.offer.immediate_package_id == packages[index].id,
                    "Native package order changed"
                );
                ensure!(
                    files::hash(&manifest.target)? == installed_hash,
                    "Native bridge source executable changed"
                );
                ensure!(
                    files::hash(&copies[index])? == packages[index].sha256,
                    "Native snapshot changed"
                );
                gate.claim("session", index, now())?; // Consume before starting; never replay an ambiguous install.
                prior = super::trust::TrustRecord::from_catalog(&fresh);
                let log = private.0.join(format!("install-{index}.log"));
                let code = platform::install(
                    &copies[index],
                    &manifest.target,
                    &manifest.source.installation,
                    &log,
                )?;
                let mut log_bytes = Vec::new();
                fs::File::open(log)?
                    .take(128 * 1024)
                    .read_to_end(&mut log_bytes)?;
                reply.log = String::from_utf8_lossy(&log_bytes).into_owned();
                ensure!(
                    code == 0
                        || (manifest.source.installation == InstallationKind::Msi && code == 3010),
                    "Native package manager failed ({code})"
                );
                // Do not execute a user-writable installed executable as root. Normal
                // unelevated preparation/helper verifies its signed build identity before ack.
                platform::verify_registered(
                    &manifest.target,
                    &copies[index],
                    packages[index],
                    &catalog,
                )?;
                installed_hash = files::hash(&manifest.target)?;
                reply.code = Some(code);
                Ok(false)
            })();
            let finished = match operation {
                Ok(done) => done,
                Err(e) => {
                    reply.error = Some(format!("{e:#}"));
                    true
                }
            };
            let sent = wire_send(&mut stream, &manifest.secret, &reply);
            idle = Instant::now(); // A long package transaction does not consume restart/ack grace.
            if finished {
                sent?;
                return Ok(());
            }
            // A lost install reply never causes a replay. The broker only accepts its ack.
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "native_admin/tests.rs"]
mod tests;
