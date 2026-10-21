use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Installed {
    pub name: String,
    pub version: String,
    pub path: String,
    #[serde(default)]
    pub architecture: String,
    #[serde(default)]
    pub install_kind: String,
    #[serde(default)]
    pub product_code: String,
    #[serde(default)]
    pub installation_label: String,
    #[serde(default)]
    pub package_type: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub apps_root: String,
    pub apps: Vec<Installed>,
    #[serde(default)]
    pub installations: Vec<Installed>,
    #[serde(default)]
    pub portable_label_counters: std::collections::BTreeMap<String, u32>,
}
