use super::verification::{MAX_APP, MAX_ROOT};
use super::*;
use anyhow::{ensure, Context, Result};
use std::{io::Read, path::Path, time::Duration};
pub enum Reply {
    Unchanged,
    Data(Vec<u8>, Option<String>),
}
pub trait Transport {
    fn get(&self, url: &str, etag: Option<&str>, limit: usize) -> Result<Reply>;
}
pub struct Http(reqwest::blocking::Client, std::time::Instant);
impl Http {
    pub fn new() -> Result<Self> {
        Ok(Self(
            reqwest::blocking::Client::builder()
                .connect_timeout(Duration::from_secs(2))
                .timeout(Duration::from_secs(4))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent("CraftApps-AppCatalog/1")
                .build()?,
            std::time::Instant::now() + Duration::from_secs(4),
        ))
    }
}
impl Transport for Http {
    fn get(&self, url: &str, etag: Option<&str>, limit: usize) -> Result<Reply> {
        let remaining = self.1.saturating_duration_since(std::time::Instant::now());
        ensure!(!remaining.is_zero(), "Catalog refresh budget exhausted");
        let mut request = self.0.get(url).timeout(remaining);
        if let Some(etag) = etag {
            request = request.header(reqwest::header::IF_NONE_MATCH, etag);
        }
        let response = request.send()?;
        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(Reply::Unchanged);
        }
        ensure!(
            response.status().is_success(),
            "Catalog metadata request failed ({})",
            response.status()
        );
        let etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .filter(|v| v.len() <= 1024)
            .map(str::to_owned);
        let mut bytes = Vec::new();
        response.take(limit as u64 + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= limit, "Catalog response too large");
        Ok(Reply::Data(bytes, etag))
    }
}
pub fn refresh(
    root: &Path,
    source: &Source,
    repo: &str,
    transport: &dyn Transport,
) -> Result<std::sync::Arc<store::Snapshot>> {
    let epoch = store::peek(root).map(|s| s.epoch.clone());
    refresh_guarded(root, source, repo, transport, epoch.as_deref())
}
pub fn refresh_guarded(
    root: &Path,
    source: &Source,
    repo: &str,
    transport: &dyn Transport,
    epoch: Option<&str>,
) -> Result<std::sync::Arc<store::Snapshot>> {
    refresh_inner(root, source, &[repo], transport, None, epoch)
}

/// Refresh shared discovery metadata without downloading any app objects.
pub fn refresh_index(
    root: &Path,
    source: &Source,
    transport: &dyn Transport,
) -> Result<std::sync::Arc<store::Snapshot>> {
    let epoch = store::peek(root).map(|snapshot| snapshot.epoch.clone());
    refresh_inner(root, source, &[], transport, None, epoch.as_deref())
}

pub(super) fn refresh_batch(
    root: &Path,
    source: &Source,
    repos: &[&str],
    transport: &(dyn Transport + Sync),
) -> Result<std::sync::Arc<store::Snapshot>> {
    let epoch = store::peek(root).map(|snapshot| snapshot.epoch.clone());
    refresh_inner(
        root,
        source,
        repos,
        transport,
        Some(transport),
        epoch.as_deref(),
    )
}

fn refresh_inner(
    root: &Path,
    source: &Source,
    repos: &[&str],
    transport: &dyn Transport,
    batch_transport: Option<&(dyn Transport + Sync)>,
    epoch: Option<&str>,
) -> Result<std::sync::Arc<store::Snapshot>> {
    let _lock = store::lock(root)?;
    let mut state = store::load(root, source)?;
    if epoch.is_some_and(|expected| expected != state.epoch) {
        store::publish_memory(root, store::snapshot(&state, source)?);
        anyhow::bail!("Catalog request invalidated by cache clear or source change");
    }

    let now = chrono::Utc::now().timestamp();
    let previous_envelope = state.envelope.clone();
    let previous_etag = state.etag.clone();
    let cooldown = if state.failures == 0 {
        60
    } else {
        match state.failures {
            1 => 300,
            2 => 900,
            _ => 3600,
        }
    };
    let attempt = source.enabled && now.saturating_sub(state.attempted) >= cooldown;
    if attempt {
        state.attempted = now;
        let update = (|| -> Result<()> {
            let url = source
                .index_url
                .as_deref()
                .context("Catalog endpoint is not configured")?;
            let parsed = reqwest::Url::parse(url)?;
            ensure!(
                parsed.scheme() == "https"
                    && parsed.username().is_empty()
                    && parsed.password().is_none()
                    && parsed.query().is_none()
                    && parsed.fragment().is_none()
                    && parsed.port().is_none()
                    && parsed.path().ends_with("/index.json"),
                "Invalid catalog endpoint"
            );
            let old = state
                .envelope
                .as_ref()
                .and_then(|e| verify_index(e.as_bytes(), source, 0).ok());
            match transport.get(url, state.etag.as_deref(), MAX_ROOT)? {
                Reply::Unchanged => ensure!(old.is_some(), "304 without verified catalog"),
                Reply::Data(bytes, etag) => {
                    let verified =
                        verify_index(&bytes, source, old.as_ref().map_or(0, |v| v.index.sequence))?;
                    if let Some(old) = &old {
                        ensure!(
                            old.index.sequence != verified.index.sequence
                                || old.envelope.payload == verified.envelope.payload,
                            "Conflicting catalog sequence"
                        );
                    }
                    state.envelope = Some(String::from_utf8(bytes)?);
                    state.etag = etag;
                }
            }
            Ok(())
        })();
        state.failures = if update.is_ok() {
            0
        } else {
            state.failures.saturating_add(1)
        };
    }
    let load = load_objects(
        &mut state,
        source,
        repos,
        transport,
        batch_transport,
        now,
        attempt,
    );
    // A failed single-app browse keeps its previous view. An automatic batch
    // keeps the verified new index and any successful objects, so one failure
    // cannot discard other repositories' results.
    if load.is_err() && batch_transport.is_none() && previous_envelope.is_some() {
        state.envelope = previous_envelope;
        state.etag = previous_etag;
        state.failures = state.failures.max(1);
    }
    for repo in repos {
        state.used.insert((*repo).into(), now);
    }
    let snapshot = store::snapshot(&state, source)?;
    store::save(root, &mut state, repos.first().copied().unwrap_or_default())?;
    store::publish_memory(root, snapshot);
    // Failed hosting is a fallback decision; never convert it into an empty release list.
    let _ = load;
    store::peek(root).context("Catalog snapshot unavailable")
}

fn load_objects(
    state: &mut store::State,
    source: &Source,
    repos: &[&str],
    transport: &dyn Transport,
    batch_transport: Option<&(dyn Transport + Sync)>,
    now: i64,
    attempted_index: bool,
) -> Result<()> {
    if repos.is_empty() {
        return Ok(());
    }
    let envelope = state.envelope.as_ref().context("No verified catalog")?;
    let index = verify_index(envelope.as_bytes(), source, 0)?.index;
    let missing: Vec<_> = repos
        .iter()
        .filter_map(|&repo| {
            index
                .repositories
                .get(repo)
                .map(|reference| (repo, reference))
        })
        .filter(|(repo, reference)| {
            (batch_transport.is_none() || resolution::eligible(reference.checked_at, now))
                && state
                    .objects
                    .get(&reference.sha256)
                    .is_none_or(|body| verify_app(body.as_bytes(), repo, reference).is_err())
        })
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let base = source
        .index_url
        .as_deref()
        .context("Missing catalog URL")?
        .strip_suffix("index.json")
        .context("Invalid catalog base")?;
    let used = &state.used;
    let fetch = |transport: &dyn Transport, (repo, reference): (&str, &AppReference)| {
        ensure!(
            source.enabled
                && (attempted_index || now.saturating_sub(*used.get(repo).unwrap_or(&0)) >= 300),
            "Catalog object retry is deferred"
        );
        let Reply::Data(bytes, _) = transport.get(
            &format!("{base}apps/{}.json", reference.sha256),
            None,
            MAX_APP,
        )?
        else {
            anyhow::bail!("304 for uncached app object");
        };
        verify_app(&bytes, repo, reference)?;
        Ok((reference.sha256.clone(), String::from_utf8(bytes)?))
    };
    let Some(batch_transport) = batch_transport else {
        for request in missing {
            let (hash, body) = fetch(transport, request)?;
            state.objects.insert(hash, body);
        }
        return Ok(());
    };
    for chunk in missing.chunks(4) {
        let fetched = std::thread::scope(|scope| {
            let workers: Vec<_> = chunk
                .iter()
                .map(|&request| scope.spawn(move || fetch(batch_transport, request)))
                .collect();
            workers
                .into_iter()
                .filter_map(|worker| worker.join().ok()?.ok())
                .collect::<Vec<_>>()
        });
        state.objects.extend(fetched);
    }
    Ok(())
}
