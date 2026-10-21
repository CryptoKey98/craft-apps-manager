use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    pub source: String,
    pub repository: String,
    pub channel: String,
    pub public_key_file: String,
    pub service: Service,
    pub revisions: Revisions,
    pub network: Network,
    pub packages: Packages,
    pub publishing: Publishing,
    pub worker: Worker,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: 1,
            source: "github".into(),
            repository: "CryptoKey98/craft-apps-manager".into(),
            channel: "stable".into(),
            public_key_file: "assets/update-public-key.hex".into(),
            service: Service::default(),
            revisions: Revisions::default(),
            network: Network::default(),
            packages: Packages::default(),
            publishing: Publishing::default(),
            worker: Worker::default(),
        }
    }
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Service {
    pub catalog_url: String,
    pub mirror_url: String,
    pub check_url: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Revisions {
    pub windows: u32,
    pub linux: u32,
    pub macos: u32,
}
impl Default for Revisions {
    fn default() -> Self {
        Self {
            windows: 1,
            linux: 1,
            macos: 1,
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Network {
    pub connect_timeout_seconds: u64,
    pub metadata_timeout_seconds: u64,
    pub download_timeout_seconds: u64,
}
impl Default for Network {
    fn default() -> Self {
        Self {
            connect_timeout_seconds: 10,
            metadata_timeout_seconds: 30,
            download_timeout_seconds: 300,
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Packages {
    pub windows: Vec<String>,
    pub linux: Vec<String>,
    pub macos: Vec<String>,
}
impl Default for Packages {
    fn default() -> Self {
        Self {
            windows: vec!["zip".into(), "msi".into()],
            linux: ["zip", "tar.gz", "deb", "rpm", "arch"]
                .map(String::from)
                .to_vec(),
            macos: vec!["zip".into(), "dmg".into()],
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Publishing {
    pub enabled: bool,
    pub asset_name_pattern: String,
    pub retain_legacy_downloads: bool,
    pub profiles_file: String,
    pub next_catalog_file: String,
    pub mirror_tag: String,
    pub catalog_validity_days: u32,
    pub renew_before_days: u32,
}
impl Default for Publishing {
    fn default() -> Self {
        Self {
            enabled: false,
            asset_name_pattern: "{stem}-update{revision_suffix}{suffix}".into(),
            retain_legacy_downloads: true,
            profiles_file: String::new(),
            next_catalog_file: String::new(),
            mirror_tag: String::new(),
            catalog_validity_days: 30,
            renew_before_days: 7,
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Worker {
    pub name: String,
    pub dynamic_enabled: bool,
    pub rate_limit_requests: u32,
    pub rate_limit_period_seconds: u32,
    pub rate_limit_namespace: String,
}
impl Default for Worker {
    fn default() -> Self {
        Self {
            name: "craft-apps-manager-updates".into(),
            dynamic_enabled: false,
            rate_limit_requests: 120,
            rate_limit_period_seconds: 60,
            rate_limit_namespace: "168300900".into(),
        }
    }
}
