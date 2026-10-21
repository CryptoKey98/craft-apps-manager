use super::model::MAX_ENVELOPE;
use anyhow::{ensure, Result};
use std::{io::Read, time::Duration};

#[derive(Clone, Debug)]
pub struct ValidatedEndpoint(reqwest::Url);
impl ValidatedEndpoint {
    pub fn new(value: &str) -> Result<Self> {
        ensure!(value.len() <= 2048, "Metadata endpoint too long");
        let url = reqwest::Url::parse(value)?;
        ensure!(
            url.scheme() == "https"
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.fragment().is_none()
                && url.query().is_none(),
            "Untrusted metadata endpoint"
        );
        Ok(Self(url))
    }
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
    pub fn is_dynamic(&self) -> bool {
        self.0.path() == "/v2/check"
    }
}
pub enum FetchResponse {
    Body {
        bytes: Vec<u8>,
        etag: Option<String>,
    },
    NotModified,
    RateLimited(u64),
}
pub trait FetchMetadata {
    fn fetch(&self, endpoint: &ValidatedEndpoint, etag: Option<&str>) -> Result<FetchResponse>;
    fn fetch_for_profile(
        &self,
        endpoint: &ValidatedEndpoint,
        etag: Option<&str>,
        _profile: &super::model::ClientProfile,
    ) -> Result<FetchResponse> {
        self.fetch(endpoint, etag)
    }
}
// Only a configured GitHub release mirror can redirect, and only through the
// same bounded HTTPS asset hosts used for package downloads. Other metadata
// origins must continue serving their signed envelope directly.
fn metadata_redirect_allowed(previous: &[reqwest::Url], target: &reqwest::Url) -> bool {
    let Some(origin) = previous.first() else {
        return false;
    };
    let release_asset = origin.path_segments().is_some_and(|segments| {
        let parts: Vec<_> = segments.collect();
        matches!(parts.as_slice(), [owner, repo, "releases", "download", tag, asset]
            if !owner.is_empty() && !repo.is_empty() && !tag.is_empty() && !asset.is_empty())
    });
    previous.len() <= 3
        && origin.host_str() == Some("github.com")
        && release_asset
        && origin.query().is_none()
        && previous.iter().all(asset_host_allowed)
        && asset_host_allowed(target)
}

pub struct MetadataTransport {
    client: reqwest::blocking::Client,
}
impl MetadataTransport {
    pub fn new() -> Result<Self> {
        Self::with_timeouts(Duration::from_secs(10), Duration::from_secs(30))
    }
    pub fn with_timeouts(connect: Duration, metadata: Duration) -> Result<Self> {
        Ok(Self {
            client: reqwest::blocking::Client::builder()
                .connect_timeout(connect)
                .timeout(metadata)
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    if metadata_redirect_allowed(attempt.previous(), attempt.url()) {
                        attempt.follow()
                    } else {
                        attempt.stop()
                    }
                }))
                .user_agent("CraftAppsManager-Updater/2")
                .build()?,
        })
    }
    fn request(
        &self,
        endpoint: &ValidatedEndpoint,
        etag: Option<&str>,
    ) -> reqwest::blocking::RequestBuilder {
        let mut request = self
            .client
            .get(endpoint.0.clone())
            .header(reqwest::header::ACCEPT, "application/json");
        if let Some(etag) = etag {
            request = request.header(reqwest::header::IF_NONE_MATCH, etag);
        }
        request
    }
}
impl FetchMetadata for MetadataTransport {
    fn fetch_for_profile(
        &self,
        endpoint: &ValidatedEndpoint,
        etag: Option<&str>,
        profile: &super::model::ClientProfile,
    ) -> Result<FetchResponse> {
        if !endpoint.is_dynamic() {
            return self.fetch(endpoint, etag);
        }
        let mut url = endpoint.0.clone();
        let kind = serde_json::to_value(&profile.installation)?
            .as_str()
            .expect("installation enum string")
            .to_owned();
        url.query_pairs_mut().extend_pairs([
            ("repository", profile.repository.clone()),
            ("channel", profile.channel.clone()),
            ("version", profile.version.clone()),
            ("revision", profile.revision.to_string()),
            ("platform", profile.platform.clone()),
            ("architecture", profile.architecture.clone()),
            ("installation", kind),
            (
                "capabilities",
                profile
                    .capabilities
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(","),
            ),
        ]);
        self.fetch(&ValidatedEndpoint(url), None)
    }
    fn fetch(&self, endpoint: &ValidatedEndpoint, etag: Option<&str>) -> Result<FetchResponse> {
        // Deliberately independent of Network: no GitHub bearer or cookies.
        let response = self.request(endpoint, etag).send()?;
        if response.status().as_u16() == 429 {
            let seconds = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .map(retry_after)
                .unwrap_or(60);
            return Ok(FetchResponse::RateLimited(seconds));
        }
        if response.status().as_u16() == 304 {
            return Ok(FetchResponse::NotModified);
        }
        ensure!(
            response.status().is_success(),
            "Metadata request failed ({})",
            response.status()
        );
        ensure!(
            response
                .content_length()
                .is_none_or(|n| n <= MAX_ENVELOPE as u64),
            "Metadata too large"
        );
        let etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .filter(|s| s.len() <= 256)
            .map(str::to_owned);
        let mut bytes = Vec::new();
        response
            .take(MAX_ENVELOPE as u64 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= MAX_ENVELOPE, "Metadata too large");
        Ok(FetchResponse::Body { bytes, etag })
    }
}
pub fn retry_after(text: &str) -> u64 {
    text.parse::<u64>()
        .ok()
        .or_else(|| {
            chrono::DateTime::parse_from_rfc2822(text).ok().map(|date| {
                date.timestamp()
                    .saturating_sub(chrono::Utc::now().timestamp())
                    .max(0) as u64
            })
        })
        .unwrap_or(60)
        .clamp(1, 3600)
}
pub fn asset_host_allowed(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && matches!(
            url.host_str(),
            Some(
                "github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
            )
        )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_mirror_redirects_are_limited_to_github_release_assets() {
        let url = |s: &str| reqwest::Url::parse(s).unwrap();
        let github =
            url("https://github.com/owner/manager/releases/download/v1.0.0/updates-v2.json");
        let asset =
            url("https://release-assets.githubusercontent.com/asset?signature=public-download");
        assert!(metadata_redirect_allowed(
            std::slice::from_ref(&github),
            &asset
        ));
        assert!(metadata_redirect_allowed(
            &[github.clone(), asset.clone()],
            &asset
        ));
        for target in [
            "https://evil.example/catalog.json",
            "http://release-assets.githubusercontent.com/asset",
            "https://user:secret@github.com/asset",
            "https://github.com:444/asset",
        ] {
            assert!(!metadata_redirect_allowed(
                std::slice::from_ref(&github),
                &url(target)
            ));
        }
        for origin in [
            "https://updates.example/catalog.json",
            "https://github.com/owner/manager",
            "https://github.com/owner/manager/releases/latest",
            "https://github.com/owner/manager/releases/download/v1.0.0/a/b",
        ] {
            assert!(!metadata_redirect_allowed(&[url(origin)], &asset));
        }
        assert!(!metadata_redirect_allowed(&[], &asset));
        assert!(!metadata_redirect_allowed(&vec![github; 4], &asset));
    }

    #[test]
    fn never_sends_github_authorization() {
        let transport = MetadataTransport::new().unwrap();
        let request = transport
            .request(
                &ValidatedEndpoint::new("https://updates.example/v2/catalog.json").unwrap(),
                Some("cached"),
            )
            .build()
            .unwrap();
        assert!(!request
            .headers()
            .contains_key(reqwest::header::AUTHORIZATION));
        assert!(!request.headers().contains_key(reqwest::header::COOKIE));
        assert_eq!(request.headers()[reqwest::header::IF_NONE_MATCH], "cached");
    }
}
