use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub sha: String,
    pub branch: String,
    pub repository: String,
    pub archive_sha256: String,
    pub downloaded_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildInfo {
    pub app: String,
    pub commit: String,
    pub source_branch: String,
    pub built_at: String,
    pub profile: String,
    pub log: String,
}
