use super::*;
#[path = "rollback_layout.rs"]
mod layout;
use craft_apps_manager::installations::rollbacks::{self, RollbackPoint, RollbackStatus};
use std::sync::mpsc::{self, Receiver};

type StatusResult = Result<RollbackStatus, (String, Option<RollbackPoint>)>;

enum Pending {
    Status(Receiver<StatusResult>),
    Save(Receiver<Result<(), String>>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Confirmation {
    Restore,
    Remove,
}

/// A draft is intentionally separate from the retained-version action.
#[derive(Default)]
struct Draft {
    original: Option<bool>,
    value: Option<bool>,
    confirmation: Option<Confirmation>,
}

#[derive(Default)]
struct ViewTransition {
    view: Option<Confirmation>,
    started: Option<f64>,
}

impl ViewTransition {
    fn opacity(&mut self, view: Option<Confirmation>, now: f64) -> f32 {
        if self.view != view {
            self.view = view;
            self.started = Some(now);
        }
        let t = self
            .started
            .map_or(1.0, |start| ((now - start) / 0.16).clamp(0.0, 1.0) as f32);
        t * t * (3.0 - 2.0 * t)
    }
}

impl Draft {
    fn loaded(preference: Option<bool>) -> Self {
        Self {
            original: preference,
            value: preference,
            confirmation: None,
        }
    }

    fn changed(&self) -> bool {
        self.value != self.original
    }
}

pub(in crate::ui) struct RollbackDialog {
    installed: model::Installed,
    status: Option<RollbackStatus>,
    draft: Draft,
    pending: Option<Pending>,
    error: Option<String>,
    transition: ViewTransition,
    loaded_at: Option<f64>,
    failed_point: Option<RollbackPoint>,
}

pub(in crate::ui) fn same_installation(a: &model::Installed, b: &model::Installed) -> bool {
    a.name == b.name
        && a.path == b.path
        && a.version == b.version
        && a.architecture == b.architecture
        && a.install_kind == b.install_kind
        && a.package_type == b.package_type
        && a.installation_label == b.installation_label
}

impl RollbackDialog {
    pub(in crate::ui) fn save_pending(&self) -> bool {
        matches!(self.pending, Some(Pending::Save(_)))
    }

    fn load(&mut self, paths: model::Paths, ctx: &egui::Context) {
        let installed = self.installed.clone();
        let ctx = ctx.clone();
        let (tx, rx) = mpsc::channel();
        self.pending = Some(Pending::Status(rx));
        self.error = None;
        self.status = None;
        self.loaded_at = None;
        self.failed_point = None;
        std::thread::spawn(move || {
            let result = rollbacks::status(&paths, &installed).map_err(|e| {
                (
                    format!("{e:#}"),
                    rollbacks::removal_candidate(&paths, &installed)
                        .ok()
                        .flatten(),
                )
            });
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }
}

impl App {
    pub(in crate::ui) fn open_rollback_dialog(
        &mut self,
        installed: model::Installed,
        ctx: &egui::Context,
    ) {
        let mut dialog = RollbackDialog {
            installed,
            status: None,
            draft: Draft::default(),
            pending: None,
            error: None,
            transition: ViewTransition::default(),
            loaded_at: None,
            failed_point: None,
        };
        dialog.load(self.paths.clone(), ctx);
        self.rollback_dialog = Some(dialog);
    }

    pub(super) fn render_rollback_dialog(&mut self, ctx: &egui::Context, busy: bool) {
        let Some(mut dialog) = self.rollback_dialog.take() else {
            return;
        };
        let completed = match &dialog.pending {
            Some(Pending::Status(rx)) => crate::ui::poll_worker_result(rx, || {
                ("Rollback status worker stopped unexpectedly.".into(), None)
            })
            .map(|result| {
                match result {
                    Ok(status) => {
                        dialog.loaded_at = Some(ctx.input(|input| input.time));
                        dialog.draft = Draft::loaded(status.preference);
                        self.display_rollbacks
                            .insert(dialog.installed.name.clone(), Ok(status.clone()));
                        dialog.status = Some(status);
                    }
                    Err((error, candidate)) => {
                        dialog.error = Some(error);
                        dialog.failed_point = candidate;
                    }
                }
                false
            }),
            Some(Pending::Save(rx)) => crate::ui::poll_worker_result(rx, || {
                "Rollback settings worker stopped unexpectedly.".into()
            })
            .map(|result| {
                match result {
                    Ok(()) => {
                        // Refresh the settings snapshot rather than overwriting unrelated drafts.
                        self.config_refresh_at = std::time::Instant::now();
                        true
                    }
                    Err(error) => {
                        dialog.error = Some(error);
                        false
                    }
                }
            }),
            None => None,
        };
        if let Some(close) = completed {
            dialog.pending = None;
            if close {
                return;
            }
        }
        let selected = self.status(&dialog.installed.name).installed;
        let stale = !selected
            .as_ref()
            .is_some_and(|selected| same_installation(selected, &dialog.installed));
        let opacity = dialog
            .transition
            .opacity(dialog.draft.confirmation, ctx.input(|input| input.time));
        let transitioning = opacity < 1.0;
        let locked = busy
            || dialog.pending.is_some()
            || self.settings_save.is_some()
            || stale
            || transitioning;
        let saving = dialog.save_pending();
        let checking = matches!(dialog.pending, Some(Pending::Status(_)));
        let details_opacity = if dialog.status.is_none() {
            0.0
        } else {
            dialog.loaded_at.map_or(1.0, |start| {
                ((ctx.input(|input| input.time) - start) / 0.18).clamp(0.0, 1.0) as f32
            })
        };
        if dialog.status.is_some() && details_opacity < 1.0 {
            ctx.request_repaint();
        }
        let mut close = false;
        let mut save = false;
        let mut retry = false;
        let mut execute: Option<(Confirmation, RollbackPoint)> = None;
        let confirmation_view = dialog.draft.confirmation.is_some();
        let response = modal(ctx, "Rollback", 510.0, |ui| {
            layout::animated_view(
                ui,
                egui::Id::new("rollback-view-size"),
                confirmation_view,
                |ui| {
                    if transitioning {
                        ui.multiply_opacity(opacity);
                        ui.disable();
                        ctx.request_repaint();
                    }
                    if let Some(confirmation) = dialog.draft.confirmation {
                        title_bar(
                            ui,
                            if confirmation == Confirmation::Restore {
                                "Confirm rollback?"
                            } else {
                                "Remove previous version?"
                            },
                            |_| {},
                        );
                        band(ui, egui::Margin::symmetric(22, 20), 0.0, |ui| {
                            egui::ScrollArea::vertical().max_height((ctx.screen_rect().height() - 230.0).clamp(100.0, 400.0)).show(ui, |ui| {
                        if let Some(point) = dialog.status.as_ref().and_then(|status| status.previous.as_ref()).or(dialog.failed_point.as_ref()) {
                            paragraph(ui, &if confirmation == Confirmation::Restore {
                                format!("Replace {} with the retained {} version of {}.", version_label(&dialog.installed.version), version_label(&point.version), model::title(&dialog.installed.name))
                            } else if dialog.failed_point.is_some() {
                                format!("Delete the saved {} rollback files? Your current installation stays unchanged.", version_label(&point.version))
                            } else {
                                format!("Delete the retained {} copy and free {:.1} MB. Your current installation stays unchanged.", version_label(&point.version), point.bytes as f64 / 1_048_576.0)
                            });
                            ui.add_space(16.0);
                            note(ui, if confirmation == Confirmation::Restore {
                                "Close the app before continuing. Older versions may not understand settings or data changed by a newer version."
                            } else {
                                "Local rollback will no longer be available for this installation."
                            });
                            if confirmation == Confirmation::Restore && dialog.installed.install_kind == "installer" {
                                note(ui, "The verified installer will restore this version. Administrator approval may be required.");
                            }
                            note(ui, "Unsaved retention changes will not be applied.");
                        }
                        if stale { note(ui, "The selected installation changed. Cancel and reopen Rollback."); }
                    });
                        });
                        layout::footer(ui, |ui| {
                            if layout::action(
                                ui,
                                "Confirm",
                                if confirmation == Confirmation::Remove {
                                    Kind::Danger
                                } else {
                                    Kind::Primary
                                },
                                !locked,
                            )
                            .clicked()
                            {
                                if let Some(point) = dialog
                                    .status
                                    .as_ref()
                                    .and_then(|status| status.previous.clone())
                                    .or_else(|| {
                                        dialog
                                            .failed_point
                                            .clone()
                                            .filter(|_| confirmation == Confirmation::Remove)
                                    })
                                {
                                    execute = Some((confirmation, point));
                                }
                            }
                            if layout::action(ui, "Cancel", Kind::Secondary, true).clicked() {
                                dialog.draft.confirmation = None;
                            }
                        });
                        return;
                    }

                    let subtitle = format!(
                        "{} · {} · {}",
                        model::title(&dialog.installed.name),
                        if dialog.installed.install_kind == "portable" {
                            "Portable"
                        } else {
                            "Installed app"
                        },
                        dialog.installed.architecture
                    );
                    if layout::header(ui, &subtitle, !saving) {
                        close = true;
                    }
                    band(ui, egui::Margin::symmetric(22, 22), 0.0, |ui| {
                        egui::ScrollArea::vertical().max_height((ctx.screen_rect().height() - 230.0).clamp(100.0, 480.0)).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 21.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        theme::text(ui, "Current version", 12.0, theme::palette().muted);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            theme::text(ui, if checking { String::new() } else { version_label(&dialog.installed.version) }, 14.0, theme::palette().text.gamma_multiply(details_opacity));
                        });
                    });
                    if stale { note(ui, "The selected installation changed. Cancel and reopen Rollback for the current copy."); }
                    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 147.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    if let Some(error) = &dialog.error {
                        if dialog.status.is_none() {
                            ui.add_space(18.0);
                            layout::previous_card(ui, "", 0, false, layout::CardStatus::Failed, 0.0);
                            ui.add_space(8.0);
                        }
                        ui.label(RichText::new(error).color(theme::palette().red_text));
                        if !saving { ui.horizontal(|ui| {
                            if action(ui, "Retry", Kind::Secondary, !locked).clicked() { retry = true; }
                            if dialog.failed_point.is_some() && action(ui, "Delete", Kind::Danger, !locked).clicked() { dialog.draft.confirmation = Some(Confirmation::Remove); }
                        }); }
                    }
                    if checking {
                        ui.add_space(18.0);
                        layout::previous_card(ui, "", 0, false, layout::CardStatus::Checking, 0.0);
                        ui.add_space(8.0);
                        layout::retained_row(ui, false, 0.0);
                    }
                    if let Some(status) = &dialog.status {
                        if let Some(reason) = &status.unsupported {
                            note(ui, format!("{reason} Choose an older version using the app’s version selector."));
                        } else if let Some(point) = &status.previous {
                            ui.add_space(18.0);
                            let card = layout::previous_card(ui, &version_label(&point.version), point.bytes, !locked, layout::CardStatus::Ready, details_opacity);
                            if card.clicked() { dialog.draft.confirmation = Some(Confirmation::Restore); }
                            ui.add_space(8.0);
                            if layout::retained_row(ui, !locked, details_opacity) { dialog.draft.confirmation = Some(Confirmation::Remove); }
                        } else {
                            note(ui, "No previous version retained for this installation. A successful supported update can retain one.");
                        }
                    }
                    });
                        let supported = dialog.status.as_ref().is_some_and(|status| status.unsupported.is_none());
                        ui.add_space(20.0);
                        rule(ui);
                        ui.add_space(20.0);
                        ui.scope(|ui| {
                        if dialog.status.is_none() { ui.multiply_opacity(0.45); }
                        theme::heading(ui, "Keep a rollback version", 14.0);
                        ui.add_space(5.0);
                        layout::note(ui, "For this installation");
                        ui.add_space(10.0);
                        ui.add_enabled_ui(!locked && supported, |ui| {
                            ui.spacing_mut().button_padding = egui::vec2(10.0, 8.0);
                            egui::ComboBox::from_id_salt("rollback-retention")
                                .width(ui.available_width())
                                .selected_text(RichText::new(dialog.status.as_ref().map_or(" ".into(), |status| preference_label(dialog.draft.value, status.global_enabled))).color(theme::palette().text.gamma_multiply(details_opacity)))
                                .show_ui(ui, |ui| {
                                    for value in [None, Some(true), Some(false)] {
                                        ui.selectable_value(&mut dialog.draft.value, value, preference_label(value, dialog.status.as_ref().is_some_and(|status| status.global_enabled)));
                                    }
                                });
                        });
                        });
                        if supported || dialog.status.is_none() {
                            ui.add_space(26.0);
                            ui.add_space(16.0);
                            layout::note(ui, "Changes the app version. Older versions may not support settings or data changed by a newer version.");
                        }

                    if saving { ui.spinner(); note(ui, "Saving rollback setting…"); }
                });
                    });
                    layout::footer(ui, |ui| {
                        if layout::action(
                            ui,
                            "Save",
                            Kind::Primary,
                            !locked
                                && dialog
                                    .status
                                    .as_ref()
                                    .is_some_and(|status| status.unsupported.is_none())
                                && dialog.draft.changed()
                                && dialog.draft.confirmation.is_none(),
                        )
                        .clicked()
                        {
                            save = true;
                        }
                        if layout::action(ui, "Cancel", Kind::Secondary, !saving).clicked() {
                            close = true;
                        }
                    });
                },
            );
        });
        if response.should_close() && !saving {
            if confirmation_view {
                dialog.draft.confirmation = None;
            } else {
                close = true;
            }
        }
        if close {
            return;
        }
        if retry {
            dialog.load(self.paths.clone(), ctx);
        }
        if save {
            let paths = self.paths.clone();
            let installed = dialog.installed.clone();
            let original = dialog.draft.original;
            let preference = dialog.draft.value;
            let (tx, rx) = mpsc::channel();
            dialog.pending = Some(Pending::Save(rx));
            dialog.error = None;
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(
                    rollbacks::save_override(&paths, &installed, original, preference)
                        .map_err(|e| format!("{e:#}")),
                );
                ctx.request_repaint();
            });
        }
        if let Some((confirmation, point)) = execute {
            let installed = dialog.installed.clone();
            let paths = self.paths.clone();
            self.release_checks.remove(&installed.name);
            self.job = Job::interactive(paths.at("logs/updates.log"), &self.build_preferences);
            self.job_target = Some((
                installed.name.clone(),
                if confirmation == Confirmation::Restore {
                    "rollback"
                } else {
                    "remove-rollback"
                }
                .into(),
            ));
            self.job_action.clear();
            self.failure_dismissed = false;
            self.operation_was_busy = true;
            self.job.describe(
                if confirmation == Confirmation::Restore {
                    "Rollback"
                } else {
                    "Remove rollback version"
                },
                &installed.name,
            );
            self.job.spawn(move |job| {
                if confirmation == Confirmation::Restore {
                    rollbacks::rollback(&paths, &installed, &point, &job)
                } else {
                    rollbacks::remove_previous(&paths, &installed, &point)
                }
            });
            self.display_rollbacks.remove(&dialog.installed.name);
            return;
        }
        self.rollback_dialog = Some(dialog);
    }
}

pub(in crate::ui) fn version_label(version: &str) -> String {
    format!("v{}", version.trim_start_matches(['v', 'V']))
}

fn preference_label(value: Option<bool>, global: bool) -> String {
    match value {
        None => format!(
            "Use global setting · {}",
            if global { "Enabled" } else { "Disabled" }
        ),
        Some(true) => "Enabled".into(),
        Some(false) => "Disabled".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirmation_transition_fades_in_both_directions_and_settles() {
        let mut transition = ViewTransition::default();
        assert_eq!(transition.opacity(None, 0.0), 1.0);
        assert_eq!(transition.opacity(Some(Confirmation::Restore), 1.0), 0.0);
        let middle = transition.opacity(Some(Confirmation::Restore), 1.08);
        assert!((middle - 0.5).abs() < 0.001);
        assert_eq!(transition.opacity(Some(Confirmation::Restore), 1.2), 1.0);
        assert_eq!(transition.opacity(None, 2.0), 0.0);
        assert_eq!(transition.opacity(None, 2.2), 1.0);
        assert_eq!(transition.opacity(None, 20.0), 1.0);
    }

    fn draw_dialog(
        app: &mut App,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                time: Some(ctx.input(|input| input.time) + 0.1),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 800.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.render_rollback_dialog(ctx, false),
        )
    }

    fn text_center(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|clipped| {
                if let egui::Shape::Text(text) = &clipped.shape {
                    (text.galley.job.text == label).then(|| text.pos + text.galley.size() / 2.0)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| panic!("Dialog did not render {label:?}"))
    }

    fn click_dialog(app: &mut App, ctx: &egui::Context, pos: egui::Pos2) {
        for pressed in [true, false] {
            draw_dialog(
                app,
                ctx,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
    }

    #[test]
    fn real_dialog_previous_card_confirmation_and_cancel_preserve_pending_draft() {
        use craft_apps_manager::storage::config::{ConfigPatch, ConfigStore, FieldPatch};
        let root =
            std::env::temp_dir().join(format!("craft-rollback-dialog-{}", uuid::Uuid::new_v4()));
        let store = ConfigStore::new(root.clone());
        let preferences = model::Preferences {
            check_catalog_on_startup: false,
            ..Default::default()
        };
        store
            .commit(ConfigPatch {
                fields: vec![FieldPatch {
                    path: vec!["manager".into()],
                    before: store.load().unwrap().document.get("manager").cloned(),
                    after: Some(serde_json::to_value(preferences).unwrap()),
                }],
            })
            .unwrap();
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = App::new(
            &cc,
            model::Paths::new(root.clone(), None),
            root.clone(),
            false,
        )
        .unwrap();
        let installed = model::Installed {
            name: "filmcraft".into(),
            version: "0.6.0".into(),
            path: root.join("portable-copy").display().to_string(),
            architecture: "x64".into(),
            install_kind: "portable".into(),
            ..Default::default()
        };
        app.display_config = Some(model::Config {
            apps_root: root.display().to_string(),
            apps: vec![installed.clone()],
            installations: vec![installed.clone()],
            portable_label_counters: Default::default(),
        });
        let point: RollbackPoint = serde_json::from_value(serde_json::json!({
            "id": uuid::Uuid::new_v4().to_string(), "version": "0.5.0", "bytes": 190840832,
            "architecture": "x64", "created_at": "2026-10-25T12:00:00Z",
            "installation_id": uuid::Uuid::new_v4().to_string(), "app": "filmcraft", "package_type": "zip",
            "executable": "filmcraft.exe", "files": {}
        })).unwrap();
        let before = std::fs::read(root.join("config.yaml")).unwrap();
        app.rollback_dialog = Some(RollbackDialog {
            transition: ViewTransition::default(),
            loaded_at: None,
            failed_point: None,
            installed: installed.clone(),
            status: Some(RollbackStatus {
                installed,
                previous: Some(point),
                preference: None,
                global_enabled: true,
                unsupported: None,
            }),
            draft: Draft {
                value: Some(false),
                ..Draft::loaded(None)
            },
            pending: None,
            error: None,
        });
        draw_dialog(&mut app, &ctx, vec![]);
        draw_dialog(&mut app, &ctx, vec![]);
        draw_dialog(&mut app, &ctx, vec![]);
        let main = draw_dialog(&mut app, &ctx, vec![]);
        let settings_position = text_center(&main, "Keep a rollback version");
        let cancel_position = text_center(&main, "Cancel");
        let retained_status = app.rollback_dialog.as_mut().unwrap().status.take().unwrap();
        let (status_tx, status_rx) = mpsc::channel();
        app.rollback_dialog.as_mut().unwrap().pending = Some(Pending::Status(status_rx));
        let loading = draw_dialog(&mut app, &ctx, vec![]);
        assert!(
            (text_center(&loading, "Keep a rollback version").y - settings_position.y).abs() < 1.0,
            "loading settings {:?}, loaded {:?}",
            text_center(&loading, "Keep a rollback version"),
            settings_position
        );
        assert!((text_center(&loading, "Cancel").y - cancel_position.y).abs() < 1.0);
        text_center(&loading, "Checking files…");
        click_dialog(&mut app, &ctx, text_center(&loading, "Previous version"));
        assert!(app
            .rollback_dialog
            .as_ref()
            .unwrap()
            .draft
            .confirmation
            .is_none());
        status_tx.send(Ok(retained_status)).unwrap();
        draw_dialog(&mut app, &ctx, vec![]);
        draw_dialog(&mut app, &ctx, vec![]);
        app.rollback_dialog.as_mut().unwrap().draft.value = Some(false);
        let main = draw_dialog(&mut app, &ctx, vec![]);
        let subtitle = text_center(&main, "FilmCraft · Portable · x64");
        let current = text_center(&main, "Current version");
        let retained = text_center(&main, "One previous version retained");
        let remove = text_center(&main, "Remove previous version…");
        assert!(
            subtitle.y < current.y,
            "installation subtitle belongs in the header"
        );
        assert!(
            (retained.y - remove.y).abs() < 2.0,
            "retention caption and removal action share one row"
        );
        assert!(retained.x < remove.x);
        let ready = main
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == "Ready to restore" => Some(text),
                _ => None,
            })
            .unwrap();
        // Modal opening fades the whole surface; preserve the green hue through that opacity.
        let expected = theme::palette()
            .green
            .gamma_multiply(f32::from(ready.fallback_color.a()) / 255.0);
        for (painted, expected) in ready.fallback_color.to_array()[..3]
            .iter()
            .zip(&expected.to_array()[..3])
        {
            assert!(painted.abs_diff(*expected) <= 1);
        }
        click_dialog(&mut app, &ctx, text_center(&main, "Previous version"));
        assert_eq!(
            app.rollback_dialog.as_ref().unwrap().draft.confirmation,
            Some(Confirmation::Restore)
        );
        assert_eq!(
            app.rollback_dialog.as_ref().unwrap().draft.value,
            Some(false)
        );
        assert!(
            !app.job.snapshot().busy,
            "opening confirmation must not start rollback"
        );
        draw_dialog(&mut app, &ctx, vec![]);
        draw_dialog(&mut app, &ctx, vec![]);
        draw_dialog(&mut app, &ctx, vec![]);
        let confirmation = draw_dialog(&mut app, &ctx, vec![]);
        click_dialog(&mut app, &ctx, text_center(&confirmation, "Cancel"));
        let draft = &app.rollback_dialog.as_ref().unwrap().draft;
        assert_eq!(draft.confirmation, None);
        assert_eq!(draft.original, None);
        assert_eq!(draft.value, Some(false));
        let healthy_status = app.rollback_dialog.as_mut().unwrap().status.take().unwrap();
        app.rollback_dialog.as_mut().unwrap().failed_point = healthy_status.previous.clone();
        app.rollback_dialog.as_mut().unwrap().error =
            Some("Retained installer files have changed".into());
        for _ in 0..3 {
            draw_dialog(&mut app, &ctx, vec![]);
        }
        let failed = draw_dialog(&mut app, &ctx, vec![]);
        let retry = text_center(&failed, "Retry");
        let delete = text_center(&failed, "Delete");
        assert!((retry.y - delete.y).abs() < 1.0 && retry.x < delete.x);
        click_dialog(&mut app, &ctx, delete);
        assert_eq!(
            app.rollback_dialog.as_ref().unwrap().draft.confirmation,
            Some(Confirmation::Remove)
        );
        assert!(
            !app.job.snapshot().busy,
            "Delete must ask for confirmation before running a job"
        );
        for _ in 0..3 {
            draw_dialog(&mut app, &ctx, vec![]);
        }
        let deletion_confirmation = draw_dialog(&mut app, &ctx, vec![]);
        click_dialog(
            &mut app,
            &ctx,
            text_center(&deletion_confirmation, "Cancel"),
        );
        assert!(app.rollback_dialog.as_ref().unwrap().failed_point.is_some());
        app.rollback_dialog.as_mut().unwrap().status = Some(healthy_status);
        app.rollback_dialog.as_mut().unwrap().failed_point = None;
        app.rollback_dialog.as_mut().unwrap().error = None;
        draw_dialog(&mut app, &ctx, vec![]);
        draw_dialog(&mut app, &ctx, vec![]);
        draw_dialog(&mut app, &ctx, vec![]);
        let main = draw_dialog(&mut app, &ctx, vec![]);
        click_dialog(&mut app, &ctx, text_center(&main, "Cancel"));
        assert!(
            app.rollback_dialog.is_none(),
            "main Cancel discards the dialog draft"
        );
        assert_eq!(std::fs::read(root.join("config.yaml")).unwrap(), before);
        assert!(!app.job.snapshot().busy);
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn version_labels_have_one_prefix() {
        assert_eq!(version_label("0.6.0"), "v0.6.0");
        assert_eq!(version_label("v0.6.0"), "v0.6.0");
        assert_eq!(version_label("V0.6.0"), "v0.6.0");
    }

    #[test]
    fn draft_changes_are_pending_and_confirmation_does_not_apply_them() {
        let mut draft = Draft::loaded(None);
        draft.value = Some(false);
        assert!(draft.changed());
        draft.confirmation = Some(Confirmation::Restore);
        assert_eq!(draft.original, None);
        draft.confirmation = None;
        assert_eq!(draft.value, Some(false));
        assert_eq!(
            Draft::loaded(None).value,
            None,
            "reopening discards cancelled draft"
        );
    }

    #[test]
    fn selection_validation_rejects_another_copy_or_version() {
        let installed = model::Installed {
            name: "filmcraft".into(),
            path: "copy-one".into(),
            version: "1".into(),
            ..Default::default()
        };
        let mut other = installed.clone();
        assert!(same_installation(&installed, &other));
        other.path = "copy-two".into();
        assert!(!same_installation(&installed, &other));
        other = installed.clone();
        other.version = "2".into();
        assert!(!same_installation(&installed, &other));
    }
}
