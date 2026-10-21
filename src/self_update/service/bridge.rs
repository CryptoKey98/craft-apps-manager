//! Local consent for a bounded, signed bridge journey. PendingBridge remains check-only.
use super::{
    catalog::{verify_catalog, TrustContext},
    model::*,
    transaction::{validate_offer, SignedSelection},
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};

const CONSENT_SECONDS: i64 = 3600;
const RESTART_FLAG: &str = "--continue-manager-update";
const MAX_RECORD: u64 = 12 * 1024 * 1024;
use crate::{files, self_update::Plan};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Never constructed from pending-bridge.json or unsigned server advice.
pub struct ResumePermit {
    journey: Journey,
}
/// Shared activity records include work owned by a different Manager/Builder process.
pub fn foreground_busy(records: &[crate::operations::Operation], own: Option<&str>) -> bool {
    records.iter().any(|op| {
        !op.lifecycle.terminal()
            && Some(op.id.as_str()) != own
            && !(own.is_some() && op.parent.as_deref() == own)
            && !op.kind.starts_with("Background ")
    })
}
pub(crate) fn ensure_idle(root: &Path, own: Option<&str>) -> Result<()> {
    ensure!(
        !foreground_busy(&crate::operations::Manager::new(root).list(), own),
        "Manager update paused while another operation is running; check again to continue"
    );
    Ok(())
}
pub(crate) fn execution_gate() -> Result<(crate::platform::Lock, crate::platform::Lock)> {
    // Same ordering as the existing source-cache/remove operations. Never wait on a UI thread.
    let builder = crate::platform::Lock::take("Local\\CraftAppsSourceBuilder")?;
    let manager = crate::platform::Lock::take("Local\\CraftAppsManager")?;
    Ok((builder, manager))
}
impl ResumePermit {
    pub fn authorize(
        self,
        selection: &SignedSelection,
        current: &ClientProfile,
        trust: &TrustContext,
        now: i64,
    ) -> Result<Journey> {
        self.journey.next(selection, current, trust, now)
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    schema: u32,
    journey: Journey,
    root: PathBuf,
    target: PathBuf,
    tools: PathBuf,
    stage: PathBuf,
    helper_hash: String,
    prepared_hash: String,
    expected: super::transaction::ExpectedIdentity,
    parent_pid: u32,
    msi: bool,
    linux_package: bool,
    token_hash: Option<String>,
    installed_hash: Option<String>,
}
fn state_path(root: &Path) -> PathBuf {
    root.join("runtime/manager-updates/bridge-approval.json")
}
fn guard(root: &Path) -> Result<crate::platform::Lock> {
    let path = root.canonicalize()?;
    crate::platform::Lock::take(&super::trust::lock_name(
        "Bridge",
        path.to_string_lossy().as_bytes(),
    ))
}
fn read_state(root: &Path) -> Result<Option<State>> {
    let path = state_path(root);
    files::inside(&path, root)?;
    if !path.exists() {
        return Ok(None);
    }
    ensure!(
        fs::metadata(&path)?.len() <= MAX_RECORD,
        "Bridge approval is too large"
    );
    let bytes = fs::read(path)?;
    ensure!(
        bytes.len() as u64 <= MAX_RECORD,
        "Bridge approval is too large"
    );
    let record: State = serde_json::from_slice(&bytes)?;
    ensure!(record.schema == 1, "Unsupported bridge approval");
    Ok(Some(record))
}
fn write_state(root: &Path, record: &State) -> Result<()> {
    let path = state_path(root);
    files::inside(&path, root)?;
    fs::create_dir_all(path.parent().context("Missing bridge state folder")?)?;
    let bytes = serde_json::to_vec(record)?;
    ensure!(
        bytes.len() as u64 <= MAX_RECORD,
        "Bridge approval is too large"
    );
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    let result = (|| -> Result<()> {
        let mut file = options.open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        crate::platform::atomic_replace(&temporary, &path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
pub(crate) fn revoke(root: &Path) -> Result<()> {
    let _guard = guard(root)?;
    let path = state_path(root);
    files::inside(&path, root)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}
pub(crate) fn record_prepared(plan: &Plan, path: &Path, journey: Option<&Journey>) -> Result<()> {
    let root = &plan.root;
    let _guard = guard(root)?;
    let Some(journey) = journey.filter(|j| j.has_more_hops()) else {
        let path = state_path(root);
        files::inside(&path, root)?;
        if path.exists() {
            fs::remove_file(path)?;
        }
        return Ok(());
    };
    let stage = path
        .parent()
        .context("Missing bridge stage")?
        .canonicalize()?;
    let helper = stage.join(crate::self_update::helper_name());
    write_state(
        root,
        &State {
            schema: 1,
            journey: journey.clone(),
            root: root.canonicalize()?,
            target: plan.target.canonicalize()?,
            tools: std::path::absolute(&plan.tools)?,
            stage,
            helper_hash: files::hash(&helper)?,
            prepared_hash: plan.hash.clone(),
            expected: plan
                .expected_identity
                .clone()
                .context("Missing bridge identity")?,
            parent_pid: plan.parent_pid,
            msi: plan.msi,
            linux_package: plan.linux_package,
            token_hash: None,
            installed_hash: None,
        },
    )
}
fn issue_at(
    plan: &Plan,
    helper: &Path,
    trust: &TrustContext,
    now: i64,
    mut verify: impl FnMut(&Path, &super::transaction::ExpectedIdentity, &Path) -> Result<()>,
) -> Result<Option<String>> {
    let root = &plan.root;
    let _guard = guard(root)?;
    let Some(mut record) = read_state(root)? else {
        return Ok(None);
    };
    // Consume before validation: failed/replayed commits cannot leave resumable authority.
    fs::remove_file(state_path(root))?;
    ensure!(
        record.token_hash.is_none() && record.installed_hash.is_none(),
        "Bridge handoff already issued"
    );
    let c = record.journey.verified(trust, now)?;
    let package = c
        .catalog()
        .packages
        .iter()
        .find(|p| p.id == record.journey.offer.immediate_package_id)
        .context("Missing bridge package")?;
    let expected = super::transaction::expected(&c, package);
    ensure!(
        record.root == root.canonicalize()?
            && record.target == plan.target.canonicalize()?
            && record.tools == std::path::absolute(&plan.tools)?
            && record.stage
                == helper
                    .parent()
                    .context("Missing helper folder")?
                    .canonicalize()?
            && record.helper_hash == files::hash(helper)?
            && record.prepared_hash == plan.hash
            && record.parent_pid == plan.parent_pid
            && record.msi == plan.msi
            && record.linux_package == plan.linux_package
            && serde_json::to_value(&record.expected)? == serde_json::to_value(&expected)?
            && serde_json::to_value(&record.expected)?
                == serde_json::to_value(
                    plan.expected_identity
                        .as_ref()
                        .context("Missing bridge identity")?
                )?,
        "Bridge commit binding changed"
    );
    let executable = crate::self_update::staged_executable(&plan.target);
    files::no_links(&executable)?;
    if !plan.msi && !plan.linux_package {
        ensure!(
            files::hash(&executable)? == plan.hash,
            "Bridge replacement did not succeed"
        );
    }
    verify(&executable, &expected, &record.stage)?;
    record.installed_hash = Some(files::hash(&executable)?);
    let token = uuid::Uuid::new_v4().simple().to_string();
    record.token_hash = Some(format!("{:x}", Sha256::digest(token.as_bytes())));
    write_state(root, &record)?;
    Ok(Some(token))
}
/// Called only at the normal verified-success restart boundary, never on the GUI frame path.
pub(crate) fn append_restart(command: &mut Command, plan: &Plan) {
    let result = (|| -> Result<Option<String>> {
        let helper = std::env::current_exe()?.canonicalize()?;
        issue_at(
            plan,
            &helper,
            &super::transaction::production_trust()?,
            chrono::Utc::now().timestamp(),
            super::transaction::verify_executable,
        )
    })();
    match result {
        Ok(Some(token)) => {
            command.arg(RESTART_FLAG).arg(token);
        }
        Ok(None) => {}
        Err(error) => {
            // A paused continuation must not roll back a successfully installed manager.
            // The fresh manager can still offer an ordinary explicit update.
            eprintln!("Automatic bridge update paused: {error:#}");
        }
    }
}
fn take_at(
    root: &Path,
    target: &Path,
    executable: &Path,
    token: Option<&str>,
    now: i64,
) -> Result<Option<ResumePermit>> {
    let _guard = guard(root)?;
    let Some(record) = read_state(root)? else {
        ensure!(token.is_none(), "No verified bridge restart remains");
        return Ok(None);
    };
    fs::remove_file(state_path(root))?;
    let Some(token) = token else {
        return Ok(None);
    };
    ensure!(
        now < record.journey.expires
            && record.journey.expires.saturating_sub(now) <= CONSENT_SECONDS,
        "Bridge consent expired"
    );
    ensure!(
        record.root == root.canonicalize()?
            && record.target == target.canonicalize()?
            && record.token_hash.as_deref()
                == Some(format!("{:x}", Sha256::digest(token.as_bytes())).as_str())
            && record.installed_hash.as_deref() == Some(files::hash(executable)?.as_str()),
        "Bridge restart is not authorized"
    );
    Ok(Some(ResumePermit {
        journey: record.journey,
    }))
}
pub fn restart_requested() -> bool {
    std::env::args().any(|a| a == RESTART_FLAG)
}
pub(crate) fn take_restart(root: &Path) -> Result<Option<ResumePermit>> {
    let args: Vec<_> = std::env::args().collect();
    let token = restart_token(&args)?;
    let executable = std::env::current_exe()?.canonicalize()?;
    #[cfg(not(target_os = "macos"))]
    let target = executable.clone();
    #[cfg(target_os = "macos")]
    let target = crate::self_update::app_bundle(&executable)?;
    take_at(
        root,
        &target,
        &executable,
        token,
        chrono::Utc::now().timestamp(),
    )
}
fn restart_token(args: &[String]) -> Result<Option<&str>> {
    let mut flags = args
        .iter()
        .enumerate()
        .filter(|(_, a)| a.as_str() == RESTART_FLAG);
    let Some((index, _)) = flags.next() else {
        return Ok(None);
    };
    ensure!(flags.next().is_none(), "Duplicate bridge restart proof");
    let token = args
        .get(index + 1)
        .context("Missing bridge restart proof")?;
    ensure!(
        token.len() == 32 && token.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid bridge restart proof"
    );
    Ok(Some(token))
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Journey {
    envelope: Vec<u8>,
    source: ClientProfile,
    offer: UpdateOffer,
    expires: i64,
}

impl Journey {
    pub(crate) fn approval_expires(&self) -> i64 {
        self.expires
    }
    pub(crate) fn validate_selection(
        &self,
        selection: &SignedSelection,
        current: &ClientProfile,
        trust: &TrustContext,
        now: i64,
    ) -> Result<()> {
        self.verified(trust, now)?;
        ensure!(
            self.envelope == selection.catalog.envelope()
                && serde_json::to_value(&self.offer)? == serde_json::to_value(&selection.offer)?
                && serde_json::to_value(&self.source)? == serde_json::to_value(current)?,
            "Bridge selection changed after approval"
        );
        Ok(())
    }
    pub fn approve(
        selection: &SignedSelection,
        source: &ClientProfile,
        trust: &TrustContext,
        now: i64,
    ) -> Result<Self> {
        let catalog = verify_catalog(selection.catalog.envelope(), trust, now, None)?;
        validate_offer(&catalog, source, &selection.offer)?;
        ensure!(selection.offer.route_ids.len() > 1, "Not a bridge journey");
        Ok(Self {
            envelope: catalog.envelope().to_vec(),
            source: source.clone(),
            offer: selection.offer.clone(),
            expires: catalog
                .catalog()
                .expires
                .min(now.saturating_add(CONSENT_SECONDS)),
        })
    }
    pub fn has_more_hops(&self) -> bool {
        self.offer.route_ids.len() > 1
    }
    fn verified(&self, trust: &TrustContext, now: i64) -> Result<super::catalog::VerifiedCatalog> {
        ensure!(
            now < self.expires && self.expires.saturating_sub(now) <= CONSENT_SECONDS,
            "Bridge consent expired"
        );
        let catalog = verify_catalog(&self.envelope, trust, now, None)?;
        validate_offer(&catalog, &self.source, &self.offer)?;
        Ok(catalog)
    }
    pub fn next(
        &self,
        selection: &SignedSelection,
        current: &ClientProfile,
        trust: &TrustContext,
        now: i64,
    ) -> Result<Self> {
        ensure!(self.has_more_hops(), "Bridge journey is complete");
        let approved = self.verified(trust, now)?;
        let fresh = verify_catalog(selection.catalog.envelope(), trust, now, None)?;
        validate_offer(&fresh, current, &selection.offer)?;
        ensure!(
            fresh.catalog().sequence >= approved.catalog().sequence,
            "Bridge catalog moved backward"
        );
        let packages =
            |c: &Catalog,
             offer: &UpdateOffer|
             -> Result<Vec<(serde_json::Value, serde_json::Value, serde_json::Value)>> {
                offer
                    .route_ids
                    .iter()
                    .map(|id| {
                        let route = c
                            .routes
                            .iter()
                            .find(|r| &r.id == id)
                            .context("Missing bridge route")?;
                        let package = c
                            .packages
                            .iter()
                            .find(|p| p.id == route.target)
                            .context("Missing bridge package")?;
                        Ok((
                            serde_json::to_value(route)?,
                            serde_json::to_value(package)?,
                            serde_json::to_value(c.release(package))?,
                        ))
                    })
                    .collect()
            };
        let c = approved.catalog();
        let immediate = c
            .packages
            .iter()
            .find(|p| p.id == self.offer.immediate_package_id)
            .context("Missing intermediate package")?;
        ensure!(
            current.repository == self.source.repository
                && current.channel == self.source.channel
                && current.platform == self.source.platform
                && current.architecture == self.source.architecture
                && current.installation == self.source.installation
                && current.position().key()? == c.position(immediate).key()?,
            "Bridge installation scope changed"
        );
        ensure!(
            packages(c, &self.offer)?[1..] == packages(fresh.catalog(), &selection.offer)?,
            "Bridge route, destination or packages changed; approval is required"
        );
        Ok(Self {
            envelope: fresh.envelope().to_vec(),
            source: current.clone(),
            offer: selection.offer.clone(),
            expires: self.expires.min(fresh.catalog().expires),
        })
    }
}

#[cfg(test)]
#[path = "bridge/tests.rs"]
mod tests;
