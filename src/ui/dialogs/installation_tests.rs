use super::*;

#[test]
fn cached_version_list_expands_again_on_each_open() {
    let ctx = egui::Context::default();
    let id = egui::Id::new("cached-version-list");
    let mut heights = Vec::new();
    for (time, opening) in [
        (0.0, true),
        (0.11, false),
        (0.3, false),
        (1.0, true),
        (1.11, false),
        (1.3, false),
    ] {
        let mut height = 0.0;
        let _ = ctx.run(
            egui::RawInput {
                time: Some(time),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600.0, 600.0),
                )),
                ..Default::default()
            },
            |ctx| {
                if opening {
                    begin_version_list_transition(ctx, id, true);
                }
                egui::CentralPanel::default().show(ctx, |ui| {
                    let before = ui.cursor().top();
                    animated_version_list(ui, id, true, |ui| {
                        ui.allocate_space(egui::vec2(260.0, 240.0));
                    });
                    height = ui.cursor().top() - before;
                });
            },
        );
        heights.push(height);
    }
    for start in [0, 3] {
        assert!(
            heights[start] < heights[start + 1] && heights[start + 1] < heights[start + 2],
            "Each cached opening must expand: {heights:?}"
        );
    }
    assert!((heights[2] - heights[5]).abs() < 1.0);
}

#[test]
fn version_list_loading_height_transitions_once_and_settles_without_a_jump() {
    let ctx = egui::Context::default();
    let mut heights = Vec::new();
    for (time, ready) in [
        (0.0, false),
        (0.1, true),
        (0.2, true),
        (0.4, true),
        (0.5, true),
    ] {
        let mut measured = 0.0;
        let _ = ctx.run(
            egui::RawInput {
                time: Some(time),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600.0, 600.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.set_width(278.0);
                    let before = ui.cursor().top();
                    animated_version_list(ui, egui::Id::new("loading-version-list"), ready, |ui| {
                        ui.allocate_space(egui::vec2(260.0, if ready { 240.0 } else { 24.0 }));
                    });
                    measured = ui.cursor().top() - before;
                });
            },
        );
        heights.push(measured);
    }
    assert!(
        (heights[0] - heights[1]).abs() < 1.0,
        "New data must not jump to its final size immediately: {heights:?}"
    );
    assert!(
        heights[2] > heights[1] && heights[2] < heights[3],
        "Height must transition smoothly: {heights:?}"
    );
    assert!(
        (heights[3] - heights[4]).abs() < 1.0,
        "Settled list must stop resizing: {heights:?}"
    );
}

#[test]
fn incompatible_version_is_disabled_but_its_info_button_is_clickable() {
    let ctx = egui::Context::default();
    let mut selected = None;
    let mut clicked = false;
    let mut info_rect = egui::Rect::NOTHING;
    for (time, events) in [
        (0.0, Vec::new()),
        (0.1, Vec::new()),
        (0.2, vec![egui::Event::PointerMoved(egui::pos2(0.0, 0.0))]),
    ] {
        let _=ctx.run(egui::RawInput{time:Some(time),screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(600.0,400.0))),events,..Default::default()},|ctx|{
            egui::CentralPanel::default().show(ctx,|ui|{
                ui.set_width(300.0);
                let (row,info)=install_version_row(ui,&mut selected,"v0.13.0",Some("This version requires Everyone; your selected installation is Just me."),false);
                assert!(!row.clicked());
                let info=info.expect("Disabled version must provide an active explanation button");
                assert!(info.enabled());
                info_rect=info.rect;
            });
        });
    }
    let position = info_rect.center();
    for (time, pressed) in [(0.3, true), (0.4, false)] {
        let _ = ctx.run(
            egui::RawInput {
                time: Some(time),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600.0, 400.0),
                )),
                events: vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.set_width(300.0);
                    let (_, info) = install_version_row(
                        ui,
                        &mut selected,
                        "v0.13.0",
                        Some("Incompatible installation scope"),
                        false,
                    );
                    clicked |= info.unwrap().clicked();
                });
            },
        );
    }
    assert!(
        clicked,
        "Information must remain clickable next to a disabled version"
    );
    assert!(
        selected.is_none(),
        "Clicking information must never select the incompatible version"
    );
}

#[test]
fn version_popup_stays_anchored_and_bounded_after_reopening() {
    for (screen_height, field_top, above) in [
        (900.0, 100.0, false),
        (600.0, 460.0, true),
        (240.0, 95.0, false),
    ] {
        let ctx = egui::Context::default();
        ctx.style_mut(|style| {
            style.animation_time = 0.0;
            style.spacing.combo_height = 300.0;
        });
        let id = egui::Id::new("version-field");
        let popup_id = id.with("popup");
        let mut button = egui::Rect::NOTHING;
        let mut button_id = egui::Id::NULL;
        let mut selected = 0;
        let mut frame = |time, open, events| {
            let _ = ctx.run(
                egui::RawInput {
                    time: Some(time),
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900.0, screen_height),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.set_width(470.0);
                        ui.add_space(field_top);
                        if open {
                            ui.memory_mut(|memory| memory.open_popup(popup_id));
                        }
                        let response = install_version_menu(ui, id, "Latest release", |ui| {
                            for index in 0..20 {
                                ui.selectable_value(
                                    &mut selected,
                                    index,
                                    format!("Release {index}"),
                                );
                            }
                        });
                        button = response.rect;
                        button_id = response.id;
                    });
                },
            );
        };
        frame(0.0, true, Vec::new());
        frame(0.1, false, Vec::new());
        frame(
            0.2,
            false,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(!ctx.memory(|memory| memory.any_popup_open()));
        frame(0.3, true, Vec::new());
        frame(0.4, false, Vec::new());
        let popup = ctx.memory(|memory| memory.area_rect(popup_id)).unwrap();
        assert!(ctx.screen_rect().contains_rect(popup), "{popup:?}");
        assert!(popup.height() <= 320.0, "{popup:?}");
        if above {
            assert!(
                (popup.bottom() - button.top()).abs() < 2.0,
                "{popup:?} {button:?}"
            );
        } else {
            assert!(
                (popup.top() - button.bottom()).abs() < 2.0,
                "{popup:?} {button:?}"
            );
        }
        ctx.memory_mut(|memory| {
            memory.close_popup();
            memory.request_focus(button_id);
        });
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.set_width(470.0);
                    ui.add_space(field_top);
                    install_version_menu(ui, id, "Latest release", |_| {});
                });
            },
        );
        assert!(
            ctx.memory(|memory| memory.is_popup_open(popup_id)),
            "Focused field must open with Enter"
        );
    }
}

#[test]
fn version_dropdown_grows_when_lazy_history_arrives() {
    let root = std::env::temp_dir().join(format!("craft-version-popup-{}", uuid::Uuid::new_v4()));
    let paths = model::Paths::new(root.clone(), None);
    craft_apps_manager::files::write_json(
        &paths.at("updater-settings.json"),
        &model::Preferences {
            check_catalog_on_startup: false,
            check_manager_on_startup: false,
            check_installed_apps_on_startup: false,
            ..Default::default()
        },
    )
    .unwrap();
    let ctx = egui::Context::default();
    ctx.style_mut(|style| {
        style.animation_time = 0.0;
        style.spacing.combo_height = 300.0;
    });
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = App::new(&cc, paths, root.clone(), false).unwrap();
    app.versions_app = Some("filmcraft".into());
    // Model an in-flight history fetch without contacting the hosted catalog.
    // Deliver its result through the same poll path as the real worker.
    let (versions_sender, versions_receiver) = std::sync::mpsc::channel();
    app.versions_receiver = Some(versions_receiver);
    app.token_check_started = true;
    let frame = |app: &mut App, time, events| {
        ctx.run(
            egui::RawInput {
                time: Some(time),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 900.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.set_width(470.0);
                    ui.add_space(300.0);
                    app.install_version_control(ui, "filmcraft", None, ctx);
                });
            },
        )
    };
    let output = frame(&mut app, 0.0, Vec::new());
    let button = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == "Latest release" => {
                Some(text.pos + text.galley.size() / 2.0)
            }
            _ => None,
        })
        .unwrap();
    for (time, pressed) in [(0.1, true), (0.2, false)] {
        frame(
            &mut app,
            time,
            vec![
                egui::Event::PointerMoved(button),
                egui::Event::PointerButton {
                    pos: button,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    frame(&mut app, 0.3, Vec::new());
    assert!(ctx.memory(|memory| memory.any_popup_open()));
    assert!(app.versions_result.is_none());
    assert!(app.versions_receiver.is_some());
    versions_sender
        .send(Ok((0..20)
            .map(|index| model::Release {
                tag_name: format!("v1.0.{index}"),
                name: None,
                draft: false,
                prerelease: false,
                assets: Vec::new(),
            })
            .collect()))
        .unwrap();
    app.poll(&ctx);
    assert!(app.versions_receiver.is_none());
    assert_eq!(
        app.versions_result
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap()
            .len(),
        20
    );
    frame(&mut app, 0.4, Vec::new());
    let output = frame(&mut app, 0.5, Vec::new());
    let visible_versions = output
        .shapes
        .iter()
        .filter(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text.starts_with("v1.0.") => shape
                .clip_rect
                .contains_rect(egui::Rect::from_min_size(text.pos, text.galley.size())),
            _ => false,
        })
        .count();
    assert!(
        visible_versions >= 5,
        "Only {visible_versions} release rows fit after loading"
    );
    assert!(app.versions_receiver.is_none());
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn closed_version_dropdown_does_not_start_a_history_request() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/api-cache-fixtures")
        .join(uuid::Uuid::new_v4().to_string());
    let paths = model::Paths::new(root.clone(), None);
    craft_apps_manager::files::write_json(
        &paths.at("updater-settings.json"),
        &model::Preferences {
            check_catalog_on_startup: false,
            check_manager_on_startup: false,
            check_installed_apps_on_startup: false,
            ..Default::default()
        },
    )
    .unwrap();
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = App::new(&cc, paths, root.clone(), false).unwrap();
    assert!(app.catalog_receiver.is_none());
    assert!(!app.apps_startup_pending);
    assert!(!app.manager_startup_pending);
    assert!(app.token_check_started);
    app.token_check_started = true;
    let _ = ctx.run(Default::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            app.install_version_control(ui, "filmcraft", None, ctx);
        });
    });
    assert!(
        app.versions_receiver.is_none(),
        "Closed version menus must not fetch release history"
    );
    drop(app);
    // No cache preview or history worker was started by this rendering path.
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn pending_downgrade_keeps_its_title_and_verified_package_fields() {
    const CHILD_ROOT: &str = "CRAFT_DOWNGRADE_UI_TEST_ROOT";
    let Some(root) = std::env::var_os(CHILD_ROOT) else {
        // The real dialog owns asynchronous cache previews. Let its process exit
        // before removing the fixture, rather than racing those worker threads.
        let root =
            std::env::temp_dir().join(format!("craft-downgrade-ui-{}", uuid::Uuid::new_v4()));
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "ui::dialogs::installation::tests::pending_downgrade_keeps_its_title_and_verified_package_fields", "--test-threads=1"])
            .env(CHILD_ROOT, &root)
            .status().unwrap();
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        assert!(status.success());
        return;
    };
    let root = std::path::PathBuf::from(root);
    let paths = model::Paths::new(root.clone(), None);
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = App::new(&cc, paths, root.clone(), false).unwrap();
    app.token_check_started = true;
    app.confirm_install = Some("photocraft".into());
    app.install_draft_app = Some("photocraft".into());
    app.install_selected_tag = Some("v1.0.0".into());
    app.versions_app = Some("photocraft".into());
    app.versions_result = Some(Ok(Vec::new()));
    let installed = model::Installed {
        name: "photocraft".into(),
        version: "2.0.0".into(),
        install_kind: "portable".into(),
        architecture: model::MANAGER_ARCH.into(),
        path: root.join("photocraft").display().to_string(),
        ..Default::default()
    };
    app.display_config = Some(model::Config {
        apps_root: root.display().to_string(),
        apps: vec![installed.clone()],
        installations: vec![installed],
        portable_label_counters: Default::default(),
    });
    let mut text = Vec::new();
    for time in [1.0, 1.05, 1.1] {
        let output = ctx.run(
            egui::RawInput {
                time: Some(time),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 900.0),
                )),
                ..Default::default()
            },
            |ctx| app.render_install_dialog(ctx),
        );
        text.extend(output.shapes.iter().filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text.galley.job.text.clone()),
            _ => None,
        }));
    }
    assert!(
        text.iter().any(|text| text == "Downgrade PhotoCraft"),
        "{text:?}"
    );
    assert!(!text.iter().any(|text| text == "Install PhotoCraft"));
    assert!(text
        .iter()
        .any(|text| text == model::architecture_label(model::MANAGER_ARCH)));
    assert!(text
        .iter()
        .any(|text| text.contains("before changing its format or architecture")));
    assert!(
        app.install_result.is_none(),
        "Animation must not fabricate a verified plan"
    );
    drop(app);
}

#[test]
fn managed_note_space_collapses_and_expands_without_leaving_a_gap() {
    let ctx = egui::Context::default();
    let frame = |time, visible| {
        let mut height = 0.0;
        let _ = ctx.run(
            egui::RawInput {
                time: Some(time),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let top = ui.cursor().top();
                    managed_location_note(ui, egui::Id::new("managed-note"), visible, false);
                    height = ui.cursor().top() - top;
                });
            },
        );
        height
    };
    let full = frame(0.0, true);
    assert!(full > 6.0);
    frame(0.1, false);
    let middle = frame(0.3, false);
    assert!(middle > 0.0 && middle < full);
    assert_eq!(frame(0.6, false), 0.0);
    frame(0.7, true);
    assert!((frame(1.1, true) - full).abs() < 1.0);
}

#[test]
fn release_generation_changes_refresh_once_without_mixing_app_dialogs() {
    let ctx = egui::Context::default();
    let id = egui::Id::new("first-release");
    assert!(!release_generation_changed(&ctx, id, None));
    assert!(!release_generation_changed(&ctx, id, Some("old".into())));
    assert!(!release_generation_changed(&ctx, id, Some("old".into())));
    assert!(release_generation_changed(&ctx, id, Some("new".into())));
    assert!(!release_generation_changed(&ctx, id, Some("new".into())));
    assert!(!release_generation_changed(
        &ctx,
        egui::Id::new("other-release"),
        Some("new".into())
    ));
}

fn plan(app: &str, tag: Option<&str>, additional: bool) -> updates::ReleasePlan {
    serde_json::from_value(serde_json::json!({
        "additional_installation": additional,
        "entries": [{
            "app": app, "action": "Install", "version": "1.0.0",
            "asset": null, "destination": null, "error": null,
            "preferences": model::Preferences::default(),
        }],
        "preferences": model::Preferences::default(),
        "requested_tag": tag, "snapshot": [], "desktop_shortcut": false,
        "installer_destination": null,
    }))
    .unwrap()
}

#[test]
fn old_version_plan_cannot_enable_install_for_new_version_or_latest() {
    let prefs = model::Preferences::default();
    let old = plan("filmcraft", Some("v1.0.0"), false);
    assert!(install_plan_matches_selection(
        &old,
        "filmcraft",
        Some("v1.0.0"),
        &prefs,
        false
    ));
    assert!(!install_plan_matches_selection(
        &old,
        "filmcraft",
        Some("v2.0.0"),
        &prefs,
        false
    ));
    assert!(!install_plan_matches_selection(
        &old,
        "filmcraft",
        None,
        &prefs,
        false
    ));
    let latest = plan("filmcraft", None, false);
    assert!(!install_plan_matches_selection(
        &latest,
        "filmcraft",
        Some("v2.0.0"),
        &prefs,
        false
    ));
}

#[test]
fn preview_must_match_app_copy_mode_and_package_options() {
    let mut prefs = model::Preferences::default();
    let current = plan("filmcraft", None, false);
    assert!(!install_plan_matches_selection(
        &current,
        "photocraft",
        None,
        &prefs,
        false
    ));
    assert!(!install_plan_matches_selection(
        &current,
        "filmcraft",
        None,
        &prefs,
        true
    ));
    prefs.release_format = if prefs.release_format == "portable" {
        "installer".into()
    } else {
        "portable".into()
    };
    assert!(!install_plan_matches_selection(
        &current,
        "filmcraft",
        None,
        &prefs,
        false
    ));
}

#[test]
fn package_change_resets_destination_once_without_restarting_dialog_draft() {
    let paths = model::Paths::new(std::env::temp_dir().join("install-dialog-unit-test"), None);
    let mut prefs = model::Preferences {
        release_format: "installer".into(),
        architecture: "x64".into(),
        ..Default::default()
    };
    let mut context = None;
    let mut parent = String::new();
    let next = |prefs: &model::Preferences| {
        (
            "filmcraft".into(),
            prefs.release_format.clone(),
            prefs.architecture.clone(),
            false,
        )
    };
    assert!(initialize_install_location(
        &mut context,
        &mut parent,
        next(&prefs),
        &paths,
        None,
        &prefs,
        true
    ));
    prefs.release_format = "portable".into();
    assert!(initialize_install_location(
        &mut context,
        &mut parent,
        next(&prefs),
        &paths,
        None,
        &prefs,
        false
    ));
    assert_eq!(parent, paths.at("releases").display().to_string());
    parent = "user-selected-folder".into();
    assert!(!initialize_install_location(
        &mut context,
        &mut parent,
        next(&prefs),
        &paths,
        None,
        &prefs,
        false
    ));
    assert_eq!(parent, "user-selected-folder");
    prefs.release_format = "installer".into();
    assert!(initialize_install_location(
        &mut context,
        &mut parent,
        next(&prefs),
        &paths,
        None,
        &prefs,
        false
    ));
    assert_eq!(parent, default_install_parent(&paths, None, &prefs, false));
    prefs.architecture = "x86".into();
    assert!(initialize_install_location(
        &mut context,
        &mut parent,
        next(&prefs),
        &paths,
        None,
        &prefs,
        false
    ));
    assert_eq!(parent, default_install_parent(&paths, None, &prefs, false));
}

#[cfg(target_os = "windows")]
#[test]
fn uninspected_windows_installer_does_not_promise_a_setup_choice() {
    assert_eq!(native_install_location(None), "Set by installer");
    let portable = model::Installed {
        install_kind: "portable".into(),
        path: "C:/Portable/FilmCraft".into(),
        ..Default::default()
    };
    assert_eq!(native_install_location(Some(&portable)), "Set by installer");
    let legacy = model::Installed {
        install_kind: "installer".into(),
        ..Default::default()
    };
    assert_eq!(native_install_location(Some(&legacy)), "Set by installer");
}

#[test]
fn native_maintenance_keeps_the_verified_install_location() {
    let installed = model::Installed {
        install_kind: "installer".into(),
        path: "verified-existing-app-folder".into(),
        ..Default::default()
    };
    assert_eq!(native_install_location(Some(&installed)), installed.path);
}

#[test]
fn incompatible_version_row_cannot_be_selected_but_compatible_row_can() {
    for reason in [None, Some("Requires Everyone")] {
        let ctx = egui::Context::default();
        let mut selected = Some("v2.0.0".to_owned());
        let mut target = egui::Pos2::ZERO;
        for time in [0.0, 0.1] {
            let _ = ctx.run(
                egui::RawInput {
                    time: Some(time),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        target = install_version_row(ui, &mut selected, "v1.0.0", reason, false)
                            .0
                            .rect
                            .center();
                    });
                },
            );
        }
        let events = vec![
            egui::Event::PointerMoved(target),
            egui::Event::PointerButton {
                pos: target,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos: target,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ];
        let _ = ctx.run(
            egui::RawInput {
                time: Some(0.2),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    install_version_row(ui, &mut selected, "v1.0.0", reason, false);
                });
            },
        );
        assert_eq!(
            selected.as_deref(),
            Some(if reason.is_some() { "v2.0.0" } else { "v1.0.0" })
        );
    }
}

#[test]
fn refreshing_install_details_does_not_fade_the_entire_dialog() {
    const CHILD_ROOT: &str = "CRAFT_STABLE_INSTALL_UI_ROOT";
    let Some(root) = std::env::var_os(CHILD_ROOT) else {
        let root =
            std::env::temp_dir().join(format!("craft-install-refresh-{}", uuid::Uuid::new_v4()));
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "ui::dialogs::installation::tests::refreshing_install_details_does_not_fade_the_entire_dialog", "--test-threads=1"])
            .env(CHILD_ROOT, &root).status().unwrap();
        if root.exists() {
            std::fs::remove_dir_all(root).unwrap();
        }
        assert!(status.success());
        return;
    };
    let root = std::path::PathBuf::from(root);
    let paths = model::Paths::new(root.clone(), None);
    craft_apps_manager::files::write_json(
        &paths.at("updater-settings.json"),
        &model::Preferences {
            check_catalog_on_startup: false,
            check_manager_on_startup: false,
            check_installed_apps_on_startup: false,
            ..Default::default()
        },
    )
    .unwrap();
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = App::new(&cc, paths, root.clone(), false).unwrap();
    app.confirm_install = Some("filmcraft".into());
    app.install_draft_app = Some("filmcraft".into());
    app.install_result = Some(Ok(plan("filmcraft", None, false)));
    app.versions_app = Some("filmcraft".into());
    app.versions_result = Some(Ok(Vec::new()));
    let frame = |app: &mut App, time| {
        ctx.run(
            egui::RawInput {
                time: Some(time),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 1000.0),
                )),
                ..Default::default()
            },
            |ctx| app.render_install_dialog(ctx),
        )
    };
    frame(&mut app, 0.0);
    frame(&mut app, 0.3);
    if let Some(Ok(plan)) = &mut app.install_result {
        plan.entries[0].version = "2.0.0".into();
    }
    let output = frame(&mut app, 0.4);
    let opacity = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == "Install FilmCraft" => {
                Some(text.fallback_color.a())
            }
            _ => None,
        })
        .expect("The dialog title must stay visible");
    assert_eq!(
        opacity, 255,
        "Changing release metadata must not fade unchanged dialog controls"
    );
}

#[test]
fn version_selector_stays_usable_while_an_install_preview_refreshes() {
    let root = std::env::temp_dir().join(format!("craft-version-refresh-{}", uuid::Uuid::new_v4()));
    let paths = model::Paths::new(root.clone(), None);
    craft_apps_manager::files::write_json(
        &paths.at("updater-settings.json"),
        &model::Preferences {
            check_catalog_on_startup: false,
            check_manager_on_startup: false,
            check_installed_apps_on_startup: false,
            ..Default::default()
        },
    )
    .unwrap();
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = App::new(&cc, paths, root.clone(), false).unwrap();
    app.versions_app = Some("filmcraft".into());
    app.versions_result = Some(Ok(Vec::new()));
    let (history_sender, history_receiver) = std::sync::mpsc::channel();
    app.versions_receiver = Some(history_receiver);
    app.load_versions_for("filmcraft", &ctx);
    assert!(app
        .versions_result
        .as_ref()
        .is_some_and(|result| result.is_ok()));
    history_sender
        .send(Ok(Vec::new()))
        .expect("Reopening must reuse the existing history request");
    assert!(app
        .versions_receiver
        .take()
        .unwrap()
        .try_recv()
        .unwrap()
        .is_ok());
    let (_sender, receiver) = std::sync::mpsc::channel();
    app.install_receiver = Some(receiver);
    let mut button = egui::Pos2::ZERO;
    for time in [0.0, 0.1] {
        let output = ctx.run(
            egui::RawInput {
                time: Some(time),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    app.install_version_control(ui, "filmcraft", None, ctx);
                });
            },
        );
        button = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == "Latest release" => {
                    Some(text.pos + text.galley.size() / 2.0)
                }
                _ => None,
            })
            .unwrap();
    }
    for (time, pressed) in [(0.2, true), (0.3, false)] {
        let _ = ctx.run(
            egui::RawInput {
                time: Some(time),
                events: vec![
                    egui::Event::PointerMoved(button),
                    egui::Event::PointerButton {
                        pos: button,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    app.install_version_control(ui, "filmcraft", None, ctx);
                });
            },
        );
    }
    assert!(
        ctx.memory(|memory| memory.any_popup_open()),
        "A background preview must not disable the version selector"
    );
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}
