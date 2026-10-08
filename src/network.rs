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
impl Network {
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
            Ok(())
        })();
        if let Err(e) = result {
            let _ = fs::remove_file(&partial);
            return Err(e);
        }
        crate::platform::atomic_replace(&partial, dest)?;
        Ok(())
    }
    pub fn asset(&self, a: &Asset, dest: &Path, job: &Job) -> Result<()> {
        self.download(&a.browser_download_url, dest, job)?;
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
