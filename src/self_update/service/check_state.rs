use super::{
    catalog::{verify_catalog, TrustContext},
    model::*,
    routing::select_update,
    transport::*,
    trust::{lock_name, TrustStore},
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub use super::api::CheckTrigger;
#[derive(Default, Serialize, Deserialize)]
struct State {
    #[serde(default)]
    bytes: Vec<u8>,
    etag: Option<String>,
    endpoint: Option<String>,
    checked: i64,
    retry: i64,
    failures: u32,
}
pub struct CheckStore {
    root: PathBuf,
}
pub struct CheckPermit {
    _lock: crate::platform::Lock,
}
impl CheckStore {
    pub fn new(root: &Path) -> Self {
        Self { root: root.into() }
    }
    pub fn begin(&self, _now: i64, _trigger: CheckTrigger) -> Result<CheckPermit> {
        std::fs::create_dir_all(&self.root)?;
        let root = self.root.canonicalize()?;
        Ok(CheckPermit {
            _lock: crate::platform::Lock::take(&lock_name(
                "Check",
                root.to_string_lossy().as_bytes(),
            ))?,
        })
    }
}
pub struct CheckContext<'a> {
    pub root: &'a Path,
    pub trust: &'a TrustContext,
    pub endpoints: &'a [ValidatedEndpoint],
    pub transport: &'a dyn FetchMetadata,
    pub now: i64,
    pub trigger: CheckTrigger,
}
fn cache_path(root: &Path, trust: &TrustContext) -> PathBuf {
    root.join("runtime/manager-updates").join(format!(
        "{}.json",
        lock_name("Cache", format!("{}\n{}", trust.repository, trust.channel))
            .trim_start_matches("Local\\")
    ))
}
pub fn verified_cached(
    root: &Path,
    trust: &TrustContext,
    now: i64,
) -> Result<super::catalog::VerifiedCatalog> {
    let prior = TrustStore::new(root, &trust.repository, &trust.channel).load()?;
    let path = cache_path(root, trust);
    crate::files::no_links(&path)?;
    ensure!(
        std::fs::metadata(&path)?.len() <= MAX_ENVELOPE as u64 * 5,
        "Updater cache too large"
    );
    let state: State = serde_json::from_slice(&std::fs::read(path)?)?;
    verify_catalog(&state.bytes, trust, now, prior.as_ref())
}
pub fn check(profile: &ClientProfile, context: &CheckContext<'_>) -> CheckOutcome {
    inner(profile, context).unwrap_or_else(|e| CheckOutcome::CheckFailed(format!("{e:#}")))
}
fn inner(profile: &ClientProfile, ctx: &CheckContext<'_>) -> Result<CheckOutcome> {
    let _permit = CheckStore::new(ctx.root).begin(ctx.now, ctx.trigger)?;
    ensure!(!ctx.endpoints.is_empty(), "No updater endpoint configured");
    let trust = TrustStore::new(ctx.root, &ctx.trust.repository, &ctx.trust.channel);
    let prior = trust.load()?;
    let path = cache_path(ctx.root, ctx.trust);
    let mut state: State = if path.exists() {
        crate::files::no_links(&path)?;
        let bytes = std::fs::read(&path)?;
        ensure!(bytes.len() <= MAX_ENVELOPE * 5, "Updater cache too large");
        serde_json::from_slice(&bytes).context("Corrupt updater cache")?
    } else {
        State::default()
    };
    let cached = || verify_catalog(&state.bytes, ctx.trust, ctx.now, prior.as_ref());
    let interval = match ctx.trigger {
        CheckTrigger::Periodic(mins) => mins.clamp(10, 60) * 60,
        _ => 60,
    } as i64;
    // A backward clock cannot suppress requests indefinitely or authorize expired bytes.
    if !matches!(ctx.trigger, CheckTrigger::BridgeRestart)
        && (ctx.now >= state.checked && ctx.now.saturating_sub(state.checked) < interval
            || ctx.now < state.retry && state.retry.saturating_sub(ctx.now) <= 3600)
        && !state.bytes.is_empty()
    {
        if let Ok(catalog) = cached() {
            return select_update(&catalog, profile);
        }
    }
    if ctx.now < state.retry && state.retry.saturating_sub(ctx.now) <= 3600 {
        anyhow::bail!("Update service temporarily unavailable; retry later");
    }
    let mut failure = "Update service unavailable".to_owned();
    let mut retry = 0;
    for endpoint in ctx.endpoints {
        let etag =
            if state.endpoint.as_deref() == Some(endpoint.as_str()) && !state.bytes.is_empty() {
                state.etag.as_deref()
            } else {
                None
            };
        match ctx.transport.fetch_for_profile(endpoint, etag, profile) {
            Ok(FetchResponse::RateLimited(seconds)) => {
                retry = retry.max(seconds.clamp(1, 3600));
                failure = "Update service rate limit; retry later".into();
            }
            Err(error) => failure = format!("{error:#}"),
            Ok(response) => {
                let (bytes, etag) = match response {
                    FetchResponse::Body { bytes, etag } => (bytes, etag),
                    FetchResponse::NotModified => {
                        ensure!(etag.is_some(), "Unexpected metadata 304");
                        (state.bytes.clone(), state.etag.clone())
                    }
                    FetchResponse::RateLimited(_) => unreachable!(),
                };
                // Any trust failure ends this check: never try a weaker mirror/legacy path.
                #[derive(Deserialize)]
                struct DynamicResponse {
                    envelope: crate::update_revisions::Envelope,
                    selection: DynamicSelection,
                }
                #[derive(Deserialize)]
                struct DynamicSelection {
                    outcome: String,
                    offer: Option<UpdateOffer>,
                }
                let dynamic = if endpoint.is_dynamic() {
                    Some(serde_json::from_slice::<DynamicResponse>(&bytes)?)
                } else {
                    None
                };
                let bytes = if let Some(response) = &dynamic {
                    serde_json::to_vec(&response.envelope)?
                } else {
                    bytes
                };
                let verified = verify_catalog(&bytes, ctx.trust, ctx.now, prior.as_ref())?;
                let outcome = select_update(&verified, profile)?;
                if let Some(response) = dynamic {
                    let (kind, offer) = match &outcome {
                        CheckOutcome::Current => ("current", None),
                        CheckOutcome::DirectUpdate(o) => ("direct", Some(o)),
                        CheckOutcome::BridgeUpdate(o) => ("bridge", Some(o)),
                        CheckOutcome::ManualRequired(_) => ("manual", None),
                        CheckOutcome::UnsupportedPlatform(_) => ("unsupported", None),
                        _ => ("external", None),
                    };
                    ensure!(
                        response.selection.outcome == kind
                            && response.selection.offer.as_ref() == offer,
                        "Unsigned server selection disagrees with signed policy"
                    );
                }
                trust.accept(&verified)?;
                state = State {
                    bytes,
                    etag,
                    endpoint: Some(endpoint.as_str().into()),
                    checked: ctx.now,
                    retry: 0,
                    failures: 0,
                };
                crate::files::write_bytes(&path, &serde_json::to_vec(&state)?)?;
                return Ok(outcome);
            }
        }
    }
    state.failures = state.failures.saturating_add(1);
    let backoff = (60u64 << state.failures.saturating_sub(1).min(6)).min(3600);
    state.retry = ctx.now.saturating_add(retry.max(backoff) as i64);
    crate::files::write_bytes(&path, &serde_json::to_vec(&state)?)?;
    anyhow::bail!("{failure}")
}
