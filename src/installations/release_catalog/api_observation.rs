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
}
impl Evidence {
    pub(super) fn valid(&self) -> bool {
        let now = chrono::Utc::now().timestamp();
        self.at > 0
            && self.at <= now + 300
            && now.saturating_sub(self.at) <= 86400
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
    let evidence = Evidence {
        repository,
        query,
        at,
        hash,
        history_prefix,
    };
    evidence.valid().then_some(evidence)
}

pub(crate) fn observe_api(root: &Path, url: &str, body: &serde_json::Value, at: i64) {
    observe_with(root, provider::source(), url, body, at);
}
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
        {
            return;
        }
    }
    let _ = (|| -> anyhow::Result<()> {
        let _lock = store::lock(root)?;
        let mut state = store::load(root, source)?;
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
