use super::*;
use craft_apps_manager::{
    model::Paths,
    storage::config::{ConfigPatch, FieldPatch},
};

pub(super) fn token_verification_fixture() -> (std::path::PathBuf, egui::Context, App) {
    let root = std::env::temp_dir().join(format!("craft-settings-token-{}", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(root.clone());
    let manager = model::Preferences {
        check_catalog_on_startup: false,
        ..Default::default()
    };
    store
        .commit(ConfigPatch {
            fields: vec![FieldPatch {
                path: vec!["manager".into()],
                before: store.load().unwrap().document.get("manager").cloned(),
                after: Some(serde_json::to_value(manager).unwrap()),
            }],
        })
        .unwrap();
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = App::new(&cc, Paths::new(root.clone(), None), root.clone(), false).unwrap();
    let snapshot = store.load().unwrap();
    app.settings_edit = Some(
        edit_session::SettingsEditSession::begin(
            snapshot.document,
            app.preferences.clone(),
            app.build_preferences.clone(),
            Vec::new(),
            edit_session::Locations {
                root: root.clone(),
                tools: app.paths.tools.clone(),
            },
            app.auto,
        )
        .unwrap(),
    );
    app.settings = true;
    app.token_check_started = true;
    (root, ctx, app)
}

pub(super) fn finish_settings_save(app: &mut App, ctx: &egui::Context) {
    let outcome = app
        .settings_save
        .take()
        .unwrap()
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    assert!(outcome.config.error().is_none());
    assert!(matches!(outcome.locations, save::Stage::NotRequested));
    assert!(matches!(outcome.scheduler, save::Stage::NotRequested));
    assert!(matches!(outcome.token, save::Stage::NotRequested));
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(outcome)
        .unwrap_or_else(|_| panic!("save receiver closed"));
    app.settings_save = Some(rx);
    app.poll_settings(ctx);
}

pub(super) fn reopen_settings(app: &mut App, ctx: &egui::Context) {
    app.open_settings();
    let loaded = app
        .settings_load
        .take()
        .unwrap()
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(loaded)
        .unwrap_or_else(|_| panic!("settings receiver closed"));
    app.settings_load = Some(rx);
    app.poll_settings(ctx);
    assert!(app.settings_edit.is_some());
}

#[test]
fn manage_token_navigation_preserves_drafts_and_uses_the_existing_session() {
    let (root, _, mut app) = token_verification_fixture();
    let session = app.settings_edit.as_ref().unwrap().id();
    app.settings_draft.app_check_interval_minutes = 40;
    app.token_draft.input = "fixture-not-a-real-token".into();
    app.open_token_settings();
    assert_eq!(app.settings_edit.as_ref().unwrap().id(), session);
    assert_eq!(app.settings_draft.app_check_interval_minutes, 40);
    assert_eq!(app.token_draft.input, "fixture-not-a-real-token");
    assert!(app.settings_token_entry && app.settings_token_focus);
    app.token_help_open = true;
    app.discard_settings(CloseRoute::Cancel);
    assert!(!app.settings_token_entry && !app.settings_token_focus && !app.token_help_open);
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn builder_token_save_reaches_validation_instead_of_silently_discarding_the_edit() {
    let (root, ctx, mut app) = token_verification_fixture();
    app.builder = true;
    app.open_token_settings();
    // Deliberately invalid input exercises the real Save path but cannot reach
    // the credential store. A discarded edit would incorrectly report success.
    app.token_draft.input = "invalid fixture with spaces".into();
    app.save_settings(&ctx);
    let outcome = app
        .settings_save
        .take()
        .unwrap()
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    assert!(
        outcome.config.error().is_some(),
        "Builder must validate pending token changes"
    );
    assert!(matches!(outcome.token, save::Stage::NotRequested));
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn manage_token_from_builder_loads_settings_then_focuses_existing_editor() {
    let (root, ctx, mut app) = token_verification_fixture();
    app.discard_settings(CloseRoute::Cancel);
    app.builder = true;
    app.open_token_settings();
    assert!(app.settings && app.settings_load.is_some());
    assert!(app.settings_token_entry && app.settings_token_focus);
    let loaded = app
        .settings_load
        .take()
        .unwrap()
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(loaded)
        .unwrap_or_else(|_| panic!("settings receiver closed"));
    app.settings_load = Some(rx);
    app.poll_settings(&ctx);
    let mut editor_visible = false;
    for time in [0.0, 0.2, 0.5, 0.8] {
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 900.0),
                )),
                time: Some(time),
                ..Default::default()
            },
            |ctx| app.settings_ui(ctx),
        );
        editor_visible = output.shapes.iter().any(|shape| match &shape.shape {
            egui::Shape::Text(text) => {
                text.galley.job.text == "Paste your GitHub token"
                    && shape.clip_rect.contains(text.pos)
            }
            _ => false,
        });
    }
    assert!(ctx.memory(|memory| memory.focused()).is_some());
    assert!(
        editor_visible,
        "Manage token must scroll the editor into view in Builder settings"
    );
    assert!(!app.settings_token_focus);
    assert!(app.settings_token_entry);
    assert!(app.settings_edit.is_some());
    app.discard_settings(CloseRoute::Cancel);
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn manage_token_keeps_navigation_pending_during_the_first_modal_sizing_pass() {
    let (root, ctx, mut app) = token_verification_fixture();
    app.open_token_settings();
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let mut sizing = ui.new_child(egui::UiBuilder::new().sizing_pass().invisible());
            assert!(sizing.is_sizing_pass());
            app.github_token_settings(&mut sizing);
        });
    });
    let pending = app.settings_token_focus;
    app.discard_settings(CloseRoute::Cancel);
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
    assert!(
        pending,
        "Invisible first-open layout must not consume token centering"
    );
}

#[test]
fn manage_token_centers_the_whole_group_in_the_settings_viewport() {
    let (root, ctx, mut app) = token_verification_fixture();
    let mut centers = Vec::new();
    for builder in [false, true] {
        app.builder = builder;
        app.open_token_settings();
        let mut group_center = 0.0;
        let mut viewport_center = 0.0;
        for time in [0.0, 0.2, 0.5, 0.8] {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 600.0),
                    )),
                    time: Some(time + if builder { 1.0 } else { 0.0 }),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        // Match Settings' explicit body clip, which excludes the
                        // scroll area's default shadow margin from its viewport.
                        ui.set_clip_rect(egui::Rect::from_min_size(
                            ui.cursor().min,
                            egui::vec2(ui.available_width(), 400.0),
                        ));
                        let scroll = egui::ScrollArea::vertical()
                            .id_salt(("token-centering", builder))
                            .max_height(400.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.add_space(500.0);
                                let top = ui.cursor().top();
                                app.github_token_settings(ui);
                                group_center = (top + ui.cursor().top()) * 0.5;
                                ui.add_space(500.0);
                            });
                        viewport_center = scroll.inner_rect.center().y;
                    });
                },
            );
        }
        centers.push((builder, (group_center - viewport_center).abs()));
    }
    app.discard_settings(CloseRoute::Cancel);
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
    for (builder, offset) in centers {
        assert!(
            offset < 1.0,
            "builder={builder}: token section is {offset}pt off center"
        );
    }
}

#[test]
fn custom_app_add_cancel_save_restart_and_remove_follow_the_same_settings_flow() {
    let (root, ctx, mut app) = token_verification_fixture();
    let entry: catalog::Entry = serde_json::from_value(serde_json::json!({
        "key":"settingslifecyclefixture", "repository":"example/settings-lifecycle-fixture",
        "title":"Settings lifecycle fixture", "release":true, "source":false,
        "scheme":"generic", "assetPrefix":"settingslifecyclefixture"
    }))
    .unwrap();
    app.custom_apps_draft.push(entry.clone());
    app.discard_settings(CloseRoute::Cancel);
    assert!(ConfigStore::new(root.clone())
        .load()
        .unwrap()
        .custom_apps()
        .unwrap()
        .is_empty());
    assert!(catalog::get(&entry.key).is_none());
    reopen_settings(&mut app, &ctx);
    app.custom_apps_draft.push(entry.clone());
    app.save_settings(&ctx);
    finish_settings_save(&mut app, &ctx);
    assert!(catalog::get(&entry.key).is_some());
    assert!(app.preferences.app_order.contains(&entry.key));
    assert!(app.preferences.home_app_order.contains(&entry.key));
    drop(app);
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = App::new(&cc, Paths::new(root.clone(), None), root.clone(), false).unwrap();
    app.select(&entry.key);
    reopen_settings(&mut app, &ctx);
    assert_eq!(app.custom_apps_draft.len(), 1);
    app.custom_apps_draft.clear();
    app.save_settings(&ctx);
    finish_settings_save(&mut app, &ctx);
    assert!(catalog::get(&entry.key).is_none());
    assert!(matches!(app.page, crate::ui::Page::Overview));
    assert!(ConfigStore::new(root.clone())
        .load()
        .unwrap()
        .custom_apps()
        .unwrap()
        .is_empty());
    drop(app);
    std::fs::remove_dir_all(&root).unwrap();
    catalog::load(&root);
}

#[test]
fn conflicting_settings_save_preserves_other_writer_and_can_be_reopened_for_retry() {
    let (root, ctx, mut app) = token_verification_fixture();
    app.settings_draft.app_check_interval_minutes = 40;
    let store = ConfigStore::new(root.clone());
    store
        .commit(ConfigPatch {
            fields: vec![FieldPatch {
                path: vec!["manager".into(), "appCheckIntervalMinutes".into()],
                before: Some(serde_json::json!(20)),
                after: Some(serde_json::json!(30)),
            }],
        })
        .unwrap();
    app.save_settings(&ctx);
    let outcome = app
        .settings_save
        .take()
        .unwrap()
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    assert!(outcome.config.error().is_some());
    assert!(matches!(outcome.locations, save::Stage::NotRequested));
    assert!(matches!(outcome.scheduler, save::Stage::NotRequested));
    assert!(matches!(outcome.token, save::Stage::NotRequested));
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(outcome)
        .unwrap_or_else(|_| panic!("save receiver closed"));
    app.settings_save = Some(rx);
    app.poll_settings(&ctx);
    assert!(app.settings);
    assert!(app.error.is_some());
    assert_eq!(app.settings_draft.app_check_interval_minutes, 40);
    assert_eq!(
        store
            .load()
            .unwrap()
            .manager()
            .unwrap()
            .app_check_interval_minutes,
        30
    );
    app.discard_settings(CloseRoute::Cancel);
    app.error = None;
    reopen_settings(&mut app, &ctx);
    assert_eq!(app.settings_draft.app_check_interval_minutes, 30);
    app.settings_draft.app_check_interval_minutes = 40;
    app.save_settings(&ctx);
    finish_settings_save(&mut app, &ctx);
    assert!(app.error.is_none());
    assert_eq!(
        store
            .load()
            .unwrap()
            .manager()
            .unwrap()
            .app_check_interval_minutes,
        40
    );
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cancelling_settings_discards_ui_drafts_without_changing_saved_files() {
    for route in [
        CloseRoute::Cancel,
        CloseRoute::TitleBar,
        CloseRoute::Escape,
        CloseRoute::Outside,
    ] {
        let (root, _, mut app) = token_verification_fixture();
        let original = std::fs::read(root.join("config.yaml")).unwrap();
        app.settings_draft.theme = model::Theme::Light;
        app.settings_draft.hidden_apps.push("filmcraft".into());
        app.settings_draft.app_check_interval_minutes = 50;
        app.build_draft.log_size_mb = 45;
        app.auto_draft = !app.auto;
        app.root_text = root.join("unsaved-library").display().to_string();
        app.token_draft.input = "unsaved-fixture-token".into();
        app.token_draft.stage();
        app.discard_settings(route);
        assert!(!app.settings);
        assert!(app.settings_edit.is_none());
        assert!(!app.token_draft.has_pending());
        assert_eq!(
            serde_json::to_value(&app.settings_draft).unwrap(),
            serde_json::to_value(&app.preferences).unwrap()
        );
        assert_eq!(
            app.build_draft.log_size_mb,
            app.build_preferences.log_size_mb
        );
        assert_eq!(app.auto_draft, app.auto);
        assert_eq!(std::fs::read(root.join("config.yaml")).unwrap(), original);
        assert!(!root.join("data-root.json").exists());
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn saving_multiple_settings_through_ui_survives_reopening_the_manager() {
    let (root, ctx, mut app) = token_verification_fixture();
    app.settings_draft.theme = model::Theme::Light;
    app.settings_draft.close_to_tray = true;
    app.settings_draft.hide_activity_badge = true;
    app.settings_draft.hidden_apps = vec!["filmcraft".into()];
    app.settings_draft.selected_apps = vec!["filmcraft".into()];
    app.settings_draft.rollback_enabled = Some(false);
    app.settings_draft
        .rollback_overrides
        .insert(uuid::Uuid::new_v4().to_string(), true);
    app.settings_draft.app_check_interval_minutes = 45;
    app.build_draft.log_size_mb = 45;
    app.build_draft.log_archives = 4;
    app.build_draft.delete_cache_after_success = false;
    app.save_settings(&ctx);
    finish_settings_save(&mut app, &ctx);
    assert!(!app.settings);
    assert!(app.error.is_none(), "{:?}", app.error);
    let expected_manager = serde_json::to_value(&app.preferences).unwrap();
    let expected_builder = serde_json::to_value(&app.build_preferences).unwrap();
    drop(app);
    for _ in 0..2 {
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let reopened = App::new(&cc, Paths::new(root.clone(), None), root.clone(), false).unwrap();
        assert_eq!(
            serde_json::to_value(&reopened.preferences).unwrap(),
            expected_manager
        );
        assert_eq!(
            serde_json::to_value(&reopened.build_preferences).unwrap(),
            expected_builder
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn settings_save_merges_another_windows_unrelated_changes() {
    let (root, ctx, mut app) = token_verification_fixture();
    app.settings_draft.app_check_interval_minutes = 40;
    let store = ConfigStore::new(root.clone());
    store
        .commit(ConfigPatch {
            fields: vec![FieldPatch {
                path: vec!["manager".into(), "theme".into()],
                before: Some(serde_json::json!("system")),
                after: Some(serde_json::json!("dark")),
            }],
        })
        .unwrap();
    app.save_settings(&ctx);
    finish_settings_save(&mut app, &ctx);
    assert_eq!(app.preferences.theme, model::Theme::Dark);
    assert_eq!(app.preferences.app_check_interval_minutes, 40);
    assert_eq!(
        store.load().unwrap().manager().unwrap().theme,
        model::Theme::Dark
    );
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_token_save_keeps_current_credential_verification() {
    let (root, ctx, mut app) = token_verification_fixture();
    let (verification_tx, verification_rx) = std::sync::mpsc::channel();
    app.token_receiver = Some(verification_rx);
    // Validation rejects this before any credential or scheduler operation.
    app.token_draft.input = "invalid token with spaces".into();
    app.save_settings(&ctx);
    assert!(
        app.token_receiver.is_some(),
        "starting a save must keep verification of the current credential"
    );
    let outcome = app
        .settings_save
        .take()
        .unwrap()
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert!(outcome.config.error().is_some());
    assert!(matches!(outcome.token, save::Stage::NotRequested));
    let (save_tx, save_rx) = std::sync::mpsc::channel();
    save_tx
        .send(outcome)
        .unwrap_or_else(|_| panic!("save receiver closed"));
    app.settings_save = Some(save_rx);
    app.poll_settings(&ctx);
    app.discard_settings(CloseRoute::Cancel);
    verification_tx.send(TokenStatus::Connected).unwrap();
    assert_eq!(
        app.token_receiver.as_ref().unwrap().try_recv().unwrap(),
        TokenStatus::Connected,
        "a failed save must leave the unchanged credential's result available"
    );
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn successful_token_save_invalidates_old_credential_verification() {
    let (root, ctx, mut app) = token_verification_fixture();
    let (verification_tx, verification_rx) = std::sync::mpsc::channel();
    app.token_receiver = Some(verification_rx);
    let session = app.settings_edit.as_mut().unwrap();
    let session_id = session.freeze_for_save().unwrap().session_id;
    let (save_tx, save_rx) = std::sync::mpsc::channel();
    save_tx
        .send(save::SaveOutcome {
            session_id,
            config: save::Stage::NotRequested,
            locations: save::Stage::NotRequested,
            scheduler: save::Stage::NotRequested,
            token: save::Stage::Applied(TokenStatus::NotVerified),
        })
        .unwrap_or_else(|_| panic!("save receiver closed"));
    app.settings_save = Some(save_rx);
    app.poll_settings(&ctx);
    assert!(app.token_receiver.is_none());
    assert!(verification_tx.send(TokenStatus::Connected).is_err());
    assert!(!app.token_check_started);
    assert!(app.token_saved_verification_pending);
    assert_eq!(app.token_status, TokenStatus::NotVerified);
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn saving_other_settings_keeps_the_existing_app_page() {
    let (root, ctx, mut app) = token_verification_fixture();
    app.select("filmcraft");
    let session = app.settings_edit.as_mut().unwrap();
    let session_id = session.freeze_for_save().unwrap().session_id;
    let snapshot = ConfigStore::new(root.clone()).load().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(save::SaveOutcome {
        session_id,
        config: save::Stage::Applied(snapshot),
        locations: save::Stage::NotRequested,
        scheduler: save::Stage::NotRequested,
        token: save::Stage::NotRequested,
    })
    .unwrap_or_else(|_| panic!("save receiver closed"));
    app.settings_save = Some(rx);
    app.poll_settings(&ctx);
    assert!(matches!(app.page, crate::ui::Page::App));
    assert_eq!(app.app, "filmcraft");
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn saving_removal_of_selected_custom_app_returns_to_home() {
    let root =
        std::env::temp_dir().join(format!("craft-settings-removal-{}", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(root.clone());
    let manager = model::Preferences {
        check_catalog_on_startup: false,
        ..Default::default()
    };
    store.commit(ConfigPatch { fields: vec![
        FieldPatch { path: vec!["manager".into()], before: store.load().unwrap().document.get("manager").cloned(), after: Some(serde_json::to_value(manager).unwrap()) },
        FieldPatch { path: vec!["custom_apps".into()], before: Some(serde_json::json!([])), after: Some(serde_json::json!([{"key":"removedfixture","repository":"example/removedfixture","title":"Removed fixture","release":true,"source":false,"scheme":"generic","assetPrefix":"removedfixture"}])) },
    ] }).unwrap();
    catalog::load(&root);
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let paths = Paths::new(root.clone(), None);
    let mut app = App::new(&cc, paths, root.clone(), false).unwrap();
    app.select("removedfixture");
    let snapshot = store.load().unwrap();
    let mut session = edit_session::SettingsEditSession::begin(
        snapshot.document,
        app.preferences.clone(),
        app.build_preferences.clone(),
        store.load().unwrap().custom_apps().unwrap(),
        edit_session::Locations {
            root: root.clone(),
            tools: app.paths.tools.clone(),
        },
        false,
    )
    .unwrap();
    session.drafts_mut().unwrap().custom_apps.clear();
    let request = save::SaveRequest {
        paths: app.paths.clone(),
        home: root.clone(),
        edit: session.freeze_for_save().unwrap(),
    };
    let saved = save::Backend::config(&mut save::SystemBackend, &request).unwrap();
    assert!(ConfigStore::new(root.clone())
        .load()
        .unwrap()
        .custom_apps()
        .unwrap()
        .is_empty());
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(save::SaveOutcome {
        session_id: session.id(),
        config: save::Stage::Applied(saved),
        locations: save::Stage::NotRequested,
        scheduler: save::Stage::NotRequested,
        token: save::Stage::NotRequested,
    })
    .unwrap_or_else(|_| panic!("save receiver closed"));
    app.settings = true;
    app.settings_edit = Some(session);
    app.settings_save = Some(rx);
    app.poll_settings(&ctx);
    assert!(catalog::get("removedfixture").is_none());
    assert!(
        matches!(app.page, crate::ui::Page::Overview),
        "removed app page must no longer expose actions"
    );
    std::fs::remove_dir_all(&root).unwrap();
    catalog::load(&root);
}

#[test]
fn unrelated_settings_save_preserves_available_updates_and_inflight_checks() {
    let (root, ctx, mut app) = token_verification_fixture();
    let copies = ["printcraft", "photocraft"].map(|name| model::Installed {
        name: name.into(),
        version: "1.0.0".into(),
        path: root.join(name).display().to_string(),
        architecture: "x64".into(),
        install_kind: "portable".into(),
        ..Default::default()
    });
    app.display_config = Some(model::Config {
        apps_root: root.display().to_string(),
        apps: copies.to_vec(),
        installations: copies.to_vec(),
        portable_label_counters: Default::default(),
    });
    for copy in copies {
        app.release_checks
            .insert(copy.name, ("1.0.0".into(), Ok(Some("2.0.0".into()))));
    }
    app.check_generation = 7;
    app.checking_apps.insert("filmcraft".into());
    app.settings_draft.app_check_interval_minutes = 45;
    app.save_settings(&ctx);
    finish_settings_save(&mut app, &ctx);
    assert_eq!(app.status("printcraft").update.as_deref(), Some("2.0.0"));
    assert_eq!(app.status("photocraft").update.as_deref(), Some("2.0.0"));
    assert_eq!(app.check_generation, 7);
    assert!(app.checking_apps.contains("filmcraft"));
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn settings_package_change_invalidates_only_the_affected_app_update() {
    let (root, ctx, mut app) = token_verification_fixture();
    for name in ["printcraft", "photocraft"] {
        app.release_checks
            .insert(name.into(), ("1.0.0".into(), Ok(Some("2.0.0".into()))));
    }
    app.settings_draft.app_install_options.insert(
        craft_apps_manager::install_options::identity("printcraft"),
        craft_apps_manager::install_options::InstallOptions {
            release_format: "portable".into(),
            architecture: "x86".into(),
        },
    );
    app.save_settings(&ctx);
    finish_settings_save(&mut app, &ctx);
    assert!(!app.release_checks.contains_key("printcraft"));
    assert!(
        matches!(app.release_checks.get("photocraft"), Some((_, Ok(Some(version)))) if version == "2.0.0")
    );
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn switching_installations_preserves_each_copy_update_result() {
    let (root, ctx, mut app) = token_verification_fixture();
    let first = model::Installed {
        name: "photocraft".into(),
        version: "1.0.0".into(),
        path: root.join("first").display().to_string(),
        architecture: "x64".into(),
        install_kind: "portable".into(),
        ..Default::default()
    };
    let mut second = first.clone();
    second.path = root.join("second").display().to_string();
    // Same version, different format: a version-only result cannot identify a copy.
    second.install_kind = "installer".into();
    let config = model::Config {
        apps_root: root.display().to_string(),
        apps: vec![first.clone()],
        installations: vec![first.clone(), second.clone()],
        portable_label_counters: Default::default(),
    };
    app.display_config = Some(config.clone());
    app.release_checks.insert(
        first.name.clone(),
        (first.version.clone(), Ok(Some("2.0.0".into()))),
    );
    // Never bind the visible result to a different same-version copy.
    app.remember_installation_check(&ctx, &second);
    let mut next = config.clone();
    next.apps[0] = second.clone();
    app.apply_installation_selection(
        crate::ui::InstallationSelection {
            app: first.name.clone(),
            config: next.clone(),
            preferences: app.preferences.clone(),
            check: None,
        },
        &ctx,
    );
    assert!(app.status(&first.name).update.is_none());
    app.release_checks
        .insert(second.name.clone(), (second.version.clone(), Ok(None)));
    app.apply_installation_selection(
        crate::ui::InstallationSelection {
            app: first.name.clone(),
            config: config.clone(),
            preferences: app.preferences.clone(),
            check: None,
        },
        &ctx,
    );
    assert_eq!(app.status(&first.name).update.as_deref(), Some("2.0.0"));
    app.apply_installation_selection(
        crate::ui::InstallationSelection {
            app: first.name.clone(),
            config: next,
            preferences: app.preferences.clone(),
            check: None,
        },
        &ctx,
    );
    assert!(matches!(app.status(&first.name).check, Some(Ok(None))));
    // A slow result belongs to the captured copy, not the newly selected one.
    app.apply_release_check(
        &ctx,
        app.check_generation,
        first.clone(),
        Ok(Some("3.0.0".into())),
    );
    assert!(matches!(app.status(&first.name).check, Some(Ok(None))));
    app.apply_installation_selection(
        crate::ui::InstallationSelection {
            app: first.name.clone(),
            config: config.clone(),
            preferences: app.preferences.clone(),
            check: None,
        },
        &ctx,
    );
    assert_eq!(app.status(&first.name).update.as_deref(), Some("3.0.0"));
    let mut changed = config;
    changed.apps[0].version = "1.5.0".into();
    app.apply_installation_selection(
        crate::ui::InstallationSelection {
            app: first.name.clone(),
            config: changed,
            preferences: app.preferences.clone(),
            check: None,
        },
        &ctx,
    );
    assert!(app.status(&first.name).check.is_none());
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}
