use super::super::edit_session::Locations;
use super::*;
use craft_apps_manager::storage::config::{ConfigPatch, ConfigRevision};
use std::sync::{Arc, Mutex};

struct Fake {
    calls: Arc<Mutex<Vec<&'static str>>>,
    fail: Option<&'static str>,
    gate: Option<mpsc::Receiver<()>>,
}
impl Fake {
    fn call(&self, name: &'static str) -> anyhow::Result<()> {
        self.calls.lock().unwrap().push(name);
        anyhow::ensure!(self.fail != Some(name), "fake {name} failure");
        Ok(())
    }
}
impl Backend for Fake {
    fn validate(&mut self, _: &SaveRequest) -> anyhow::Result<()> {
        self.call("validate")
    }
    fn config(&mut self, _: &SaveRequest) -> anyhow::Result<ConfigSnapshot> {
        if let Some(gate) = self.gate.take() {
            gate.recv().unwrap();
        }
        self.call("config")?;
        Ok(ConfigSnapshot {
            revision: ConfigRevision("fake".into()),
            document: serde_json::json!({"schema_version":1}),
        })
    }
    fn locations(&mut self, _: &SaveRequest) -> anyhow::Result<()> {
        self.call("locations")
    }
    fn scheduler(&mut self, _: &SaveRequest, _: bool) -> anyhow::Result<()> {
        self.call("scheduler")
    }
    fn token(&mut self, _: &TokenChange) -> anyhow::Result<network::TokenStatus> {
        self.call("token")?;
        Ok(network::TokenStatus::NotVerified)
    }
}
fn request() -> SaveRequest {
    let locations = Locations {
        root: "root".into(),
        tools: "tools".into(),
    };
    SaveRequest {
        paths: Paths::new("test-only-unused-root".into(), None),
        home: "unused-home".into(),
        edit: EditSnapshot {
            session_id: 42,
            config: ConfigPatch::default(),
            locations: Some((locations.clone(), locations)),
            scheduler: Some((false, true)),
            token: Some(TokenChange::Set("fake-secret".into())),
        },
    }
}

#[test]
fn saving_custom_app_publishes_sidebar_and_home_order_immediately() {
    use craft_apps_manager::{
        catalog, model,
        storage::config::{ConfigStore, FieldPatch},
    };
    let root = std::env::temp_dir().join(format!("craft-sidebar-save-{}", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(root.clone());
    store
        .commit(ConfigPatch {
            fields: vec![FieldPatch {
                path: vec!["manager".into()],
                before: store.load().unwrap().document.get("manager").cloned(),
                after: Some(serde_json::to_value(model::Preferences::default()).unwrap()),
            }],
        })
        .unwrap();
    let before = store.load().unwrap();
    let entry = serde_json::json!({"key":"obssidebarfixture","repository":"obsproject/obs-studio","title":"OBS","release":true,"source":false,"scheme":"generic","assetPrefix":"obsstudio"});
    let mut request = request();
    request.paths = Paths::new(root.clone(), None);
    request.edit.config.fields = vec![FieldPatch {
        path: vec!["custom_apps".into()],
        before: Some(before.document["custom_apps"].clone()),
        after: Some(serde_json::json!([entry])),
    }];
    let saved = SystemBackend.config(&request).unwrap();
    let preferences = saved.manager().unwrap();
    let original_selection = before.manager().unwrap().selected_apps;
    assert_eq!(
        preferences
            .selected_apps
            .iter()
            .collect::<std::collections::BTreeSet<_>>(),
        original_selection
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
    );
    assert!(
        model::ordered_release_apps(&preferences.app_order, &preferences.release_format)
            .contains(&"obssidebarfixture".to_owned())
    );
    assert!(preferences
        .home_app_order
        .contains(&"obssidebarfixture".to_owned()));
    assert!(store
        .load()
        .unwrap()
        .manager()
        .unwrap()
        .app_order
        .contains(&"obssidebarfixture".to_owned()));
    std::fs::remove_dir_all(&root).unwrap();
    catalog::load(&root);
}
fn fake(fail: Option<&'static str>) -> (Fake, Arc<Mutex<Vec<&'static str>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    (
        Fake {
            calls: calls.clone(),
            fail,
            gate: None,
        },
        calls,
    )
}
#[test]
fn validation_and_config_failure_do_not_start_other_effects() {
    for stage in ["validate", "config"] {
        let (mut backend, calls) = fake(Some(stage));
        let result = execute(&request(), &mut backend);
        assert!(result.config.error().is_some());
        assert!(matches!(result.token, Stage::NotRequested));
        assert_eq!(calls.lock().unwrap().last(), Some(&stage));
    }
}
#[test]
fn failed_locations_keep_config_and_stop_external_effects() {
    let (mut backend, calls) = fake(Some("locations"));
    let result = execute(&request(), &mut backend);
    assert!(result.config.applied());
    assert!(result.locations.error().is_some());
    assert_eq!(*calls.lock().unwrap(), ["validate", "config", "locations"]);
}
#[test]
fn scheduler_failure_does_not_lose_applied_token_and_retry_skips_it() {
    let (mut backend, calls) = fake(Some("scheduler"));
    let mut request = request();
    let result = execute(&request, &mut backend);
    assert!(result.token.applied());
    assert!(result.scheduler.error().is_some());
    request.edit.locations = None;
    request.edit.token = None;
    backend.fail = None;
    execute(&request, &mut backend);
    assert_eq!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|&&name| name == "token")
            .count(),
        1
    );
}
#[test]
fn token_failure_is_reported_after_other_effects_and_stores_no_secret_in_error() {
    let (mut backend, _) = fake(Some("token"));
    let result = execute(&request(), &mut backend);
    assert!(result.config.applied() && result.locations.applied() && result.scheduler.applied());
    assert!(!result.token.error().unwrap().contains("fake-secret"));
}
#[test]
fn slow_worker_leaves_result_poll_nonblocking() {
    let (mut backend, _) = fake(None);
    let (tx, gate) = mpsc::channel();
    backend.gate = Some(gate);
    let receiver = spawn(request(), backend, eframe::egui::Context::default());
    assert!(matches!(
        receiver.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    tx.send(()).unwrap();
    assert_eq!(
        receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .session_id,
        42
    );
}
