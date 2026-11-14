use super::selection::check_installed_release;
use crate::{
    installations::release_catalog::{provider, store},
    model::{Installed, Paths, Preferences, Release},
    network::Network,
};
use anyhow::Result;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{mpsc, Arc, OnceLock},
};

pub fn check_installation_batch(
    paths: &Paths,
    apps: &[Installed],
    prefs: &Preferences,
    receive: impl FnMut(Installed, Result<Option<String>>),
) {
    let custom = crate::storage::config::ConfigStore::new(paths.root.clone())
        .load()
        .and_then(|settings| settings.custom_apps());
    let Ok(custom) = custom else {
        let mut receive = receive;
        for app in apps {
            receive(
                app.clone(),
                Err(anyhow::anyhow!("Could not read app registrations")),
            );
        }
        return;
    };
    let custom = custom.into_iter().map(|entry| entry.key).collect();
    let network = Network::new(&paths.root).map_err(|error| format!("{error:#}"));
    check_batch(
        apps,
        prefs,
        &custom,
        |repos| provider::refresh_latest_batch(&paths.root, repos),
        |repo, use_catalog_evidence| {
            let network = network
                .as_ref()
                .map_err(|error| anyhow::anyhow!("{error}"))?;
            network.update_release_from_github(
                &format!("https://api.github.com/repos/{repo}/releases/latest"),
                use_catalog_evidence,
            )
        },
        receive,
    );
}

fn check_batch(
    apps: &[Installed],
    prefs: &Preferences,
    custom: &BTreeSet<String>,
    refresh: impl FnOnce(&[&str]) -> Option<Arc<store::Snapshot>> + Send,
    github: impl Fn(&str, bool) -> Result<Release> + Sync,
    mut receive: impl FnMut(Installed, Result<Option<String>>),
) {
    let mut catalog = Vec::new();
    let mut direct = Vec::new();
    let mut lookups = BTreeMap::new();
    for app in apps {
        let repo = crate::model::repository(&app.name).to_ascii_lowercase();
        lookups.entry(repo.clone()).or_insert_with(OnceLock::new);
        if !custom.contains(&app.name) && provider::batch_repository(&repo).is_some() {
            catalog.push((app, repo));
        } else {
            direct.push((app, repo));
        }
    }
    let (tx, rx) = mpsc::channel();
    let compare = |app: &Installed, release: &Release| {
        check_installed_release(release, &app.name, app, prefs)
    };
    let catalog_repos: BTreeSet<_> = catalog.iter().map(|(_, repo)| repo.clone()).collect();
    let fallback = |app: &Installed, repo: &str| {
        let release = lookups[repo].get_or_init(|| {
            github(repo, catalog_repos.contains(repo)).map_err(|error| format!("{error:#}"))
        });
        match release {
            Ok(release) => compare(app, release),
            Err(error) => Err(anyhow::anyhow!("{error}")),
        }
    };

    std::thread::scope(|scope| {
        let direct_tx = tx.clone();
        let fallback = &fallback;
        scope.spawn(move || {
            for (app, repo) in direct {
                let _ = direct_tx.send((app.clone(), fallback(app, &repo)));
            }
        });
        scope.spawn(move || {
            let repositories: BTreeSet<_> = catalog.iter().map(|(_, repo)| repo.as_str()).collect();
            let repositories: Vec<_> = repositories.into_iter().collect();
            let snapshot = (!repositories.is_empty())
                .then(|| refresh(&repositories))
                .flatten();
            let mut missing = Vec::new();
            for (app, repo) in &catalog {
                if let Some(release) = snapshot
                    .as_ref()
                    .and_then(|snapshot| provider::latest_from_snapshot(snapshot, repo))
                {
                    let _ = tx.send(((*app).clone(), compare(app, &release)));
                } else {
                    missing.push((*app, repo.as_str()));
                }
            }
            // A slow fallback only delays its own result and the next item on its lane.
            std::thread::scope(|scope| {
                for lane in 0..missing.len().min(3) {
                    let tx = &tx;
                    let missing = &missing;
                    scope.spawn(move || {
                        for &(app, repo) in missing.iter().skip(lane).step_by(3) {
                            let _ = tx.send((app.clone(), fallback(app, repo)));
                        }
                    });
                }
            });
        });
        for (app, result) in rx {
            receive(app, result);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn app(name: &str) -> Installed {
        Installed {
            name: name.into(),
            version: "1.0.0".into(),
            ..Default::default()
        }
    }
    fn release() -> Release {
        Release {
            tag_name: "v1.0.0".into(),
            name: None,
            draft: false,
            prerelease: false,
            assets: Vec::new(),
        }
    }

    #[test]
    fn catalog_results_and_custom_github_results_arrive_without_waiting_for_other_fallbacks() {
        let apps = [app("filmcraft"), app("designcraft"), app("photocraft")];
        let custom = BTreeSet::from(["photocraft".into()]);
        let calls = Mutex::new(Vec::new());
        let (catalog_can_finish, custom_delivered) = mpsc::channel();
        let (fallback_can_finish, catalog_delivered) = mpsc::channel();
        let catalog_delivered = Mutex::new(catalog_delivered);
        let mut delivered = Vec::new();
        check_batch(
            &apps,
            &Preferences::default(),
            &custom,
            move |repos| {
                assert_eq!(repos, ["storytold/designcraft", "storytold/filmcraft"]);
                custom_delivered
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
                let repo = "storytold/filmcraft".to_owned();
                Some(Arc::new(store::Snapshot {
                    checked: BTreeMap::from([(repo.clone(), chrono::Utc::now().timestamp())]),
                    apps: BTreeMap::from([(
                        repo.clone(),
                        Arc::new(crate::installations::release_catalog::AppCatalog {
                            schema: 1,
                            repository: repo,
                            latest_tag: Some("v1.0.0".into()),
                            releases: vec![release()],
                            scopes: Vec::new(),
                        }),
                    )]),
                    ..Default::default()
                }))
            },
            |repo, observes_catalog| {
                calls
                    .lock()
                    .unwrap()
                    .push((repo.to_owned(), observes_catalog));
                if repo == "storytold/designcraft" {
                    catalog_delivered
                        .lock()
                        .unwrap()
                        .recv_timeout(std::time::Duration::from_secs(5))
                        .unwrap();
                    anyhow::bail!("GitHub unavailable");
                }
                Ok(release())
            },
            |app, result| {
                match app.name.as_str() {
                    "photocraft" => {
                        catalog_can_finish.send(()).unwrap();
                    }
                    "filmcraft" => {
                        fallback_can_finish.send(()).unwrap();
                    }
                    _ => {}
                }
                delivered.push((app.name, result.is_ok()));
            },
        );
        assert_eq!(
            delivered,
            [
                ("photocraft".into(), true),
                ("filmcraft".into(), true),
                ("designcraft".into(), false)
            ]
        );
        assert_eq!(
            calls.into_inner().unwrap(),
            [
                ("storytold/photocraft".into(), false),
                ("storytold/designcraft".into(), true)
            ]
        );
    }

    #[test]
    fn duplicate_repository_fallback_is_shared_but_each_copy_gets_a_result() {
        let apps = [app("filmcraft"), app("filmcraft")];
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let mut results = 0;
        check_batch(
            &apps,
            &Preferences::default(),
            &BTreeSet::new(),
            |repos| {
                assert_eq!(repos, ["storytold/filmcraft"]);
                None
            },
            |_, _| {
                calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(release())
            },
            |_, result| {
                assert!(result.unwrap().is_none());
                results += 1;
            },
        );
        assert_eq!(calls.into_inner(), 1);
        assert_eq!(results, 2);
    }
}
