use super::*;

impl App {
    pub(super) fn render_release_review(&mut self, ctx: &egui::Context) {
        if let Some(plan) = self
            .release_plan
            .clone()
            .filter(|_| !self.selection_from_review)
        {
            let response = modal(ctx, "Review release operations", 600.0, |ui| {
                band(ui, BODY, 0.0, |ui| {
                    header(
                        ui,
                        Art::Icon(
                            Icon::Refresh,
                            theme::palette().accent_soft,
                            theme::palette().accent_text,
                            40.0,
                        ),
                        if plan.requested_tag.is_some() {
                            "Review version change"
                        } else {
                            "Review app updates"
                        },
                        "Each app keeps its installed format and architecture.",
                        |_| {},
                    );
                    if let Some(tag) = &plan.requested_tag {
                        let installed = plan
                            .entries
                            .first()
                            .and_then(|entry| self.status(&entry.app).installed)
                            .map(|i| i.version)
                            .unwrap_or_else(|| "Not installed".into());
                        note(ui, format!("Installed: {installed} → Selected: {tag}."));
                    }
                    ui.add_space(18.0);
                    if plan.entries.is_empty() {
                        theme::group().show(ui, |ui| {
                            egui::Frame::new()
                                .inner_margin(egui::Margin::same(18))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.vertical_centered(|ui| {
                                        ui.add_space(6.0);
                                        let (rect, _) = ui.allocate_exact_size(
                                            egui::vec2(24.0, 24.0),
                                            egui::Sense::hover(),
                                        );
                                        theme::paint_icon(
                                            ui.painter(),
                                            rect,
                                            Icon::Grid,
                                            theme::palette().text_3,
                                        );
                                        ui.add_space(12.0);
                                        theme::text(
                                            ui,
                                            "No apps selected",
                                            15.0,
                                            theme::palette().text,
                                        );
                                        ui.add_space(6.0);
                                        note(ui, "Choose installed apps to include in Update All.");
                                        ui.add_space(6.0);
                                    });
                                });
                        });
                    } else {
                        theme::group().inner_margin(egui::Margin::symmetric(0, 8)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                            for (index, entry) in plan.entries.iter().enumerate() {
                                if index > 0 {
                                    rule(ui);
                                }
                                egui::Frame::new()
                                    .inner_margin(egui::Margin::symmetric(18, 14))
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.spacing_mut().item_spacing.y = 4.0;
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = 12.0;
                                            if let Some(texture) = self.icons.get(&entry.app) {
                                                ui.add(
                                                    egui::Image::new((
                                                        texture.id(),
                                                        egui::vec2(24.0, 24.0),
                                                    ))
                                                    .corner_radius(CornerRadius::same(6)),
                                                );
                                            }
                                            theme::text(ui, model::title(&entry.app), 14.0, theme::palette().text);
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    let (fill, color) = match entry.action.as_str() {
                                                        "Install" | "Update" => {
                                                            (theme::palette().accent_soft, theme::palette().accent_text)
                                                        }
                                                        "Error" => (theme::palette().red_bg, theme::palette().red),
                                                        _ => (theme::palette().button, theme::palette().text_3),
                                                    };
                                                    theme::badge(
                                                        ui,
                                                        format!("{} {}", if plan.requested_tag.is_some() && entry.action == "Update" { "Switch to" } else { &entry.action }, entry.version).trim(),
                                                        fill,
                                                        color,
                                                    );
                                                },
                                            );
                                        });
                                        if let Some(error) = &entry.error {
                                            ui.label(RichText::new(error).size(12.0).color(theme::palette().red));
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
                                                theme::hyperlink(
                                                    ui,
                                                    &asset.name,
                                                    &asset.browser_download_url,
                                                    12.0,
                                                );
                                            }
                                        }
                                    });
                            }
                        });
                    });
                    }
                });
                footer(ui, |ui| {
                    let count = plan.executable_count();
                    if action(
                        ui,
                        &if count == 0 {
                            "Update".into()
                        } else {
                            format!("Run {count} operation{}", if count == 1 { "" } else { "s" })
                        },
                        Kind::Primary,
                        count > 0,
                    )
                    .clicked()
                    {
                        self.release_plan = None;
                        let paths = self.paths.clone();
                        self.job =
                            Job::interactive(paths.at("logs/updates.log"), &self.build_preferences);
                        // A selected version is still a per-app install. Keep its
                        // target so the app page shows progress and install failures.
                        self.job_target = plan.requested_tag.as_ref().and_then(|_| {
                            plan.entries
                                .iter()
                                .find(|entry| {
                                    entry.error.is_none()
                                        && matches!(entry.action.as_str(), "Install" | "Update")
                                })
                                .map(|entry| (entry.app.clone(), "install-app".into()))
                        });
                        self.job_action = if self.job_target.is_some() {
                            "install-app"
                        } else {
                            "releases"
                        }
                        .into();
                        self.failure_dismissed = false;
                        self.operation_was_busy = true;
                        let plan = plan.clone();
                        self.job.describe("Bulk update", "Application library");
                        self.job
                            .spawn(move |job| updates::execute_plan(&paths, &plan, &job));
                    }
                    if action(ui, "Cancel", Kind::Secondary, true).clicked() {
                        self.release_plan = None;
                    }
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), 36.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            if plan.requested_tag.is_none()
                                && action(ui, "Choose…", Kind::Secondary, true).clicked()
                            {
                                self.selection_draft = plan.preferences.selected_apps.clone();
                                self.source_selection = false;
                                self.selection_from_review = true;
                                self.selection = true;
                            }
                        },
                    );
                });
            });
            if response.should_close() {
                self.release_plan = None;
            }
        }
    }

    pub(super) fn render_batch_selection(&mut self, ctx: &egui::Context) {
        if self.selection {
            let choices = if self.source_selection {
                model::sources()
            } else {
                model::apps()
            };
            let eligible: Vec<String> = if self.source_selection {
                choices.clone()
            } else {
                self.display_config
                    .as_ref()
                    .map(|config| {
                        config
                            .apps
                            .iter()
                            .filter(|app| updates::installed_for_updates(app, &self.settings_draft))
                            .map(|app| app.name.clone())
                            .collect()
                    })
                    .unwrap_or_default()
            };
            self.selection_draft.retain(|app| eligible.contains(app));
            let modal = modal(
                ctx,
                if self.source_selection {
                    "Choose source apps"
                } else {
                    "Choose release apps"
                },
                480.0,
                |ui| {
                    let top = ui.cursor().top();
                    band(ui, egui::Margin::symmetric(20, 16), 0.0, |ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        ui.spacing_mut().interact_size.y = 0.0;
                        theme::heading(
                            ui,
                            if self.source_selection {
                                "Sources in Update all sources"
                            } else {
                                "Apps in Update all"
                            },
                            16.0,
                        );
                        if self.source_selection {
                            theme::text(
                                ui,
                                "Choose which source ZIPs to update. ArtCraft X is source only.",
                                13.0,
                                theme::palette().text_3,
                            );
                        }
                        if !self.source_selection {
                            note(ui, "Update All only updates installed apps. Uninstalled apps cannot be selected.");
                        }
                        ui.horizontal(|ui| {
                            theme::text(
                                ui,
                                format!(
                                    "{} of {} selected",
                                    self.selection_draft.len(),
                                    choices.len()
                                ),
                                13.0,
                                theme::palette().muted,
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.spacing_mut().item_spacing.x = 14.0;
                                    if theme::link(ui, "Deselect all").clicked() {
                                        self.selection_draft.clear();
                                    }
                                    if theme::link(ui, "Select all").clicked() {
                                        self.selection_draft = eligible.clone();
                                    }
                                },
                            );
                        });
                    });
                    rule(ui);
                    let header = ui.cursor().top() - top;
                    let total = (ctx.screen_rect().height() - 48.0).min(600.0);
                    let height = (total - 2.0 - header - BAR_HEIGHT).max(120.0);
                    egui::ScrollArea::vertical()
                        .id_salt("selection-scroll")
                        .auto_shrink([false, false])
                        .max_height(height)
                        .min_scrolled_height(height)
                        .show(ui, |ui| {
                            band(ui, egui::Margin::symmetric(12, 8), height, |ui| {
                                for name in &choices {
                                    let name = name.as_str();
                                    let enabled = eligible.iter().any(|app| app == name);
                                    let checked = enabled && self.selection_draft.iter().any(|s| s == name);
                                    let (rect, response) = ui.allocate_exact_size(
                                        egui::vec2(ui.available_width(), 48.0),
                                        if enabled { Sense::click() } else { Sense::hover() },
                                    );
                                    response.widget_info(|| {
                                        egui::WidgetInfo::selected(
                                            egui::WidgetType::Checkbox,
                                            enabled,
                                            checked,
                                            model::title(name),
                                        )
                                    });
                                    if response.hovered() && enabled {
                                        ui.painter().rect_filled(
                                            rect,
                                            CornerRadius::same(8),
                                            theme::palette().hover,
                                        );
                                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                    }
                                    if response.has_focus() {
                                        ui.painter().rect_stroke(
                                            rect,
                                            CornerRadius::same(8),
                                            Stroke::new(2.0_f32, theme::palette().link),
                                            StrokeKind::Inside,
                                        );
                                    }
                                    let mark = egui::Rect::from_min_size(
                                        egui::pos2(rect.left() + 12.0, rect.center().y - 8.0),
                                        egui::Vec2::splat(16.0),
                                    );
                                    theme::paint_check(
                                        ui,
                                        mark,
                                        checked,
                                        theme::palette().accent,
                                        enabled,
                                    );
                                    if let Some(texture) = self.icons.get(name) {
                                        ui.painter().image(
                                            texture.id(),
                                            egui::Rect::from_min_size(
                                                egui::pos2(
                                                    mark.right() + 15.0,
                                                    rect.center().y - 12.0,
                                                ),
                                                egui::Vec2::splat(24.0),
                                            ),
                                            egui::Rect::from_min_max(
                                                egui::pos2(0.0, 0.0),
                                                egui::pos2(1.0, 1.0),
                                            ),
                                            if enabled {
                                                Color32::WHITE
                                            } else if ui.visuals().dark_mode {
                                                Color32::from_white_alpha(80)
                                            } else {
                                                Color32::from_rgba_unmultiplied(160, 160, 160, 60)
                                            },
                                        );
                                    }
                                    theme::painter_text(
                                        ui.painter(),
                                        egui::pos2(mark.right() + 51.0, rect.center().y),
                                        egui::Align2::LEFT_CENTER,
                                        model::title(name),
                                        14.0,
                                        if enabled {
                                            theme::palette().text
                                        } else if ui.visuals().dark_mode {
                                            theme::palette().muted
                                        } else {
                                            Color32::from_rgb(153, 153, 153)
                                        },
                                    );
                                    if !enabled { response.clone().on_hover_text("Not installed in the selected format. Install this app from its app page first."); }
                                    if response.clicked() && enabled {
                                        if checked {
                                            self.selection_draft.retain(|s| s != name)
                                        } else {
                                            self.selection_draft.push(name.into())
                                        }
                                    }
                                }
                            });
                        });
                    footer(ui, |ui| {
                        if action(ui, "Save", Kind::Primary, true).clicked() {
                            if self.selection_from_review {
                                match craft_apps_manager::settings::select_release_apps(
                                    &self.paths,
                                    &self.selection_draft,
                                ) {
                                    Ok(preferences) => {
                                        self.preferences = preferences.clone();
                                        self.settings_draft = preferences;
                                        self.selection = false;
                                        self.selection_from_review = false;
                                        self.release_plan = None;
                                        self.begin_release_review();
                                    }
                                    Err(error) => self.error = Some(format!("{error:#}")),
                                }
                            } else if self.source_selection {
                                self.settings_draft.selected_sources = self.selection_draft.clone();
                                self.selection = false;
                            } else {
                                self.settings_draft.selected_apps = self.selection_draft.clone();
                                self.selection = false;
                            }
                        }
                        if action(ui, "Cancel", Kind::Secondary, true).clicked() {
                            self.selection = false;
                            self.selection_from_review = false;
                        }
                    });
                },
            );
            if modal.should_close() {
                self.selection = false;
                self.selection_from_review = false;
            }
        }
    }
}
