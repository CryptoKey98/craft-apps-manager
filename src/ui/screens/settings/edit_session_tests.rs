use super::*;
use serde_json::json;

fn session() -> SettingsEditSession {
    SettingsEditSession::begin(
        json!({"manager": {"theme": "system"}, "builder": {}, "custom_apps": []}),
        Preferences::default(),
        BuilderPreferences::default(),
        Vec::new(),
        Locations {
            root: "root".into(),
            tools: "tools".into(),
        },
        false,
    )
    .unwrap()
}

#[test]
fn every_close_route_discards_the_complete_session() {
    for route in [
        CloseRoute::Cancel,
        CloseRoute::TitleBar,
        CloseRoute::Escape,
        CloseRoute::Outside,
    ] {
        let mut s = session();
        let d = s.drafts_mut().unwrap();
        d.manager.close_to_tray = true;
        d.builder.log_size_mb = 50;
        d.locations.root = "elsewhere".into();
        d.scheduler_enabled = true;
        d.token.input = "secret".into();
        s.discard(route);
        assert!(s.drafts().is_none());
        assert!(s.snapshot_for_save().is_err());
    }
}

#[test]
fn nested_accept_is_pending_and_release_origin_cannot_edit_settings() {
    let mut s = session();
    assert!(!s.accept_nested_edit(SelectionOrigin::ReleaseReview, |d| d
        .manager
        .close_to_tray =
        true));
    assert!(
        s.accept_nested_edit(SelectionOrigin::Settings(s.id()), |d| d
            .manager
            .close_to_tray =
            true)
    );
    assert_eq!(s.snapshot_for_save().unwrap().config.fields.len(), 1);
    s.discard(CloseRoute::Cancel);
    assert!(s.snapshot_for_save().is_err());
}

#[test]
fn late_custom_result_does_not_enter_a_reopened_session() {
    let mut old = session();
    let origin = old.id();
    old.discard(CloseRoute::Cancel);
    let mut next = session();
    assert_ne!(origin, next.id());
    assert!(!old.accept_custom_result(origin, Vec::new()));
    assert!(!next.accept_custom_result(origin, Vec::new()));
}

#[test]
fn patch_contains_only_dirty_fields_and_never_secrets_or_external_theme() {
    let mut s = session();
    let d = s.drafts_mut().unwrap();
    d.manager.close_to_tray = true;
    d.token.input = "secret".into();
    let save = s.snapshot_for_save().unwrap();
    assert_eq!(save.config.fields.len(), 1);
    assert_eq!(save.config.fields[0].path, ["manager", "closeToTray"]);
    assert!(save.config.fields[0].before.is_none());
    assert!(matches!(save.token, Some(TokenChange::Set(value)) if value == "secret"));
    // A concurrent theme change has no theme patch to overwrite it.
    assert!(save
        .config
        .fields
        .iter()
        .all(|f| !f.path.iter().any(|p| p == "theme")));
}

#[test]
fn scheduler_and_paths_are_separate_effects_and_save_freezes_controls() {
    let mut s = session();
    assert!(s.snapshot_for_save().unwrap().config.fields.is_empty());
    let d = s.drafts_mut().unwrap();
    d.scheduler_enabled = true;
    d.locations.tools = "new-tools".into();
    let save = s.freeze_for_save().unwrap();
    assert!(save.config.fields.is_empty());
    assert_eq!(save.scheduler, Some((false, true)));
    assert_eq!(save.locations.unwrap().1.tools, PathBuf::from("new-tools"));
    assert!(s.drafts_mut().is_none());
    assert!(s.freeze_for_save().is_err());
    s.resume_after_failure();
    assert!(s.drafts_mut().is_some());
}

#[test]
fn custom_registrations_are_atomic_and_unknown_fields_are_preserved() {
    let mut s = session();
    s.base_document["manager"]["future"] = json!({"unknown": true});
    let entry = craft_apps_manager::catalog::all()
        .into_iter()
        .next()
        .unwrap();
    let origin = s.id();
    assert!(s.accept_custom_result(origin, vec![entry]));
    let save = s.snapshot_for_save().unwrap();
    assert_eq!(save.config.fields.len(), 1);
    assert_eq!(save.config.fields[0].path, ["custom_apps"]);
    assert!(save.config.fields[0].after.as_ref().unwrap().is_array());
    assert_eq!(
        s.base_document["manager"]["future"],
        json!({"unknown": true})
    );
}

#[test]
fn frozen_session_rejects_nested_and_custom_results() {
    let mut s = session();
    let id = s.id();
    s.freeze_for_save().unwrap();
    assert!(
        !s.accept_nested_edit(SelectionOrigin::Settings(id), |d| d.manager.close_to_tray =
            true)
    );
    assert!(!s.accept_custom_result(id, Vec::new()));
}

#[test]
fn partial_success_retries_only_failed_effects() {
    let mut s = session();
    let d = s.drafts_mut().unwrap();
    d.manager.close_to_tray = true;
    d.scheduler_enabled = true;
    d.token.input = "secret".into();
    let saved = s.freeze_for_save().unwrap();
    assert_eq!(saved.config.fields.len(), 1);
    s.acknowledge_saved(
        Some(json!({"manager":{"closeToTray":true}})),
        false,
        false,
        true,
    )
    .unwrap();
    s.resume_after_failure();
    let retry = s.snapshot_for_save().unwrap();
    assert!(retry.config.fields.is_empty());
    assert!(retry.token.is_none());
    assert_eq!(retry.scheduler, Some((false, true)));
}

#[test]
fn release_context_changes_are_scoped_to_the_custom_app_definition() {
    let mut s = session();
    s.base_document["custom_apps"] = json!([{"key":"example", "repository":"owner/app"}]);
    let mut changed = s.base_document.clone();
    assert!(!s.custom_app_context_changed(&changed, "example"));
    changed["manager"]["theme"] = json!("dark");
    assert!(!s.custom_app_context_changed(&changed, "example"));
    changed["custom_apps"][0]["repository"] = json!("owner/other");
    assert!(s.custom_app_context_changed(&changed, "example"));
    assert!(!s.custom_app_context_changed(&changed, "another"));
    changed["custom_apps"] = json!([]);
    assert!(s.custom_app_context_changed(&changed, "example"));
}
