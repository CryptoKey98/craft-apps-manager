use crate::schema::*;
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
};

#[derive(Default)]
pub struct ConfigOverrides {
    pub repository: Option<String>,
    pub channel: Option<String>,
    pub revision: Option<u32>,
    pub catalog_url: Option<String>,
    pub mirror_url: Option<String>,
    pub check_url: Option<String>,
}
#[derive(Serialize)]
pub struct ResolvedConfig {
    #[serde(flatten)]
    pub config: Config,
    pub target_os: String,
    pub revision: u32,
    pub public_key_hex: String,
    pub public_key_fingerprint: String,
    pub project_root: PathBuf,
    pub config_path: PathBuf,
    pub public_key_path: PathBuf,
    pub profiles_path: Option<PathBuf>,
    pub next_catalog_path: Option<PathBuf>,
    pub provenance: BTreeMap<String, String>,
    pub required_features: Vec<String>,
}
fn link(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_type().is_symlink() || meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}
fn checked_path(root: &Path, base: &Path, value: &Path, required: bool) -> Result<PathBuf> {
    ensure!(
        !value
            .components()
            .any(|p| matches!(p, Component::ParentDir)),
        "Parent paths are not allowed in updater configuration"
    );
    let path = if value.is_absolute() {
        value.to_path_buf()
    } else {
        base.join(value)
    };
    ensure!(path.starts_with(root), "Updater path escapes project root");
    let mut cursor = PathBuf::new();
    for part in path.components() {
        cursor.push(part);
        // A Windows drive/UNC prefix alone is not a queryable filesystem path.
        if matches!(part, Component::Prefix(_)) {
            continue;
        }
        match fs::symlink_metadata(&cursor) {
            Ok(meta) => ensure!(!link(&meta), "Linked updater configuration path"),
            Err(e) if !required && e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).context("Missing updater configuration file"),
        }
    }
    if required {
        ensure!(path.is_file(), "Updater configuration path must be a file");
    }
    Ok(path)
}
pub fn endpoint(value: &str) -> Result<()> {
    if value.is_empty() {
        return Ok(());
    }
    ensure!(
        value.len() <= 2048 && !value.chars().any(char::is_control),
        "Invalid service endpoint"
    );
    let url = url::Url::parse(value).context("Invalid service endpoint")?;
    ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "Service endpoints require HTTPS without credentials/query/fragment"
    );
    Ok(())
}
fn origins(
    value: &serde_json::Value,
    prefix: &str,
    label: &str,
    out: &mut BTreeMap<String, String>,
) {
    if let Some(map) = value.as_object() {
        for (k, v) in map {
            origins(
                v,
                &if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                },
                label,
                out,
            );
        }
    } else {
        out.insert(prefix.into(), label.into());
    }
}
fn validate(c: &Config) -> Result<()> {
    ensure!(c.schema_version == 1, "Unsupported updater schema_version");
    ensure!(
        matches!(c.source.as_str(), "github" | "service"),
        "source must be github or service"
    );
    let parts: Vec<_> = c.repository.split('/').collect();
    ensure!(
        parts.len() == 2
            && parts.iter().all(|p| !p.is_empty()
                && *p != "."
                && *p != ".."
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_ .".contains(&b) && b != b' ')),
        "repository must be owner/name"
    );
    ensure!(
        !c.channel.is_empty()
            && c.channel.len() <= 64
            && c.channel
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)),
        "Invalid channel"
    );
    ensure!(
        c.source != "github" || c.channel == "stable",
        "GitHub updater supports only stable channel"
    );
    for url in [
        &c.service.catalog_url,
        &c.service.mirror_url,
        &c.service.check_url,
    ] {
        endpoint(url)?;
    }
    ensure!(
        c.source != "service" || !c.service.catalog_url.is_empty(),
        "Service updater requires service.catalog_url"
    );
    ensure!(
        [c.revisions.windows, c.revisions.linux, c.revisions.macos]
            .iter()
            .all(|r| (1..=65535).contains(r)),
        "Revision must be 1..65535"
    );
    ensure!(
        (1..=60).contains(&c.network.connect_timeout_seconds)
            && (5..=120).contains(&c.network.metadata_timeout_seconds)
            && (30..=1800).contains(&c.network.download_timeout_seconds),
        "Network timeout out of bounds"
    );
    for (values, allowed) in [
        (&c.packages.windows, &["zip", "msi"][..]),
        (
            &c.packages.linux,
            &["zip", "tar.gz", "deb", "rpm", "arch"][..],
        ),
        (&c.packages.macos, &["zip", "dmg"][..]),
    ] {
        let unique: BTreeSet<_> = values.iter().collect();
        ensure!(
            !values.is_empty()
                && unique.len() == values.len()
                && values.iter().all(|s| allowed.contains(&s.as_str())),
            "Unsupported or duplicate package format"
        );
    }
    ensure!(
        !c.packages.linux.iter().any(|v| v == "tar.gz")
            || c.packages.linux.iter().any(|v| v == "zip"),
        "Linux tar.gz packaging currently requires the ZIP payload output"
    );
    let p = &c.publishing;
    ensure!(
        (2..=30).contains(&p.catalog_validity_days)
            && (1..p.catalog_validity_days).contains(&p.renew_before_days),
        "Invalid catalog validity/renewal window"
    );
    let pattern = &p.asset_name_pattern;
    ensure!(
        pattern.len() <= 200
            && pattern.contains("{stem}")
            && pattern.ends_with("{suffix}")
            && (pattern.contains("{revision}") || pattern.contains("{revision_suffix}")),
        "Unsafe immutable asset template"
    );
    let remaining = pattern
        .replace("{stem}", "manager")
        .replace("{suffix}", ".zip")
        .replace("{revision}", "1")
        .replace("{revision_suffix}", "");
    ensure!(
        remaining != "manager.zip"
            && remaining
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)),
        "Unsafe immutable asset template"
    );
    let w = &c.worker;
    ensure!(
        !w.name.is_empty()
            && w.name.len() <= 63
            && w.name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
        "Invalid Worker name"
    );
    ensure!(
        (1..=1000).contains(&w.rate_limit_requests)
            && [10, 60].contains(&w.rate_limit_period_seconds)
            && !w.rate_limit_namespace.is_empty()
            && w.rate_limit_namespace.len() <= 32
            && w.rate_limit_namespace.bytes().all(|b| b.is_ascii_digit()),
        "Invalid Worker rate limit"
    );
    if p.enabled {
        ensure!(
            c.source == "service"
                && !p.profiles_file.is_empty()
                && !p.next_catalog_file.is_empty()
                && !p.mirror_tag.is_empty()
                && !c.service.mirror_url.is_empty(),
            "Publishing requires service, profile/catalog inputs and mirror"
        );
    }
    ensure!(
        p.mirror_tag.is_empty()
            || p.mirror_tag
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)),
        "Invalid mirror tag"
    );
    Ok(())
}
pub fn resolve(
    project: &Path,
    config: Option<&Path>,
    env: &BTreeMap<String, String>,
    target_os: &str,
    overrides: &ConfigOverrides,
) -> Result<ResolvedConfig> {
    let root = std::path::absolute(project).context("Invalid updater project root")?;
    ensure!(root.is_dir(), "Invalid updater project root");
    let explicit = config
        .map(PathBuf::from)
        .or_else(|| env.get("CRAFT_UPDATER_CONFIG").map(PathBuf::from));
    let config_path = checked_path(
        &root,
        &root,
        explicit.as_deref().unwrap_or(Path::new("updater.toml")),
        explicit.is_some(),
    )?;
    let mut provenance = BTreeMap::new();
    origins(
        &serde_json::to_value(Config::default())?,
        "",
        "default",
        &mut provenance,
    );
    let mut c = if config_path.exists() {
        ensure!(
            fs::metadata(&config_path)?.len() <= 128 * 1024,
            "Updater TOML too large"
        );
        let text = fs::read_to_string(&config_path)?;
        // TOML's default diagnostic includes the source line. Never echo a
        // pasted private key/token even when its field is rejected.
        let safe_error = |error: toml_edit::de::Error| {
            anyhow::anyhow!(
                "Invalid updater TOML field, type or syntax at byte {}",
                error.span().map_or(0, |span| span.start)
            )
        };
        let value: serde_json::Value = toml_edit::de::from_str(&text).map_err(safe_error)?;
        origins(&value, "", "file", &mut provenance);
        toml_edit::de::from_str::<Config>(&text).map_err(safe_error)?
    } else {
        Config::default()
    };
    let mut set = |key: &str, target: &mut String, name: &str| {
        if let Some(v) = env.get(name) {
            *target = v.clone();
            provenance.insert(key.into(), format!("env:{name}"));
        }
    };
    set("source", &mut c.source, "CRAFT_UPDATE_SOURCE");
    set("repository", &mut c.repository, "CRAFT_MANAGER_REPOSITORY");
    set(
        "service.catalog_url",
        &mut c.service.catalog_url,
        "CRAFT_UPDATE_CATALOG_URL",
    );
    set(
        "service.mirror_url",
        &mut c.service.mirror_url,
        "CRAFT_UPDATE_CATALOG_MIRROR",
    );
    set(
        "service.check_url",
        &mut c.service.check_url,
        "CRAFT_UPDATE_CHECK_URL",
    );
    for (key, dest, value) in [
        ("repository", &mut c.repository, &overrides.repository),
        ("channel", &mut c.channel, &overrides.channel),
        (
            "service.catalog_url",
            &mut c.service.catalog_url,
            &overrides.catalog_url,
        ),
        (
            "service.mirror_url",
            &mut c.service.mirror_url,
            &overrides.mirror_url,
        ),
        (
            "service.check_url",
            &mut c.service.check_url,
            &overrides.check_url,
        ),
    ] {
        if let Some(v) = value {
            *dest = v.clone();
            provenance.insert(key.into(), "cli".into());
        }
    }
    let revision = match target_os {
        "windows" => &mut c.revisions.windows,
        "linux" => &mut c.revisions.linux,
        "macos" => &mut c.revisions.macos,
        _ => anyhow::bail!("Unsupported updater target OS"),
    };
    let revision_key = format!("revisions.{target_os}");
    if let Some(v) = env.get("CRAFT_BUILD_REVISION") {
        *revision = v.parse().context("Invalid CRAFT_BUILD_REVISION")?;
        provenance.insert(revision_key.clone(), "env:CRAFT_BUILD_REVISION".into());
    }
    if let Some(v) = overrides.revision {
        *revision = v;
        provenance.insert(revision_key, "cli".into());
    }
    let revision = *revision;
    validate(&c)?;
    let base = config_path.parent().unwrap();
    ensure!(!c.public_key_file.is_empty(), "Missing public_key_file");
    let public_key_path = checked_path(&root, base, Path::new(&c.public_key_file), true)?;
    ensure!(
        fs::metadata(&public_key_path)?.len() <= 1024,
        "Public key file too large"
    );
    let public_key_hex = fs::read_to_string(&public_key_path)?.trim().to_lowercase();
    ensure!(
        public_key_hex.len() == 64 && public_key_hex.bytes().all(|b| b.is_ascii_hexdigit()),
        "public_key_file must contain a 32-byte Ed25519 hex key"
    );
    let key: Vec<u8> = (0..32)
        .map(|i| u8::from_str_radix(&public_key_hex[2 * i..2 * i + 2], 16).unwrap())
        .collect();
    let public_key_fingerprint = format!("{:x}", Sha256::digest(key));
    let input = |value: &str| -> Result<Option<PathBuf>> {
        if value.is_empty() {
            Ok(None)
        } else {
            Ok(Some(checked_path(&root, base, Path::new(value), true)?))
        }
    };
    let profiles_path = input(&c.publishing.profiles_file)?;
    let next_catalog_path = input(&c.publishing.next_catalog_file)?;
    let required_features = if c.source == "service" {
        vec!["service-updater".into()]
    } else {
        vec![]
    };
    Ok(ResolvedConfig {
        config: c,
        target_os: target_os.into(),
        revision,
        public_key_hex,
        public_key_fingerprint,
        project_root: root.clone(),
        config_path,
        public_key_path,
        profiles_path,
        next_catalog_path,
        provenance,
        required_features,
    })
}

impl ResolvedConfig {
    /// Only values embedded into the executable; no local paths or publisher policy.
    pub fn runtime_fingerprint(&self) -> String {
        // An ordered array avoids serializer map-order feature differences
        // between Cargo build dependencies and standalone configuration tools.
        let value = serde_json::json!([
            "craft-updater-runtime-v1",
            self.config.source,
            self.config.repository,
            self.config.channel,
            self.revision,
            self.public_key_hex,
            self.target_os,
            self.config.service.catalog_url,
            self.config.service.mirror_url,
            self.config.service.check_url,
            self.config.network.connect_timeout_seconds,
            self.config.network.metadata_timeout_seconds,
            self.config.network.download_timeout_seconds
        ]);
        format!("{:x}", Sha256::digest(value.to_string().as_bytes()))
    }
    pub fn build_identity(&self, version: &str, target: &str) -> serde_json::Value {
        serde_json::json!({"version":version,"revision":self.revision,"target":target,
            "update_source":self.config.source,"update_repository":self.config.repository,
            "update_channel":self.config.channel,"update_key_fingerprint":self.public_key_fingerprint,
            "update_config_fingerprint":self.runtime_fingerprint()})
    }
    pub fn require_build_support(&self, service_compiled: bool) -> Result<()> {
        ensure!(
            self.config.source != "service" || service_compiled,
            "Service updater selected without its library; build with --features service-updater"
        );
        Ok(())
    }
}
