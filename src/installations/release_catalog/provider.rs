use super::*;
use anyhow::Result;
use serde::de::DeserializeOwned;
use std::{
    path::Path,
    sync::{Arc, OnceLock},
};
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum Query {
    History,
    Latest,
    Tag(String),
}
pub(super) fn catalog_repository(repo: &str) -> bool {
    crate::catalog::known_entries().iter().any(|entry| {
        entry.release
            && !entry.repository.contains('/')
            && crate::catalog::repository_slug(&entry.repository).eq_ignore_ascii_case(repo)
    })
}
pub fn source() -> &'static Source {
    static SOURCE: OnceLock<Source> = OnceLock::new();
    SOURCE.get_or_init(|| {
        let bundled = include_str!("../../../assets/app-release-catalog/source.json");
        #[cfg(feature = "app-catalog-test-source")]
        if let Some(test_source) = option_env!("CRAFT_APP_CATALOG_TEST_SOURCE_JSON") {
            return serde_json::from_str(test_source).expect("Invalid test app catalog source");
        }
        let mut source: Source =
            serde_json::from_str(bundled).expect("Invalid bundled app catalog source");
        source.enabled &= env!("CRAFT_APP_RELEASE_CATALOG_ENABLED") == "true";
        source
    })
}

/// Called on the discovery worker; the UI only sees already verified dates.
pub(crate) fn refresh_discovery_dates(root: &Path) -> std::collections::BTreeMap<String, i64> {
    if source().enabled {
        if let Ok(transport) = client::Http::new() {
            if let Ok(snapshot) = client::refresh_index(root, source(), &transport) {
                return snapshot.discovery_added_at.clone();
            }
        }
    }
    store::peek(root)
        .map(|snapshot| snapshot.discovery_added_at.clone())
        .unwrap_or_default()
}
pub fn endpoint(url: &str) -> Option<(String, Query)> {
    let url = reqwest::Url::parse(url).ok()?;
    if url.scheme() != "https"
        || url.host_str() != Some("api.github.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let parts: Vec<_> = url.path().trim_matches('/').split('/').collect();
    let (owner, repo, query) = match parts.as_slice() {
        ["repos", o, r, "releases"] if url.query().is_none_or(|q| q == "per_page=100") => {
            (*o, *r, Query::History)
        }
        ["repos", o, r, "releases", "latest"] if url.query().is_none() => (*o, *r, Query::Latest),
        ["repos", o, r, "releases", "tags", tag] if url.query().is_none() => {
            let tag = decode_tag(tag)?;
            (*o, *r, Query::Tag(tag))
        }
        _ => return None,
    };
    Some((
        verification::canonical_repo(&format!("{owner}/{repo}")).ok()?,
        query,
    ))
}
pub(crate) fn value(
    snapshot: &store::Snapshot,
    repo: &str,
    query: &Query,
) -> Option<serde_json::Value> {
    let checked = snapshot
        .query_checked
        .get(&(repo.to_owned(), query.clone()))
        .or_else(|| snapshot.checked.get(repo))
        .copied()?;
    if snapshot.blocked.contains(&(repo.to_owned(), query.clone())) {
        return None;
    }
    if !resolution::eligible(checked, chrono::Utc::now().timestamp()) {
        return None;
    }
    let app = snapshot.apps.get(repo)?;
    match query {
        Query::History => serde_json::to_value(&app.releases).ok(),
        Query::Latest => serde_json::to_value(
            app.releases
                .iter()
                .find(|r| Some(&r.tag_name) == app.latest_tag.as_ref())?,
        )
        .ok(),
        Query::Tag(tag) => {
            serde_json::to_value(app.releases.iter().find(|r| r.tag_name == *tag)?).ok()
        }
    }
}
pub fn resolve<T: DeserializeOwned>(
    root: &Path,
    url: &str,
    fallback: impl FnOnce() -> Result<T>,
) -> Result<T> {
    if !endpoint(url).is_some_and(|(repo, _)| catalog_repository(&repo)) {
        return fallback();
    }
    if !source().enabled {
        if store::peek(root).is_none() {
            if let Ok(_lock) = store::lock(root) {
                if let Ok(state) = store::load(root, source()) {
                    if let Ok(snapshot) = store::snapshot(&state, source()) {
                        store::publish_memory(root, snapshot);
                    }
                }
            }
        }
        return fallback();
    }
    if let Some((repo, query)) = endpoint(url) {
        let fingerprint = verification::hash(&serde_json::to_vec(source())?);
        let snapshot = store::peek(root)
            .filter(|s| s.source == fingerprint)
            .or_else(|| {
                let _lock = store::lock(root).ok()?;
                let state = store::load(root, source()).ok()?;
                let snapshot = store::snapshot(&state, source()).ok()?;
                store::publish_memory(root, snapshot);
                store::peek(root)
            });
        if let Some(snapshot) = snapshot {
            if !matches!(query, Query::Latest)
                || chrono::Utc::now()
                    .timestamp()
                    .saturating_sub(snapshot.attempted)
                    < 60
            {
                if let Some(value) =
                    value(&snapshot, &repo, &query).and_then(|v| serde_json::from_value(v).ok())
                {
                    refresh_background(root, &repo, snapshot.attempted, &snapshot.epoch);
                    return Ok(value);
                }
            }
        }
    }
    match client::Http::new() {
        Ok(transport) => resolve_with(root, url, source(), &transport, fallback),
        Err(_) => fallback(),
    }
}
pub fn peek_release(
    root: &Path,
    repo: &str,
    tag: Option<&str>,
) -> Option<Arc<crate::model::Release>> {
    let repo = verification::canonical_repo(repo).ok()?;
    if !catalog_repository(&repo) {
        return None;
    }
    let snapshot = store::peek(root)?;
    let query = tag.map_or(Query::Latest, |tag| Query::Tag(tag.into()));
    let checked = snapshot
        .query_checked
        .get(&(repo.clone(), query.clone()))
        .or_else(|| snapshot.checked.get(&repo))
        .copied()?;
    if snapshot.blocked.contains(&(repo.clone(), query)) {
        return None;
    }
    if !resolution::eligible(checked, chrono::Utc::now().timestamp()) {
        return None;
    }
    let app = snapshot.apps.get(&repo)?;
    let tag = tag.or(app.latest_tag.as_deref())?;
    let index = *snapshot.tags.get(&(repo.clone(), tag.into()))?;
    Some(Arc::new(app.releases.get(index)?.clone()))
}
pub fn resolve_with<T: DeserializeOwned>(
    root: &Path,
    url: &str,
    source: &Source,
    transport: &dyn client::Transport,
    fallback: impl FnOnce() -> Result<T>,
) -> Result<T> {
    if source.enabled {
        if let Some((repo, query)) = endpoint(url).filter(|(repo, _)| catalog_repository(repo)) {
            if let Ok(snapshot) = client::refresh(root, source, &repo, transport) {
                if let Some(value) = value(&snapshot, &repo, &query) {
                    if let Ok(value) = serde_json::from_value(value) {
                        return Ok(value);
                    }
                }
            }
            // Only the API cache knows when data was actually received. Merely
            // taking this fallback may reuse an old response and proves nothing
            // about it being newer than the signed catalog.
            return fallback();
        }
    }
    fallback()
}

pub(crate) fn decode_tag(tag: &str) -> Option<String> {
    let bytes = tag.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            out.push(u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

pub fn generation(root: &Path, repo: &str) -> Option<String> {
    let repo = verification::canonical_repo(repo).ok()?;
    if !catalog_repository(&repo) {
        return None;
    }
    let snapshot = store::peek(root)?;
    snapshot.generation.get(&repo).cloned()
}
fn refresh_background(root: &Path, repo: &str, last: i64, epoch: &str) {
    if chrono::Utc::now().timestamp().saturating_sub(last) < 60 {
        return;
    }
    use std::sync::Mutex;
    static ACTIVE: OnceLock<Mutex<std::collections::BTreeSet<std::path::PathBuf>>> =
        OnceLock::new();
    let root = store::root_id(root);
    let repo = repo.to_owned();
    let mut active = ACTIVE.get_or_init(Default::default).lock().unwrap();
    if active.len() >= 32 || !active.insert(root.clone()) {
        return;
    }
    drop(active);
    let epoch = epoch.to_owned();
    std::thread::spawn(move || {
        struct Clear(std::path::PathBuf);
        impl Drop for Clear {
            fn drop(&mut self) {
                ACTIVE.get().unwrap().lock().unwrap().remove(&self.0);
            }
        }
        let _clear = Clear(root.clone());
        if let Ok(transport) = client::Http::new() {
            let _ = client::refresh_guarded(&root, source(), &repo, &transport, Some(&epoch));
        }
    });
}
