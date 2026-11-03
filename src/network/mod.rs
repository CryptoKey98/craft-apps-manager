use crate::{files, jobs::Job, model::Asset};
use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use serde::de::DeserializeOwned;
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    time::Duration,
};
mod api_status;
mod cache;
#[cfg(test)]
#[path = "cache_tests.rs"]
pub(crate) mod cache_tests;
mod disk_cache;
mod policy;
mod rate_limit;
pub use api_status::{ApiObservation, ApiStatus};
#[cfg(test)]
pub(crate) mod request_tests;
pub use policy::CachePolicy;
pub(crate) fn is_not_found(error: &anyhow::Error) -> bool {
    cache::status(error).is_some_and(|status| status.as_u16() == 404)
}
/// Remove cached API responses for this library. Call from a maintenance worker.
pub fn clear_api_cache(root: &Path) -> Result<()> {
    cache::clear(root)
}
pub struct Network {
    client: Client,
    root: std::path::PathBuf,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenStatus {
    NotConfigured,
    NotVerified,
    Connected,
    Invalid,
    UnableToVerify,
}
fn environment_token() -> Option<(&'static str, String)> {
    ["CRAFT_GITHUB_TOKEN", "GITHUB_TOKEN"]
        .into_iter()
        .find_map(|name| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(|value| (name, value.trim().to_owned()))
        })
}
static SAVED_TOKEN: std::sync::OnceLock<std::sync::RwLock<Option<String>>> =
    std::sync::OnceLock::new();
// Separate metadata keeps frame-thread reads independent of keyring/cache locks.
static SAVED_TOKEN_PRESENT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
fn credential_entry() -> Result<keyring::Entry> {
    keyring::Entry::new("Craft Apps Manager", "github-api-token")
        .map_err(|_| anyhow::anyhow!("Cannot access the system credential store."))
}
fn saved_token_cache() -> &'static std::sync::RwLock<Option<String>> {
    SAVED_TOKEN.get_or_init(|| {
        let value = credential_entry()
            .ok()
            .and_then(|entry| entry.get_password().ok());
        SAVED_TOKEN_PRESENT.store(value.is_some(), std::sync::atomic::Ordering::Release);
        std::sync::RwLock::new(value)
    })
}
fn github_token() -> Option<(&'static str, String)> {
    environment_token().or_else(|| {
        saved_token_cache()
            .read()
            .ok()
            .and_then(|value| value.clone())
            .map(|value| ("System credential store", value))
    })
}
/// Cached metadata only; safe to read while drawing the interface.
pub fn saved_token_present() -> bool {
    SAVED_TOKEN_PRESENT.load(std::sync::atomic::Ordering::Acquire)
}
/// Nonblocking last-observed metadata for the active credential. No HTTP or
/// keychain access is performed from the UI frame.
pub fn api_status() -> Option<ApiObservation> {
    if let Some((_, token)) = environment_token() {
        return api_status::read(Some(&token));
    }
    let saved = SAVED_TOKEN.get()?.try_read().ok()?;
    api_status::read(saved.as_deref())
}
fn observe_api_response(
    response: &mut reqwest::blocking::Response,
    token: Option<&str>,
    request: u64,
) -> Option<String> {
    let body = if matches!(response.status().as_u16(), 403 | 429) {
        let mut body = String::new();
        let _ = (&mut *response).take(64 * 1024).read_to_string(&mut body);
        Some(body)
    } else {
        None
    };
    if response.url().scheme() == "https" && response.url().host_str() == Some("api.github.com") {
        let status = response_status(
            response.status(),
            response.headers(),
            body.as_deref().unwrap_or_default(),
        );
        api_status::observe(token, request, status);
    }
    body
}
fn response_status(
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
    body: &str,
) -> ApiStatus {
    if status.is_success() || status == reqwest::StatusCode::NOT_MODIFIED {
        ApiStatus::Available
    } else if rate_limit::is_limited(status, headers, body) {
        ApiStatus::RateLimited
    } else {
        ApiStatus::Unknown
    }
}
fn validate_token(value: &str) -> Result<&str> {
    let value = value.trim();
    anyhow::ensure!(
        !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_graphic()),
        "Enter a token without spaces, line breaks, or hidden characters. Copy it again from GitHub."
    );
    Ok(value)
}
pub fn save_token(value: &str) -> Result<()> {
    let value = validate_token(value)?;
    let cache = saved_token_cache();
    let mut saved = cache
        .write()
        .map_err(|_| anyhow::anyhow!("Token storage is unavailable."))?;
    credential_entry()?.set_password(value)
        .map_err(|_| anyhow::anyhow!("Could not save the token. Unlock or enable your system credential store and try again."))?;
    *saved = Some(value.into());
    SAVED_TOKEN_PRESENT.store(true, std::sync::atomic::Ordering::Release);
    Ok(())
}
pub fn remove_saved_token() -> Result<()> {
    let cache = saved_token_cache();
    let mut saved = cache
        .write()
        .map_err(|_| anyhow::anyhow!("Token storage is unavailable."))?;
    match credential_entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(_) => bail!("Could not remove the token from the system credential store."),
    }
    *saved = None;
    SAVED_TOKEN_PRESENT.store(false, std::sync::atomic::Ordering::Release);
    Ok(())
}
pub fn token_source() -> Option<&'static str> {
    environment_token()
        .map(|(source, _)| source)
        .or_else(|| saved_token_present().then_some("System credential store"))
}
fn token_status_from_http(status: reqwest::StatusCode) -> TokenStatus {
    if status.is_success() {
        TokenStatus::Connected
    } else if status == reqwest::StatusCode::UNAUTHORIZED {
        TokenStatus::Invalid
    } else {
        TokenStatus::UnableToVerify
    }
}
/// Validate authentication without caching credentials or exposing response bodies.
pub fn verify_token() -> TokenStatus {
    // Load saved credentials on this worker even when an environment token takes priority.
    let _ = saved_token_cache();
    let Some((_, token)) = github_token() else {
        return TokenStatus::NotConfigured;
    };
    verify_token_value(&token)
}
fn verify_token_value(token: &str) -> TokenStatus {
    let Ok(token) = validate_token(token) else {
        return TokenStatus::Invalid;
    };
    let observation = api_status::request_started();
    let result = (|| -> reqwest::Result<reqwest::blocking::Response> {
        Client::builder()
            .user_agent(concat!("CraftApps-Manager/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .build()?
            .get("https://api.github.com/user")
            .bearer_auth(token)
            .send()
    })();
    result
        .map(|mut response| {
            observe_api_response(&mut response, Some(token), observation);
            token_status_from_http(response.status())
        })
        .unwrap_or_else(|_| {
            api_status::observe(Some(token), observation, ApiStatus::Unknown);
            TokenStatus::UnableToVerify
        })
}
impl Network {
    /// Memory-only observation for UI invalidation; never reads disk on a frame.
    pub fn cached_generation(root: &Path, url: &str) -> Option<String> {
        let token = url
            .starts_with("https://api.github.com/")
            .then(github_token)
            .flatten();
        cache::generation(&cache::key(
            root,
            url,
            token.as_ref().map(|(_, token)| token.as_str()),
        ))
    }
    /// Signed update metadata is small. Bound the body before parsing it.
    pub fn json_bounded<T: DeserializeOwned>(&self, url: &str, limit: u64) -> Result<T> {
        let response = self.client.get(url).send()?.error_for_status()?;
        let mut body = Vec::new();
        response.take(limit + 1).read_to_end(&mut body)?;
        anyhow::ensure!(
            body.len() as u64 <= limit,
            "Update metadata exceeds its size limit"
        );
        Ok(serde_json::from_slice(&body)?)
    }
    /// Reads known release metadata without constructing a client or contacting GitHub.
    pub fn cached_json<T: DeserializeOwned>(root: &Path, url: &str) -> Option<T> {
        let token = url
            .starts_with("https://api.github.com/")
            .then(github_token)
            .flatten();
        let key = cache::key(root, url, token.as_ref().map(|(_, token)| token.as_str()));
        serde_json::from_value(cache::read(&key)?.value.clone()).ok()
    }
    /// Nonblocking preview: disk I/O and typed decoding run on bounded workers.
    pub fn peek_json<T: DeserializeOwned + Send + Sync + 'static>(
        root: &Path,
        url: &str,
        wake: impl Fn() + Send + 'static,
    ) -> Option<std::sync::Arc<T>> {
        let api = url.starts_with("https://api.github.com/");
        let environment = api.then(environment_token).flatten();
        if api && environment.is_none() && SAVED_TOKEN.get().is_none() {
            static LOADING: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);
            if !LOADING.swap(true, std::sync::atomic::Ordering::AcqRel)
                && !cache::background(Box::new(move || {
                    let _ = saved_token_cache();
                    wake();
                }))
            {
                LOADING.store(false, std::sync::atomic::Ordering::Release);
            }
            return None;
        }
        let token = if let Some((_, token)) = environment {
            Some(token)
        } else if api {
            SAVED_TOKEN.get()?.try_read().ok()?.clone()
        } else {
            None
        };
        cache::peek(cache::key(root, url, token.as_deref()), wake)
    }
    pub fn new(root: &Path) -> Result<Self> {
        static CLIENT: std::sync::OnceLock<std::result::Result<Client, String>> =
            std::sync::OnceLock::new();
        let client = CLIENT
            .get_or_init(|| {
                Client::builder()
                    .user_agent(concat!("CraftApps-Manager/", env!("CARGO_PKG_VERSION")))
                    .connect_timeout(Duration::from_secs(20))
                    .timeout(Duration::from_secs(300))
                    .build()
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(|error| anyhow::anyhow!(error.clone()))?
            .clone();
        Ok(Self {
            client,
            root: root.to_path_buf(),
        })
    }
    /// Like `json`, but `None` when GitHub reports the resource missing (404),
    /// such as `releases/latest` for a repository without releases.
    pub fn json_optional<T: DeserializeOwned>(&self, url: &str) -> Result<Option<T>> {
        match self.json(url) {
            Ok(value) => Ok(Some(value)),
            Err(error) if cache::status(&error).is_some_and(|s| s.as_u16() == 404) => Ok(None),
            Err(error) => Err(error),
        }
    }
    /// Application-release metadata only. Self-update/discovery keep their existing paths.
    pub fn release_json<T: DeserializeOwned>(&self, url: &str) -> Result<T> {
        self.release_json_with_policy(url, CachePolicy::for_url(url))
    }
    pub fn release_json_with_policy<T: DeserializeOwned>(
        &self,
        url: &str,
        policy: CachePolicy,
    ) -> Result<T> {
        // Loopback API fixtures must stay isolated even when a preview build's
        // bundled catalog points at a live test feed. Shipping builds omit this.
        #[cfg(test)]
        if matches!(
            request_tests::route(&self.root, url),
            std::borrow::Cow::Owned(_)
        ) {
            return self.json_with_policy(url, policy);
        }
        crate::installations::release_catalog::provider::resolve(&self.root, url, || {
            self.json_with_policy(url, policy)
        })
    }
    pub fn json<T: DeserializeOwned>(&self, url: &str) -> Result<T> {
        self.json_with_policy(url, CachePolicy::for_url(url))
    }
    pub fn json_with_policy<T: DeserializeOwned>(
        &self,
        url: &str,
        policy: CachePolicy,
    ) -> Result<T> {
        self.json_with_cache(url, false, policy)
    }
    pub fn json_optional_with_policy<T: DeserializeOwned>(
        &self,
        url: &str,
        policy: CachePolicy,
    ) -> Result<Option<T>> {
        match self.json_with_policy(url, policy) {
            Ok(value) => Ok(Some(value)),
            Err(error) if cache::status(&error).is_some_and(|s| s.as_u16() == 404) => Ok(None),
            Err(error) => Err(error),
        }
    }
    pub fn json_fresh<T: DeserializeOwned>(&self, url: &str) -> Result<T> {
        self.json_with_cache(url, true, CachePolicy::Standard)
    }
    fn json_with_cache<T: DeserializeOwned>(
        &self,
        url: &str,
        fresh: bool,
        policy: CachePolicy,
    ) -> Result<T> {
        let token = url
            .starts_with("https://api.github.com/")
            .then(github_token)
            .flatten();
        let key = cache::key(
            &self.root,
            url,
            token.as_ref().map(|(_, token)| token.as_str()),
        );
        let decoded = std::cell::RefCell::new(None);
        let related = std::cell::RefCell::new(Vec::new());
        if !fresh && matches!(policy, CachePolicy::SelectedRelease) {
            if let Some((history, tag)) = policy::history_for_tag(url) {
                let source = cache::key(
                    &self.root,
                    &history,
                    token.as_ref().map(|(_, token)| token.as_str()),
                );
                if let Some((value, at)) = cache::release_from_history(
                    &key,
                    &source,
                    &tag,
                    CachePolicy::Versions.seconds(),
                ) {
                    crate::installations::release_catalog::observe_api(&self.root, url, &value, at);
                    return Ok(serde_json::from_value(value)?);
                }
            }
        }
        let record = cache::request_finishing(
            key.clone(),
            fresh,
            policy.seconds(),
            |old| {
                #[cfg(test)]
                let request_url = request_tests::route(&self.root, url);
                #[cfg(not(test))]
                let request_url = std::borrow::Cow::Borrowed(url);
                let mut request = self
                    .client
                    .get(request_url.as_ref())
                    .header("Accept", "application/vnd.github+json");
                if url.starts_with("https://api.github.com/") {
                    // Local cache hits never reach this path. An actual fetch
                    // must revalidate intermediary copies and their quota headers.
                    request = request.header(reqwest::header::CACHE_CONTROL, "no-cache");
                }
                if let Some(etag) = old.and_then(|c| c.etag.as_ref()) {
                    request = request.header("If-None-Match", etag);
                }
                // An optional token raises GitHub's hourly API limit; it is sent only to the API host.
                if let Some((_, token)) = &token {
                    if request_url.starts_with("https://api.github.com/") {
                        request = request.bearer_auth(token);
                    }
                }
                let observation = api_status::request_started();
                let mut response = request.send().inspect_err(|_| {
                    if url.starts_with("https://api.github.com/") {
                        api_status::observe(
                            token.as_ref().map(|(_, token)| token.as_str()),
                            observation,
                            ApiStatus::Unknown,
                        );
                    }
                })?;
                let error_body = observe_api_response(
                    &mut response,
                    token.as_ref().map(|(_, token)| token.as_str()),
                    observation,
                );
                let cacheable = !response
                    .headers()
                    .get(reqwest::header::CACHE_CONTROL)
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| {
                        value.split(',').any(|part| {
                            matches!(
                                part.trim().to_ascii_lowercase().as_str(),
                                "no-store" | "no-cache"
                            )
                        })
                    })
                    && !response
                        .headers()
                        .get(reqwest::header::VARY)
                        .and_then(|value| value.to_str().ok())
                        .is_some_and(|value| value.trim() == "*");
                if response.status().as_u16() == 304 {
                    return Ok(cache::Response {
                        value: None,
                        etag: None,
                        cacheable,
                    });
                }
                if response.status().as_u16() == 403 || response.status().as_u16() == 429 {
                    let status = response.status();
                    let headers = response.headers().clone();
                    bail!(
                        "{}",
                        rate_limit::error(
                            status,
                            &headers,
                            error_body.as_deref().unwrap_or_default()
                        )
                    );
                }
                if response.status() == reqwest::StatusCode::NOT_FOUND {
                    if let Some((history, _)) = policy::history_for_tag(url) {
                        // Retire history after releasing this request's stripe;
                        // otherwise a restart could recreate a withdrawn tag.
                        related.borrow_mut().push(history);
                    }
                    if cacheable {
                        return Err(cache::missing_response());
                    }
                }
                let response = response.error_for_status()?;
                let etag = response
                    .headers()
                    .get("etag")
                    .and_then(|h| h.to_str().ok())
                    .map(str::to_owned);
                const MAX_RESPONSE: u64 = 32 * 1024 * 1024;
                let mut bytes = Vec::new();
                response.take(MAX_RESPONSE + 1).read_to_end(&mut bytes)?;
                anyhow::ensure!(
                    bytes.len() as u64 <= MAX_RESPONSE,
                    "API response exceeds its size limit"
                );
                let value: serde_json::Value = serde_json::from_slice(&bytes)?;
                // Reject incompatible payloads before publishing them as reusable data.
                *decoded.borrow_mut() = Some(serde_json::from_value::<T>(value.clone())?);
                if old.is_none_or(|old| old.value != value) {
                    *related.borrow_mut() =
                        policy::related_release_urls(url, old.map(|record| &record.value), &value);
                }
                Ok(cache::Response {
                    value: Some(value),
                    etag,
                    cacheable,
                })
            },
            || {
                // Do this after the request releases its root read gate/key stripe.
                // A changed latest release invalidates history and tag previews, even
                // when a publisher replaced an asset without changing the version.
                for url in related.borrow_mut().drain(..) {
                    cache::evict(&cache::key(
                        &self.root,
                        &url,
                        token.as_ref().map(|(_, token)| token.as_str()),
                    ));
                }
            },
        )?;
        // Publish the UI generation only after dependent invalidation finishes.
        cache::observe_completed(key, record.generation.clone());
        crate::installations::release_catalog::observe_api(
            &self.root,
            url,
            &record.value,
            record.at,
        );
        match decoded.into_inner() {
            Some(value) => Ok(value),
            None => Ok(serde_json::from_value(record.value.clone())?),
        }
    }
    pub fn text(&self, url: &str) -> Result<String> {
        Ok(self.client.get(url).send()?.error_for_status()?.text()?)
    }
    pub fn download(&self, url: &str, dest: &Path, job: &Job) -> Result<()> {
        job.check()?;
        fs::create_dir_all(dest.parent().unwrap())?;
        let partial = dest.with_extension(format!("{}.partial", uuid::Uuid::new_v4().simple()));
        let result = (|| -> Result<()> {
            let mut response = self.client.get(url).send()?.error_for_status()?;
            let total = response.content_length();
            let mut f = fs::File::create(&partial)?;
            let mut buf = [0; 65536];
            let mut done = 0;
            loop {
                job.check()?;
                let n = response.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                f.write_all(&buf[..n])?;
                done += n as u64;
                job.stage(
                    "Downloading",
                    total.map(|t| done as f32 / t.max(1) as f32),
                    format!(
                        "{} MB{}",
                        done / 1048576,
                        total
                            .map(|t| format!(" of {} MB", t / 1048576))
                            .unwrap_or_default()
                    ),
                );
            }
            f.sync_all()?;
            drop(f);
            if let Some(total) = total {
                if done != total {
                    bail!("Incomplete download");
                }
            }
            job.check()?;
            crate::platform::atomic_replace(&partial, dest)?;
            Ok(())
        })();
        if let Err(e) = result {
            let _ = fs::remove_file(&partial);
            return Err(e);
        }
        Ok(())
    }
    pub fn asset(&self, a: &Asset, dest: &Path, job: &Job) -> Result<()> {
        self.download(&asset_download_url(a)?, dest, job)?;
        job.check()?;
        job.stage(
            "Verifying checksum",
            None,
            "Checking SHA-256 before installation",
        );
        verify_asset(dest, a)
    }
}
/// A replaced GitHub asset must not reuse a cached download redirect from the
/// previous bytes. The published digest versions the URL without extra files.
pub(crate) fn asset_download_url(asset: &Asset) -> Result<String> {
    let mut url = reqwest::Url::parse(&asset.browser_download_url)?;
    if url.host_str() == Some("github.com") && url.path().contains("/releases/download/") {
        if let Some(digest) = &asset.digest {
            url.query_pairs_mut().append_pair("asset_digest", digest);
        }
    }
    Ok(url.into())
}

pub fn verify_asset(p: &Path, a: &Asset) -> Result<()> {
    if fs::metadata(p)?.len() != a.size {
        bail!("Download size mismatch");
    }
    let digest = a
        .digest
        .as_deref()
        .and_then(|d| d.strip_prefix("sha256:"))
        .context("Download has no published SHA-256 digest")?;
    if digest.len() != 64
        || !digest.bytes().all(|b| b.is_ascii_hexdigit())
        || !files::hash(p)?.eq_ignore_ascii_case(digest)
    {
        bail!("Download checksum mismatch");
    }
    Ok(())
}

#[cfg(test)]
mod token_tests {
    use super::*;

    #[test]
    fn replaced_release_assets_use_distinct_urls_without_changing_unverified_urls() {
        let mut asset = Asset {
            name: "manager.zip".into(),
            size: 1,
            digest: Some(format!("sha256:{}", "ab".repeat(32))),
            browser_download_url:
                "https://github.com/owner/manager/releases/download/v0.8.3/manager.zip?download=1"
                    .into(),
        };
        let first = asset_download_url(&asset).unwrap();
        asset.digest = Some(format!("sha256:{}", "cd".repeat(32)));
        let second = asset_download_url(&asset).unwrap();
        assert_ne!(first, second);
        let url = reqwest::Url::parse(&second).unwrap();
        assert_eq!(url.host_str(), Some("github.com"));
        assert!(url
            .query_pairs()
            .any(|(key, value)| key == "download" && value == "1"));
        assert!(url
            .query_pairs()
            .any(|(key, value)| key == "asset_digest"
                && Some(value.as_ref()) == asset.digest.as_deref()));
        asset.digest = None;
        assert_eq!(
            asset_download_url(&asset).unwrap(),
            asset.browser_download_url
        );
        asset.browser_download_url = "https://example.com/file?download=1".into();
        asset.digest = Some("sha256:fixture".into());
        assert_eq!(
            asset_download_url(&asset).unwrap(),
            asset.browser_download_url
        );
    }

    #[test]
    fn failed_download_publication_removes_partial_file() {
        use std::net::TcpListener;
        let root =
            std::env::temp_dir().join(format!("craft-download-publish-{}", uuid::Uuid::new_v4()));
        let dest = root.join("package.zip");
        fs::create_dir_all(&dest).unwrap(); // A directory cannot be replaced by the download.
        fs::write(dest.join("keep"), b"untouched").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            assert!(stream.read(&mut request).unwrap() > 0);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndata")
                .unwrap();
        });
        let job = Job::new(
            root.join("download.log"),
            &crate::model::BuilderPreferences::default(),
        );
        assert!(Network::new(&root)
            .unwrap()
            .download(&format!("http://{address}/file"), &dest, &job)
            .is_err());
        server.join().unwrap();
        assert_eq!(fs::read(dest.join("keep")).unwrap(), b"untouched");
        assert!(!fs::read_dir(&root).unwrap().any(|entry| entry
            .unwrap()
            .path()
            .extension()
            .is_some_and(|ext| ext == "partial")));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_input_is_rejected_before_accessing_the_credential_store() {
        for value in ["", "   ", "token with spaces", "token\nsecondline"] {
            assert!(save_token(value).is_err());
        }
    }

    #[test]
    fn token_validation_rejects_hidden_characters_before_storage() {
        for value in ["token\0suffix", "token\u{7f}suffix", "token\u{200b}suffix"] {
            assert!(
                validate_token(value).is_err(),
                "Hidden token characters must be rejected"
            );
        }
    }

    #[test]
    fn token_verification_reports_malformed_credentials_as_invalid() {
        assert_eq!(verify_token_value("token\0suffix"), TokenStatus::Invalid);
    }

    #[test]
    fn token_validation_preserves_printable_tokens_and_trims_outer_whitespace() {
        for value in ["ghp_example_123", "github_pat_example_123", "abc123-XYZ"] {
            assert_eq!(validate_token(&format!(" \n{value}\r\n")).unwrap(), value);
        }
    }

    #[test]
    fn authentication_status_distinguishes_rejected_tokens_from_service_failures() {
        assert_eq!(
            token_status_from_http(reqwest::StatusCode::OK),
            TokenStatus::Connected
        );
        assert_eq!(
            token_status_from_http(reqwest::StatusCode::UNAUTHORIZED),
            TokenStatus::Invalid
        );
        for status in [
            reqwest::StatusCode::FORBIDDEN,
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            reqwest::StatusCode::FOUND,
        ] {
            assert_eq!(token_status_from_http(status), TokenStatus::UnableToVerify);
        }
    }
}
