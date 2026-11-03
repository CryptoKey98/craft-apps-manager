//! Process-local hot responses and worker-only disk/network coordination.
use super::disk_cache::{DiskCache, Record};
use anyhow::Result;
use serde::de::DeserializeOwned;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    any::{Any, TypeId},
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Condvar, Mutex, OnceLock, RwLock, Weak},
    time::{Duration, Instant},
};

const MEMORY_BUDGET: usize = 16 * 1024 * 1024;
const DISK_BUDGET: u64 = 20 * 1024 * 1024;
const MAX_AGE: i64 = 30 * 86400;

// Only worker paths wait on this gate. A clear drains old readers, including
// network responses and typed warming, before deleting disk and memory entries.
type CacheGate = RwLock<()>;
fn root_gate(root: &Path) -> Arc<CacheGate> {
    static GATES: OnceLock<Mutex<HashMap<PathBuf, Weak<CacheGate>>>> = OnceLock::new();
    let root = key(root, "", None).root;
    let mut gates = GATES.get_or_init(Mutex::default).lock().unwrap();
    if let Some(gate) = gates.get(&root).and_then(Weak::upgrade) {
        return gate;
    }
    gates.retain(|_, gate| gate.strong_count() != 0);
    let gate = Arc::new(RwLock::new(()));
    gates.insert(root, Arc::downgrade(&gate));
    gate
}

pub(super) fn clear(root: &Path) -> Result<()> {
    let root = key(root, "", None).root;
    let gate = root_gate(&root);
    let _exclusive = gate.write().unwrap();
    // Forget memory even on a disk failure, then report the failure honestly.
    let result = disk(&root).clear();
    forget_root(&root);
    result
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(super) struct Key {
    pub root: PathBuf,
    pub hash: String,
    pub legacy: Option<String>,
}

pub(super) fn key(root: &Path, url: &str, token: Option<&str>) -> Key {
    let absolute = std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf());
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => (),
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    #[cfg(windows)]
    let normalized = PathBuf::from(
        normalized
            .as_os_str()
            .to_string_lossy()
            .to_ascii_lowercase(),
    );
    let legacy = format!("{:x}", Sha256::digest(url.as_bytes()));
    let hash = if let Some(token) = token {
        let mut digest = Sha256::new();
        digest.update(url.as_bytes());
        digest.update(b"\0credential\0");
        digest.update(Sha256::digest(token.as_bytes()));
        format!("{:x}", digest.finalize())
    } else {
        legacy.clone()
    };
    Key {
        root: normalized,
        hash,
        legacy: token.is_none().then_some(legacy),
    }
}

#[derive(Clone, Hash, PartialEq, Eq)]
enum MemoryKey {
    Raw(Key),
    Typed(Key, TypeId),
}
struct Hot {
    value: Arc<dyn Any + Send + Sync>,
    weight: usize,
    used: u64,
}
#[derive(Default)]
struct Memory {
    entries: HashMap<MemoryKey, Hot>,
    bytes: usize,
    clock: u64,
}
static MEMORY: OnceLock<Mutex<Memory>> = OnceLock::new();
fn memory() -> &'static Mutex<Memory> {
    MEMORY.get_or_init(Mutex::default)
}

fn hot<T: Any + Send + Sync>(key: &MemoryKey, nonblocking: bool) -> Option<Arc<T>> {
    let mut memory = if nonblocking {
        memory().try_lock().ok()?
    } else {
        memory().lock().ok()?
    };
    memory.clock = memory.clock.wrapping_add(1);
    let clock = memory.clock;
    let entry = memory.entries.get_mut(key)?;
    entry.used = clock;
    entry.value.clone().downcast().ok()
}
fn insert(memory: &mut Memory, key: MemoryKey, value: Arc<dyn Any + Send + Sync>, weight: usize) {
    if weight > MEMORY_BUDGET {
        return;
    }
    if let Some(old) = memory.entries.remove(&key) {
        memory.bytes -= old.weight;
    }
    while memory.bytes.saturating_add(weight) > MEMORY_BUDGET || memory.entries.len() >= 512 {
        let oldest = memory
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.used)
            .map(|(key, _)| key.clone());
        let Some(oldest) = oldest else { break };
        memory.bytes -= memory.entries.remove(&oldest).unwrap().weight;
    }
    memory.clock = memory.clock.wrapping_add(1);
    let used = memory.clock;
    memory.bytes += weight;
    memory.entries.insert(
        key,
        Hot {
            value,
            weight,
            used,
        },
    );
}
fn remember_record(key: &Key, record: Arc<Record>) {
    // A replacement invalidates previously decoded views of this representation.
    let mut memory = memory().lock().unwrap();
    let remove: Vec<_> = memory
        .entries
        .keys()
        .filter(|candidate| matches!(candidate, MemoryKey::Typed(k, _) if k == key))
        .cloned()
        .collect();
    for candidate in remove {
        memory.bytes -= memory.entries.remove(&candidate).unwrap().weight;
    }
    if let Some(old) = memory.entries.remove(&MemoryKey::Raw(key.clone())) {
        memory.bytes -= old.weight;
    }
    if record.bytes <= MEMORY_BUDGET {
        insert(
            &mut memory,
            MemoryKey::Raw(key.clone()),
            record.clone(),
            record.bytes.max(1),
        );
    }
}

type Work = Box<dyn FnOnce() + Send>;
fn enqueue(work: Work) -> bool {
    #[cfg(test)]
    let work = tests::tracked(work);
    static POOL: OnceLock<mpsc::SyncSender<Work>> = OnceLock::new();
    let sender = POOL.get_or_init(|| {
        let (tx, rx) = mpsc::sync_channel::<Work>(64);
        let rx = Arc::new(Mutex::new(rx));
        for index in 0..2 {
            let rx = rx.clone();
            std::thread::Builder::new()
                .name(format!("api-cache-{index}"))
                .spawn(move || loop {
                    let Ok(work) = rx.lock().unwrap().recv() else {
                        break;
                    };
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work));
                })
                .expect("API cache worker");
        }
        tx
    });
    sender.try_send(work).is_ok()
}
pub(super) fn background(work: Work) -> bool {
    enqueue(work)
}
fn disk(root: &Path) -> DiskCache {
    DiskCache::new(&root.join("runtime/api-cache"))
}
fn unchanged(key: &Key, record: &Record) -> bool {
    if !disk_allowed(key) {
        // A successful HTTP response can remain fresh in memory even when disk
        // replacement/removal failed. Rejected disk bytes cannot supersede it.
        return true;
    }
    // Worker reads observe other-process writes without loading/decompressing bodies.
    disk(&key.root)
        .generation(&key.hash)
        .ok()
        .is_some_and(|generation| generation == record.generation)
}

static OBSERVED: OnceLock<Mutex<HashMap<Key, String>>> = OnceLock::new();
pub(super) fn generation(key: &Key) -> Option<String> {
    OBSERVED
        .get_or_init(Mutex::default)
        .try_lock()
        .ok()?
        .get(key)
        .cloned()
}
pub(super) fn observe_completed(key: Key, generation: String) {
    let mut observed = OBSERVED.get_or_init(Mutex::default).lock().unwrap();
    if observed.len() >= 512 && !observed.contains_key(&key) {
        if let Some(oldest) = observed.keys().next().cloned() {
            observed.remove(&oldest);
        }
    }
    observed.insert(key, generation);
}

pub(super) fn evict(key: &Key) {
    let gate = root_gate(&key.root);
    let _exclusive = gate.write().unwrap();
    forget_key(key);
    OBSERVED
        .get_or_init(Mutex::default)
        .lock()
        .unwrap()
        .remove(key);
    invalidate_disk(key);
    let disk = disk(&key.root);
    if let Ok(_guard) = disk.lock(&key.hash) {
        let _ = disk.remove(&key.hash);
    }
}
#[derive(Default)]
struct InvalidDisk {
    keys: HashSet<Key>,
    disabled: bool,
}
impl InvalidDisk {
    fn reject(&mut self, key: &Key) {
        if self.disabled {
            return;
        }
        if self.keys.len() >= 512 && !self.keys.contains(key) {
            // Forgetting a rejected entry could revive bytes whose deletion failed.
            // On overflow, disable disk reuse for this process; HTTP still works.
            self.disabled = true;
            self.keys.clear();
        } else {
            self.keys.insert(key.clone());
        }
    }
    fn allows(&self, key: &Key) -> bool {
        !self.disabled && !self.keys.contains(key)
    }
}
static INVALID_DISK: OnceLock<Mutex<InvalidDisk>> = OnceLock::new();
fn invalidate_disk(key: &Key) {
    INVALID_DISK
        .get_or_init(Mutex::default)
        .lock()
        .unwrap()
        .reject(key);
}
fn disk_allowed(key: &Key) -> bool {
    // Only a verified successful write clears rejection. Time or metadata-only
    // changes from another process do not establish that the body was replaced.
    INVALID_DISK
        .get_or_init(Mutex::default)
        .lock()
        .unwrap()
        .allows(key)
}

pub(super) fn read(key: &Key) -> Option<Arc<Record>> {
    let gate = root_gate(&key.root);
    let _reading = gate.read().unwrap();
    read_guarded(key)
}

fn read_guarded(key: &Key) -> Option<Arc<Record>> {
    if let Some(record) =
        hot::<Record>(&MemoryKey::Raw(key.clone()), false).filter(|record| unchanged(key, record))
    {
        if chrono::Utc::now()
            .timestamp()
            .saturating_sub(record.accessed)
            >= 300
        {
            accessed(key);
        }
        return Some(record);
    }
    let disk = disk(&key.root);
    if !disk_allowed(key) {
        return None;
    }
    let _guard = disk.lock(&key.hash).ok()?;
    let record = Arc::new(disk.read(&key.hash, key.legacy.as_deref()).ok()??);
    let _ = disk.touch(&key.hash, chrono::Utc::now().timestamp());
    remember_record(key, record.clone());
    maintain(&key.root, false);
    Some(record)
}

/// Reuse a fresh list object for a preview without publishing a derived cache
/// entry. Authoritative tag data and invalidations always take precedence.
pub(super) fn release_from_history(
    target: &Key,
    source: &Key,
    tag: &str,
    max_age: i64,
) -> Option<(Value, i64)> {
    if target.root != source.root || target == source {
        return None;
    }
    let gate = root_gate(&target.root);
    let _reading = gate.read().unwrap();
    if !disk_allowed(target) || read_guarded(target).is_some() {
        return None;
    }
    let history = read_guarded(source)?;
    if !fresh(&history, chrono::Utc::now().timestamp(), max_age) {
        return None;
    }
    let value = history
        .value
        .as_array()?
        .iter()
        .find(|item| item.get("tag_name").and_then(Value::as_str) == Some(tag))?
        .clone();
    serde_json::from_value::<crate::model::Release>(value.clone()).ok()?;
    observe_completed(source.clone(), history.generation.clone());
    Some((value, history.at))
}
fn preview_read(key: &Key) -> Option<Arc<Record>> {
    if let Some(record) = hot(&MemoryKey::Raw(key.clone()), false) {
        return Some(record);
    }
    let disk = disk(&key.root);
    if !disk_allowed(key) {
        return None;
    }
    let _guard = disk.try_lock(&key.hash).ok()??;
    let record = Arc::new(disk.read(&key.hash, key.legacy.as_deref()).ok()??);
    remember_record(key, record.clone());
    Some(record)
}

fn accessed(key: &Key) {
    static TOUCHED: OnceLock<Mutex<HashMap<Key, Instant>>> = OnceLock::new();
    let mut touched = match TOUCHED.get_or_init(Mutex::default).try_lock() {
        Ok(touched) => touched,
        Err(_) => return,
    };
    if touched
        .get(key)
        .is_some_and(|at| at.elapsed() < Duration::from_secs(300))
    {
        return;
    }
    if touched.len() >= 1024 {
        touched.retain(|_, at| at.elapsed() < Duration::from_secs(300));
    }
    if touched.len() >= 1024 {
        return;
    }
    touched.insert(key.clone(), Instant::now());
    drop(touched);
    let key = key.clone();
    let _ = enqueue(Box::new(move || {
        let disk = disk(&key.root);
        if let Ok(Some(_guard)) = disk.try_lock(&key.hash) {
            let _ = disk.touch(&key.hash, chrono::Utc::now().timestamp());
        }
    }));
}

/// Cold disk reads and typed deserialization are queued; never performed by a frame.
#[derive(Clone)]
struct WarmState {
    running: bool,
    at: Instant,
    generation: Option<String>,
}
fn warming_blocks(state: &WarmState, generation: &Option<String>) -> bool {
    state.running
        || (state.at.elapsed() < Duration::from_secs(2) && &state.generation == generation)
}
pub(super) fn peek<T: DeserializeOwned + Send + Sync + 'static>(
    key: Key,
    wake: impl Fn() + Send + 'static,
) -> Option<Arc<T>> {
    let gate = root_gate(&key.root);
    let _reading = gate.try_read().ok()?;
    let typed = MemoryKey::Typed(key.clone(), TypeId::of::<T>());
    if let Some(value) = hot(&typed, true) {
        accessed(&key);
        return Some(value);
    }
    static WARMING: OnceLock<Mutex<HashMap<MemoryKey, WarmState>>> = OnceLock::new();
    let generation = {
        let memory = memory().try_lock().ok()?;
        memory
            .entries
            .get(&MemoryKey::Raw(key.clone()))
            .and_then(|entry| entry.value.clone().downcast::<Record>().ok())
            .map(|record| record.generation.clone())
    };
    let warming = WARMING.get_or_init(Mutex::default);
    let Ok(mut warming) = warming.try_lock() else {
        return None;
    };
    if warming
        .get(&typed)
        .is_some_and(|state| warming_blocks(state, &generation))
    {
        return None;
    }
    if warming.len() >= 1024 {
        warming.retain(|_, state| state.running || state.at.elapsed() < Duration::from_secs(2));
    }
    if warming.len() >= 1024 {
        return None;
    }
    warming.insert(
        typed.clone(),
        WarmState {
            running: true,
            at: Instant::now(),
            generation: generation.clone(),
        },
    );
    drop(warming);
    let work_key = typed.clone();
    if !enqueue(Box::new(move || {
        let gate = root_gate(&key.root);
        let _reading = gate.read().unwrap();
        let mut completed = false;
        let mut failed_generation = generation;
        if let Some(record) = preview_read(&key) {
            failed_generation = Some(record.generation.clone());
            if let Ok(value) = serde_json::from_value::<T>(record.value.clone()) {
                let mut memory = memory().lock().unwrap();
                let current = memory
                    .entries
                    .get(&MemoryKey::Raw(key.clone()))
                    .and_then(|entry| entry.value.clone().downcast::<Record>().ok());
                if current.is_some_and(|current| current.generation == record.generation) {
                    insert(
                        &mut memory,
                        work_key.clone(),
                        Arc::new(value),
                        record.bytes.max(1),
                    );
                    drop(memory);
                    completed = true;
                    accessed(&key);
                    maintain(&key.root, false);
                    wake();
                }
            }
        }
        let mut warming = WARMING.get().unwrap().lock().unwrap();
        if completed {
            warming.remove(&work_key);
        } else {
            warming.insert(
                work_key,
                WarmState {
                    running: false,
                    at: Instant::now(),
                    generation: failed_generation,
                },
            );
        }
    })) {
        if let Ok(mut warming) = WARMING.get().unwrap().lock() {
            warming.remove(&typed);
        }
    }
    None
}

#[derive(Clone, Debug)]
struct Failure {
    text: String,
    status: Option<reqwest::StatusCode>,
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}
impl std::error::Error for Failure {}
// Only explicit, cacheable HTTP 404 responses qualify. Transport/authentication
// errors are never remembered as an absent release.
#[derive(Debug)]
struct MissingResponse;
impl std::fmt::Display for MissingResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Release or repository was not found (HTTP 404)")
    }
}
impl std::error::Error for MissingResponse {}
pub(super) fn missing_response() -> anyhow::Error {
    anyhow::Error::new(MissingResponse)
}
static MISSING: OnceLock<Mutex<HashMap<Key, (Instant, Failure)>>> = OnceLock::new();

pub(super) fn status(error: &anyhow::Error) -> Option<reqwest::StatusCode> {
    if error.is::<MissingResponse>() {
        return Some(reqwest::StatusCode::NOT_FOUND);
    }
    error
        .downcast_ref::<Failure>()
        .and_then(|e| e.status)
        .or_else(|| {
            error
                .downcast_ref::<reqwest::Error>()
                .and_then(|e| e.status())
        })
}

#[derive(Default)]
struct FlightState {
    running: bool,
    round: u64,
    revalidated: bool,
    result: Option<std::result::Result<Arc<Record>, Failure>>,
}
#[derive(Default)]
struct Flight {
    state: Mutex<FlightState>,
    ready: Condvar,
}
struct FlightOwner {
    flight: Arc<Flight>,
    completed: bool,
}
impl Drop for FlightOwner {
    fn drop(&mut self) {
        if !self.completed {
            let mut state = self
                .flight
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.running = false;
            state.round = state.round.wrapping_add(1);
            state.revalidated = true;
            state.result = Some(Err(Failure {
                text: "API request worker stopped unexpectedly".into(),
                status: None,
            }));
            self.flight.ready.notify_all();
        }
    }
}
fn flight(key: &Key) -> Arc<Flight> {
    static FLIGHTS: OnceLock<Mutex<HashMap<Key, Weak<Flight>>>> = OnceLock::new();
    let mut flights = FLIGHTS.get_or_init(Mutex::default).lock().unwrap();
    if let Some(flight) = flights.get(key).and_then(Weak::upgrade) {
        return flight;
    }
    if flights.len() >= 256 {
        flights.retain(|_, flight| flight.strong_count() != 0);
    }
    let flight = Arc::new(Flight::default());
    flights.insert(key.clone(), Arc::downgrade(&flight));
    flight
}

pub(super) struct Response {
    pub value: Option<Value>,
    pub etag: Option<String>,
    pub cacheable: bool,
}
fn fresh(record: &Record, now: i64, max_age: i64) -> bool {
    record.at <= now && now.saturating_sub(record.at) < max_age
}

#[cfg(test)]
pub(super) fn request(
    key: Key,
    force: bool,
    fetch: impl FnOnce(Option<&Record>) -> Result<Response>,
) -> Result<Arc<Record>> {
    request_with_age(key, force, 300, fetch)
}

#[cfg(test)]
pub(super) fn request_with_age(
    key: Key,
    force: bool,
    max_age: i64,
    fetch: impl FnOnce(Option<&Record>) -> Result<Response>,
) -> Result<Arc<Record>> {
    request_finishing(key, force, max_age, fetch, || {})
}

pub(super) fn request_finishing(
    key: Key,
    force: bool,
    max_age: i64,
    fetch: impl FnOnce(Option<&Record>) -> Result<Response>,
    finish: impl FnOnce(),
) -> Result<Arc<Record>> {
    let gate = root_gate(&key.root);
    let reading = gate.read().unwrap();
    let flight = flight(&key);
    let mut state = flight.state.lock().unwrap();

    if state.running {
        // The owner finalizes dependent invalidations under the root write gate.
        // Waiters must neither hold that gate nor bypass completion via hot data.
        drop(reading);
        let round = state.round;
        while state.running && state.round == round {
            state = flight.ready.wait(state).unwrap();
        }
        let too_old = state.result.as_ref().is_some_and(|result| {
            result
                .as_ref()
                .is_ok_and(|record| !fresh(record, chrono::Utc::now().timestamp(), max_age))
        });
        if (force || too_old) && !state.revalidated {
            drop(state);
            return request_finishing(key, force, max_age, fetch, finish);
        }
        return state
            .result
            .clone()
            .expect("Completed request")
            .map_err(anyhow::Error::new);
    }
    if !force {
        let mut missing = MISSING.get_or_init(Mutex::default).lock().unwrap();
        missing.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(60));
        if let Some((at, failure)) = missing.get(&key) {
            if at.elapsed().as_secs() < max_age.clamp(0, 60) as u64 {
                return Err(anyhow::Error::new(failure.clone()));
            }
        }
    }
    if !force {
        if let Some(record) = hot::<Record>(&MemoryKey::Raw(key.clone()), false) {
            if fresh(&record, chrono::Utc::now().timestamp(), max_age) && unchanged(&key, &record) {
                accessed(&key);
                return Ok(record);
            }
        }
    }
    state.running = true;
    drop(state);
    let mut owner = FlightOwner {
        flight: flight.clone(),
        completed: false,
    };
    let mut cacheable = true;
    let mut revalidated = false;
    let result = (|| {
        let disk = disk(&key.root);
        // Cache coordination is best effort. Failure cannot prohibit HTTP.
        let guard = disk.lock(&key.hash).ok();
        let old = if guard.is_some() && disk_allowed(&key) {
            disk.read(&key.hash, key.legacy.as_deref()).ok().flatten()
        } else {
            None
        };
        let now = chrono::Utc::now().timestamp();
        if !force {
            if let Some(record) = old.as_ref().filter(|record| fresh(record, now, max_age)) {
                let _ = disk.touch(&key.hash, now);
                return Ok(Arc::new(record.clone()));
            }
        }
        revalidated = true;
        let response = match fetch(old.as_ref()) {
            Ok(response) => response,
            Err(error) => {
                if status(&error) == Some(reqwest::StatusCode::NOT_FOUND) {
                    // Retire withdrawn metadata under the same disk lock as the
                    // request so another process cannot replace it in between.
                    invalidate_disk(&key);
                    if guard.is_some() {
                        let _ = disk.remove(&key.hash);
                    }
                }
                return Err(error);
            }
        };
        cacheable = response.cacheable;
        let record = if let Some(value) = response.value {
            let saved = if response.cacheable && guard.is_some() {
                disk.write(&key.hash, now, response.etag.clone(), &value)
                    .ok()
            } else {
                None
            };
            match saved {
                Some(record) => {
                    INVALID_DISK
                        .get_or_init(Mutex::default)
                        .lock()
                        .unwrap()
                        .keys
                        .remove(&key);
                    record
                }
                None => {
                    invalidate_disk(&key);
                    if guard.is_some() {
                        let _ = disk.remove(&key.hash);
                    }
                    Record {
                        at: now,
                        accessed: now,
                        etag: response.etag,
                        bytes: serde_json::to_vec(&value)?.len(),
                        value,
                        generation: uuid::Uuid::new_v4().to_string(),
                    }
                }
            }
        } else {
            let old =
                old.ok_or_else(|| anyhow::anyhow!("No cached response for unchanged resource"))?;
            disk.refresh(&key.hash, &old, now).unwrap_or(Record {
                at: now,
                accessed: now,
                ..old
            })
        };
        if !response.cacheable {
            // Do not retain no-store responses in either the process cache or disk.
            invalidate_disk(&key);
            if guard.is_some() {
                let _ = disk.remove(&key.hash);
            }
            return Ok(Arc::new(record));
        }
        Ok(Arc::new(record))
    })();
    let remember_missing = result
        .as_ref()
        .is_err_and(|error: &anyhow::Error| error.is::<MissingResponse>());
    let shared = result.map_err(|error: anyhow::Error| Failure {
        status: status(&error),
        text: format!("{error:#}"),
    });
    if shared
        .as_ref()
        .is_err_and(|failure| failure.status == Some(reqwest::StatusCode::NOT_FOUND))
    {
        // A withdrawn release must not revive an older successful disk entry.
        forget_key(&key);
        invalidate_disk(&key);
    }
    {
        let mut missing = MISSING.get_or_init(Mutex::default).lock().unwrap();
        missing.remove(&key);
        if remember_missing {
            missing.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(60));
            if missing.len() >= 256 {
                if let Some(oldest) = missing
                    .iter()
                    .min_by_key(|(_, (at, _))| *at)
                    .map(|(key, _)| key.clone())
                {
                    missing.remove(&oldest);
                }
            }
            missing.insert(
                key.clone(),
                (Instant::now(), shared.as_ref().unwrap_err().clone()),
            );
        }
    }
    if cacheable {
        if let Ok(record) = &shared {
            remember_record(&key, record.clone());
            maintain(&key.root, true);
        }
    } else {
        forget_key(&key);
    }
    drop(reading);
    if shared.is_ok()
        || shared
            .as_ref()
            .is_err_and(|failure| failure.status == Some(reqwest::StatusCode::NOT_FOUND))
    {
        finish();
    }
    let mut state = flight.state.lock().unwrap();
    state.running = false;
    state.revalidated = revalidated;
    state.round = state.round.wrapping_add(1);
    state.result = Some(shared.clone());
    flight.ready.notify_all();
    owner.completed = true;
    shared.map_err(anyhow::Error::new)
}

pub(super) fn maintain(root: &Path, force: bool) {
    #[derive(Default)]
    struct Maintenance {
        running: HashSet<PathBuf>,
        dirty: HashSet<PathBuf>,
        last: HashMap<PathBuf, Instant>,
    }
    static MAINTENANCE: OnceLock<Mutex<Maintenance>> = OnceLock::new();
    let state = MAINTENANCE.get_or_init(Mutex::default);
    let Ok(mut state) = state.try_lock() else {
        return;
    };
    if state.running.contains(root) {
        if force {
            state.dirty.insert(root.to_path_buf());
        }
        return;
    }
    if !force
        && state
            .last
            .get(root)
            .is_some_and(|at| at.elapsed() < Duration::from_secs(86400))
    {
        return;
    }
    if state.last.len() >= 64 {
        state
            .last
            .retain(|_, at| at.elapsed() < Duration::from_secs(86400));
    }
    state.running.insert(root.to_path_buf());
    drop(state);
    let root = root.to_path_buf();
    let work_root = root.clone();
    if !enqueue(Box::new(move || loop {
        if let Ok(stats) =
            disk(&work_root).prune(chrono::Utc::now().timestamp(), DISK_BUDGET, MAX_AGE)
        {
            let _ = (stats.bytes, stats.files);
        }
        let mut state = MAINTENANCE.get().unwrap().lock().unwrap();
        if state.dirty.remove(&work_root) {
            drop(state);
            continue;
        }
        state.running.remove(&work_root);
        state.last.insert(work_root, Instant::now());
        break;
    })) {
        MAINTENANCE
            .get()
            .unwrap()
            .lock()
            .unwrap()
            .running
            .remove(&root);
    }
}

fn forget_key(key: &Key) {
    MISSING
        .get_or_init(Mutex::default)
        .lock()
        .unwrap()
        .remove(key);
    let mut memory = memory().lock().unwrap();
    let remove: Vec<_> = memory
        .entries
        .keys()
        .filter(|candidate| match candidate {
            MemoryKey::Raw(k) | MemoryKey::Typed(k, _) => k == key,
        })
        .cloned()
        .collect();
    for key in remove {
        memory.bytes -= memory.entries.remove(&key).unwrap().weight;
    }
}

#[cfg(test)]
pub(super) fn forget(root: &Path) {
    tests::wait_for_background();
    let root = key(root, "", None).root;
    forget_root(&root);
}

fn forget_root(root: &Path) {
    MISSING
        .get_or_init(Mutex::default)
        .lock()
        .unwrap()
        .retain(|key, _| key.root != root);
    OBSERVED
        .get_or_init(Mutex::default)
        .lock()
        .unwrap()
        .retain(|key, _| key.root != root);
    let mut memory = memory().lock().unwrap();
    let remove: Vec<_> = memory
        .entries
        .keys()
        .filter(|key| match key {
            MemoryKey::Raw(k) | MemoryKey::Typed(k, _) => k.root == root,
        })
        .cloned()
        .collect();
    for key in remove {
        memory.bytes -= memory.entries.remove(&key).unwrap().weight;
    }
}

#[cfg(test)]
#[path = "cache_runtime_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "cache_benchmark_tests.rs"]
mod benchmark;
