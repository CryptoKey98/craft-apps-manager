use super::tests::{finish_settings_save, reopen_settings, token_verification_fixture};
use super::*;
use eframe::App as _;

fn tick(app: &mut App, ctx: &egui::Context, close: bool) -> Vec<egui::ViewportCommand> {
    let mut input = egui::RawInput::default();
    if close {
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .events
            .push(egui::ViewportEvent::Close);
    }
    let output = ctx.run(input, |ctx| {
        app.update(ctx, &mut eframe::Frame::_new_kittest())
    });
    output.viewport_output[&egui::ViewportId::ROOT]
        .commands
        .clone()
}

fn no_background_checks(app: &mut App) {
    app.apps_startup_pending = false;
    app.manager_startup_pending = false;
    app.token_check_started = true;
    app.tray_notice_shown = true;
}

#[test]
fn saved_tray_recreation_hide_open_and_exit_work_with_builder_open_or_closed() {
    for builder_open in [false, true] {
        let (root, ctx, mut app) = token_verification_fixture();
        no_background_checks(&mut app);
        app.discard_settings(edit_session::CloseRoute::Cancel);
        let builder_ctx = egui::Context::default();
        let mut builder = builder_open.then(|| {
            let cc = eframe::CreationContext::_new_kittest(builder_ctx.clone());
            let mut window = App::new(&cc, app.paths.clone(), root.clone(), true).unwrap();
            no_background_checks(&mut window);
            window
        });
        for _ in 0..3 {
            for enabled in [false, true] {
                reopen_settings(&mut app, &ctx);
                app.settings_draft.close_to_tray = enabled;
                app.save_settings(&ctx);
                finish_settings_save(&mut app, &ctx);
                tick(&mut app, &ctx, false);
                assert_eq!(app.tray.is_some(), enabled);
                assert_eq!(
                    ConfigStore::new(root.clone())
                        .load()
                        .unwrap()
                        .manager()
                        .unwrap()
                        .close_to_tray,
                    enabled
                );
            }
            let commands = tick(&mut app, &ctx, true);
            assert!(app.tray_hidden);
            assert!(commands.contains(&egui::ViewportCommand::CancelClose));
            assert!(commands.contains(&egui::ViewportCommand::Visible(false)));
            assert!(!commands.contains(&egui::ViewportCommand::Close));
            app.tray.as_ref().unwrap().send_native_menu_command(0);
            let commands = tick(&mut app, &ctx, false);
            assert!(!app.tray_hidden);
            assert!(commands.contains(&egui::ViewportCommand::Visible(true)));
            assert!(commands.contains(&egui::ViewportCommand::Focus));
            assert!(!app.closing);
        }
        // Exit from the hidden window must request termination, not hide again.
        tick(&mut app, &ctx, true);
        app.tray.as_ref().unwrap().send_native_menu_command(3);
        let commands = tick(&mut app, &ctx, false);
        assert!(app.closing);
        assert!(commands.contains(&egui::ViewportCommand::Close));
        assert!(app.job.cancel.load(std::sync::atomic::Ordering::Relaxed));
        assert!(app
            .build_job
            .cancel
            .load(std::sync::atomic::Ordering::Relaxed));
        if let Some(builder) = builder.as_mut() {
            assert!(!builder.closing);
            assert!(!builder
                .job
                .cancel
                .load(std::sync::atomic::Ordering::Relaxed));
            assert!(!tick(builder, &builder_ctx, false).contains(&egui::ViewportCommand::Close));
            assert!(builder.tray.is_none());
        }
        drop(builder);
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn tray_exit_requests_cancellation_and_waits_for_active_job_completion() {
    let (root, ctx, mut app) = token_verification_fixture();
    no_background_checks(&mut app);
    app.discard_settings(edit_session::CloseRoute::Cancel);
    app.preferences.close_to_tray = true;
    tick(&mut app, &ctx, false);
    app.job.state.lock().unwrap().busy = true;
    app.tray.as_ref().unwrap().send_native_menu_command(3);
    let commands = tick(&mut app, &ctx, false);
    assert!(app.closing);
    assert!(app.job.cancel.load(std::sync::atomic::Ordering::Relaxed));
    assert!(!commands.contains(&egui::ViewportCommand::Close));
    app.job.state.lock().unwrap().busy = false;
    assert!(tick(&mut app, &ctx, false).contains(&egui::ViewportCommand::Close));
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn tray_exit_waits_for_pending_settings_save_before_closing() {
    let (root, ctx, mut app) = token_verification_fixture();
    no_background_checks(&mut app);
    app.preferences.close_to_tray = true;
    tick(&mut app, &ctx, false);
    let session_id = app
        .settings_edit
        .as_mut()
        .unwrap()
        .freeze_for_save()
        .unwrap()
        .session_id;
    let (tx, rx) = std::sync::mpsc::channel();
    app.settings_save = Some(rx);
    app.tray.as_ref().unwrap().send_native_menu_command(3);
    assert!(!tick(&mut app, &ctx, false).contains(&egui::ViewportCommand::Close));
    assert!(app.closing);
    assert!(app.settings_save.is_some());
    // Completion follows the same frame-thread publication path as the worker.
    tx.send(save::SaveOutcome {
        session_id,
        config: save::Stage::Applied(ConfigStore::new(root.clone()).load().unwrap()),
        locations: save::Stage::NotRequested,
        scheduler: save::Stage::NotRequested,
        token: save::Stage::NotRequested,
    })
    .unwrap_or_else(|_| panic!("Save receiver closed before completion"));
    let commands = tick(&mut app, &ctx, false);
    assert!(app.settings_save.is_none());
    assert!(commands.contains(&egui::ViewportCommand::Close));
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}
