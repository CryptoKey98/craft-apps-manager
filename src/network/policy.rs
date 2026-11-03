//! Freshness is a caller requirement, not part of the credential-scoped cache key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CachePolicy {
    Update,
    Versions,
    SelectedRelease,
    Repository,
    Discovery,
    Standard,
}

pub(crate) fn history_for_tag(url: &str) -> Option<(String, String)> {
    let mut url = reqwest::Url::parse(url).ok()?;
    if url.scheme() != "https"
        || url.host_str() != Some("api.github.com")
        || url.query().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return None;
    }
    let parts: Vec<_> = url.path_segments()?.map(str::to_owned).collect();
    let [repos, owner, repo, releases, tags, tag] = parts.as_slice() else {
        return None;
    };
    if repos != "repos" || releases != "releases" || tags != "tags" {
        return None;
    }
    let tag = tag.clone();
    url.set_path(&format!("/repos/{owner}/{repo}/releases"));
    url.set_query(Some("per_page=100"));
    Some((url.to_string(), tag))
}

pub(crate) fn related_release_urls(
    url: &str,
    old: Option<&serde_json::Value>,
    value: &serde_json::Value,
) -> Vec<String> {
    let Ok(mut base) = reqwest::Url::parse(url) else {
        return Vec::new();
    };
    let parts: Vec<_> = base.path().trim_matches('/').split('/').collect();
    let ["repos", owner, repo, "releases", "latest"] = parts.as_slice() else {
        return Vec::new();
    };
    let path = format!("/repos/{owner}/{repo}/releases");
    base.set_path(&path);
    base.set_query(Some("per_page=100"));
    let mut related = vec![base.to_string()];
    base.set_query(None);
    related.push(base.to_string());
    for tag in old
        .into_iter()
        .chain(std::iter::once(value))
        .filter_map(|release| release.get("tag_name").and_then(serde_json::Value::as_str))
    {
        let mut tagged = base.clone();
        if let Ok(mut segments) = tagged.path_segments_mut() {
            segments.push("tags").push(tag);
        }
        if !related.contains(&tagged.to_string()) {
            related.push(tagged.to_string());
        }
    }
    related
}
impl CachePolicy {
    pub const fn seconds(self) -> i64 {
        match self {
            Self::Update | Self::Discovery => 60,
            Self::Versions => 15 * 60,
            Self::SelectedRelease | Self::Repository => 24 * 3600,
            Self::Standard => 5 * 60,
        }
    }
    pub(crate) fn for_url(url: &str) -> Self {
        let Ok(url) = reqwest::Url::parse(url) else {
            return Self::Standard;
        };
        let parts: Vec<_> = url.path().trim_matches('/').split('/').collect();
        match parts.as_slice() {
            ["repos", _, _] => Self::Repository,
            ["repos", _, _, "releases"] | ["repos", _, _, "releases", "latest"] => Self::Versions,
            ["repos", _, _, "releases", "tags", ..] | ["repos", _, _, "releases", _] => {
                Self::SelectedRelease
            }
            ["orgs", _, "repos"] => Self::Discovery,
            _ => Self::Standard,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoint_defaults_do_not_cache_moving_source_refs_for_a_day() {
        for (path, policy) in [
            ("/repos/owner/app", CachePolicy::Repository),
            ("/repos/owner/app/releases/latest", CachePolicy::Versions),
            (
                "/repos/owner/app/releases?per_page=100",
                CachePolicy::Versions,
            ),
            (
                "/repos/owner/app/releases/tags/v1.0",
                CachePolicy::SelectedRelease,
            ),
            ("/repos/owner/app/commits/main", CachePolicy::Standard),
            (
                "/orgs/storytold/repos?per_page=100&page=1",
                CachePolicy::Discovery,
            ),
        ] {
            assert_eq!(
                CachePolicy::for_url(&format!("https://api.github.com{path}")),
                policy
            );
        }
        assert_eq!(CachePolicy::Update.seconds(), 60);
        assert_eq!(CachePolicy::SelectedRelease.seconds(), 86400);
    }
}
