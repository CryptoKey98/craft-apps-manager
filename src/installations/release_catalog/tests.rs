use super::*;
use ed25519_dalek::{Signer, SigningKey};
use std::collections::BTreeMap;
const DOMAIN: &[u8] = b"craft-apps-manager/app-release-catalog/v1\n";
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn signed(payload: &str) -> (Source, Vec<u8>) {
    let key = SigningKey::from_bytes(&[37; 32]); // isolated fixture, never a production key
    let source = Source {
        schema: 1,
        enabled: true,
        index_url: Some(
            "https://raw.githubusercontent.com/fixture/catalog/main/v1/index.json".into(),
        ),
        keys: BTreeMap::from([("fixture".into(), hex(key.verifying_key().as_bytes()))]),
    };
    let mut message = DOMAIN.to_vec();
    message.extend_from_slice(payload.as_bytes());
    let envelope = Envelope {
        key_id: "fixture".into(),
        payload: payload.into(),
        signature: hex(&key.sign(&message).to_bytes()),
    };
    (source, serde_json::to_vec(&envelope).unwrap())
}
fn root() -> String {
    format!(
        r#"{{"schema":1,"purpose":"app-release-catalog","sequence":7,"generated_at":{},"repositories":{{}}}}"#,
        chrono::Utc::now().timestamp()
    )
}
#[test]
fn accepts_only_signed_domain_bound_index() {
    let (source, bytes) = signed(&root());
    assert_eq!(verify_index(&bytes, &source, 7).unwrap().index.sequence, 7);
    assert!(verify_index(&bytes, &source, 8).is_err());
    let mut changed: Envelope = serde_json::from_slice(&bytes).unwrap();
    changed.payload.push(' ');
    assert!(verify_index(&serde_json::to_vec(&changed).unwrap(), &source, 0).is_err());
}
#[test]
fn rejects_duplicate_keys_even_when_signed() {
    let payload = root().replace("\"sequence\":7", "\"sequence\":7,\"sequence\":8");
    let (source, bytes) = signed(&payload);
    assert!(verify_index(&bytes, &source, 0).is_err());
}

#[test]
fn preview_unknown_scope_does_not_download_or_inspect_an_installer() {
    let root = std::env::temp_dir().join(format!("craft-catalog-preview-{}", uuid::Uuid::new_v4()));
    let paths = crate::model::Paths::new(root.clone(), None);
    let prefs = crate::model::Preferences {
        release_format: "installer".into(),
        architecture: "x64".into(),
        ..Default::default()
    };
    let installed = crate::model::Installed {
        name: "filmcraft".into(),
        version: "2.0.0".into(),
        path: root.join("app").display().to_string(),
        install_kind: "installer".into(),
        architecture: "x64".into(),
        windows_msi_scope: Some("user".into()),
        ..Default::default()
    };
    let config = crate::model::Config {
        apps_root: root.display().to_string(),
        apps: vec![installed],
        installations: Vec::new(),
        portable_label_counters: Default::default(),
    };
    let asset = crate::model::Asset {
        name: "fixture.msi".into(),
        size: 1,
        browser_download_url:
            "https://github.com/storytold/filmcraft/releases/download/v1.0.0/fixture.msi".into(),
        digest: Some(format!("sha256:{}", verification::hash(b"x"))),
    };
    let package = crate::updates::installer_package_path(&paths, "filmcraft", &asset).unwrap();
    crate::files::write_bytes(&package, b"x").unwrap(); // valid bytes/hash, deliberately NOT an MSI database
    let snapshot = serde_json::to_vec(&(prefs.clone(), config, serde_json::json!([]))).unwrap();
    let mut plan:crate::updates::ReleasePlan=serde_json::from_value(serde_json::json!({"native_rollback":null,"additional_installation":false,"entries":[{"app":"filmcraft","action":"Update","version":"1.0.0","asset":asset,"destination":null,"error":null,"preferences":prefs}],"preferences":prefs,"requested_tag":"v1.0.0","snapshot":snapshot,"desktop_shortcut":false,"installer_destination":null})).unwrap();
    let job = crate::jobs::Job::new(root.join("logs/test.log"), &Default::default());
    let result = crate::updates::check_installer_scope_preview(&paths, &mut plan, &job);
    assert!(
        result.is_ok(),
        "Preview must not inspect/download package: {result:?}"
    );
    assert!(plan.entries[0].error.is_none());
    assert_eq!(std::fs::read(&package).unwrap(), b"x");
    std::fs::remove_dir_all(root).unwrap();
}

fn app_fixture() -> AppCatalog {
    AppCatalog {
        schema: 1,
        repository: "storytold/filmcraft".into(),
        latest_tag: Some("v2.0.0".into()),
        releases: vec![crate::model::Release {
            tag_name: "v2.0.0".into(),
            name: None,
            draft: false,
            prerelease: false,
            assets: vec![],
        }],
        scopes: vec![],
    }
}
fn bundle(app: &AppCatalog, checked: i64, sequence: u64) -> (Source, Vec<u8>, Vec<u8>) {
    let body = serde_json::to_vec(app).unwrap();
    let index = Index {
        schema: 1,
        purpose: "app-release-catalog".into(),
        sequence,
        generated_at: chrono::Utc::now().timestamp(),
        repositories: BTreeMap::from([(
            app.repository.clone(),
            AppReference {
                sha256: verification::hash(&body),
                size: body.len() as u64,
                checked_at: checked,
                discovery_added_at: None,
            },
        )]),
    };
    let (source, bytes) = signed(&serde_json::to_string(&index).unwrap());
    (source, bytes, body)
}
struct Transport {
    index: Vec<u8>,
    app: Vec<u8>,
    count: std::cell::Cell<usize>,
}
impl client::Transport for Transport {
    fn get(&self, url: &str, etag: Option<&str>, _limit: usize) -> anyhow::Result<client::Reply> {
        self.count.set(self.count.get() + 1);
        if url.ends_with("index.json") {
            assert!(etag.is_none());
            Ok(client::Reply::Data(
                self.index.clone(),
                Some("fixture-etag".into()),
            ))
        } else {
            Ok(client::Reply::Data(self.app.clone(), None))
        }
    }
}
struct Root(std::path::PathBuf);
impl Root {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("craft-catalog-tests-{}", uuid::Uuid::new_v4())))
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn catalog_fetches_one_app_and_reuses_verified_cache_without_network() {
    let root = Root::new();
    let app = app_fixture();
    let (source, index, body) = bundle(&app, chrono::Utc::now().timestamp(), 7);
    let transport = Transport {
        index,
        app: body,
        count: std::cell::Cell::new(0),
    };
    let first = client::refresh(&root.0, &source, &app.repository, &transport).unwrap();
    assert_eq!(first.apps[&app.repository].releases[0].tag_name, "v2.0.0");
    assert_eq!(transport.count.get(), 2);
    let second = client::refresh(&root.0, &source, &app.repository, &transport).unwrap();
    assert_eq!(transport.count.get(), 2);
    assert_eq!(
        second.checked[&app.repository],
        first.checked[&app.repository]
    );
    store::clear(&root.0).unwrap();
    assert!(store::peek(&root.0).is_none());
    assert!(!root.0.join("runtime/app-release-catalog").exists());
}
#[test]
fn rejects_object_tampering_and_wrong_repository() {
    let app = app_fixture();
    let (source, index, body) = bundle(&app, chrono::Utc::now().timestamp(), 7);
    let root = verify_index(&index, &source, 0).unwrap();
    let reference = &root.index.repositories[&app.repository];
    assert!(verify_app(&body, "storytold/other", reference).is_err());
    let mut changed = body;
    changed.push(b' ');
    assert!(verify_app(&changed, &app.repository, reference).is_err());
}
#[test]
fn endpoints_do_not_capture_discovery_self_update_or_other_hosts() {
    assert!(provider::endpoint("https://api.github.com/orgs/storytold/repos").is_none());
    assert!(
        provider::endpoint("https://evil.invalid/repos/storytold/filmcraft/releases").is_none()
    );
    let (_, query) = provider::endpoint(
        "https://api.github.com/repos/storytold/filmcraft/releases/tags/a%2Fb+c%26d",
    )
    .unwrap();
    assert!(matches!(query,provider::Query::Tag(ref tag) if tag=="a/b+c&d"));
}

#[test]
fn custom_release_requests_skip_catalog_transport_and_storage() {
    struct NoCatalog;
    impl client::Transport for NoCatalog {
        fn get(&self, _: &str, _: Option<&str>, _: usize) -> anyhow::Result<client::Reply> {
            panic!("Custom app must go directly to the GitHub API");
        }
    }
    let (source, _) = signed(&root());
    let root = Root::new();
    for repo in [
        "fixture/custom-app",
        "storytold/unregistered-custom-fixture",
    ] {
        for suffix in [
            "releases?per_page=100",
            "releases/latest",
            "releases/tags/v1.0.0",
        ] {
            let value: String = provider::resolve_with(
                &root.0,
                &format!("https://api.github.com/repos/{repo}/{suffix}"),
                &source,
                &NoCatalog,
                || Ok("api-result".into()),
            )
            .unwrap();
            assert_eq!(value, "api-result");
        }
    }
    assert!(store::peek(&root.0).is_none());
    assert!(!root.0.join("runtime/app-release-catalog").exists());
}

#[test]
fn custom_release_lookups_ignore_previously_loaded_catalog_data() {
    let root = Root::new();
    let mut app = app_fixture();
    app.repository = "fixture/custom-app".into();
    let (source, index, body) = bundle(&app, chrono::Utc::now().timestamp(), 7);
    client::refresh(
        &root.0,
        &source,
        &app.repository,
        &Transport {
            index,
            app: body,
            count: std::cell::Cell::new(0),
        },
    )
    .unwrap();
    assert!(provider::peek_release(&root.0, &app.repository, None).is_none());
    assert!(provider::generation(&root.0, &app.repository).is_none());
}

#[test]
fn publisher_preserves_failed_repository_without_refreshing_its_check_time() {
    let app = app_fixture();
    let checked = chrono::Utc::now().timestamp() - 3600;
    let scan = publisher::Discovery {
        schema: 1,
        repositories: BTreeMap::from([(
            app.repository.clone(),
            publisher::Scan {
                checked_at: checked,
                latest_tag: app.latest_tag.clone(),
                releases: app.releases.clone(),
                error: None,
            },
        )]),
        pages: BTreeMap::new(),
    };
    let first = publisher::aggregate(None, &scan, &[]).unwrap();
    let mut failed = scan;
    failed.repositories.get_mut(&app.repository).unwrap().error = Some("Unavailable".into());
    failed
        .repositories
        .get_mut(&app.repository)
        .unwrap()
        .checked_at = chrono::Utc::now().timestamp();
    let second = publisher::aggregate(Some(&first), &failed, &[]).unwrap();
    assert_eq!(
        second.index.repositories[&app.repository].checked_at,
        checked
    );
    assert_eq!(second.index.sequence, first.index.sequence + 1);
}

#[test]
fn provider_prefers_catalog_and_falls_back_without_erasing_newer_api_observation() {
    let root = Root::new();
    let app = app_fixture();
    let (source, index, body) = bundle(&app, chrono::Utc::now().timestamp(), 7);
    let transport = Transport {
        index,
        app: body,
        count: std::cell::Cell::new(0),
    };
    let url = "https://api.github.com/repos/storytold/filmcraft/releases/latest";
    let value: crate::model::Release =
        provider::resolve_with(&root.0, url, &source, &transport, || {
            panic!("Unnecessary API request")
        })
        .unwrap();
    assert_eq!(value.tag_name, "v2.0.0");
    let missing = "https://api.github.com/repos/storytold/filmcraft/releases/tags/v3.0.0";
    let mut newer = app.releases[0].clone();
    newer.tag_name = "v3.0.0".into();
    let value: crate::model::Release =
        provider::resolve_with(&root.0, missing, &source, &transport, || {
            super::api_observation::observe_with(
                &root.0,
                &source,
                missing,
                &serde_json::to_value(&newer).unwrap(),
                chrono::Utc::now().timestamp(),
            );
            Ok(newer.clone())
        })
        .unwrap();
    assert_eq!(value.tag_name, "v3.0.0");
    let value: crate::model::Release =
        provider::resolve_with(&root.0, url, &source, &transport, || Ok(newer.clone())).unwrap();
    assert_eq!(
        value.tag_name, "v3.0.0",
        "An older catalog must not displace a newer API result"
    );
    assert_eq!(transport.count.get(), 2);
}
#[test]
fn conditional_response_does_not_refresh_upstream_freshness() {
    struct Unchanged;
    impl client::Transport for Unchanged {
        fn get(
            &self,
            _url: &str,
            _etag: Option<&str>,
            _limit: usize,
        ) -> anyhow::Result<client::Reply> {
            Ok(client::Reply::Unchanged)
        }
    }
    let root = Root::new();
    let app = app_fixture();
    let old = chrono::Utc::now().timestamp() - 3600;
    let (source, index, body) = bundle(&app, old, 7);
    let first = Transport {
        index,
        app: body,
        count: std::cell::Cell::new(0),
    };
    client::refresh(&root.0, &source, &app.repository, &first).unwrap();
    let mut state = store::load(&root.0, &source).unwrap();
    state.attempted -= 120;
    store::save(&root.0, &mut state, &app.repository).unwrap();
    let snapshot = client::refresh(&root.0, &source, &app.repository, &Unchanged).unwrap();
    assert_eq!(snapshot.checked[&app.repository], old);
    assert!(provider::value(&snapshot, &app.repository, &provider::Query::Latest).is_none());
    assert!(provider::value(&snapshot, &app.repository, &provider::Query::History).is_some());
}
#[test]
fn clear_preserves_unknown_files_and_discards_cached_metadata() {
    let root = Root::new();
    let app = app_fixture();
    let (source, index, body) = bundle(&app, chrono::Utc::now().timestamp(), 7);
    client::refresh(
        &root.0,
        &source,
        &app.repository,
        &Transport {
            index,
            app: body,
            count: std::cell::Cell::new(0),
        },
    )
    .unwrap();
    let unknown = root.0.join("runtime/app-release-catalog/user-note.txt");
    std::fs::write(&unknown, b"keep").unwrap();
    store::clear(&root.0).unwrap();
    assert_eq!(std::fs::read(unknown).unwrap(), b"keep");
    assert!(store::load(&root.0, &source).unwrap().envelope.is_none());
}

#[test]
fn first_catalog_request_after_empty_initialization_or_clear_is_not_rejected_as_stale() {
    let root = Root::new();
    let app = app_fixture();
    let (source, index, body) = bundle(&app, chrono::Utc::now().timestamp(), 7);
    let transport = Transport {
        index,
        app: body,
        count: std::cell::Cell::new(0),
    };
    for iteration in 0..2 {
        let state = store::load(&root.0, &source).unwrap();
        store::publish_memory(&root.0, store::snapshot(&state, &source).unwrap());
        assert!(!root
            .0
            .join("runtime/app-release-catalog/cache.json.gz")
            .exists());
        let result = client::refresh(&root.0, &source, &app.repository, &transport);
        assert!(
            result.is_ok(),
            "First request was incorrectly invalidated: {:?}",
            result.err()
        );
        assert!(store::peek(&root.0)
            .unwrap()
            .apps
            .contains_key(&app.repository));
        assert_eq!(transport.count.get(), (iteration + 1) * 2);
        store::clear(&root.0).unwrap();
    }
}
#[test]
fn publisher_signs_objects_before_root_and_client_verifies_every_object() {
    let (source, _) = signed(&root());
    let root = Root::new();
    let app = app_fixture();
    let scan = publisher::Discovery {
        schema: 1,
        repositories: BTreeMap::from([(
            app.repository.clone(),
            publisher::Scan {
                checked_at: chrono::Utc::now().timestamp(),
                latest_tag: app.latest_tag.clone(),
                releases: app.releases.clone(),
                error: None,
            },
        )]),
        pages: BTreeMap::new(),
    };
    let publication = publisher::aggregate(None, &scan, &[]).unwrap();
    let key = SigningKey::from_bytes(&[37; 32]);
    publisher::sign(&publication, &key, "fixture", &root.0).unwrap();
    let verified = verify_index(
        &std::fs::read(root.0.join("index.json")).unwrap(),
        &source,
        0,
    )
    .unwrap();
    for (repo, reference) in &verified.index.repositories {
        let body = std::fs::read(
            root.0
                .join("apps")
                .join(format!("{}.json", reference.sha256)),
        )
        .unwrap();
        verify_app(&body, repo, reference).unwrap();
    }
}

#[test]
fn queued_refresh_cannot_repopulate_a_cleared_cache() {
    let root = Root::new();
    let app = app_fixture();
    let (source, index, body) = bundle(&app, chrono::Utc::now().timestamp(), 7);
    let transport = Transport {
        index,
        app: body,
        count: std::cell::Cell::new(0),
    };
    let snapshot = client::refresh(&root.0, &source, &app.repository, &transport).unwrap();
    let epoch = snapshot.epoch.clone();
    store::clear(&root.0).unwrap();
    assert!(
        client::refresh_guarded(&root.0, &source, &app.repository, &transport, Some(&epoch))
            .is_err()
    );
    assert_eq!(transport.count.get(), 2);
    assert!(!root.0.join("runtime/app-release-catalog").exists());
    assert!(store::peek(&root.0).unwrap().apps.is_empty());
}

#[test]
fn scope_evidence_is_bound_to_digest_size_tag_repository_and_selected_scope() {
    if !cfg!(target_os = "windows") {
        return;
    }
    let root = Root::new();
    let digest = format!("sha256:{}", "a".repeat(64));
    let asset = crate::model::Asset {
        name: "fixture-windows-x64.msi".into(),
        size: 42,
        digest: Some(digest.clone()),
        browser_download_url:
            "https://github.com/fixture/app/releases/download/v1.0.0/fixture-windows-x64.msi".into(),
    };
    let evidence = ScopeEvidence {
        tag: "v1.0.0".into(),
        asset_name: asset.name.clone(),
        digest,
        size: 42,
        os: "windows".into(),
        architecture: "x64".into(),
        policy: ScopePolicy::MachineOnly,
        inspector_version: 1,
    };
    let mut state = store::load(&root.0, provider::source()).unwrap();
    state.observations.push(store::Observation {
        repository: "fixture/app".into(),
        evidence,
        used: chrono::Utc::now().timestamp(),
    });
    store::save(&root.0, &mut state, "fixture/app").unwrap();
    store::publish_memory(
        &root.0,
        store::snapshot(&state, provider::source()).unwrap(),
    );
    let lookup = |repo, tag, asset: &crate::model::Asset, scope| {
        inspection::decision(&root.0, repo, tag, asset, "x64", scope)
    };
    assert!(matches!(
        lookup("fixture/app", "v1.0.0", &asset, Some("user")),
        CompatibilityDecision::Incompatible(_)
    ));
    assert_eq!(
        lookup("fixture/app", "v1.0.0", &asset, Some("machine")),
        CompatibilityDecision::Compatible
    );
    assert!(matches!(
        lookup("fixture/other", "v1.0.0", &asset, Some("user")),
        CompatibilityDecision::Unknown(_)
    ));
    assert!(matches!(
        lookup("fixture/app", "v2.0.0", &asset, Some("user")),
        CompatibilityDecision::Unknown(_)
    ));
    let mut changed = asset.clone();
    changed.size += 1;
    assert!(matches!(
        lookup("fixture/app", "v1.0.0", &changed, Some("user")),
        CompatibilityDecision::Unknown(_)
    ));
    changed = asset.clone();
    changed.digest = Some(format!("sha256:{}", "b".repeat(64)));
    assert!(matches!(
        lookup("fixture/app", "v1.0.0", &changed, Some("user")),
        CompatibilityDecision::Unknown(_)
    ));
}
#[test]
#[ignore = "Explicit local catalog performance measurement; no real HTTP"]
fn measure_catalog_cold_disk_and_warm_lookup() {
    let root = Root::new();
    let mut app = app_fixture();
    app.releases.clear();
    for index in 0..1000 {
        let tag = format!("v1.0.{index}");
        let assets=(0..8).map(|n|crate::model::Asset{name:format!("fixture-{n}-windows-x64.zip"),size:4096,digest:Some(format!("sha256:{}","a".repeat(64))),browser_download_url:format!("https://github.com/storytold/filmcraft/releases/download/{tag}/fixture-{n}-windows-x64.zip")}).collect();
        app.releases.push(crate::model::Release {
            tag_name: tag,
            name: None,
            draft: false,
            prerelease: false,
            assets,
        });
    }
    app.latest_tag = Some("v1.0.999".into());
    let (source, index, body) = bundle(&app, chrono::Utc::now().timestamp(), 7);
    let decoded = body.len();
    let transport = Transport {
        index,
        app: body,
        count: std::cell::Cell::new(0),
    };
    let start = std::time::Instant::now();
    client::refresh(&root.0, &source, &app.repository, &transport).unwrap();
    let cold = start.elapsed();
    let start = std::time::Instant::now();
    let state = store::load(&root.0, &source).unwrap();
    let snapshot = store::snapshot(&state, &source).unwrap();
    let disk = start.elapsed();
    store::publish_memory(&root.0, snapshot);
    let mut samples = Vec::new();
    for _ in 0..5000 {
        let start = std::time::Instant::now();
        assert!(provider::peek_release(&root.0, &app.repository, Some("v1.0.500")).is_some());
        samples.push(start.elapsed().as_nanos());
    }
    samples.sort();
    println!("profile=debug fixture=1000-releases/8000-assets decoded_bytes={decoded} cache_bytes={} cold_fixture_ms={:.3} disk_load_ms={:.3} warm_p50_us={:.3} warm_p95_us={:.3} metadata_requests={} installer_downloads=0",std::fs::metadata(root.0.join("runtime/app-release-catalog/cache.json.gz")).unwrap().len(),cold.as_secs_f64()*1000.0,disk.as_secs_f64()*1000.0,samples[2500] as f64/1000.0,samples[4750] as f64/1000.0,transport.count.get());
    assert_eq!(transport.count.get(), 2);
}

#[test]
fn publisher_checkpoint_retains_only_installation_metadata() {
    struct Upstream;
    impl publisher::Upstream for Upstream {
        fn page(
            &self,
            url: &str,
            _: Option<&publisher::Page>,
        ) -> anyhow::Result<Option<publisher::Page>> {
            let mut release = serde_json::to_value(&app_fixture().releases[0])?;
            release["body"] = serde_json::json!("unused-upstream-field");
            release["author"] = serde_json::json!({"login":"unused-upstream-field"});
            Ok(Some(publisher::Page {
                etag: Some("fixture-etag".into()),
                body: if url.ends_with("/latest") {
                    release
                } else {
                    serde_json::json!([release])
                },
            }))
        }
    }
    let result = publisher::discover(&[app_fixture().repository], None, &Upstream).unwrap();
    assert!(result
        .repositories
        .values()
        .all(|scan| scan.error.is_none()));
    assert_eq!(result.pages.len(), 2);
    assert!(result
        .pages
        .values()
        .all(|page| page.etag.as_deref() == Some("fixture-etag")));
    assert!(!serde_json::to_string(&result)
        .unwrap()
        .contains("unused-upstream-field"));
}

#[test]
fn publisher_enrolls_only_verified_new_craft_apps_without_removing_existing_entries() {
    struct Organization;
    impl publisher::Upstream for Organization {
        fn page(
            &self,
            url: &str,
            _: Option<&publisher::Page>,
        ) -> anyhow::Result<Option<publisher::Page>> {
            let body = if url.contains("/orgs/") {
                serde_json::json!([
                    {"name":"newcraft"},{"name":"wrongcraft"},{"name":"sourcecraft"},
                    {"name":"archivedcraft","archived":true},{"name":"forkcraft","fork":true},
                    {"name":"disabledcraft","disabled":true},{"name":"unrelated"}
                ])
            } else if url.contains("/sourcecraft/") || url.contains("/repos/tangency-labs/") {
                return Ok(None);
            } else {
                assert!(
                    url.contains("/newcraft/") || url.contains("/wrongcraft/"),
                    "Excluded candidate was probed: {url}"
                );
                let repo = if url.contains("/newcraft/") {
                    "newcraft"
                } else {
                    "othercraft"
                };
                serde_json::json!({"tag_name":"v1.0.0","draft":false,"prerelease":false,"assets":[{"name":format!("{repo}-1.0.0-windows-x64-portable.zip"),"size":42,"browser_download_url":format!("https://github.com/storytold/{repo}/releases/download/v1.0.0/{repo}-1.0.0-windows-x64-portable.zip")} ]})
            };
            Ok(Some(publisher::Page { etag: None, body }))
        }
    }
    let result = publisher::enroll(&["storytold/filmcraft".into()], &Organization).unwrap();
    assert_eq!(result, vec!["storytold/filmcraft", "storytold/newcraft"]);
    struct Offline;
    impl publisher::Upstream for Offline {
        fn page(
            &self,
            _: &str,
            _: Option<&publisher::Page>,
        ) -> anyhow::Result<Option<publisher::Page>> {
            anyhow::bail!("Offline")
        }
    }
    assert_eq!(publisher::enroll(&result, &Offline).unwrap(), result);
}

#[test]
fn inspection_queue_reuses_exact_evidence_and_attempts_each_unverified_file_once_per_run() {
    let mut app = app_fixture();
    for n in 0..5 {
        app.releases[0].assets.push(crate::model::Asset {
            name: format!("fixture-{n}-windows-x64.msi"),
            size: 42,
            digest: Some(format!("sha256:{}", "a".repeat(64))),
            browser_download_url: format!("https://github.com/storytold/filmcraft/releases/download/v2.0.0/fixture-{n}-windows-x64.msi"),
        });
    }
    let discovery = publisher::Discovery {
        schema: 1,
        repositories: BTreeMap::from([(
            app.repository.clone(),
            publisher::Scan {
                checked_at: chrono::Utc::now().timestamp(),
                latest_tag: app.latest_tag.clone(),
                releases: app.releases.clone(),
                error: None,
            },
        )]),
        pages: BTreeMap::new(),
    };
    let mut prior = publisher::InspectionBatch::default();
    assert_eq!(
        publisher::inspection_queue(&discovery, None, &prior, Some(200)).len(),
        5
    );
    let input = publisher::pending(&discovery, None).remove(0);
    prior.observations.push(store::Observation {
        repository: input.repository.clone(),
        used: 100,
        evidence: ScopeEvidence {
            tag: input.tag.clone(),
            asset_name: input.asset.name.clone(),
            digest: input.asset.digest.clone().unwrap(),
            size: input.asset.size,
            os: "windows".into(),
            architecture: "x64".into(),
            policy: ScopePolicy::MachineOnly,
            inspector_version: inspection::INSPECTOR_VERSION,
        },
    });
    let failed = publisher::pending(&discovery, None).remove(1);
    prior
        .attempts
        .insert(publisher::inspection_identity(&failed), 200);
    assert_eq!(
        publisher::inspection_queue(&discovery, None, &prior, Some(200)).len(),
        3
    );
    assert_eq!(
        publisher::inspection_queue(&discovery, None, &prior, Some(201)).len(),
        4
    );
    let published = publisher::aggregate(None, &discovery, &prior.observations).unwrap();
    assert_eq!(
        publisher::inspection_queue(
            &discovery,
            Some(&published),
            &publisher::InspectionBatch::default(),
            None
        )
        .len(),
        4
    );
    let mut changed = discovery.clone();
    changed
        .repositories
        .get_mut(&app.repository)
        .unwrap()
        .releases[0]
        .assets[0]
        .digest = Some(format!("sha256:{}", "b".repeat(64)));
    assert_eq!(
        publisher::inspection_queue(&changed, Some(&published), &prior, Some(200)).len(),
        4
    );
    prior.observations[0].evidence.inspector_version = 0;
    assert_eq!(
        publisher::inspection_queue(&discovery, None, &prior, None).len(),
        5
    );
}

fn hosted_smoke_source() -> Source {
    let source: Source = serde_json::from_slice(
        &std::fs::read(std::env::var_os("CRAFT_CATALOG_SMOKE_SOURCE").expect("Source descriptor"))
            .unwrap(),
    )
    .unwrap();
    assert!(matches!(source.index_url.as_deref(),
        Some("https://raw.githubusercontent.com/CryptoKey98/craft-apps-manager-update-tests/app-catalog-internal/v1/index.json")
        | Some("https://raw.githubusercontent.com/CryptoKey98/craft-apps-manager/app-release-catalog/v1/index.json")));
    source
}

#[test]
#[ignore = "Explicit opt-in read-only smoke test of a configured hosted catalog"]
fn hosted_catalog_serves_history_latest_tag_and_scope_without_api_fallback() {
    let source = hosted_smoke_source();
    struct Count(client::Http, std::cell::Cell<usize>);
    impl client::Transport for Count {
        fn get(
            &self,
            url: &str,
            etag: Option<&str>,
            limit: usize,
        ) -> anyhow::Result<client::Reply> {
            assert!(url.starts_with("https://raw.githubusercontent.com/"));
            assert!(url.ends_with(".json"));
            self.1.set(self.1.get() + 1);
            self.0.get(url, etag, limit)
        }
    }
    let root = Root::new();
    let transport = Count(client::Http::new().unwrap(), std::cell::Cell::new(0));
    let start = std::time::Instant::now();
    let history: Vec<crate::model::Release> = provider::resolve_with(
        &root.0,
        "https://api.github.com/repos/storytold/filmcraft/releases?per_page=100",
        &source,
        &transport,
        || panic!("Unexpected GitHub API fallback"),
    )
    .unwrap();
    let cold = start.elapsed();
    let latest: crate::model::Release = provider::resolve_with(
        &root.0,
        "https://api.github.com/repos/storytold/filmcraft/releases/latest",
        &source,
        &transport,
        || panic!("Unexpected GitHub API fallback"),
    )
    .unwrap();
    let old: crate::model::Release = provider::resolve_with(
        &root.0,
        "https://api.github.com/repos/storytold/filmcraft/releases/tags/v0.13.0",
        &source,
        &transport,
        || panic!("Unexpected GitHub API fallback"),
    )
    .unwrap();
    assert!(history.iter().any(|r| r.tag_name == latest.tag_name));
    assert_eq!(old.tag_name, "v0.13.0");
    if cfg!(windows) {
        let asset = old
            .assets
            .iter()
            .find(|a| a.name.ends_with("windows-x64.msi"))
            .unwrap();
        assert!(matches!(
            inspection::decision(
                &root.0,
                "storytold/filmcraft",
                &old.tag_name,
                asset,
                "x64",
                Some("user")
            ),
            CompatibilityDecision::Incompatible(_)
        ));
    }
    assert_eq!(transport.1.get(), 2);
    store::clear(&root.0).unwrap();
    println!("hosted_history_releases={} latest={} cold_ms={:.1} static_metadata_requests=2 github_api_requests=0 installer_downloads=0", history.len(), latest.tag_name, cold.as_secs_f64()*1000.0);
}

#[test]
fn publisher_automatically_enrolls_verified_supported_external_app() {
    struct External;
    impl publisher::Upstream for External {
        fn page(
            &self,
            url: &str,
            _: Option<&publisher::Page>,
        ) -> anyhow::Result<Option<publisher::Page>> {
            let body = match url {
                "https://api.github.com/orgs/storytold/repos?per_page=100&page=1" => {
                    serde_json::json!([])
                }
                "https://api.github.com/repos/tangency-labs/solidcraft" => {
                    serde_json::json!({"name":"solidcraft","full_name":"tangency-labs/solidcraft"})
                }
                "https://api.github.com/repos/tangency-labs/solidcraft/releases/latest" => {
                    serde_json::json!({"tag_name":"v0.29.0","draft":false,"prerelease":false,"assets":[{"name":"solidcraft-0.29.0-windows-x64.msi","size":42,"browser_download_url":"https://github.com/tangency-labs/solidcraft/releases/download/v0.29.0/solidcraft-0.29.0-windows-x64.msi"}]})
                }
                _ => panic!("Unexpected probe: {url}"),
            };
            Ok(Some(publisher::Page { etag: None, body }))
        }
    }
    assert_eq!(
        publisher::enroll(&[], &External).unwrap(),
        vec!["tangency-labs/solidcraft"]
    );
}

#[test]
fn supported_external_app_falls_back_until_its_signed_catalog_entry_exists() {
    let root = Root::new();
    let mut app = app_fixture();
    app.repository = "tangency-labs/solidcraft".into();
    app.releases[0].assets.push(crate::model::Asset {
        name: "solidcraft-2.0.0-windows-x64.msi".into(), size: 42, digest: None,
        browser_download_url: "https://github.com/tangency-labs/solidcraft/releases/download/v2.0.0/solidcraft-2.0.0-windows-x64.msi".into(),
    });
    let entry = crate::catalog::classify("solidcraft", Some(&app.releases[0]));
    crate::files::write_json(
        &root.0.join("runtime/catalog.json"),
        &serde_json::json!({"checkedAt":1,"entries":[entry]}),
    )
    .unwrap();
    crate::catalog::load(&root.0);
    struct ResetCatalog;
    impl Drop for ResetCatalog {
        fn drop(&mut self) {
            crate::catalog::load(
                &std::env::temp_dir().join(format!("craft-empty-{}", uuid::Uuid::new_v4())),
            );
        }
    }
    let _reset = ResetCatalog;
    assert!(
        crate::catalog::get("solidcraft")
            .is_some_and(|entry| entry.release && entry.repository == "solidcraft"),
        "{:?}",
        crate::catalog::get("solidcraft")
    );
    let url = "https://api.github.com/repos/tangency-labs/solidcraft/releases/latest";
    let (source, index) = signed(&self::root());
    let empty = Transport {
        index,
        app: vec![],
        count: std::cell::Cell::new(0),
    };
    let result: crate::model::Release = provider::resolve_with(
        &root.0,
        url,
        &source,
        &empty,
        || Ok(app.releases[0].clone()),
    )
    .unwrap();
    assert_eq!(result.tag_name, "v2.0.0");
    assert_eq!(
        empty.count.get(),
        1,
        "Absent app needs only the root, then API fallback"
    );

    // An actual newer signed publication becomes usable after ordinary cache revalidation.
    let mut state = store::load(&root.0, &source).unwrap();
    state.attempted = chrono::Utc::now().timestamp() - 61;
    store::save(&root.0, &mut state, &app.repository).unwrap();
    store::publish_memory(&root.0, store::snapshot(&state, &source).unwrap());
    let (source, index, body) = bundle(&app, chrono::Utc::now().timestamp() + 2, 8);
    struct UpdatedPublication(Transport);
    impl client::Transport for UpdatedPublication {
        fn get(
            &self,
            url: &str,
            etag: Option<&str>,
            limit: usize,
        ) -> anyhow::Result<client::Reply> {
            if url.ends_with("index.json") {
                assert_eq!(etag, Some("fixture-etag"));
                self.0.count.set(self.0.count.get() + 1);
                Ok(client::Reply::Data(
                    self.0.index.clone(),
                    Some("updated-etag".into()),
                ))
            } else {
                self.0.get(url, etag, limit)
            }
        }
    }
    let transport = UpdatedPublication(Transport {
        index,
        app: body,
        count: std::cell::Cell::new(0),
    });
    let result: crate::model::Release =
        provider::resolve_with(&root.0, url, &source, &transport, || {
            anyhow::bail!("Catalog should satisfy this lookup")
        })
        .unwrap();
    assert_eq!(result.tag_name, "v2.0.0");
    assert_eq!(transport.0.count.get(), 2);

    // An explicit custom registration remains API-only, even for this supported source.
    let mut custom = entry;
    custom.repository = "tangency-labs/solidcraft".into();
    crate::files::write_json(&root.0.join("runtime/custom-apps.json"), &vec![custom]).unwrap();
    crate::catalog::load(&root.0);
    let result: String =
        provider::resolve_with(&root.0, url, &source, &transport, || Ok("api-only".into()))
            .unwrap();
    assert_eq!(result, "api-only");
    assert_eq!(transport.0.count.get(), 2);
}

#[test]
fn publisher_preserves_official_admission_date_across_refreshes() {
    let repo = "storytold/newcraft".to_string();
    let discovery = publisher::Discovery {
        schema: 1,
        repositories: BTreeMap::from([(
            repo.clone(),
            publisher::Scan {
                checked_at: chrono::Utc::now().timestamp(),
                latest_tag: None,
                releases: vec![],
                error: None,
            },
        )]),
        ..Default::default()
    };
    let first = publisher::aggregate(None, &discovery, &[]).unwrap();
    let value = serde_json::to_value(&first.index.repositories[&repo]).unwrap();
    let added = value["discovery_added_at"]
        .as_i64()
        .expect("First official admission needs a date");
    assert!(added > 0);
    let next = publisher::aggregate(Some(&first), &discovery, &[]).unwrap();
    assert_eq!(
        serde_json::to_value(&next.index.repositories[&repo]).unwrap()["discovery_added_at"],
        added
    );
}

#[test]
fn official_discovery_dates_require_only_a_verified_root_request() {
    let app = app_fixture();
    let (source, index, _) = bundle(&app, chrono::Utc::now().timestamp(), 7);
    let verified = verify_index(&index, &source, 0).unwrap();
    let mut index = verified.index;
    let added = chrono::Utc::now().timestamp() - 3600;
    index
        .repositories
        .get_mut(&app.repository)
        .unwrap()
        .discovery_added_at = Some(added);
    let (source, index) = signed(&serde_json::to_string(&index).unwrap());
    let transport = Transport {
        index,
        app: vec![],
        count: std::cell::Cell::new(0),
    };
    let root = Root::new();
    let snapshot = client::refresh_index(&root.0, &source, &transport).unwrap();
    assert_eq!(snapshot.discovery_added_at[&app.repository], added);
    assert!(snapshot.apps.is_empty());
    assert_eq!(
        transport.count.get(),
        1,
        "Discovery dates do not need any per-app downloads"
    );
    let cached = client::refresh_index(&root.0, &source, &transport).unwrap();
    assert_eq!(cached.discovery_added_at[&app.repository], added);
    assert_eq!(
        transport.count.get(),
        1,
        "Normal cache cooldown still applies"
    );
}

#[test]
fn malformed_signed_admission_dates_are_rejected() {
    let (source, index, _) = bundle(&app_fixture(), chrono::Utc::now().timestamp(), 7);
    let verified = verify_index(&index, &source, 0).unwrap();
    for added in [-1, 0, verified.index.generated_at + 1] {
        let mut index = verified.index.clone();
        index
            .repositories
            .values_mut()
            .next()
            .unwrap()
            .discovery_added_at = Some(added);
        let (source, bytes) = signed(&serde_json::to_string(&index).unwrap());
        assert!(verify_index(&bytes, &source, 0).is_err());
    }
}

#[test]
#[ignore = "Explicit read-only validation of a configured hosted catalog's enrollments"]
fn hosted_newly_enrolled_apps_use_catalog_without_api_fallback() {
    let source = hosted_smoke_source();
    let root = Root::new();
    if let Some(cache) = std::env::var_os("CRAFT_CATALOG_SMOKE_CACHE") {
        let destination = root.0.join("runtime/app-release-catalog/cache.json.gz");
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::copy(cache, destination).unwrap();
    }
    let entries: Vec<_> = ["solidcraft", "texturecraft"]
        .into_iter()
        .map(|name| {
            let mut entry = crate::catalog::builtin().remove(0);
            entry.key = name.into();
            entry.repository = name.into();
            entry.title = name.into();
            entry.asset_prefix = name.into();
            entry
        })
        .collect();
    crate::files::write_json(
        &root.0.join("runtime/catalog.json"),
        &serde_json::json!({"checkedAt":1,"entries":entries}),
    )
    .unwrap();
    crate::catalog::load(&root.0);
    for repository in ["tangency-labs/solidcraft", "storytold/texturecraft"] {
        let transport = client::Http::new().unwrap();
        let history: Vec<crate::model::Release> = provider::resolve_with(
            &root.0,
            &format!("https://api.github.com/repos/{repository}/releases?per_page=100"),
            &source,
            &transport,
            || anyhow::bail!("Unexpected API fallback for {repository}"),
        )
        .unwrap();
        assert!(!history.is_empty());
        let latest: crate::model::Release = provider::resolve_with(
            &root.0,
            &format!("https://api.github.com/repos/{repository}/releases/latest"),
            &source,
            &transport,
            || anyhow::bail!("Unexpected latest API fallback for {repository}"),
        )
        .unwrap();
        assert!(history
            .iter()
            .any(|release| release.tag_name == latest.tag_name));
        assert!(store::peek(&root.0)
            .unwrap()
            .discovery_added_at
            .contains_key(repository));
        println!(
            "{repository}: {} releases, latest {}, signed admission date, no API fallback",
            history.len(),
            latest.tag_name
        );
    }
    crate::catalog::load(&root.0.join("empty"));
}

#[test]
fn repeated_identical_cached_fallback_does_not_advance_api_override_time() {
    let root = Root::new();
    let app = app_fixture();
    let now = chrono::Utc::now().timestamp();
    let (source, index, body) = bundle(&app, now - 120, 7);
    let transport = Transport {
        index,
        app: body,
        count: std::cell::Cell::new(0),
    };
    client::refresh(&root.0, &source, &app.repository, &transport).unwrap();
    let mut state = store::load(&root.0, &source).unwrap();
    let previous = now - 60;
    state.overrides.insert(app.repository.clone(), previous);
    store::save(&root.0, &mut state, &app.repository).unwrap();
    for _ in 0..2 {
        let _: crate::model::Release = provider::resolve_with(
            &root.0,
            "https://api.github.com/repos/storytold/filmcraft/releases/latest",
            &source,
            &transport,
            || Ok(app.releases[0].clone()),
        )
        .unwrap();
    }
    assert_eq!(
        store::load(&root.0, &source).unwrap().overrides[&app.repository],
        previous,
        "Using a fallback is not proof of newly fetched or changed API data"
    );
}

#[test]
fn bundled_metadata_is_scoped_to_its_trust_source_without_blocking_other_sources() {
    let app = app_fixture();
    let now = chrono::Utc::now().timestamp();
    let (source, index, body) = bundle(&app, now, 7);
    let envelope = String::from_utf8(index).unwrap();
    let reference = serde_json::from_str::<Index>(
        &serde_json::from_str::<Envelope>(&envelope).unwrap().payload,
    )
    .unwrap()
    .repositories[&app.repository]
        .clone();
    let raw=serde_json::json!({"schema":1,"envelope":envelope,"objects":{reference.sha256:String::from_utf8(body).unwrap()}}).to_string();
    let mut other = source.clone();
    other.keys = BTreeMap::from([("different-feed".into(), source.keys["fixture"].clone())]);
    let mut state = store::State::default();
    assert!(
        store::merge_baseline(&mut state, &other, &raw).is_ok(),
        "A different configured feed must not fail because of an unrelated bundled snapshot"
    );
    assert!(
        state.envelope.is_none(),
        "Foreign baseline data must never be adopted"
    );
    store::merge_baseline(&mut state, &source, &raw).unwrap();
    assert_eq!(
        store::snapshot(&state, &source).unwrap().apps[&app.repository].latest_tag,
        app.latest_tag
    );
    let previous = state.envelope.clone();
    let mut malformed: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let mut invalid: Envelope =
        serde_json::from_str(malformed["envelope"].as_str().unwrap()).unwrap();
    invalid.signature = "00".repeat(64);
    malformed["envelope"] = serde_json::to_string(&invalid).unwrap().into();
    assert!(store::merge_baseline(&mut state, &source, &malformed.to_string()).is_ok());
    assert_eq!(
        state.envelope, previous,
        "An invalid baseline must not erase verified data"
    );
}

#[test]
fn matching_api_responses_do_not_suppress_the_catalog() {
    let root = Root::new();
    let app = app_fixture();
    let now = chrono::Utc::now().timestamp();
    let (source, index, body) = bundle(&app, now - 120, 7);
    let transport = Transport {
        index,
        app: body,
        count: std::cell::Cell::new(0),
    };
    client::refresh(&root.0, &source, &app.repository, &transport).unwrap();
    for (suffix, value) in [
        ("latest", serde_json::to_value(&app.releases[0]).unwrap()),
        (
            "tags/v2.0.0",
            serde_json::to_value(&app.releases[0]).unwrap(),
        ),
        (
            "?per_page=100",
            serde_json::to_value(&app.releases).unwrap(),
        ),
    ] {
        let separator = if suffix.starts_with('?') { "" } else { "/" };
        let url = format!(
            "https://api.github.com/repos/{}/releases{separator}{suffix}",
            app.repository
        );
        for _ in 0..2 {
            super::api_observation::observe_with(&root.0, &source, &url, &value, now);
        }
    }
    let snapshot = store::peek(&root.0).unwrap();
    assert!(snapshot.overrides.is_empty());
    assert!(
        snapshot.api_observations.is_empty(),
        "Matching metadata needs no extra cache record"
    );
    let _: Vec<crate::model::Release> = provider::resolve_with(
        &root.0,
        "https://api.github.com/repos/storytold/filmcraft/releases?per_page=100",
        &source,
        &transport,
        || panic!("Matching API data must not force fallback"),
    )
    .unwrap();
}

#[test]
fn actual_api_evidence_survives_late_enrollment_but_matching_catalog_is_adopted() {
    let root = Root::new();
    let app = app_fixture();
    let now = chrono::Utc::now().timestamp();
    let (source, index, body) = bundle(&app, now - 120, 7);
    let url = "https://api.github.com/repos/storytold/filmcraft/releases/latest";
    super::api_observation::observe_with(
        &root.0,
        &source,
        url,
        &serde_json::to_value(&app.releases[0]).unwrap(),
        now - 60,
    );
    assert_eq!(
        store::load(&root.0, &source)
            .unwrap()
            .api_observations
            .len(),
        1
    );
    let transport = Transport {
        index,
        app: body,
        count: std::cell::Cell::new(0),
    };
    let snapshot = client::refresh(&root.0, &source, &app.repository, &transport).unwrap();
    assert!(
        snapshot.overrides.is_empty(),
        "Older but identical signed contents are usable"
    );
    assert!(provider::value(&snapshot, &app.repository, &provider::Query::Latest).is_some());

    let mut newer = app.releases[0].clone();
    newer.tag_name = "v3.0.0".into();
    let newer_json = serde_json::to_value(&newer).unwrap();
    super::api_observation::observe_with(&root.0, &source, url, &newer_json, now);
    assert!(provider::value(
        &store::peek(&root.0).unwrap(),
        &app.repository,
        &provider::Query::Latest
    )
    .is_none());
    // A delayed old cached response must not erase knowledge of the newer release.
    super::api_observation::observe_with(
        &root.0,
        &source,
        url,
        &serde_json::to_value(&app.releases[0]).unwrap(),
        now - 30,
    );
    assert!(provider::value(
        &store::peek(&root.0).unwrap(),
        &app.repository,
        &provider::Query::Latest
    )
    .is_none());
    // A later signed scan is authoritative; reopening does not renew the old API time.
    let (_, index, _) = bundle(&app, now + 1, 8);
    let mut state = store::load(&root.0, &source).unwrap();
    state.envelope = Some(String::from_utf8(index).unwrap());
    let snapshot = store::snapshot(&state, &source).unwrap();
    assert!(provider::value(&snapshot, &app.repository, &provider::Query::Latest).is_some());
}

#[test]
fn replaced_assets_and_withdrawn_tags_keep_api_protection_until_verified_again() {
    let root = Root::new();
    let mut app = app_fixture();
    app.releases[0].assets.push(crate::model::Asset {
        name: "filmcraft.zip".into(),
        size: 2,
        browser_download_url:
            "https://github.com/storytold/filmcraft/releases/download/v2.0.0/filmcraft.zip".into(),
        digest: None,
    });
    let now = chrono::Utc::now().timestamp();
    let (source, index, body) = bundle(&app, now - 60, 7);
    client::refresh(
        &root.0,
        &source,
        &app.repository,
        &Transport {
            index,
            app: body,
            count: std::cell::Cell::new(0),
        },
    )
    .unwrap();
    let url = "https://api.github.com/repos/storytold/filmcraft/releases/tags/v2.0.0";
    let mut changed = app.releases[0].clone();
    changed.assets[0].size += 1;
    super::api_observation::observe_with(
        &root.0,
        &source,
        url,
        &serde_json::to_value(changed).unwrap(),
        now - 2,
    );
    assert!(store::peek(&root.0)
        .unwrap()
        .overrides
        .contains_key(&app.repository));
    // Newer evidence of the original bytes can retire the conflict for that query.
    super::api_observation::observe_with(
        &root.0,
        &source,
        url,
        &serde_json::to_value(&app.releases[0]).unwrap(),
        now - 1,
    );
    assert!(store::peek(&root.0).unwrap().overrides.is_empty());
    super::api_observation::observe_with(
        &root.0,
        &source,
        "https://api.github.com/repos/storytold/filmcraft/releases?per_page=100",
        &serde_json::json!([]),
        now,
    );
    assert!(
        store::peek(&root.0)
            .unwrap()
            .overrides
            .contains_key(&app.repository),
        "A newly empty history must not revive withdrawn releases"
    );
    // A matching latest response cannot hide a separate history conflict.
    super::api_observation::observe_with(
        &root.0,
        &source,
        "https://api.github.com/repos/storytold/filmcraft/releases/latest",
        &serde_json::to_value(&app.releases[0]).unwrap(),
        now,
    );
    assert!(store::peek(&root.0)
        .unwrap()
        .overrides
        .contains_key(&app.repository));
}

#[test]
fn api_history_pages_compare_only_the_observed_stable_prefix() {
    for page_size in [30, 100] {
        let root = Root::new();
        let mut app = app_fixture();
        app.releases = (0..130)
            .map(|index| {
                let mut release = app.releases[0].clone();
                release.tag_name = format!("v{index}.0.0");
                release
            })
            .collect();
        app.latest_tag = Some(app.releases[0].tag_name.clone());
        let now = chrono::Utc::now().timestamp();
        let (source, index, body) = bundle(&app, now - 60, 7);
        let url = if page_size == 100 {
            "https://api.github.com/repos/storytold/filmcraft/releases?per_page=100"
        } else {
            "https://api.github.com/repos/storytold/filmcraft/releases"
        };
        // Receive the API prefix before the complete signed app object arrives.
        super::api_observation::observe_with(
            &root.0,
            &source,
            url,
            &serde_json::to_value(&app.releases[..page_size]).unwrap(),
            now,
        );
        let snapshot = client::refresh(
            &root.0,
            &source,
            &app.repository,
            &Transport {
                index,
                app: body,
                count: std::cell::Cell::new(0),
            },
        )
        .unwrap();
        assert!(
            snapshot.overrides.is_empty(),
            "A full page is not proof that older releases disappeared"
        );
        assert_eq!(snapshot.api_observations.len(), 1);
    }
}

#[test]
fn api_observation_cache_is_optional_bounded_and_cleared_with_metadata() {
    let root = Root::new();
    let app = app_fixture();
    // Account for time passing while the fixture is prepared on a busy runner.
    let now = chrono::Utc::now().timestamp() - 2;
    let (source, _, _) = bundle(&app, now, 7);
    let state = store::load(&root.0, &source).unwrap();
    let mut old = serde_json::to_value(state).unwrap();
    old.as_object_mut().unwrap().remove("api_observations");
    assert!(serde_json::from_value::<store::State>(old)
        .unwrap()
        .api_observations
        .is_empty());
    let body = serde_json::to_value(&app.releases[0]).unwrap();
    // Stay well outside the 300-second skew allowance as the test clock advances.
    for at in [0, now - 86401, now + 3600] {
        super::api_observation::observe_with(
            &root.0,
            &source,
            "https://api.github.com/repos/storytold/filmcraft/releases/latest",
            &body,
            at,
        );
    }
    super::api_observation::observe_with(
        &root.0,
        &source,
        "https://api.github.com/repos/custom/app/releases/latest",
        &body,
        now,
    );
    assert!(
        !root.0.exists(),
        "Invalid evidence and custom apps do not create catalog files"
    );
    super::api_observation::observe_with(
        &root.0,
        &source,
        "https://api.github.com/repos/storytold/filmcraft/releases/latest",
        &body,
        now,
    );
    let mut state = store::load(&root.0, &source).unwrap();
    state.api_observations = vec![state.api_observations[0].clone(); 513];
    store::save(&root.0, &mut state, &app.repository).unwrap();
    assert_eq!(state.api_observations.len(), 512);
    store::clear(&root.0).unwrap();
    assert!(store::peek(&root.0).is_none());
    assert!(store::load(&root.0, &source)
        .unwrap()
        .api_observations
        .is_empty());
}

#[cfg(target_os = "macos")]
#[test]
fn standard_macos_temporary_aliases_allow_catalog_storage_and_publication() {
    let root = Root(
        std::path::Path::new("/tmp").join(format!("craft-catalog-alias-{}", uuid::Uuid::new_v4())),
    );
    let app = app_fixture();
    let now = chrono::Utc::now().timestamp();
    let (source, index, body) = bundle(&app, now, 7);
    let result = client::refresh(
        &root.0,
        &source,
        &app.repository,
        &Transport {
            index,
            app: body,
            count: std::cell::Cell::new(0),
        },
    );
    assert!(
        result.is_ok(),
        "The root-owned macOS /tmp alias is a normal filesystem path: {:?}",
        result.as_ref().err()
    );
    let physical = std::fs::canonicalize(&root.0).unwrap();
    assert_eq!(
        store::root_id(&root.0),
        store::root_id(&physical),
        "Aliases must share the same cache and process lock"
    );
    let publication = publisher::Publication {
        index: Index {
            schema: 1,
            purpose: "app-release-catalog".into(),
            sequence: 1,
            generated_at: now,
            repositories: BTreeMap::new(),
        },
        objects: BTreeMap::new(),
    };
    publisher::sign(
        &publication,
        &SigningKey::from_bytes(&[37; 32]),
        "fixture",
        &root.0.join("publication"),
    )
    .unwrap();
}

#[cfg(unix)]
#[test]
fn user_created_catalog_ancestor_links_remain_rejected() {
    let root = Root::new();
    std::fs::create_dir_all(&root.0).unwrap();
    let physical = std::fs::canonicalize(&root.0).unwrap();
    let real = physical.join("real");
    std::fs::create_dir(&real).unwrap();
    let alias = physical.join("user-link");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let (source, _) = signed(&super::tests::root());
    assert!(store::load(&alias, &source).is_err());
    let publication = publisher::Publication {
        index: Index {
            schema: 1,
            purpose: "app-release-catalog".into(),
            sequence: 1,
            generated_at: chrono::Utc::now().timestamp(),
            repositories: BTreeMap::new(),
        },
        objects: BTreeMap::new(),
    };
    assert!(publisher::sign(
        &publication,
        &SigningKey::from_bytes(&[37; 32]),
        "fixture",
        &alias.join("publication")
    )
    .is_err());
    assert_eq!(
        std::fs::read_dir(&real).unwrap().count(),
        0,
        "Reject the link before writing anything through it"
    );
}
