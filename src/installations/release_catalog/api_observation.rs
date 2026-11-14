//! Bounded evidence of actual API responses, including cache hits. A fallback
//! call is not a new observation, and identical metadata never overrides a catalog.
use super::{provider::Query, *};
use crate::model::Release;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub(super) repository: String,
    query: Query,
    pub(super) at: i64,
    hash: String,
    // A full API page is only a prefix of the complete stable release history.
    history_prefix: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    body: Option<serde_json::Value>,
    #[serde(default)]
    withdrawn: bool,
}
impl Evidence {
    pub(super) fn valid(&self) -> bool {
        self.known() && super::resolution::eligible(self.at, chrono::Utc::now().timestamp())
    }
    pub(super) fn known(&self) -> bool {
        let now = chrono::Utc::now().timestamp();
        self.at > 0
            && self.at <= now + 300
            && verification::canonical_repo(&self.repository)
                .is_ok_and(|repo| repo == self.repository)
            && verification::decode::<32>(&self.hash).is_ok()
            && self.history_prefix.is_none_or(|length| length <= 100)
            && match &self.query {
                Query::Tag(tag) => !tag.is_empty() && tag.len() <= 1024,
                _ => true,
            }
    }
    pub(super) fn matches(&self, app: &AppCatalog) -> bool {
        if self.withdrawn {
            return match &self.query {
                Query::Latest => app.latest_tag.is_none(),
                Query::Tag(tag) => !app.releases.iter().any(|r| &r.tag_name == tag),
                Query::History => app.releases.is_empty(),
            };
        }
        let hash = match &self.query {
            Query::Latest => app
                .latest_tag
                .as_ref()
                .and_then(|tag| app.releases.iter().find(|release| &release.tag_name == tag))
                .and_then(release_hash),
            Query::Tag(tag) => app
                .releases
                .iter()
                .find(|release| &release.tag_name == tag)
                .and_then(release_hash),
            Query::History => {
                let count = self.history_prefix.unwrap_or(app.releases.len());
                (app.releases.len() >= count)
                    .then(|| history_hash(&app.releases[..count]))
                    .flatten()
            }
        };
        hash.as_ref() == Some(&self.hash)
    }
}
fn normalized(release: &Release) -> Release {
    let mut release = release.clone();
    release.assets.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then(a.browser_download_url.cmp(&b.browser_download_url))
    });
    release
}
fn release_hash(release: &Release) -> Option<String> {
    serde_json::to_vec(&normalized(release))
        .ok()
        .map(|bytes| verification::hash(&bytes))
}
fn history_hash(releases: &[Release]) -> Option<String> {
    serde_json::to_vec(&releases.iter().map(normalized).collect::<Vec<_>>())
        .ok()
        .map(|bytes| verification::hash(&bytes))
}
fn evidence(url: &str, body: &serde_json::Value, at: i64) -> Option<Evidence> {
    let (repository, query) = provider::endpoint(url)?;
    if !provider::catalog_repository(&repository) {
        return None;
    }
    let (hash, history_prefix) = match &query {
        Query::History => {
            let mut releases: Vec<Release> = serde_json::from_value(body.clone()).ok()?;
            let limit = if reqwest::Url::parse(url).ok()?.query().is_some() {
                100
            } else {
                30
            };
            if releases.len() > limit {
                return None;
            }
            let partial = releases.len() == limit;
            releases.retain(|release| !release.draft && !release.prerelease);
            if partial && releases.is_empty() {
                return None;
            }
            (history_hash(&releases)?, partial.then_some(releases.len()))
        }
        _ => {
            let release: Release = serde_json::from_value(body.clone()).ok()?;
            if release.draft
                || release.prerelease
                || matches!(&query, Query::Tag(tag) if *tag != release.tag_name)
            {
                return None;
            }
            (release_hash(&release)?, None)
        }
    };
    let stored_body = if query == Query::History {
        let releases: Vec<Release> = serde_json::from_value(body.clone()).ok()?;
        serde_json::to_value(
            releases
                .into_iter()
                .filter(|r| !r.draft && !r.prerelease)
                .map(|r| normalized(&r))
                .collect::<Vec<_>>(),
        )
        .ok()?
    } else {
        serde_json::to_value(normalized(
            &serde_json::from_value::<Release>(body.clone()).ok()?,
        ))
        .ok()?
    };
    let evidence = Evidence {
        repository,
        query,
        at,
        hash,
        history_prefix,
        body: Some(stored_body),
        withdrawn: false,
    };
    evidence.valid().then_some(evidence)
}

pub(crate) fn api_epoch(root: &Path, url: &str) -> Option<String> {
    if !provider::source().enabled
        || !provider::endpoint(url).is_some_and(|(repo, _)| provider::catalog_repository(&repo))
    {
        return None;
    }
    let _lock = store::lock(root).ok()?;
    let state = store::load(root, provider::source()).ok()?;
    let epoch = state.epoch.clone();
    store::publish_memory(root, store::snapshot(&state, provider::source()).ok()?);
    Some(epoch)
}
pub(crate) fn observe_api(
    root: &Path,
    url: &str,
    body: &serde_json::Value,
    at: i64,
    epoch: Option<&str>,
) {
    if let Some(observed) = evidence(url, body, at).filter(|_| provider::source().enabled) {
        persist(root, provider::source(), observed, epoch);
    }
}
pub(crate) fn observe_withdrawal(root: &Path, url: &str, epoch: Option<&str>) {
    if !provider::source().enabled {
        return;
    }
    let Some((repository, query)) =
        provider::endpoint(url).filter(|(repo, _)| provider::catalog_repository(repo))
    else {
        return;
    };
    persist(
        root,
        provider::source(),
        Evidence {
            repository,
            query,
            at: chrono::Utc::now().timestamp(),
            hash: verification::hash(b"withdrawn"),
            history_prefix: None,
            body: None,
            withdrawn: true,
        },
        epoch,
    );
}

/// Keep an unresolved conflict even after its body ages out; never resurrect
/// catalog data known to precede a successful API observation.
pub(super) fn merge(snapshot: &mut store::Snapshot) {
    let signed = snapshot.apps.clone();
    let signed_checked = snapshot.checked.clone();
    let mut observations = snapshot.api_observations.clone();
    let mut latest_proof = std::collections::BTreeMap::<String, i64>::new();
    for evidence in &observations {
        let valid_body = evidence
            .body
            .as_ref()
            .and_then(|body| serde_json::from_value::<Release>(body.clone()).ok())
            .is_some_and(|release| {
                !release.draft
                    && !release.prerelease
                    && release_hash(&release).as_ref() == Some(&evidence.hash)
            });
        if evidence.query == Query::Latest && evidence.valid() && valid_body && !evidence.withdrawn
        {
            latest_proof
                .entry(evidence.repository.clone())
                .and_modify(|time| *time = (*time).max(evidence.at))
                .or_insert(evidence.at);
        }
    }
    observations.sort_by(|a, b| a.at.cmp(&b.at).then(a.query.cmp(&b.query)));
    for observation in observations.into_iter().filter(Evidence::known) {
        let repo = &observation.repository;
        let checked = *signed_checked.get(repo).unwrap_or(&0);
        if observation.at < checked || signed.get(repo).is_some_and(|app| observation.matches(app))
        {
            continue;
        }
        let original = snapshot.apps.entry(repo.clone()).or_insert_with(|| {
            std::sync::Arc::new(AppCatalog {
                schema: 1,
                repository: repo.clone(),
                latest_tag: None,
                releases: Vec::new(),
                scopes: Vec::new(),
            })
        });
        let app = std::sync::Arc::make_mut(original);
        let body = observation.body.as_ref().filter(|_| observation.valid());
        let validated = body.and_then(|body| {
            let url = match &observation.query {
                Query::Latest => format!("https://api.github.com/repos/{repo}/releases/latest"),
                Query::History => {
                    format!("https://api.github.com/repos/{repo}/releases?per_page=100")
                }
                Query::Tag(tag) => {
                    let mut url = reqwest::Url::parse(&format!(
                        "https://api.github.com/repos/{repo}/releases/tags/"
                    ))
                    .ok()?;
                    url.path_segments_mut().ok()?.pop_if_empty().push(tag);
                    url.to_string()
                }
            };
            evidence(&url, body, observation.at)
                .filter(|e| e.hash == observation.hash)
                .map(|_| body)
        });
        if observation.withdrawn || validated.is_none() {
            snapshot
                .blocked
                .insert((repo.clone(), observation.query.clone()));
            snapshot.blocked.insert((repo.clone(), Query::History));
            match &observation.query {
                Query::Latest => {}
                Query::Tag(tag) => {
                    if app.latest_tag.as_ref() == Some(tag) {
                        snapshot.blocked.insert((repo.clone(), Query::Latest));
                    }
                }
                Query::History => {
                    snapshot.blocked.insert((repo.clone(), Query::Latest));
                    for release in &app.releases {
                        snapshot
                            .blocked
                            .insert((repo.clone(), Query::Tag(release.tag_name.clone())));
                    }
                }
            }
            if observation.withdrawn {
                match &observation.query {
                    Query::Tag(tag) => {
                        app.releases.retain(|r| &r.tag_name != tag);
                        if app.latest_tag.as_ref() == Some(tag) {
                            app.latest_tag = None;
                        }
                    }
                    Query::Latest => app.latest_tag = None,
                    Query::History => {
                        app.releases.clear();
                        app.latest_tag = None;
                    }
                }
            }
        } else if let Some(body) = validated {
            match &observation.query {
                Query::History => {
                    let mut releases: Vec<Release> =
                        serde_json::from_value(body.clone()).unwrap_or_default();
                    releases.retain(|r| !r.draft && !r.prerelease);
                    for release in &releases {
                        snapshot.query_checked.insert(
                            (repo.clone(), Query::Tag(release.tag_name.clone())),
                            observation.at,
                        );
                    }
                    if observation.history_prefix.is_some()
                        && super::resolution::eligible(checked, chrono::Utc::now().timestamp())
                    {
                        let tags: std::collections::BTreeSet<_> =
                            releases.iter().map(|r| r.tag_name.clone()).collect();
                        releases.extend(
                            app.releases
                                .iter()
                                .filter(|r| !tags.contains(&r.tag_name))
                                .cloned(),
                        );
                    }
                    app.releases = releases;
                    if app
                        .latest_tag
                        .as_ref()
                        .is_some_and(|tag| !app.releases.iter().any(|r| &r.tag_name == tag))
                    {
                        app.latest_tag = None;
                    }
                }
                Query::Latest | Query::Tag(_) => {
                    if let Ok(release) = serde_json::from_value::<Release>(body.clone()) {
                        if matches!(observation.query, Query::Tag(_))
                            && latest_proof
                                .get(repo)
                                .is_none_or(|time| *time < observation.at)
                            && !app.releases.iter().any(|r| r.tag_name == release.tag_name)
                        {
                            // Discovering a tag does not establish GitHub's latest selection.
                            snapshot.blocked.insert((repo.clone(), Query::Latest));
                        }
                        app.releases.retain(|r| r.tag_name != release.tag_name);
                        if observation.query == Query::Latest {
                            app.latest_tag = Some(release.tag_name.clone());
                        }
                        app.releases.insert(0, release);
                    }
                }
            }
            snapshot
                .blocked
                .remove(&(repo.clone(), observation.query.clone()));
            snapshot
                .query_checked
                .insert((repo.clone(), observation.query.clone()), observation.at);
            if let Query::Latest = observation.query {
                // A latest response proves only that release. It may join an
                // eligible signed history, but cannot renew unchecked old rows.
                if let Some(tag) = &app.latest_tag {
                    snapshot
                        .query_checked
                        .insert((repo.clone(), Query::Tag(tag.clone())), observation.at);
                }
            }
        }
        snapshot.generation.insert(
            repo.clone(),
            format!("api:{}:{}", observation.at, observation.hash),
        );
    }
}

pub(super) fn trim(observations: &mut Vec<Evidence>) {
    observations.retain(Evidence::known);
    observations.sort_by_key(|a| std::cmp::Reverse(a.at));
    let mut seen = std::collections::BTreeSet::new();
    observations.retain(|e| seen.insert((e.repository.clone(), e.query.clone())));
    let mut repositories = std::collections::BTreeSet::new();
    let mut count = 0;
    let mut bytes = 0;
    for observation in observations.iter_mut() {
        repositories.insert(observation.repository.clone());
        let size = observation
            .body
            .as_ref()
            .and_then(|body| serde_json::to_vec(body).ok())
            .map_or(0, |body| body.len());
        if count >= 256
            || bytes + size > 8 * 1024 * 1024
            || repositories.len() > 128
            || !observation.valid()
        {
            observation.body = None;
        }
        if observation.body.is_some() {
            count += 1;
            bytes += size;
        }
    }
}
pub(super) fn retire_resolved(state: &mut store::State) {
    // load() verified these successful scan timestamps. Keep matching API
    // latest proofs until that scan catches up, so tag lookups cannot erase them.
    let Some(index) = state
        .envelope
        .as_ref()
        .and_then(|raw| serde_json::from_str::<Envelope>(raw).ok())
        .and_then(|envelope| serde_json::from_str::<Index>(&envelope.payload).ok())
    else {
        return;
    };
    state.api_observations.retain(|observation| {
        let Some(reference) = index.repositories.get(&observation.repository) else {
            return true;
        };
        reference.checked_at <= observation.at
    });
}
#[cfg(test)]
pub(super) fn observe_with(
    root: &Path,
    source: &Source,
    url: &str,
    body: &serde_json::Value,
    at: i64,
) {
    if !source.enabled {
        return;
    }
    let Some(observed) = evidence(url, body, at) else {
        return;
    };
    persist(root, source, observed, None);
}
pub(super) fn persist(root: &Path, source: &Source, observed: Evidence, epoch: Option<&str>) {
    // A normal matching response (including repeated RAM/disk hits) needs no
    // disk write. Keep real conflicts until a newer API response or signed scan.
    let fingerprint = verification::hash(&serde_json::to_vec(source).unwrap_or_default());
    if let Some(snapshot) = store::peek(root).filter(|snapshot| snapshot.source == fingerprint) {
        if let Some(previous) = snapshot.api_observations.iter().find(|previous| {
            previous.repository == observed.repository && previous.query == observed.query
        }) {
            if previous.at >= observed.at {
                return;
            }
        } else if snapshot
            .apps
            .get(&observed.repository)
            .is_some_and(|app| observed.matches(app))
            && !snapshot
                .blocked
                .contains(&(observed.repository.clone(), observed.query.clone()))
        {
            return;
        }
    }
    let _ = (|| -> anyhow::Result<()> {
        let _lock = store::lock(root)?;
        let mut state = store::load(root, source)?;
        if epoch.is_some_and(|expected| expected != state.epoch) {
            return Ok(());
        }
        if let Some(previous) = state.api_observations.iter_mut().find(|previous| {
            previous.repository == observed.repository && previous.query == observed.query
        }) {
            if previous.at >= observed.at {
                return Ok(());
            }
            *previous = observed.clone();
        } else {
            state.api_observations.push(observed.clone());
        }
        store::save(root, &mut state, &observed.repository)?;
        store::publish_memory(root, store::snapshot(&state, source)?);
        Ok(())
    })(); // Cache observation must never turn a successful API response into an error.
}
