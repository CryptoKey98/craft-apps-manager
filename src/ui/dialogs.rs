//! Every modal the manager shows. Each keeps the behaviour of the dialog it replaces.
use super::theme::{self, btn, Icon, Kind};
use super::{modal, result_for_display, App};
use craft_apps_manager::{
    apps, backups,
    jobs::Job,
    model::{self, APPS, SOURCES},
    profiles, self_update, updates,
};
use eframe::egui::{self, Color32, RichText};

/// Icon or app artwork, title and supporting text at the top of a dialog.
fn header(ui: &mut egui::Ui, art: Art, title: &str, text: &str) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        match art {
            Art::Texture(texture) => {
                ui.add(
                    egui::Image::new((texture, egui::vec2(48.0, 48.0)))
                        .corner_radius(egui::CornerRadius::same(11)),
                );
            }
            Art::Icon(icon, fill, color) => {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(44.0, 44.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(rect, egui::CornerRadius::same(10), fill);
                theme::paint_icon(
                    ui.painter(),
                    egui::Rect::from_center_size(rect.center(), egui::vec2(20.0, 20.0)),
                    icon,
                    color,
                );
            }
            Art::None => {}
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            theme::heading(ui, title, 17.0);
            if !text.is_empty() {
                ui.label(RichText::new(text).size(14.0).color(theme::TEXT_2));
            }
        });
    });
}

enum Art {
    Texture(egui::TextureId),
    Icon(Icon, Color32, Color32),
    None,
}

/// Right-aligned action buttons separated from the body by a hairline.
/// Add the primary action first: the layout runs right to left.
fn footer(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(16.0);
    theme::divider(ui);
    ui.add_space(12.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), content);
}

fn note(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(RichText::new(text).size(12.5).color(theme::MUTED));
}

fn danger() -> (Icon, Color32, Color32) {
    (Icon::Trash, theme::RED_BG, theme::RED)
}

impl App {
    pub(super) fn dialogs(&mut self, ctx: &egui::Context) {
        let busy = self.job.state.lock().unwrap().busy;
        if let Some(plan) = self.release_plan.clone() {
            let response = modal(ctx, "Review release operations", 600.0, |ui| {
                header(
                    ui,
                    Art::Icon(Icon::Refresh, theme::ACCENT_SOFT, theme::ACCENT_TEXT),
                    "Review install and update",
                    &format!(
                        "{} · {}. Only the Install and Update entries below will run.",
                        plan.preferences.release_format,
                        model::architecture_label(&plan.preferences.architecture)
                    ),
                );
                ui.add_space(12.0);
                theme::group().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                        for (index, entry) in plan.entries.iter().enumerate() {
                            if index > 0 {
                                theme::divider(ui);
                            }
                            egui::Frame::new()
                                .inner_margin(egui::Margin::symmetric(14, 10))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.horizontal(|ui| {
                                        if let Some(texture) = self.icons.get(&entry.app) {
                                            ui.add(
                                                egui::Image::new((
                                                    texture.id(),
                                                    egui::vec2(24.0, 24.0),
                                                ))
                                                .corner_radius(egui::CornerRadius::same(6)),
                                            );
                                        }
                                        theme::text(ui, model::title(&entry.app), 14.0, theme::TEXT);
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                let (fill, color) = match entry.action.as_str() {
                                                    "Install" | "Update" => {
                                                        (theme::ACCENT_SOFT, theme::ACCENT_TEXT)
                                                    }
                                                    "Error" => (theme::RED_BG, theme::RED),
                                                    _ => (Color32::from_rgb(0x23, 0x26, 0x2c), theme::TEXT_3),
                                                };
                                                theme::badge(
                                                    ui,
                                                    format!("{} {}", entry.action, entry.version).trim(),
                                                    fill,
                                                    color,
                                                );
                                            },
                                        );
                                    });
                                    if let Some(error) = &entry.error {
                                        ui.label(RichText::new(error).size(12.5).color(theme::RED));
                                    } else {
                                        note(
                                            ui,
                                            entry
                                                .destination
                                                .as_ref()
                                                .map(|p| format!("Destination: {}", p.display()))
                                                .unwrap_or_else(|| {
                                                    "Destination: Applications / system installation (installer controlled)".into()
                                                }),
                                        );
                                        if let Some(asset) = &entry.asset {
                                            ui.hyperlink_to(
                                                RichText::new(&asset.name).size(12.5),
                                                &asset.browser_download_url,
                                            );
                                        }
                                    }
                                });
                        }
                    });
                });
                footer(ui, |ui| {
                    let count = plan.executable_count();
                    if btn(&format!(
                        "Run {count} operation{}",
                        if count == 1 { "" } else { "s" }
                    ))
                    .primary()
                    .medium()
                    .enabled(count > 0)
                    .show(ui)
                    .clicked()
                    {
                        self.release_plan = None;
                        let paths = self.paths.clone();
                        self.job = Job::new(paths.at("logs/updates.log"), &self.build_preferences);
                        self.job_target = None;
                        self.job_action.clear();
                        self.failure_dismissed = false;
                        self.operation_was_busy = true;
                        self.job
                            .spawn(move |job| updates::execute_plan(&paths, &plan, &job));
                    }
                    if btn("Cancel").medium().show(ui).clicked() {
                        self.release_plan = None;
                    }
                });
            });
            if response.should_close() {
                self.release_plan = None;
            }
        }

        if let Some(app) = self.confirm_install.clone() {
            let installer = self.preferences.release_format == "installer";
            let update = self.status(&app).update;
            let modal = modal(ctx, "Confirm installation", 500.0, |ui| {
                let title = match &update {
                    Some(version) => format!("Update {} to {version}?", model::title(&app)),
                    None => format!("Install {}?", model::title(&app)),
                };
                let text = if installer {
                    if cfg!(target_os = "linux") {
                        format!(
                            "Downloads the latest {} {} package. You will be asked to authorize the installation.",
                            model::title(&app),
                            craft_apps_manager::installers::installer_label()
                        )
                    } else if cfg!(target_os = "macos") {
                        format!(
                            "Downloads the latest release and copies {} to your Applications folder.",
                            model::executable_name(&app)
                        )
                    } else {
                        format!(
                            "Downloads the latest {} {} and opens its installer wizard. Windows may ask for administrator permission.",
                            model::title(&app),
                            craft_apps_manager::installers::installer_label()
                        )
                    }
                } else {
                    format!(
                        "Downloads the latest release of {} into your library as a portable app.",
                        model::title(&app)
                    )
                };
                let art = match self.icons.get(&app) {
                    Some(texture) => Art::Texture(texture.id()),
                    None => Art::None,
                };
                header(ui, art, &title, &text);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(64.0);
                    ui.vertical(|ui| {
                        note(
                            ui,
                            format!(
                                "Latest stable release · {} · {}",
                                model::architecture_label(&self.preferences.architecture),
                                self.preferences.release_format
                            ),
                        );
                        ui.hyperlink_to(
                            RichText::new(format!(
                                "github.com/storytold/{}/releases/latest",
                                model::repository(&app)
                            ))
                            .size(12.5),
                            format!(
                                "https://github.com/storytold/{}/releases/latest",
                                model::repository(&app)
                            ),
                        );
                    });
                });
                footer(ui, |ui| {
                    if btn(if update.is_some() {
                        "Update"
                    } else {
                        "Install"
                    })
                    .primary()
                    .medium()
                    .show(ui)
                    .clicked()
                    {
                        self.app = app.clone();
                        self.confirm_install = None;
                        self.start("install-app");
                    }
                    if btn("Cancel").medium().show(ui).clicked() {
                        self.confirm_install = None;
                    }
                });
            });
            if modal.should_close() {
                self.confirm_install = None;
            }
        }
        if self.launch_settings_open {
            let modal = modal(
                ctx,
                format!("{} launch settings", model::title(&self.app)),
                520.0,
                |ui| {
                    header(
                        ui,
                        Art::None,
                        &format!("{} launch options", model::title(&self.app)),
                        "",
                    );
                    ui.add_space(8.0);
                    theme::text(ui, "Open", 13.0, theme::TEXT_3);
                    egui::ComboBox::from_id_salt("launch-executable")
                        .width(ui.available_width())
                        .selected_text(if self.launch_draft.executable.is_empty() {
                            "Default executable"
                        } else {
                            &self.launch_draft.executable
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.launch_draft.executable,
                                String::new(),
                                "Default executable",
                            );
                            for exe in apps::executables(&self.paths, &self.app).unwrap_or_default()
                            {
                                ui.selectable_value(
                                    &mut self.launch_draft.executable,
                                    exe.clone(),
                                    &exe,
                                );
                            }
                        });
                    ui.add_space(8.0);
                    theme::text(ui, "Arguments, one per line", 13.0, theme::TEXT_3);
                    ui.add(
                        egui::TextEdit::multiline(&mut self.launch_arguments)
                            .font(egui::TextStyle::Monospace)
                            .hint_text("--example-flag")
                            .desired_width(ui.available_width())
                            .desired_rows(5),
                    );
                    note(
                        ui,
                        "Passed to the app exactly as written. Don't add shell quotes.",
                    );
                    footer(ui, |ui| {
                        if btn("Save").primary().medium().show(ui).clicked() {
                            self.launch_draft.arguments = self
                                .launch_arguments
                                .lines()
                                .filter(|s| !s.is_empty())
                                .map(String::from)
                                .collect();
                            let result = apps::save(&self.paths, &self.app, &self.launch_draft);
                            if result.is_ok() {
                                self.launch_settings_open = false;
                            }
                            self.result(result);
                        }
                        if btn("Cancel").medium().show(ui).clicked() {
                            self.launch_settings_open = false;
                        }
                    });
                },
            );
            if modal.should_close() {
                self.launch_settings_open = false;
            }
        }
        if self.confirm_uninstall {
            let modal = modal(
                ctx,
                format!("Uninstall {}?", model::title(&self.app)),
                520.0,
                |ui| {
                    let installed = self
                        .display_config
                        .as_ref()
                        .and_then(|c| c.apps.iter().find(|a| a.name == self.app))
                        .map(|a| a.path.clone())
                        .filter(|path| !path.is_empty());
                    let art = match self.icons.get(&self.app) {
                        Some(texture) => Art::Texture(texture.id()),
                        None => Art::None,
                    };
                    header(
                        ui,
                        art,
                        &format!("Uninstall {}?", model::title(&self.app)),
                        &format!(
                            "Removes {}. Source ZIPs, builds, backups and launch settings are kept.",
                            installed.as_deref().unwrap_or("the installed copy")
                        ),
                    );
                    ui.add_space(12.0);
                    let targets = profiles::targets(&self.paths, &self.app);
                    theme::group().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        egui::Frame::new()
                            .inner_margin(egui::Margin::same(14))
                            .show(ui, |ui| {
                                ui.add_enabled(
                                    targets.is_ok(),
                                    egui::Checkbox::new(
                                        &mut self.delete_profile,
                                        format!(
                                            "Also delete {} settings and caches",
                                            model::title(&self.app)
                                        ),
                                    ),
                                );
                                note(ui, "Includes plug-ins and recovery copies. Other copies of the app may share them.");
                                match &targets {
                                    Ok(targets) => {
                                        let existing: Vec<_> =
                                            targets.iter().filter(|t| t.path.exists()).collect();
                                        if existing.is_empty() {
                                            note(ui, "No existing profile folders found.");
                                        }
                                        egui::ScrollArea::vertical().max_height(100.0).show(
                                            ui,
                                            |ui| {
                                                for target in existing {
                                                    ui.label(
                                                        RichText::new(
                                                            target.path.display().to_string(),
                                                        )
                                                        .monospace()
                                                        .color(theme::TEXT_3),
                                                    );
                                                }
                                            },
                                        );
                                        note(ui, "Custom profile locations outside these folders are kept.");
                                    }
                                    Err(error) => {
                                        note(ui, format!("Not available: {error}"));
                                    }
                                }
                            });
                    });
                    footer(ui, |ui| {
                        if btn("Uninstall")
                            .kind(Kind::Danger)
                            .medium()
                            .show(ui)
                            .clicked()
                        {
                            self.start("uninstall-app");
                            self.confirm_uninstall = false;
                        }
                        if btn("Cancel").medium().show(ui).clicked() {
                            self.confirm_uninstall = false;
                        }
                    });
                },
            );
            if modal.should_close() {
                self.confirm_uninstall = false;
            }
        }
        if self.selection {
            let choices: &[&str] = if self.source_selection {
                &SOURCES
            } else {
                &APPS
            };
            let modal = modal(
                ctx,
                if self.source_selection {
                    "Choose source apps"
                } else {
                    "Choose release apps"
                },
                480.0,
                |ui| {
                    header(
                        ui,
                        Art::None,
                        if self.source_selection {
                            "Sources in Update all sources"
                        } else {
                            "Apps in Update all"
                        },
                        if self.source_selection {
                            "Choose which source ZIPs to update. ArtCraft X is source only."
                        } else {
                            "Choose which apps are installed or updated."
                        },
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        theme::text(
                            ui,
                            format!(
                                "{} of {} selected",
                                self.selection_draft.len(),
                                choices.len()
                            ),
                            13.0,
                            theme::MUTED,
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if theme::link(ui, "Deselect all").clicked() {
                                self.selection_draft.clear();
                            }
                            if theme::link(ui, "Select all").clicked() {
                                self.selection_draft =
                                    choices.iter().map(|s| s.to_string()).collect();
                            }
                        });
                    });
                    theme::group().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        egui::ScrollArea::vertical()
                            .max_height(360.0)
                            .show(ui, |ui| {
                                egui::Frame::new()
                                    .inner_margin(egui::Margin::symmetric(12, 8))
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        for &name in choices {
                                            let mut checked =
                                                self.selection_draft.iter().any(|s| s == name);
                                            ui.horizontal(|ui| {
                                                ui.set_min_height(36.0);
                                                if ui.checkbox(&mut checked, "").changed() {
                                                    if checked {
                                                        self.selection_draft.push(name.into())
                                                    } else {
                                                        self.selection_draft.retain(|s| s != name)
                                                    }
                                                }
                                                if let Some(texture) = self.icons.get(name) {
                                                    ui.add(
                                                        egui::Image::new((
                                                            texture.id(),
                                                            egui::vec2(24.0, 24.0),
                                                        ))
                                                        .corner_radius(egui::CornerRadius::same(6)),
                                                    );
                                                }
                                                theme::text(
                                                    ui,
                                                    model::title(name),
                                                    14.0,
                                                    theme::TEXT,
                                                );
                                            });
                                        }
                                    });
                            });
                    });
                    footer(ui, |ui| {
                        if btn("Save").primary().medium().show(ui).clicked() {
                            if self.source_selection {
                                self.settings_draft.selected_sources = self.selection_draft.clone();
                            } else {
                                self.settings_draft.selected_apps = self.selection_draft.clone();
                            }
                            self.selection = false;
                        }
                        if btn("Cancel").medium().show(ui).clicked() {
                            self.selection = false;
                        }
                    });
                },
            );
            if modal.should_close() {
                self.selection = false;
            }
        }
        if let Some(app) = self.restore_app.clone() {
            let modal = modal(
                ctx,
                format!("{} backups", model::title(&app)),
                560.0,
                |ui| {
                    ui.horizontal(|ui| {
                        theme::heading(ui, format!("{} backups", model::title(&app)), 17.0);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let mut mode = self.backup_delete_mode;
                            if theme::segmented(
                                ui,
                                &mut mode,
                                &[(false, "Restore"), (true, "Delete")],
                            ) {
                                self.backup_delete_mode = mode;
                                self.confirm_restore = false;
                                self.restore_selected = None;
                                self.backup_delete_selected.clear();
                            }
                        });
                    });
                    ui.add_space(8.0);
                    theme::text(
                        ui,
                        if self.backup_delete_mode {
                            "Choose the backups to delete. Installed apps and source files are kept."
                        } else {
                            "Choose one backup to restore. It replaces the current managed copy and the backup is kept."
                        },
                        13.0,
                        theme::TEXT_3,
                    );
                    if self.backup_delete_mode {
                        ui.horizontal(|ui| {
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
                                theme::MUTED,
                            );
                        });
                    }
                    ui.add_space(4.0);
                    if self.restore_backups.is_empty() {
                        theme::text(
                            ui,
                            "No backups are available for this app.",
                            14.0,
                            theme::TEXT_2,
                        );
                    }
                    theme::group().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        egui::ScrollArea::vertical()
                            .max_height(300.0)
                            .show(ui, |ui| {
                                for (i, backup) in self.restore_backups.iter().enumerate() {
                                    if i > 0 {
                                        theme::divider(ui);
                                    }
                                    let (title, detail, kind) = backup_labels(&app, backup);
                                    let selected = if self.backup_delete_mode {
                                        self.backup_delete_selected.contains(&i)
                                    } else {
                                        self.restore_selected == Some(i)
                                    };
                                    let (rect, response) = ui.allocate_exact_size(
                                        egui::vec2(ui.available_width(), 56.0),
                                        egui::Sense::click(),
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
                                    if selected || response.hovered() {
                                        ui.painter().rect_filled(
                                            rect,
                                            0.0,
                                            if selected {
                                                theme::SELECTED
                                            } else {
                                                theme::HOVER
                                            },
                                        );
                                    }
                                    let mark = egui::Rect::from_center_size(
                                        egui::pos2(rect.left() + 22.0, rect.center().y),
                                        egui::vec2(16.0, 16.0),
                                    );
                                    let accent = if self.backup_delete_mode {
                                        theme::DANGER
                                    } else {
                                        theme::ACCENT
                                    };
                                    if self.backup_delete_mode {
                                        ui.painter().rect(
                                            mark,
                                            egui::CornerRadius::same(4),
                                            if selected { accent } else { theme::FIELD },
                                            egui::Stroke::new(
                                                1.0,
                                                if selected {
                                                    accent
                                                } else {
                                                    theme::BORDER_STRONG
                                                },
                                            ),
                                            egui::StrokeKind::Inside,
                                        );
                                        if selected {
                                            theme::paint_icon(
                                                ui.painter(),
                                                mark.shrink(2.0),
                                                Icon::Check,
                                                Color32::WHITE,
                                            );
                                        }
                                    } else {
                                        ui.painter().circle(
                                            mark.center(),
                                            8.0,
                                            theme::FIELD,
                                            egui::Stroke::new(
                                                1.0,
                                                if selected {
                                                    accent
                                                } else {
                                                    theme::BORDER_STRONG
                                                },
                                            ),
                                        );
                                        if selected {
                                            ui.painter().circle_filled(mark.center(), 4.5, accent);
                                        }
                                    }
                                    theme::painter_text(
                                        ui.painter(),
                                        egui::pos2(rect.left() + 42.0, rect.top() + 10.0),
                                        egui::Align2::LEFT_TOP,
                                        &title,
                                        14.0,
                                        theme::TEXT,
                                    );
                                    theme::painter_text(
                                        ui.painter(),
                                        egui::pos2(rect.left() + 42.0, rect.top() + 31.0),
                                        egui::Align2::LEFT_TOP,
                                        &detail,
                                        12.0,
                                        theme::MUTED,
                                    );
                                    let galley = ui.painter().layout_no_wrap(
                                        kind.to_owned(),
                                        egui::FontId::proportional(11.0),
                                        theme::TEXT_3,
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
                                        egui::CornerRadius::same(11),
                                        Color32::from_rgb(0x23, 0x26, 0x2c),
                                    );
                                    ui.painter().galley(
                                        chip.center() - galley.size() / 2.0,
                                        galley,
                                        theme::TEXT_3,
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
                    if self.confirm_restore {
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(if self.backup_delete_mode {
                                "Permanently delete the selected backups?"
                            } else {
                                "This replaces the current managed copy. Continue?"
                            })
                            .color(theme::AMBER),
                        );
                    }
                    footer(ui, |ui| {
                        let ready = (if self.backup_delete_mode {
                            !self.backup_delete_selected.is_empty()
                        } else {
                            self.restore_selected.is_some()
                        }) && !busy;
                        if btn(if self.backup_delete_mode && self.confirm_restore {
                            "Confirm deletion"
                        } else if self.backup_delete_mode {
                            "Delete selected…"
                        } else if self.confirm_restore {
                            "Confirm restore"
                        } else {
                            "Restore…"
                        })
                        .kind(if self.backup_delete_mode {
                            Kind::Danger
                        } else {
                            Kind::Primary
                        })
                        .medium()
                        .enabled(ready)
                        .show(ui)
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
                                self.job.spawn(move |job| {
                                    if deleting {
                                        backups::delete_selected(&paths, &app, &selected)?;
                                        job.log(&format!(
                                            "Deleted {} backup(s) for {}",
                                            selected.len(),
                                            model::title(&app)
                                        ));
                                        Ok(())
                                    } else {
                                        backups::restore(&paths, &app, &selected[0], &job)
                                    }
                                });
                                self.restore_app = None;
                            } else {
                                self.confirm_restore = true;
                            }
                        }
                        if btn("Close").medium().show(ui).clicked() {
                            self.restore_app = None;
                        }
                    });
                },
            );
            if modal.should_close() {
                self.restore_app = None;
            }
        }
        if self.confirm_self_update {
            let msi = self_update::installed_with_msi();
            let modal = modal(ctx, "Update Craft Apps Manager", 500.0, |ui| {
                header(
                    ui,
                    Art::Icon(Icon::ArrowUp, theme::ACCENT_SOFT, theme::ACCENT_TEXT),
                    "Update Craft Apps Manager?",
                    &format!(
                        "Version {} is available. {} Your library and settings are kept.",
                        self.manager_available
                            .as_ref()
                            .map(|a| a.version.as_str())
                            .unwrap_or("?"),
                        if msi {
                            "The manager downloads and installs it."
                        } else {
                            "The manager downloads it, checks it and restarts."
                        }
                    ),
                );
                ui.horizontal(|ui| {
                    ui.add_space(60.0);
                    ui.vertical(|ui| {
                        note(ui, "Close other manager and builder windows first.");
                        ui.hyperlink_to(
                            RichText::new(format!("From {}", self_update::REPOSITORY_NAME))
                                .size(12.5),
                            self_update::REPOSITORY,
                        );
                    });
                });
                footer(ui, |ui| {
                    if btn(if msi {
                        "Download and install"
                    } else {
                        "Download and restart"
                    })
                    .primary()
                    .medium()
                    .show(ui)
                    .clicked()
                    {
                        if let Some(available) = self.manager_available.clone() {
                            let paths = self.paths.clone();
                            let ctx = ctx.clone();
                            let (tx, rx) = std::sync::mpsc::channel();
                            self.manager_plan = Some(rx);
                            self.job = Job::new(
                                self.paths.at("logs/updates.log"),
                                &self.build_preferences,
                            );
                            self.job_target = None;
                            self.job_action.clear();
                            self.failure_dismissed = false;
                            self.job.spawn(move |job| {
                                let result = self_update::prepare(&paths, &available, &job);
                                let _ = tx.send(result_for_display(&result));
                                ctx.request_repaint();
                                result.map(|_| ())
                            });
                            self.manager_message = "Downloading and verifying manager…".into();
                        }
                        self.confirm_self_update = false;
                    }
                    if btn("Later").medium().show(ui).clicked() {
                        self.confirm_self_update = false;
                    }
                });
            });
            if modal.should_close() {
                self.confirm_self_update = false;
            }
        }
        if self.confirm_clear || self.confirm_clean {
            let modal = modal(ctx, "Confirm cleanup", 460.0, |ui| {
                let (icon, fill, color) = danger();
                header(
                    ui,
                    Art::Icon(icon, fill, color),
                    if self.confirm_clear {
                        "Delete all backups?"
                    } else {
                        "Delete temporary build files?"
                    },
                    if self.confirm_clear {
                        "Deletes managed release and source backups. Installed apps, finished builds, source ZIPs and tools are kept."
                    } else {
                        "Deletes compilation caches and extracted source workspaces. Installed apps, finished builds, source ZIPs and tools are kept."
                    },
                );
                footer(ui, |ui| {
                    if btn("Delete").kind(Kind::Danger).medium().show(ui).clicked() {
                        self.start(if self.confirm_clear { "clear" } else { "clean" });
                        self.confirm_clear = false;
                        self.confirm_clean = false;
                    }
                    if btn("Cancel").medium().show(ui).clicked() {
                        self.confirm_clear = false;
                        self.confirm_clean = false;
                    }
                });
            });
            if modal.should_close() {
                self.confirm_clear = false;
                self.confirm_clean = false;
            }
        }
        if let Some(message) = self.selection_notice.clone() {
            let modal = modal(ctx, "No apps selected", 460.0, |ui| {
                header(
                    ui,
                    Art::Icon(Icon::Alert, theme::ACCENT_SOFT, theme::ACCENT_TEXT),
                    "No apps selected",
                    &message,
                );
                footer(ui, |ui| {
                    if btn("OK").primary().medium().show(ui).clicked() {
                        self.selection_notice = None;
                    }
                });
            });
            if modal.should_close() {
                self.selection_notice = None;
            }
        }
        if let Some(error) = self.error.clone() {
            let modal = modal(ctx, "Craft Apps Manager", 500.0, |ui| {
                header(
                    ui,
                    Art::Icon(Icon::Alert, theme::RED_BG, theme::RED),
                    "Something went wrong",
                    "",
                );
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(error).color(theme::RED_TEXT))
                                .selectable(true),
                        );
                    });
                footer(ui, |ui| {
                    if btn("OK").primary().medium().show(ui).clicked() {
                        self.error = None;
                    }
                });
            });
            if modal.should_close() {
                self.error = None;
            }
        }
    }
}

/// Title, date and kind for one backup row.
fn backup_labels(app: &str, backup: &backups::Backup) -> (String, String, &'static str) {
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
