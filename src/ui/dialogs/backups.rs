use super::*;

/// Title, date and kind for one backup row.
pub(super) fn backup_labels(
    app: &str,
    backup: &backup_store::Backup,
) -> (String, String, &'static str) {
    let filename = backup.path.file_name().unwrap().to_string_lossy();
    let version = filename
        .strip_prefix(&if backup.source {
            format!("{app}-source-")
        } else {
            format!("{app}-")
        })
        .unwrap_or(&filename)
        .split('-')
        .take(1)
        .collect::<Vec<_>>()
        .join(" ");
    let date = std::fs::metadata(&backup.path)
        .and_then(|m| m.modified())
        .ok()
        .map(|t| {
            chrono::DateTime::<chrono::Local>::from(t)
                .format("%b %d, %Y · %I:%M %p")
                .to_string()
        })
        .unwrap_or_default();
    let format = if backup.path.is_dir() {
        "Folder"
    } else if backup.path.extension().is_some_and(|e| e == "7z") {
        "7-Zip archive"
    } else {
        "ZIP archive"
    };
    (
        if backup.source {
            format!("Source {version}")
        } else {
            format!("Version {version}")
        },
        format!("{date} · {format}"),
        if backup.source {
            "Source"
        } else {
            "Portable app"
        },
    )
}
impl App {
    /// The list of backups: radio buttons to restore one, checkboxes to delete several.
    fn backup_rows(&mut self, ui: &mut egui::Ui, app: &str) {
        if self.restore_backups.is_empty() {
            return;
        }
        theme::group()
            .inner_margin(egui::Margin::symmetric(0, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 0.0;
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let count = self.restore_backups.len();
                        for (i, backup) in self.restore_backups.iter().enumerate() {
                            if i > 0 {
                                rule(ui);
                            }
                            let (title, detail, kind) = backup_labels(app, backup);
                            let selected = if self.backup_delete_mode {
                                self.backup_delete_selected.contains(&i)
                            } else {
                                self.restore_selected == Some(i)
                            };
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 68.0),
                                Sense::click(),
                            );
                            response.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    if self.backup_delete_mode {
                                        egui::WidgetType::Checkbox
                                    } else {
                                        egui::WidgetType::RadioButton
                                    },
                                    true,
                                    selected,
                                    format!("{title} · {detail}"),
                                )
                            });
                            if response.hovered() {
                                let round = |on: bool| if on { 9 } else { 0 };
                                ui.painter().rect_filled(
                                    rect,
                                    CornerRadius {
                                        nw: round(i == 0),
                                        ne: round(i == 0),
                                        sw: round(i + 1 == count),
                                        se: round(i + 1 == count),
                                    },
                                    theme::palette().hover,
                                );
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            if response.has_focus() {
                                ui.painter().rect_stroke(
                                    rect.shrink(1.0),
                                    CornerRadius::same(6),
                                    Stroke::new(2.0_f32, theme::palette().link),
                                    StrokeKind::Inside,
                                );
                            }
                            let mark = egui::Rect::from_min_size(
                                egui::pos2(
                                    rect.left() + if self.backup_delete_mode { 18.0 } else { 19.0 },
                                    rect.center().y - 8.0,
                                ),
                                egui::Vec2::splat(16.0),
                            );
                            if self.backup_delete_mode {
                                theme::paint_check(ui, mark, selected, CHECK_RED, true);
                            } else {
                                theme::paint_radio(ui, mark.center(), selected);
                            }
                            theme::painter_text(
                                ui.painter(),
                                egui::pos2(rect.left() + 50.0, rect.center().y - 1.0),
                                egui::Align2::LEFT_BOTTOM,
                                &title,
                                14.0,
                                theme::palette().text,
                            );
                            theme::painter_text(
                                ui.painter(),
                                egui::pos2(rect.left() + 50.0, rect.center().y + 1.0),
                                egui::Align2::LEFT_TOP,
                                &detail,
                                12.0,
                                theme::palette().muted,
                            );
                            let galley = ui.painter().layout_no_wrap(
                                kind.to_owned(),
                                theme::bold(11.0),
                                theme::palette().text_3,
                            );
                            let chip = egui::Rect::from_min_size(
                                egui::pos2(
                                    rect.right() - 14.0 - galley.size().x - 16.0,
                                    rect.center().y - 11.0,
                                ),
                                egui::vec2(galley.size().x + 16.0, 22.0),
                            );
                            ui.painter().rect_filled(
                                chip,
                                CornerRadius::same(11),
                                theme::palette().button,
                            );
                            ui.painter().galley(
                                chip.center() - galley.size() / 2.0,
                                galley,
                                theme::palette().text_3,
                            );
                            if response.clicked() {
                                if self.backup_delete_mode {
                                    if !self.backup_delete_selected.insert(i) {
                                        self.backup_delete_selected.remove(&i);
                                    }
                                } else {
                                    self.restore_selected = Some(i);
                                }
                                self.confirm_restore = false;
                            }
                        }
                    });
            });
    }

    pub(super) fn render_restore_dialog(&mut self, ctx: &egui::Context, busy: bool) {
        if let Some(app) = self.restore_app.clone() {
            let modal = modal(
                ctx,
                format!("{} backups", model::title(&app)),
                560.0,
                |ui| {
                    let top = ui.cursor().top();
                    title_bar(ui, &format!("{} backups", model::title(&app)), |ui| {
                        let mut mode = self.backup_delete_mode;
                        if theme::segmented(
                            ui,
                            28.0,
                            12.0,
                            &mut mode,
                            &[(false, "Restore"), (true, "Delete")],
                        ) {
                            self.backup_delete_mode = mode;
                            self.confirm_restore = false;
                            self.restore_selected = None;
                            self.backup_delete_selected.clear();
                        }
                    });
                    band(
                        ui,
                        egui::Margin::symmetric(20, 16),
                        520.0 - 2.0 - BAR_HEIGHT - (ui.cursor().top() - top),
                        |ui| {
                            ui.spacing_mut().item_spacing.y = 12.0;
                            ui.add(
                                egui::Label::new(
                                    RichText::new(if self.backup_delete_mode {
                                        "Choose the backups to delete. Installed apps and source files are kept."
                                    } else {
                                        "Choose one backup to restore. It replaces the current managed copy and the backup is kept."
                                    })
                                    .size(13.0)
                                    .color(theme::palette().text_3),
                                )
                                .wrap(),
                            );
                            if self.restore_backups.is_empty() {
                                theme::text(
                                    ui,
                                    "No backups are available for this app.",
                                    14.0,
                                    theme::palette().text_2,
                                );
                            }
                            self.backup_rows(ui, &app);
                            if self.backup_delete_mode {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 16.0;
                                    if theme::link(ui, "Select all").clicked() {
                                        self.backup_delete_selected =
                                            (0..self.restore_backups.len()).collect();
                                        self.confirm_restore = false;
                                    }
                                    if theme::link(ui, "Deselect all").clicked() {
                                        self.backup_delete_selected.clear();
                                        self.confirm_restore = false;
                                    }
                                    theme::text(
                                        ui,
                                        format!("{} selected", self.backup_delete_selected.len()),
                                        13.0,
                                        theme::palette().muted,
                                    );
                                });
                            }
                            if self.confirm_restore {
                                ui.label(
                                    RichText::new(if self.backup_delete_mode {
                                        "Permanently delete the selected backups?"
                                    } else {
                                        "This replaces the current managed copy. Continue?"
                                    })
                                    .color(theme::palette().amber),
                                );
                            }
                        },
                    );
                    footer(ui, |ui| {
                        let ready = (if self.backup_delete_mode {
                            !self.backup_delete_selected.is_empty()
                        } else {
                            self.restore_selected.is_some()
                        }) && !busy;
                        if action(
                            ui,
                            if self.backup_delete_mode && self.confirm_restore {
                                "Confirm deletion"
                            } else if self.backup_delete_mode {
                                "Delete selected…"
                            } else if self.confirm_restore {
                                "Confirm restore"
                            } else {
                                "Restore…"
                            },
                            if self.backup_delete_mode {
                                Kind::Danger
                            } else {
                                Kind::Primary
                            },
                            ready,
                        )
                        .clicked()
                        {
                            if self.confirm_restore {
                                let selected: Vec<_> = if self.backup_delete_mode {
                                    self.backup_delete_selected
                                        .iter()
                                        .map(|i| self.restore_backups[*i].clone())
                                        .collect()
                                } else {
                                    vec![self.restore_backups[self.restore_selected.unwrap()]
                                        .clone()]
                                };
                                let deleting = self.backup_delete_mode;
                                let paths = self.paths.clone();
                                let app = app.clone();
                                self.release_checks.remove(&app);
                                self.job =
                                    Job::new(paths.at("logs/updates.log"), &self.build_preferences);
                                self.job_action.clear();
                                self.failure_dismissed = false;
                                self.job_target = Some((
                                    app.clone(),
                                    if deleting {
                                        "delete-backups"
                                    } else {
                                        "restore"
                                    }
                                    .into(),
                                ));
                                self.operation_was_busy = true;
                                self.job.describe(
                                    if deleting {
                                        "Delete backups"
                                    } else {
                                        "Restore backup"
                                    },
                                    &app,
                                );
                                self.job.spawn(move |job| {
                                    if deleting {
                                        backup_store::delete_selected(&paths, &app, &selected)?;
                                        job.report("Backups deleted", selected.len().to_string());
                                        job.log(&format!(
                                            "Deleted {} backup(s) for {}",
                                            selected.len(),
                                            model::title(&app)
                                        ));
                                        Ok(())
                                    } else {
                                        backup_store::restore(&paths, &app, &selected[0], &job)
                                    }
                                });
                                self.restore_app = None;
                            } else {
                                self.confirm_restore = true;
                            }
                        }
                        if action(ui, "Close", Kind::Secondary, true).clicked() {
                            self.restore_app = None;
                        }
                    });
                },
            );
            if modal.should_close() {
                self.restore_app = None;
            }
        }
    }
}
