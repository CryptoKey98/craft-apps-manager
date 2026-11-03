//! A long-lived browsing preview cannot authorize changed release packages.
use super::*;

pub(super) fn revalidate_plan(paths: &Paths, plan: &ReleasePlan, job: &Job) -> Result<()> {
    if plan.native_rollback.is_some() {
        job.check()?;
        return crate::installations::rollbacks::native::validate_plan(paths, plan);
    }
    let network = Network::new(&paths.root)?;
    for entry in plan.entries.iter().filter(|entry| {
        entry.error.is_none() && matches!(entry.action.as_str(), "Install" | "Update")
    }) {
        job.check()?;
        job.stage("Checking release", None, crate::model::title(&entry.app));
        let mut url = reqwest::Url::parse(&format!(
            "https://api.github.com/repos/{}/releases/",
            crate::model::repository(&entry.app)
        ))?;
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| anyhow::anyhow!("Invalid release endpoint"))?;
            path.pop_if_empty();
            if let Some(tag) = &plan.requested_tag {
                path.push("tags").push(tag);
            } else {
                path.push("latest");
            }
        }
        // Share a check completed within the last minute. Older browsing data
        // must pass conditional HTTP validation before downloading/elevation.
        let release: Release =
            network.release_json_with_policy(url.as_str(), crate::network::CachePolicy::Update)?;
        validate_reviewed(entry, plan.requested_tag.as_deref(), &release)?;
    }
    Ok(())
}

fn validate_reviewed(entry: &ReleaseEntry, tag: Option<&str>, release: &Release) -> Result<()> {
    anyhow::ensure!(
        !release.draft
            && !release.prerelease
            && release_version(&release.tag_name)? == entry.version
            && tag.is_none_or(|tag| release.tag_name == tag),
        "Release information changed. Review the installation again."
    );
    let asset = entry
        .asset
        .as_ref()
        .context("Reviewed download is missing")?;
    anyhow::ensure!(
        release
            .assets
            .iter()
            .any(|current| current.name == asset.name
                && current.size == asset.size
                && current.browser_download_url == asset.browser_download_url
                && current.digest == asset.digest),
        "The selected download changed or was removed. Review the installation again."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changed_or_withdrawn_packages_require_another_review() {
        let asset = Asset {
            name: "app.zip".into(),
            size: 10,
            browser_download_url: "https://github.com/owner/app/releases/download/v1.0.0/app.zip"
                .into(),
            digest: Some(format!("sha256:{}", "a".repeat(64))),
        };
        let entry = ReleaseEntry {
            app: "filmcraft".into(),
            action: "Install".into(),
            version: "1.0.0".into(),
            asset: Some(asset.clone()),
            destination: None,
            error: None,
            preferences: Preferences::default(),
        };
        let original = Release {
            tag_name: "v1.0.0".into(),
            name: None,
            draft: false,
            prerelease: false,
            assets: vec![asset],
        };
        assert!(validate_reviewed(&entry, None, &original).is_ok());
        assert!(validate_reviewed(&entry, Some("v1.0.0"), &original).is_ok());
        for field in [
            "size",
            "digest",
            "url",
            "removed",
            "draft",
            "prerelease",
            "tag",
        ] {
            let mut changed = original.clone();
            match field {
                "size" => changed.assets[0].size += 1,
                "digest" => changed.assets[0].digest = Some(format!("sha256:{}", "b".repeat(64))),
                "url" => changed.assets[0]
                    .browser_download_url
                    .push_str("?replacement"),
                "removed" => changed.assets.clear(),
                "draft" => changed.draft = true,
                "prerelease" => changed.prerelease = true,
                "tag" => changed.tag_name = "v2.0.0".into(),
                _ => unreachable!(),
            }
            assert!(
                validate_reviewed(&entry, None, &changed).is_err(),
                "{field}"
            );
        }
    }
}
