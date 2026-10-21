use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const ZIP: &str = "archive.zip.v1";
pub const TAR: &str = "archive.tar-gzip.v1";
pub const DMG: &str = "disk-image.dmg.v1";
pub const LAYOUT: &str = "layout.manager.v1";
pub const MAX_PAYLOAD: usize = 1024 * 1024;
pub const MAX_ENVELOPE: usize = 2 * MAX_PAYLOAD;
pub const VALIDITY: i64 = 30 * 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum InstallationKind {
    Portable,
    Bundle,
    Msi,
    Deb,
    Rpm,
    Arch,
    External,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Position {
    pub version: String,
    pub revision: u32,
}
impl Position {
    pub fn key(&self) -> Result<([u64; 4], u32)> {
        ensure!((1..=65535).contains(&self.revision), "Invalid revision");
        let version = self.version.strip_prefix('v').unwrap_or(&self.version);
        ensure!(
            (2..=4).contains(&version.split('.').count())
                && version
                    .split('.')
                    .all(|p| !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit())),
            "Unsupported stable catalog version"
        );
        let key = stable_version(version)?;
        ensure!(
            key.iter().all(|v| *v <= 9_007_199_254_740_991),
            "Version exceeds protocol numeric range"
        );
        Ok((key, self.revision))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientProfile {
    pub repository: String,
    pub channel: String,
    pub version: String,
    pub revision: u32,
    pub platform: String,
    pub architecture: String,
    pub installation: InstallationKind,
    pub capabilities: BTreeSet<String>,
}
impl ClientProfile {
    pub fn position(&self) -> Position {
        Position {
            version: self.version.clone(),
            revision: self.revision,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseRecord {
    pub id: String,
    pub version: String,
    pub tag: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutDescriptor {
    pub schema: u32,
    pub root: String,
    pub entrypoint: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageRecord {
    pub id: String,
    pub release: String,
    pub revision: u32,
    pub platform: String,
    pub architecture: String,
    pub installation: InstallationKind,
    pub handler: String,
    #[serde(default)]
    pub required_capabilities: BTreeSet<String>,
    #[serde(default)]
    pub native_version: Option<String>,
    pub asset: String,
    pub size: u64,
    pub sha256: String,
    pub url: String,
    pub layout: LayoutDescriptor,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteRecord {
    pub id: String,
    pub min_source: Position,
    #[serde(default)]
    pub max_source: Option<Position>,
    pub platform: String,
    pub architecture: String,
    pub installation: InstallationKind,
    #[serde(default)]
    pub required_capabilities: BTreeSet<String>,
    pub target: String,
    pub bridge: bool,
    #[serde(default)]
    pub destination_capabilities: BTreeSet<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalog {
    pub schema: u32,
    pub repository: String,
    pub channel: String,
    pub sequence: u64,
    pub issued: i64,
    pub expires: i64,
    pub releases: Vec<ReleaseRecord>,
    pub packages: Vec<PackageRecord>,
    pub routes: Vec<RouteRecord>,
    #[serde(default)]
    pub withdrawn: BTreeSet<String>,
    #[serde(default)]
    pub metadata_origins: Vec<String>,
}
fn bounded(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control),
        "Invalid metadata field"
    );
    Ok(())
}
fn unique<'a>(ids: impl Iterator<Item = &'a str>) -> Result<()> {
    let mut seen = BTreeSet::new();
    for id in ids {
        bounded(id)?;
        ensure!(
            id.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)),
            "Catalog IDs must be ASCII identifiers"
        );
        ensure!(seen.insert(id), "Duplicate catalog ID");
    }
    Ok(())
}
pub fn relative(value: &str) -> Result<()> {
    bounded(value)?;
    safe_relative(value)?;
    ensure!(
        !value.split('/').any(|s| s.is_empty() || s == "."),
        "Non-canonical relative path"
    );
    Ok(())
}
impl Catalog {
    pub fn release(&self, package: &PackageRecord) -> &ReleaseRecord {
        self.releases
            .iter()
            .find(|r| r.id == package.release)
            .expect("validated release reference")
    }
    pub fn position(&self, package: &PackageRecord) -> Position {
        Position {
            version: self.release(package).version.clone(),
            revision: package.revision,
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.metadata_origins.len() <= 4,
            "Too many metadata origins"
        );
        for origin in &self.metadata_origins {
            super::transport::ValidatedEndpoint::new(origin)?;
        }
        ensure!(self.schema == 2, "Unsupported required catalog schema");
        ensure!(
            (1..=9_007_199_254_740_991).contains(&self.sequence),
            "Invalid catalog sequence"
        );
        ensure!(
            self.releases.len() <= 128
                && self.packages.len() <= 512
                && self.routes.len() <= 512
                && self.withdrawn.len() <= 512,
            "Catalog collection limit"
        );
        bounded(&self.repository)?;
        bounded(&self.channel)?;
        let parts: Vec<_> = self.repository.split('/').collect();
        ensure!(
            parts.len() == 2
                && parts.iter().all(|p| !p.is_empty()
                    && *p != "."
                    && *p != ".."
                    && p.bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))),
            "Invalid repository"
        );
        unique(self.releases.iter().map(|r| r.id.as_str()))?;
        unique(self.packages.iter().map(|p| p.id.as_str()))?;
        unique(self.routes.iter().map(|r| r.id.as_str()))?;
        for r in &self.releases {
            bounded(&r.version)?;
            relative(&r.tag)?;
            ensure!(!r.tag.contains('/'), "Invalid release tag");
            Position {
                version: r.version.clone(),
                revision: 1,
            }
            .key()?;
        }
        for p in &self.packages {
            ensure!(
                self.releases.iter().any(|r| r.id == p.release),
                "Missing release reference"
            );
            self.position(p).key()?;
            for s in [&p.platform, &p.architecture, &p.handler] {
                bounded(s)?;
            }
            ensure!(p.required_capabilities.len() <= 32, "Too many capabilities");
            for s in &p.required_capabilities {
                bounded(s)?;
            }
            if let Some(v) = &p.native_version {
                bounded(v)?;
            }
            ensure!(
                p.size > 0
                    && p.size <= 4 * 1024 * 1024 * 1024
                    && p.sha256.len() == 64
                    && p.sha256.bytes().all(|c| c.is_ascii_hexdigit()),
                "Invalid package size/hash"
            );
            relative(&p.asset)?;
            ensure!(!p.asset.contains('/'), "Asset must be a filename");
            relative(&p.layout.root)?;
            relative(&p.layout.entrypoint)?;
            ensure!(p.url.len() <= 2048, "Package URL too long");
            let url = reqwest::Url::parse(&p.url)?;
            let mut expected = reqwest::Url::parse("https://github.com/")?;
            expected
                .path_segments_mut()
                .expect("HTTPS base")
                .clear()
                .extend(self.repository.split('/'))
                .extend(["releases", "download", &self.release(p).tag, &p.asset]);
            ensure!(url == expected, "Untrusted asset URL");
        }
        for r in &self.routes {
            r.min_source.key()?;
            if let Some(max) = &r.max_source {
                ensure!(max.key()? >= r.min_source.key()?, "Invalid source range");
            }
            ensure!(
                r.required_capabilities.len() <= 32 && r.destination_capabilities.len() <= 32,
                "Too many capabilities"
            );
            for c in r
                .required_capabilities
                .iter()
                .chain(&r.destination_capabilities)
            {
                bounded(c)?;
            }
            let p = self
                .packages
                .iter()
                .find(|p| p.id == r.target)
                .context("Missing route target")?;
            ensure!(
                r.platform == p.platform
                    && r.architecture == p.architecture
                    && r.installation == p.installation,
                "Route target mismatch"
            );
            ensure!(
                self.position(p).key()? > r.min_source.key()?,
                "Non-advancing route"
            );
        }
        for id in &self.withdrawn {
            ensure!(
                self.packages.iter().any(|p| &p.id == id),
                "Missing withdrawn package"
            );
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateOffer {
    pub immediate_package_id: String,
    pub final_package_id: String,
    pub route_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CheckOutcome {
    Current,
    DirectUpdate(UpdateOffer),
    BridgeUpdate(UpdateOffer),
    ExternalUpdate(String),
    ManualRequired(String),
    UnsupportedPlatform(String),
    CheckFailed(String),
}

fn safe_relative(name: &str) -> Result<()> {
    if name.is_empty()
        || name.contains('\\')
        || name.contains(':')
        || name.starts_with('/')
        || name.split('/').any(|part| {
            part == ".." || part.trim_end_matches([' ', '.']) != part && !part.is_empty()
        })
    {
        anyhow::bail!("Unsafe archive path: {name}");
    }
    for c in std::path::Path::new(name).components() {
        if !matches!(c, std::path::Component::Normal(_)) {
            anyhow::bail!("Unsafe archive path: {name}");
        }
    }
    for part in name.split('/').filter(|s| !s.is_empty()) {
        let base = part.split('.').next().unwrap().to_uppercase();
        if ["CON", "PRN", "AUX", "NUL"].contains(&base.as_str())
            || base.starts_with("COM")
                && base[3..].parse::<u8>().is_ok_and(|n| (1..=9).contains(&n))
            || base.starts_with("LPT")
                && base[3..].parse::<u8>().is_ok_and(|n| (1..=9).contains(&n))
        {
            anyhow::bail!("Reserved Windows archive name");
        }
    }
    Ok(())
}

fn stable_version(value: &str) -> Result<[u64; 4]> {
    let mut key = [0; 4];
    for (i, p) in value.split('.').enumerate() {
        key[i] = p.parse()?;
    }
    Ok(key)
}
