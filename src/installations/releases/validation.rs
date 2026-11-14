//! A long-lived browsing preview cannot authorize changed release packages.
use super::*;

pub(super) fn single_app_state_matches(plan: &ReleasePlan, current: &[u8]) -> Result<bool> {
    let [entry] = plan.entries.as_slice() else {
        return Ok(false);
    };
    if !plan.single_app || plan.native_rollback.is_some() {
        return Ok(false);
    }

    type Snapshot = (Preferences, crate::model::Config, Vec<serde_json::Value>);
    let (before_prefs, before, before_destinations): Snapshot =
        serde_json::from_slice(&plan.snapshot)?;
    let (after_prefs, after, after_destinations): Snapshot = serde_json::from_slice(current)?;
    let app = &entry.app;
    let identity = crate::install_options::identity(app);
    if before.apps_root != after.apps_root
        || before_prefs.app_install_options.get(&identity)
            != after_prefs.app_install_options.get(&identity)
        || before_prefs.batch_admin != after_prefs.batch_admin
        || before_prefs.rollback_enabled() != after_prefs.rollback_enabled()
        || before_prefs.compress_rollbacks != after_prefs.compress_rollbacks
        || before.portable_label_counters.get(app) != after.portable_label_counters.get(app)
    {
        return Ok(false);
    }

    let Some(before_index) = before.apps.iter().position(|copy| &copy.name == app) else {
        return Ok(false);
    };
    let Some(after_index) = after.apps.iter().position(|copy| &copy.name == app) else {
        return Ok(false);
    };
    let selected = &before.apps[before_index];
    let before_destination = before_destinations.get(before_index);
    let after_destination = after_destinations.get(after_index);
    if selected != &after.apps[after_index]
        || !same_destination(selected, before_destination, after_destination)
    {
        return Ok(false);
    }

    let targets: Vec<_> = entry
        .destination
        .iter()
        .chain(plan.installer_destination.iter())
        .map(|path| crate::installation_labels::path_identity(&path.to_string_lossy()))
        .chain(
            (!selected.path.is_empty())
                .then(|| crate::installation_labels::path_identity(&selected.path)),
        )
        .collect();
    let relevant = |copy: &&crate::model::Installed| {
        &copy.name == app
            || (!copy.path.is_empty() && {
                let path = crate::installation_labels::path_identity(&copy.path);
                targets.iter().any(|target| {
                    Path::new(&path).starts_with(target) || Path::new(target).starts_with(&path)
                })
            })
    };
    // Inventory saves move the selected copy to the end of the records even
    // when its installation did not change. The selected copy is checked above.
    let mut before_copies: Vec<_> = before.installations.iter().filter(relevant).collect();
    let mut after_copies: Vec<_> = after.installations.iter().filter(relevant).collect();
    for copies in [&mut before_copies, &mut after_copies] {
        copies.sort_by(|a, b| {
            (&a.name, &a.path, &a.install_kind).cmp(&(&b.name, &b.path, &b.install_kind))
        });
    }
    // Rollback per-copy overrides are read from current settings under the
    // operation lock by the recovery code.
    Ok(before_copies == after_copies)
}

fn same_destination(
    selected: &crate::model::Installed,
    before: Option<&serde_json::Value>,
    after: Option<&serde_json::Value>,
) -> bool {
    let (Some(before), Some(after)) = (before, after) else {
        return false;
    };
    if before == after {
        return true;
    }
    // Portable apps can write profiles and caches beside their executable.
    // Ignore the folder timestamp/size only for an existing portable copy.
    selected.install_kind == "portable"
        && !selected.path.is_empty()
        && before.get(0) == after.get(0)
        && before.get(1).is_some_and(|exe| !exe.is_null())
        && before.get(1) == after.get(1)
        && [before, after].into_iter().all(|state| {
            state
                .get(2)
                .and_then(|folder| folder.get(2))
                .and_then(serde_json::Value::as_bool)
                == Some(false)
        })
        && before.get(3).is_some()
        && before.get(3) == after.get(3)
}

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
        let release: Release = network.release_json_for(
            url.as_str(),
            crate::network::CachePolicy::Update,
            crate::installations::release_catalog::resolution::ReleaseIntent::InstallValidation,
        )?;
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
