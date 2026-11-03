use super::verification::{hash, MAX_APP};
use super::*;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};
const MAX_DISK: u64 = 16 * 1024 * 1024;
const MAX_DECODED: u64 = 128 * 1024 * 1024;
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    #[serde(default = "new_epoch")]
    pub epoch: String,
    pub schema: u32,
    pub source: String,
    pub envelope: Option<String>,
    pub objects: BTreeMap<String, String>,
    pub used: BTreeMap<String, i64>,
    pub etag: Option<String>,
    pub attempted: i64,
    pub failures: u32,
    pub overrides: BTreeMap<String, i64>,
    pub observations: Vec<Observation>,
    #[serde(default)]
    pub api_observations: Vec<super::api_observation::Evidence>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub repository: String,
    pub evidence: ScopeEvidence,
    pub used: i64,
}
#[derive(Clone, Default)]
pub struct Snapshot {
    pub epoch: String,
    pub source: String,
    pub attempted: i64,
    pub discovery_added_at: BTreeMap<String, i64>,
    pub generation: BTreeMap<String, String>,
    pub apps: BTreeMap<String, Arc<AppCatalog>>,
    pub checked: BTreeMap<String, i64>,
    pub overrides: BTreeMap<String, i64>,
    pub observations: Vec<Observation>,
    pub api_observations: Vec<super::api_observation::Evidence>,
    pub tags: std::collections::HashMap<(String, String), usize>,
    pub scope:
        std::collections::HashMap<(String, String, String, String, u64, String), ScopePolicy>,
}
fn memory() -> &'static Mutex<BTreeMap<PathBuf, Arc<Snapshot>>> {
    static MEMORY: OnceLock<Mutex<BTreeMap<PathBuf, Arc<Snapshot>>>> = OnceLock::new();
    MEMORY.get_or_init(Default::default)
}
pub fn root_id(root: &Path) -> PathBuf {
    let path = std::path::absolute(root).unwrap_or_else(|_| root.into());
    #[cfg(target_os = "macos")]
    let path = {
        // Existing and not-yet-created paths under /var and /private/var must
        // contend on the same process lock and share the same memory snapshot.
        let mut ancestor = path.as_path();
        let mut tail = Vec::new();
        loop {
            if let Ok(mut physical) = std::fs::canonicalize(ancestor) {
                for component in tail.iter().rev() {
                    physical.push(component);
                }
                break physical;
            }
            match (ancestor.file_name(), ancestor.parent()) {
                (Some(name), Some(parent)) => {
                    tail.push(name.to_owned());
                    ancestor = parent;
                }
                _ => break path,
            }
        }
    };
    #[cfg(windows)]
    {
        PathBuf::from(path.to_string_lossy().to_ascii_lowercase())
    }
    #[cfg(not(windows))]
    {
        path
    }
}
pub fn lock(root: &Path) -> Result<crate::platform::Lock> {
    crate::platform::Lock::take(&format!(
        "Local\\CraftAppReleaseCatalog-{}",
        hash(
            root_id(root)
                .to_string_lossy()
                .to_ascii_lowercase()
                .as_bytes()
        )
    ))
}
pub fn peek(root: &Path) -> Option<Arc<Snapshot>> {
    memory().try_lock().ok()?.get(&root_id(root)).cloned()
}
pub fn publish_memory(root: &Path, value: Snapshot) {
    let mut cache = memory().lock().unwrap();
    if cache.len() >= 32 && !cache.contains_key(&root_id(root)) {
        cache.clear();
    }
    cache.insert(root_id(root), Arc::new(value));
}
fn file(root: &Path) -> PathBuf {
    root.join("runtime/app-release-catalog/cache.json.gz")
}
pub fn load(root: &Path, source: &Source) -> Result<State> {
    let fingerprint = hash(&serde_json::to_vec(source)?);
    // A freshly initialized snapshot may not have been saved yet. Keep its
    // generation stable until clear() removes it, rather than invalidating the
    // first real request as though another process had cleared the cache.
    let epoch = memory()
        .lock()
        .unwrap()
        .get(&root_id(root))
        .filter(|snapshot| snapshot.source == fingerprint)
        .map(|snapshot| snapshot.epoch.clone())
        .unwrap_or_else(new_epoch);
    let mut state = State {
        epoch,
        schema: 1,
        source: fingerprint.clone(),
        ..Default::default()
    };
    let path = file(root);
    check_path(root)?;
    if path.is_file() {
        let read = (|| -> Result<State> {
            crate::files::no_links(&path)?;
            ensure!(
                std::fs::metadata(&path)?.len() <= MAX_DISK,
                "Catalog cache too large"
            );
            let mut bytes = Vec::new();
            flate2::read::GzDecoder::new(std::fs::File::open(&path)?)
                .take(MAX_DECODED + 1)
                .read_to_end(&mut bytes)?;
            ensure!(bytes.len() as u64 <= MAX_DECODED, "Decoded cache too large");
            let cached: State = serde_json::from_slice(&bytes)?;
            ensure!(
                cached.schema == 1
                    && cached.source == fingerprint
                    && cached.objects.len() <= 128
                    && cached.observations.len() <= 2048
                    && cached.api_observations.len() <= 512,
                "Invalid cache schema/source/bounds"
            );
            if let Some(envelope) = &cached.envelope {
                verify_index(envelope.as_bytes(), source, 0)?;
            }
            Ok(cached)
        })();
        if let Ok(cached) = read {
            state = cached;
        }
    }
    merge_baseline(
        &mut state,
        source,
        include_str!("../../../assets/app-release-catalog/baseline.json"),
    )?;
    Ok(state)
}

pub(super) fn merge_baseline(state: &mut State, source: &Source, bytes: &str) -> Result<()> {
    #[derive(Deserialize)]
    struct Baseline {
        schema: u32,
        envelope: Option<String>,
        objects: BTreeMap<String, String>,
    }
    let baseline: Baseline = serde_json::from_str(bytes)?;
    ensure!(baseline.schema == 1, "Unsupported bundled baseline");
    if let Some(envelope) = baseline.envelope {
        // Forks and isolated feeds can select different trusted keys. Unrelated
        // or invalid bundled data is not authority for that source and must
        // neither be adopted nor prevent use of its own verified cache/API.
        let Ok(bundled) = verify_index(envelope.as_bytes(), source, 0) else {
            return Ok(());
        };
        let current = state
            .envelope
            .as_ref()
            .and_then(|s| verify_index(s.as_bytes(), source, 0).ok());
        if current
            .as_ref()
            .is_none_or(|c| c.index.sequence < bundled.index.sequence)
        {
            state.envelope = Some(envelope);
            state.objects = baseline.objects;
            state.etag = None;
        }
    }
    Ok(())
}
pub fn snapshot(state: &State, source: &Source) -> Result<Snapshot> {
    let mut result = Snapshot {
        epoch: state.epoch.clone(),
        source: state.source.clone(),
        attempted: state.attempted,
        overrides: state.overrides.clone(),
        observations: state.observations.clone(),
        api_observations: state.api_observations.clone(),
        ..Default::default()
    };
    if let Some(envelope) = &state.envelope {
        let verified = verify_index(envelope.as_bytes(), source, 0)?;
        for (repo, reference) in &verified.index.repositories {
            if let Some(added) = reference.discovery_added_at {
                result.discovery_added_at.insert(repo.clone(), added);
            }
            if let Some(body) = state.objects.get(&reference.sha256) {
                if let Ok(app) = verify_app(body.as_bytes(), repo, reference) {
                    for (index, release) in app.releases.iter().enumerate() {
                        result
                            .tags
                            .insert((repo.clone(), release.tag_name.clone()), index);
                    }
                    for evidence in &app.scopes {
                        if evidence.inspector_version == 1 {
                            result.scope.insert(
                                (
                                    repo.clone(),
                                    evidence.tag.clone(),
                                    evidence.asset_name.clone(),
                                    evidence.digest.clone(),
                                    evidence.size,
                                    evidence.architecture.clone(),
                                ),
                                evidence.policy.clone(),
                            );
                        }
                    }
                    result.apps.insert(repo.clone(), Arc::new(app));
                    result.checked.insert(repo.clone(), reference.checked_at);
                    result
                        .generation
                        .insert(repo.clone(), reference.sha256.clone());
                }
            }
        }
    }
    for observed in &state.api_observations {
        if !observed.valid() {
            continue;
        }
        let Some(checked) = result.checked.get(&observed.repository) else {
            continue;
        };
        if observed.at >= *checked
            && result
                .apps
                .get(&observed.repository)
                .is_some_and(|app| !observed.matches(app))
        {
            let time = observed.at.max(checked.saturating_add(1));
            result
                .overrides
                .entry(observed.repository.clone())
                .and_modify(|previous| *previous = (*previous).max(time))
                .or_insert(time);
        }
    }
    for observed in &state.observations {
        let evidence = &observed.evidence;
        if evidence.inspector_version == 1
            && chrono::Utc::now().timestamp().saturating_sub(observed.used) <= 30 * 86400
        {
            result.scope.insert(
                (
                    observed.repository.clone(),
                    evidence.tag.clone(),
                    evidence.asset_name.clone(),
                    evidence.digest.clone(),
                    evidence.size,
                    evidence.architecture.clone(),
                ),
                evidence.policy.clone(),
            );
        }
    }
    Ok(result)
}
pub fn save(root: &Path, state: &mut State, active: &str) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    state
        .used
        .retain(|repo, used| repo == active || now.saturating_sub(*used) <= 30 * 86400);
    state
        .overrides
        .retain(|_, time| now.saturating_sub(*time) <= 86400);
    state
        .api_observations
        .retain(|observation| observation.valid());
    if state.api_observations.len() > 512 {
        state
            .api_observations
            .sort_by_key(|observation| observation.at);
        state
            .api_observations
            .drain(..state.api_observations.len() - 512);
    }
    state
        .observations
        .retain(|o| now.saturating_sub(o.used) <= 30 * 86400);
    if state.observations.len() > 2048 {
        state.observations.sort_by_key(|o| o.used);
        state.observations.drain(..state.observations.len() - 2048);
    }
    // Keep only objects referenced by currently used repos, never arbitrary filenames.
    if let Some(envelope) = &state.envelope {
        let envelope: Envelope = serde_json::from_str(envelope)?;
        let index: Index = serde_json::from_str(&envelope.payload)?;
        let allowed: std::collections::BTreeSet<_> = index
            .repositories
            .iter()
            .filter(|(r, _)| state.used.contains_key(*r))
            .map(|(_, v)| v.sha256.as_str())
            .collect();
        state
            .objects
            .retain(|key, _| allowed.contains(key.as_str()));
    }
    while state.objects.len() > 128 {
        let Some(repo) = state
            .used
            .iter()
            .filter(|(r, _)| r.as_str() != active)
            .min_by_key(|(_, t)| *t)
            .map(|(r, _)| r.clone())
        else {
            break;
        };
        state.used.remove(&repo);
        if let Some(envelope) = &state.envelope {
            let index: Index =
                serde_json::from_str(&serde_json::from_str::<Envelope>(envelope)?.payload)?;
            if let Some(reference) = index.repositories.get(&repo) {
                state.objects.remove(&reference.sha256);
            }
        }
    }
    for object in state.objects.values() {
        ensure!(object.len() <= MAX_APP, "Oversized cached object");
    }
    let bytes = loop {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&serde_json::to_vec(state)?)?;
        let bytes = encoder.finish()?;
        if bytes.len() as u64 <= MAX_DISK {
            break bytes;
        }
        let repo = state
            .used
            .iter()
            .filter(|(repo, _)| repo.as_str() != active)
            .min_by_key(|(_, used)| *used)
            .map(|(repo, _)| repo.clone())
            .context("Active catalog exceeds cache budget")?;
        state.used.remove(&repo);
        if let Some(envelope) = &state.envelope {
            let index: Index =
                serde_json::from_str(&serde_json::from_str::<Envelope>(envelope)?.payload)?;
            if let Some(reference) = index.repositories.get(&repo) {
                state.objects.remove(&reference.sha256);
            }
        }
    };
    check_path(root)?;
    let path = file(root);
    std::fs::create_dir_all(path.parent().context("Missing cache parent")?)?;
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _cleanup = Cleanup(temporary.clone());
    let mut output = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    output.write_all(&bytes)?;
    output.sync_all()?;
    drop(output);
    crate::platform::atomic_replace(&temporary, &path)
}
pub fn clear(root: &Path) -> Result<()> {
    let _lock = lock(root)?;
    memory().lock().unwrap().remove(&root_id(root));
    let path = file(root);
    check_path(root)?;
    if path.is_file() {
        std::fs::remove_file(path)?;
    }
    match std::fs::remove_dir(root.join("runtime/app-release-catalog")) {
        Ok(()) => Ok(()),
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
            ) =>
        {
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

pub fn check_path(root: &Path) -> Result<()> {
    verification::unlinked_ancestors(&file(root))
}

fn new_epoch() -> String {
    uuid::Uuid::new_v4().to_string()
}
