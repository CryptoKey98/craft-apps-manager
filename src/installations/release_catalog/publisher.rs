//! Local publisher artifacts. No Git, upload, installer execution or deployment.
use super::*;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Publication {
    pub index: Index,
    pub objects: BTreeMap<String, AppCatalog>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Discovery {
    pub schema: u32,
    pub repositories: BTreeMap<String, Scan>,
    #[serde(default)]
    pub pages: BTreeMap<String, Page>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scan {
    pub checked_at: i64,
    pub latest_tag: Option<String>,
    pub releases: Vec<crate::model::Release>,
    pub error: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub etag: Option<String>,
    pub body: serde_json::Value,
}

pub fn validate_publication(publication: &Publication) -> Result<()> {
    verification::validate_index(&publication.index)?;
    ensure!(publication.objects.len() <= 1024, "Too many app objects");
    for (repo, reference) in &publication.index.repositories {
        let app = publication
            .objects
            .get(&reference.sha256)
            .context("Missing app object")?;
        verify_app(&serde_json::to_vec(app)?, repo, reference)?;
    }
    ensure!(
        publication.objects.keys().all(|hash| publication
            .index
            .repositories
            .values()
            .any(|r| r.sha256 == *hash)),
        "Unreferenced app object"
    );
    Ok(())
}
pub fn aggregate(
    previous: Option<&Publication>,
    discovery: &Discovery,
    evidence: &[store::Observation],
) -> Result<Publication> {
    ensure!(
        discovery.schema == 1 && discovery.repositories.len() <= 1024 && evidence.len() <= 100000,
        "Invalid discovery/evidence bounds"
    );
    if let Some(previous) = previous {
        validate_publication(previous)?;
    }
    let mut result = Publication {
        index: Index {
            schema: 1,
            purpose: "app-release-catalog".into(),
            sequence: previous.map_or(Ok(1), |p| {
                p.index
                    .sequence
                    .checked_add(1)
                    .context("Catalog sequence overflow")
            })?,
            generated_at: chrono::Utc::now().timestamp(),
            repositories: BTreeMap::new(),
        },
        objects: BTreeMap::new(),
    };
    for (repo, scan) in &discovery.repositories {
        ensure!(
            verification::canonical_repo(repo)? == *repo,
            "Invalid discovery repository"
        );
        let old = previous.and_then(|p| {
            p.index
                .repositories
                .get(repo)
                .and_then(|r| p.objects.get(&r.sha256).map(|a| (r, a)))
        });
        if scan.error.is_some() {
            if let Some((reference, app)) = old {
                result
                    .index
                    .repositories
                    .insert(repo.clone(), reference.clone());
                result.objects.insert(reference.sha256.clone(), app.clone());
            }
            continue;
        }
        let mut app = AppCatalog {
            schema: 1,
            repository: repo.clone(),
            latest_tag: scan.latest_tag.clone(),
            releases: scan.releases.clone(),
            scopes: Vec::new(),
        };
        let existing = old.map(|(_, a)| a.scopes.as_slice()).unwrap_or(&[]);
        for item in existing.iter().chain(
            evidence
                .iter()
                .filter(|o| o.repository == *repo)
                .map(|o| &o.evidence),
        ) {
            if item.inspector_version != inspection::INSPECTOR_VERSION {
                continue;
            }
            let same = app
                .releases
                .iter()
                .find(|r| r.tag_name == item.tag)
                .and_then(|r| r.assets.iter().find(|a| a.name == item.asset_name))
                .is_some_and(|a| a.size == item.size && a.digest.as_deref() == Some(&item.digest));
            if same {
                app.scopes.retain(|s| {
                    !(s.tag == item.tag
                        && s.asset_name == item.asset_name
                        && s.architecture == item.architecture)
                });
                app.scopes.push(item.clone());
            }
        }
        app.scopes.sort_by(|a, b| {
            (&a.tag, &a.asset_name, &a.architecture).cmp(&(&b.tag, &b.asset_name, &b.architecture))
        });
        verification::validate_app(&app)?;
        let bytes = serde_json::to_vec(&app)?;
        let reference = AppReference {
            sha256: verification::hash(&bytes),
            size: bytes.len() as u64,
            checked_at: scan.checked_at,
            discovery_added_at: old
                .and_then(|(reference, _)| reference.discovery_added_at)
                .or_else(|| crate::catalog::discovery::bundled_added_at(repo))
                .or_else(|| {
                    (old.is_none()
                        && !crate::catalog::discovery::bundled_admissions().contains_key(repo))
                    .then_some(result.index.generated_at)
                }),
        };
        result.objects.insert(reference.sha256.clone(), app);
        result.index.repositories.insert(repo.clone(), reference);
    }
    validate_publication(&result)?;
    Ok(result)
}

pub trait Upstream {
    fn page(&self, url: &str, previous: Option<&Page>) -> Result<Option<Page>>;
}
/// Extend the approved set using the same repository and release checks as discovery.
/// A transient discovery failure never removes an already indexed repository.
pub fn enroll(repositories: &[String], upstream: &dyn Upstream) -> Result<Vec<String>> {
    ensure!(
        repositories.len() <= 1024,
        "Repository enrollment exceeds bounds"
    );
    let mut result = std::collections::BTreeSet::new();
    for repository in repositories {
        ensure!(
            verification::canonical_repo(repository)? == *repository,
            "Invalid enrolled repository"
        );
        result.insert(repository.clone());
    }
    #[derive(Deserialize)]
    struct Candidate {
        name: String,
        #[serde(default)]
        archived: bool,
        #[serde(default)]
        fork: bool,
        #[serde(default)]
        disabled: bool,
    }
    let mut probes = 0;
    for page in 1..=10 {
        let url = format!(
            "https://api.github.com/orgs/{}/repos?per_page=100&page={page}",
            crate::catalog::ORG
        );
        let response = match upstream.page(&url, None) {
            Ok(Some(page)) => page,
            _ => break,
        };
        let candidates: Vec<Candidate> = match serde_json::from_value(response.body) {
            Ok(value) => value,
            Err(_) => break,
        };
        let last = candidates.len() < 100;
        for candidate in candidates {
            if !crate::catalog::discovery_candidate(
                &candidate.name,
                candidate.archived,
                candidate.fork,
                candidate.disabled,
            ) || crate::catalog::EXTERNAL_APPS
                .iter()
                .any(|app| app.key == candidate.name)
            {
                continue;
            }
            let repository = format!("{}/{}", crate::catalog::ORG, candidate.name);
            if result.contains(&repository)
                || crate::catalog::builtin()
                    .iter()
                    .any(|e| e.repository == candidate.name && !e.release)
            {
                continue;
            }
            if probes >= 256 || result.len() >= 1024 {
                break;
            }
            probes += 1;
            let url = format!("https://api.github.com/repos/{repository}/releases/latest");
            let release = match upstream.page(&url, None) {
                Ok(Some(page)) => serde_json::from_value::<crate::model::Release>(page.body).ok(),
                _ => None,
            };
            let entry = crate::catalog::classify(&candidate.name, release.as_ref());
            if crate::catalog::verified_discovery(&entry) {
                result.insert(repository);
            }
        }
        if last || probes >= 256 || result.len() >= 1024 {
            break;
        }
    }
    for source in crate::catalog::EXTERNAL_APPS {
        if result.contains(source.repository) || result.len() >= 1024 {
            continue;
        }
        if matches!(
            crate::catalog::external_discovery_with(source, |url| {
                upstream
                    .page(url, None)
                    .map(|page| page.map(|page| page.body))
            }),
            Ok(Some(_))
        ) {
            result.insert(source.repository.into());
        }
    }
    Ok(result.into_iter().collect())
}
pub struct Github(reqwest::blocking::Client);
impl Github {
    pub fn new() -> Result<Self> {
        Ok(Self(
            reqwest::blocking::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent("CraftApps-CatalogPublisher/1")
                .build()?,
        ))
    }
}
impl Upstream for Github {
    fn page(&self, url: &str, previous: Option<&Page>) -> Result<Option<Page>> {
        use std::io::Read;
        let parsed = reqwest::Url::parse(url)?;
        ensure!(
            parsed.scheme() == "https"
                && parsed.host_str() == Some("api.github.com")
                && parsed.username().is_empty()
                && parsed.password().is_none()
                && parsed.port().is_none(),
            "Unexpected GitHub metadata origin"
        );
        let mut request = self
            .0
            .get(url)
            .header("Accept", "application/vnd.github+json");
        if let Some(token) = std::env::var("GITHUB_TOKEN").ok().filter(|s| !s.is_empty()) {
            request = request.bearer_auth(token);
        }
        if let Some(etag) = previous.and_then(|p| p.etag.as_deref()) {
            request = request.header(reqwest::header::IF_NONE_MATCH, etag);
        }
        let response = request.send()?;
        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(Some(previous.context("304 without prior page")?.clone()));
        }
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        ensure!(
            response.status().is_success(),
            "GitHub metadata returned {}",
            response.status()
        );
        let etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .filter(|s| s.len() <= 1024)
            .map(str::to_owned);
        let mut bytes = Vec::new();
        response.take(8 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 8 * 1024 * 1024, "Oversized upstream page");
        Ok(Some(Page {
            etag,
            body: serde_json::from_slice(&bytes)?,
        }))
    }
}
pub fn discover(
    repositories: &[String],
    previous: Option<&Discovery>,
    upstream: &dyn Upstream,
) -> Result<Discovery> {
    ensure!(repositories.len() <= 1024, "Repository scan exceeds budget");
    let mut result = Discovery {
        schema: 1,
        ..Default::default()
    };
    let mut requests = 0;
    for repo in repositories {
        ensure!(
            verification::canonical_repo(repo)? == *repo && !result.repositories.contains_key(repo),
            "Invalid/duplicate repository allowlist entry"
        );
        let scan = (|| -> Result<Scan> {
            let mut releases = Vec::new();
            for page in 1..=101 {
                requests += 1;
                ensure!(
                    requests <= 256,
                    "Upstream request budget exhausted; remaining repositories deferred"
                );
                let url = format!(
                    "https://api.github.com/repos/{repo}/releases?per_page=100&page={page}"
                );
                let mut response = upstream
                    .page(&url, previous.and_then(|p| p.pages.get(&url)))?
                    .context("Repository release history unavailable")?;
                let batch: Vec<crate::model::Release> =
                    serde_json::from_value(response.body.clone())?;
                let done = batch.len() < 100;
                // Retain only fields used for installation, including on a 304
                // from an older checkpoint. Release bodies/authors are not needed.
                response.body = serde_json::to_value(&batch)?;
                result.pages.insert(url, response);
                releases.extend(batch.into_iter().filter(|r| !r.draft && !r.prerelease));
                ensure!(
                    releases.len() <= 10000 && (page <= 100 || done),
                    "Release history exceeds supported bounds"
                );
                if done {
                    break;
                }
            }
            requests += 1;
            ensure!(requests <= 256, "Upstream request budget exhausted");
            let url = format!("https://api.github.com/repos/{repo}/releases/latest");
            let latest = upstream.page(&url, previous.and_then(|p| p.pages.get(&url)))?;
            let latest_tag = if let Some(mut response) = latest {
                let release: crate::model::Release = serde_json::from_value(response.body.clone())?;
                ensure!(
                    !release.draft && !release.prerelease,
                    "Unexpected latest release type"
                );
                response.body = serde_json::to_value(&release)?;
                result.pages.insert(url, response);
                Some(release.tag_name)
            } else {
                None
            };
            let app = AppCatalog {
                schema: 1,
                repository: repo.clone(),
                latest_tag: latest_tag.clone(),
                releases: releases.clone(),
                scopes: Vec::new(),
            };
            verification::validate_app(&app)?;
            Ok(Scan {
                checked_at: chrono::Utc::now().timestamp(),
                latest_tag,
                releases,
                error: None,
            })
        })();
        result.repositories.insert(
            repo.clone(),
            scan.unwrap_or_else(|e| Scan {
                checked_at: 0,
                latest_tag: None,
                releases: Vec::new(),
                error: Some(format!("{e:#}")),
            }),
        );
    }
    ensure!(
        serde_json::to_vec(&result)?.len() <= 128 * 1024 * 1024,
        "Discovery artifact exceeds budget"
    );
    Ok(result)
}
pub fn sign(
    publication: &Publication,
    key: &ed25519_dalek::SigningKey,
    key_id: &str,
    out: &std::path::Path,
) -> Result<()> {
    use ed25519_dalek::Signer;
    validate_publication(publication)?;
    ensure!(
        !key_id.is_empty()
            && key_id.len() <= 64
            && key_id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c)),
        "Invalid signing key ID"
    );
    let payload = serde_json::to_string(&publication.index)?;
    let mut message = verification::DOMAIN.to_vec();
    message.extend_from_slice(payload.as_bytes());
    let envelope = Envelope {
        key_id: key_id.into(),
        signature: verification::hex(&key.sign(&message).to_bytes()),
        payload,
    };
    let bytes = serde_json::to_vec(&envelope)?;
    ensure!(
        bytes.len() <= verification::MAX_ROOT,
        "Signed index exceeds limit"
    );
    // Fixed digest paths only. Objects are durable before the root is replaced.
    for (hash, app) in &publication.objects {
        let path = out.join("apps").join(format!("{hash}.json"));
        verification::unlinked_ancestors(&path)?;
        crate::files::write_bytes(&path, &serde_json::to_vec(app)?)?;
    }
    let index = out.join("index.json");
    verification::unlinked_ancestors(&index)?;
    crate::files::write_bytes(&index, &bytes)
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectionInput {
    pub repository: String,
    pub tag: String,
    pub asset: crate::model::Asset,
    pub architecture: String,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectionBatch {
    pub schema: u32,
    pub observations: Vec<store::Observation>,
    pub attempts: BTreeMap<String, i64>,
    pub errors: BTreeMap<String, String>,
}
pub fn inspection_identity(input: &InspectionInput) -> String {
    format!(
        "{}:{}:{}:{}",
        input.repository,
        input.tag,
        input.asset.name,
        input.asset.digest.as_deref().unwrap_or_default()
    )
}
pub fn inspection_queue(
    discovery: &Discovery,
    previous: Option<&Publication>,
    report: &InspectionBatch,
    attempted_since: Option<i64>,
) -> Vec<InspectionInput> {
    let mut work = pending(discovery, previous);
    work.retain(|input| {
        !report.observations.iter().any(|o| {
            o.repository == input.repository
                && o.evidence.tag == input.tag
                && o.evidence.asset_name == input.asset.name
                && Some(&o.evidence.digest) == input.asset.digest.as_ref()
                && o.evidence.size == input.asset.size
                && o.evidence.inspector_version == inspection::INSPECTOR_VERSION
        }) && !attempted_since.is_some_and(|since| {
            report
                .attempts
                .get(&inspection_identity(input))
                .is_some_and(|t| *t >= since)
        })
    });
    work.sort_by_key(|input| {
        report
            .attempts
            .get(&inspection_identity(input))
            .copied()
            .unwrap_or(0)
    });
    work
}
pub fn pending(discovery: &Discovery, previous: Option<&Publication>) -> Vec<InspectionInput> {
    let mut result = Vec::new();
    for (repo, scan) in &discovery.repositories {
        if scan.error.is_some() {
            continue;
        }
        for release in &scan.releases {
            for asset in &release.assets {
                if !asset.name.to_ascii_lowercase().ends_with(".msi")
                    || asset
                        .digest
                        .as_deref()
                        .and_then(|s| s.strip_prefix("sha256:"))
                        .is_none()
                {
                    continue;
                }
                let known = previous
                    .and_then(|p| {
                        p.index
                            .repositories
                            .get(repo)
                            .and_then(|r| p.objects.get(&r.sha256))
                    })
                    .is_some_and(|a| {
                        a.scopes.iter().any(|e| {
                            e.tag == release.tag_name
                                && e.asset_name == asset.name
                                && e.digest == asset.digest.as_deref().unwrap_or_default()
                                && e.size == asset.size
                                && e.inspector_version == inspection::INSPECTOR_VERSION
                        })
                    });
                if !known {
                    result.push(InspectionInput {
                        repository: repo.clone(),
                        tag: release.tag_name.clone(),
                        asset: asset.clone(),
                        architecture: String::new(),
                    });
                }
            }
        }
    }
    result
}
pub fn inspect_pending(
    discovery: &Discovery,
    previous: Option<&Publication>,
    prior: Option<&InspectionBatch>,
    staging: &std::path::Path,
    budget: usize,
) -> Result<InspectionBatch> {
    ensure!((1..=32).contains(&budget), "Invalid inspection budget");
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (discovery, previous, prior, staging);
        anyhow::bail!("MSI inspection requires Windows");
    }
    #[cfg(target_os = "windows")]
    {
        use std::io::{Read, Write};
        let mut report = prior.cloned().unwrap_or_default();
        report.schema = 1;
        let work = inspection_queue(discovery, previous, &report, None);
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(120))
            .user_agent("CraftApps-CatalogInspection/1")
            .build()?;
        for input in work.into_iter().take(budget) {
            let identity = inspection_identity(&input);
            report
                .attempts
                .insert(identity.clone(), chrono::Utc::now().timestamp());
            let owned = staging.join(uuid::Uuid::new_v4().to_string());
            for ancestor in owned.ancestors().filter(|p| p.exists()) {
                ensure!(
                    !crate::files::linked(ancestor)?,
                    "Linked inspection workspace"
                );
            }
            struct Clean(std::path::PathBuf);
            impl Drop for Clean {
                fn drop(&mut self) {
                    let _ = std::fs::remove_dir_all(&self.0);
                }
            }
            let _clean = Clean(owned.clone());
            let result = (|| -> Result<ScopeEvidence> {
                ensure!(
                    input.asset.size <= 512 * 1024 * 1024
                        && crate::updates::expected_asset_url(
                            &input.asset.browser_download_url,
                            &input.repository
                        ),
                    "Invalid inspection download"
                );
                std::fs::create_dir_all(&owned)?;
                let file = owned.join("package.msi");
                let response = client
                    .get(&input.asset.browser_download_url)
                    .send()?
                    .error_for_status()?;
                let mut reader = response.take(input.asset.size + 1);
                let mut output = std::fs::File::create(&file)?;
                let bytes = std::io::copy(&mut reader, &mut output)?;
                output.flush()?;
                drop(output);
                ensure!(bytes == input.asset.size, "Inspection size mismatch");
                crate::network::verify_asset(&file, &input.asset)?;
                let identity = crate::installers::rollback_package_identity(&file)?;
                inspection::inspect(
                    &file,
                    &input.repository,
                    &input.tag,
                    &input.asset,
                    &identity.architecture,
                )
            })();
            match result {
                Ok(evidence) => {
                    report.errors.remove(&identity);
                    report.observations.push(store::Observation {
                        repository: input.repository,
                        evidence,
                        used: chrono::Utc::now().timestamp(),
                    });
                }
                Err(error) => {
                    report.errors.insert(identity, format!("{error:#}"));
                }
            }
        }
        let _ = std::fs::remove_dir(staging);
        ensure!(
            report.observations.len() <= 100000 && report.attempts.len() <= 100000,
            "Inspection report exceeds bounds"
        );
        Ok(report)
    }
}
