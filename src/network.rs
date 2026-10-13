use crate::{files, jobs::Job, model::Asset};
use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    time::Duration,
};
#[derive(Serialize, Deserialize)]
struct Cached {
    at: i64,
    etag: Option<String>,
    value: serde_json::Value,
}
pub struct Network {
    client: Client,
    cache: std::path::PathBuf,
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
fn credential_entry() -> Result<keyring::Entry> {
    keyring::Entry::new("Craft Apps Manager", "github-api-token")
        .map_err(|_| anyhow::anyhow!("Cannot access the system credential store."))
}
fn saved_token_cache() -> &'static std::sync::RwLock<Option<String>> {
    SAVED_TOKEN.get_or_init(|| {
        let value = credential_entry()
            .ok()
            .and_then(|entry| entry.get_password().ok());
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
    SAVED_TOKEN
        .get()
        .and_then(|cache| cache.read().ok())
        .is_some_and(|value| value.is_some())
}
pub fn save_token(value: &str) -> Result<()> {
    let value = value.trim();
    anyhow::ensure!(
        !value.is_empty() && !value.chars().any(char::is_whitespace),
        "Enter a token without spaces or line breaks."
    );
    let cache = saved_token_cache();
    let mut saved = cache
        .write()
        .map_err(|_| anyhow::anyhow!("Token storage is unavailable."))?;
    credential_entry()?.set_password(value)
        .map_err(|_| anyhow::anyhow!("Could not save the token. Unlock or enable your system credential store and try again."))?;
    *saved = Some(value.into());
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
        .map(|response| token_status_from_http(response.status()))
        .unwrap_or(TokenStatus::UnableToVerify)
}
impl Network {
    /// Reads known release metadata without constructing a client or contacting GitHub.
    pub fn cached_json<T: DeserializeOwned>(root: &Path, url: &str) -> Option<T> {
        use sha2::{Digest, Sha256};
        let path = root
            .join("runtime/api-cache")
            .join(format!("{:x}.json", Sha256::digest(url.as_bytes())));
        let cached = files::read_json::<Cached>(&path).ok()?;
        serde_json::from_value(cached.value).ok()
    }
    pub fn new(root: &Path) -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .user_agent(concat!("CraftApps-Manager/", env!("CARGO_PKG_VERSION")))
                .connect_timeout(Duration::from_secs(20))
                .timeout(Duration::from_secs(300))
                .build()?,
            cache: root.join("runtime/api-cache"),
        })
    }
    /// Like `json`, but `None` when GitHub reports the resource missing (404),
    /// such as `releases/latest` for a repository without releases.
    pub fn json_optional<T: DeserializeOwned>(&self, url: &str) -> Result<Option<T>> {
        match self.json(url) {
            Ok(value) => Ok(Some(value)),
            Err(error)
                if error
                    .downcast_ref::<reqwest::Error>()
                    .and_then(reqwest::Error::status)
                    .is_some_and(|s| s.as_u16() == 404) =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }
    pub fn json<T: DeserializeOwned>(&self, url: &str) -> Result<T> {
        use sha2::{Digest, Sha256};
        let path = self
            .cache
            .join(format!("{:x}.json", Sha256::digest(url.as_bytes())));
        let old = files::read_json::<Cached>(&path).ok();
        if let Some(c) = &old {
            if chrono::Utc::now().timestamp() - c.at < 300 {
                return Ok(serde_json::from_value(c.value.clone())?);
            }
        }
        let mut request = self
            .client
            .get(url)
            .header("Accept", "application/vnd.github+json");
        if let Some(etag) = old.as_ref().and_then(|c| c.etag.as_ref()) {
            request = request.header("If-None-Match", etag);
        }
        // An optional token raises GitHub's hourly API limit; it is sent only to the API host.
        if url.starts_with("https://api.github.com/") {
            if let Some((_, token)) = github_token() {
                request = request.bearer_auth(token);
            }
        }
        let response = request.send()?;
        if response.status().as_u16() == 304 {
            let mut c = old.context("No cached response")?;
            c.at = chrono::Utc::now().timestamp();
            files::write_json(&path, &c)?;
            return Ok(serde_json::from_value(c.value)?);
        }
        if response.status().as_u16() == 403 || response.status().as_u16() == 429 {
            let reset = response
                .headers()
                .get("x-ratelimit-reset")
                .and_then(|h| h.to_str().ok())
                .and_then(|v| v.parse::<i64>().ok())
                .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
                .map(|d| {
                    d.with_timezone(&chrono::Local)
                        .format("%I:%M %p")
                        .to_string()
                });
            let text = response.text().unwrap_or_default();
            if text.contains("rate limit") || reset.is_some() {
                bail!(
                    "GitHub API limit reached{}. Wait before retrying.",
                    reset
                        .map(|t| format!("; resets at {t}"))
                        .unwrap_or_default()
                );
            }
            bail!("GitHub denied the request (403). {text}");
        }
        let response = response.error_for_status()?;
        let etag = response
            .headers()
            .get("etag")
            .and_then(|h| h.to_str().ok())
            .map(str::to_owned);
        let value: serde_json::Value = response.json()?;
        let result = serde_json::from_value(value.clone())?;
        files::write_json(
            &path,
            &Cached {
                at: chrono::Utc::now().timestamp(),
                etag,
                value,
            },
        )?;
        Ok(result)
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
        self.download(&a.browser_download_url, dest, job)?;
        job.check()?;
        job.stage(
            "Verifying checksum",
            None,
            "Checking SHA-256 before installation",
        );
        verify_asset(dest, a)
    }
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
