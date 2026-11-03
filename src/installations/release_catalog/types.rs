use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub schema: u32,
    pub enabled: bool,
    pub index_url: Option<String>,
    #[serde(deserialize_with = "super::verification::unique_map")]
    pub keys: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub key_id: String,
    pub payload: String,
    pub signature: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Index {
    pub schema: u32,
    pub purpose: String,
    pub sequence: u64,
    pub generated_at: i64,
    #[serde(deserialize_with = "super::verification::unique_map")]
    pub repositories: BTreeMap<String, AppReference>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppReference {
    pub sha256: String,
    pub size: u64,
    pub checked_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery_added_at: Option<i64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppCatalog {
    pub schema: u32,
    pub repository: String,
    pub latest_tag: Option<String>,
    pub releases: Vec<crate::model::Release>,
    pub scopes: Vec<ScopeEvidence>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopePolicy {
    UserOnly,
    MachineOnly,
    Dual,
    Unknown,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeEvidence {
    pub tag: String,
    pub asset_name: String,
    pub digest: String,
    pub size: u64,
    pub os: String,
    pub architecture: String,
    pub policy: ScopePolicy,
    pub inspector_version: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompatibilityDecision {
    NotApplicable,
    Compatible,
    Incompatible(String),
    Unknown(String),
}
pub struct VerifiedIndex {
    pub envelope: Envelope,
    pub index: Index,
}
