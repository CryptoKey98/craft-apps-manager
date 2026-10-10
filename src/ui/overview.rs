//! Home: app updates, launch tiles and discovery, with operation status on the right.

use super::theme::{self, btn, Icon, Size};

use super::App;

use craft_apps_manager::{jobs::State, model};

use eframe::egui::{
    self, Align, Color32, CornerRadius, FontId, Layout, Rect, RichText, Sense, Ui, UiBuilder, Vec2,
};

use std::sync::atomic::Ordering;

/// The status dot while an operation runs.
#[derive(Clone)]
struct HomeTileDrag {
    name: String,
    installed: bool,
    pointer_offset: Vec2,
}

const BUSY_DOT: Color32 = Color32::from_rgb(0x5b, 0x8c, 0xff);

impl App {
    pub(super) fn overview(&mut self, ctx: &egui::Context, state: &State) {
        egui::SidePanel::right("overview-side")
            .resizable(false)
            .exact_width(320.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::palette().panel)
                    .inner_margin(egui::Margin {
                        // One more point on the left for the panel's border line.
                        left: 21,

                        right: 20,

                        top: 20,

                        bottom: 20,
                    }),
            )
            .show(ctx, |ui| self.overview_side(ui, state));

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::palette().bg))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Frame::new()
                            .inner_margin(egui::Margin::symmetric(40, 32))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());

                                self.overview_main(ui, state);
                            });
                    });
            });
    }

    fn overview_main(&mut self, ui: &mut Ui, state: &State) {
        let order = self.preferences.home_app_order.clone();
        let statuses: Vec<_> = order.iter().map(|name| self.status(name)).collect();
        let installed = statuses
            .iter()
            .filter(|status| status.installed.is_some())
            .count();
        let chosen = self.display_config.as_ref().map_or(0, |config| {
            config
                .apps
                .iter()
                .filter(|app| {
                    self.preferences.selected_apps.contains(&app.name)
                        && craft_apps_manager::updates::installed_for_updates(
                            app,
                            &self.preferences,
                        )
                })
                .count()
        });
        let updates = statuses
            .iter()
            .filter(|status| status.update.is_some())
            .count();
        let checking = statuses.iter().any(|status| status.checking);
        let checked = statuses.iter().any(|status| status.check.is_some());
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 4.0);
        text_line(ui, "Home", theme::bold(26.0), theme::palette().text, 31.2);
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!(
                "Your creative apps, in one place · {installed} installed"
            ))
            .size(14.0)
            .color(theme::palette().muted),
        );
        ui.add_space(24.0);
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(18, 16))
                .show(ui, |ui| {
                    let summary = if checking {
                        "Checking for updates…".to_owned()
                    } else {
                        match updates {
                            1 => "1 update available".to_owned(),
                            n if n > 1 => format!("{n} updates available"),
                            _ if checked => "No updates found".to_owned(),
                            _ => "Keep your apps current".to_owned(),
                        }
                    };
                    let detail = if self.display_config.is_none() {
                        "Loading installed apps…".to_owned()
                    } else if installed == 0 {
                        "No apps installed. Install an app to get started.".to_owned()
                    } else if chosen == 0 {
                        "No apps selected. Choose apps to include in Update All.".to_owned()
                    } else {
                        format!(
                            "{} {} chosen · Review changes before updating",
                            chosen,
                            if chosen == 1 { "app" } else { "apps" }
                        )
                    };
                    let action = btn("Update All…")
                        .primary()
                        .size(Size::Medium)
                        .icon(Icon::Refresh)
                        .enabled(!state.busy && installed > 0);
                    let action_size = action.desired_size(ui);
                    // Reserve the full bar width; anchor a fixed-size action to its right edge.
                    let (bar, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 44.0),
                        Sense::hover(),
                    );
                    let text_right = (bar.right() - action_size.x - 16.0).max(bar.left());
                    let mut text = ui.new_child(
                        UiBuilder::new()
                            .max_rect(Rect::from_min_max(
                                bar.min,
                                egui::pos2(text_right, bar.bottom()),
                            ))
                            .layout(Layout::top_down(Align::Min)),
                    );
                    text.add(
                        egui::Label::new(RichText::new(&summary).font(theme::medium(16.0)).color(
                            if updates > 0 {
                                theme::palette().link
                            } else {
                                theme::palette().text
                            },
                        ))
                        .truncate(),
                    );
                    text.add(
                        egui::Label::new(
                            RichText::new(&detail)
                                .size(12.0)
                                .color(theme::palette().muted),
                        )
                        .truncate(),
                    );
                    let mut right = ui.new_child(
                        UiBuilder::new()
                            .max_rect(bar)
                            .layout(Layout::right_to_left(Align::Center)),
                    );
                    let requested = action
                        .show(&mut right)
                        .on_hover_text("Review updates for installed apps chosen in Settings")
                        .on_disabled_hover_text(if installed == 0 {
                            "Install an app before using Update All."
                        } else {
                            "Wait for the current operation to finish."
                        })
                        .clicked();
                    if requested {
                        self.start("releases");
                    }
                });
        });
        ui.add_space(28.0);
        self.home_tiles(ui, state, &order, true);
        ui.add_space(24.0);
        self.home_tiles(ui, state, &order, false);
    }

    fn home_tiles(&mut self, ui: &mut Ui, state: &State, order: &[String], installed: bool) {
        let names: Vec<_> = order
            .iter()
            .filter(|name| self.status(name).installed.is_some() == installed)
            .filter(|name| !self.preferences.hidden_apps.contains(name))
            .cloned()
            .collect();
        ui.label(
            RichText::new(format!(
                "{} · {}",
                if installed {
                    "Installed apps"
                } else {
                    "Discover apps"
                },
                names.len()
            ))
            .font(theme::bold(18.0))
            .color(theme::palette().text),
        );
        ui.add_space(12.0);
        if names.is_empty() {
            ui.label(
                RichText::new(if installed {
                    "Your installed apps will appear here. Choose an app below to get started."
                } else {
                    "All available apps are already installed."
                })
                .color(theme::palette().muted),
            );
            return;
        }
        ui.label(
            RichText::new("Drag the grip to arrange apps")
                .size(12.0)
                .color(theme::palette().muted),
        );
        ui.add_space(8.0);
        let gap = 12.0;
        let columns = ((ui.available_width() + gap) / 168.0).floor().max(1.0) as usize;
        let width =
            ((ui.available_width() - gap * (columns - 1) as f32) / columns as f32).max(80.0);
        let size = egui::vec2(width, 158.0);
        let rows = names.len().div_ceil(columns);
        let (grid, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), rows as f32 * (size.y + gap) - gap),
            Sense::hover(),
        );
        let slot = |index: usize| {
            Rect::from_min_size(
                grid.min
                    + egui::vec2(
                        (index % columns) as f32 * (width + gap),
                        (index / columns) as f32 * (size.y + gap),
                    ),
                size,
            )
        };
        let pointer = ui.ctx().input(|i| i.pointer.interact_pos());
        let payload = egui::DragAndDrop::payload::<HomeTileDrag>(ui.ctx()).filter(|drag| {
            drag.installed == installed && names.contains(&drag.name) && !state.busy
        });
        let motion_id = ui.id().with(("home-drag-motion", installed));
        let now = ui.ctx().input(|input| input.time);
        if payload.is_some() {
            ui.ctx().data_mut(|data| data.insert_temp(motion_id, now));
        }
        let animate = ui
            .ctx()
            .data_mut(|data| data.get_temp::<f64>(motion_id))
            .is_some_and(|time| now - time < 0.22);
        let duration = if animate { 0.16 } else { 0.0 };
        let mut preview = names.clone();
        let preview_id = ui.id().with(("home-drop-slot", installed));
        if let Some(drag) = &payload {
            let mut destination = ui
                .ctx()
                .data_mut(|data| data.get_temp::<usize>(preview_id))
                .unwrap_or_else(|| names.iter().position(|name| *name == drag.name).unwrap());
            if let Some(pos) = pointer.filter(|pos| grid.contains(*pos)) {
                let col = ((pos.x - grid.left()) / (width + gap)).floor().max(0.0) as usize;
                let row = ((pos.y - grid.top()) / (size.y + gap)).floor().max(0.0) as usize;
                destination = (row * columns + col.min(columns - 1)).min(names.len() - 1);
                ui.ctx()
                    .data_mut(|data| data.insert_temp(preview_id, destination));
            }
            preview.retain(|name| *name != drag.name);
            preview.insert(destination.min(preview.len()), drag.name.clone());
        } else {
            ui.ctx().data_mut(|data| data.remove::<usize>(preview_id));
        }
        for (index, name) in preview.iter().enumerate() {
            let target = slot(index);
            let position_id = ui.id().with(("home-tile-position", installed, name));
            let x = ui.ctx().animate_value_with_time(
                position_id.with("x"),
                (index % columns) as f32 * (width + gap),
                duration,
            );
            let y = ui.ctx().animate_value_with_time(
                position_id.with("y"),
                (index / columns) as f32 * (size.y + gap),
                duration,
            );
            let rect = Rect::from_min_size(grid.min + egui::vec2(x, y), size);
            if payload.as_ref().is_some_and(|drag| drag.name == *name) {
                ui.painter()
                    .rect_filled(target, CornerRadius::same(10), theme::palette().field);
                ui.painter().rect_stroke(
                    target.shrink(1.0),
                    CornerRadius::same(10),
                    egui::Stroke::new(1.0_f32, theme::palette().accent_border),
                    egui::StrokeKind::Inside,
                );
                // Keep the original handle alive while the floating tile is painted above it.
                let grip_rect =
                    Rect::from_min_size(rect.min + egui::vec2(6.0, 6.0), Vec2::splat(24.0));
                ui.interact(grip_rect, ui.id().with(("home-grip", name)), Sense::drag());
                continue;
            }
            ui.ctx()
                .animate_bool_with_time(ui.id().with(("home-lift", name)), false, 0.12);
            self.home_tile(ui, state, name, installed, rect, false);
        }
        if let (Some(drag), Some(pos)) = (&payload, pointer) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            let lift = ui.ctx().animate_bool_with_time(
                ui.id().with(("home-lift", &drag.name)),
                true,
                0.12,
            );
            let rect = Rect::from_min_size(
                pos - drag.pointer_offset - egui::vec2(0.0, 4.0 * lift),
                size,
            )
            .expand(2.0 * lift);
            let layer =
                egui::LayerId::new(egui::Order::Tooltip, ui.id().with("home-floating-tile"));
            let mut floating = ui.new_child(
                UiBuilder::new()
                    .layer_id(layer)
                    .max_rect(rect)
                    .layout(Layout::top_down(Align::Min)),
            );
            floating.set_clip_rect(ui.ctx().screen_rect());
            for spread in (1..=8).rev() {
                floating.painter().rect_filled(
                    rect.translate(egui::vec2(0.0, 5.0)).expand(spread as f32),
                    CornerRadius::same(12),
                    Color32::from_black_alpha(5),
                );
            }
            self.home_tile(&mut floating, state, &drag.name, installed, rect, true);
            ui.ctx().request_repaint();
        }
        if ui.ctx().input(|i| i.pointer.any_released()) {
            if let Some(drag) = payload {
                if pointer.is_some_and(|pos| grid.contains(pos)) && preview != names {
                    let index = preview.iter().position(|name| *name == drag.name).unwrap();
                    let (target, before) = if index + 1 < preview.len() {
                        (&preview[index + 1], true)
                    } else {
                        (&preview[index - 1], false)
                    };
                    match craft_apps_manager::settings::reorder_home(
                        &self.paths,
                        &drag.name,
                        target,
                        before,
                    ) {
                        Ok(preferences) => {
                            self.preferences.home_app_order = preferences.home_app_order.clone();
                            self.settings_draft.home_app_order = preferences.home_app_order;
                        }
                        Err(error) => {
                            self.error = Some(format!("Could not save Home order: {error:#}"))
                        }
                    }
                }
                egui::DragAndDrop::take_payload::<HomeTileDrag>(ui.ctx());
                ui.ctx().data_mut(|data| data.remove::<usize>(preview_id));
            }
        }
    }

    fn home_tile(
        &mut self,
        ui: &mut Ui,
        state: &State,
        name: &str,
        installed: bool,
        rect: Rect,
        floating: bool,
    ) {
        let status = self.status(name);
        let palette = theme::palette();
        let response = ui.interact(
            rect,
            ui.id().with(("home-tile", name)),
            if floating {
                Sense::hover()
            } else {
                Sense::click()
            },
        );
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(10),
            if response.hovered() || floating {
                palette.hover
            } else {
                palette.card
            },
        );
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(10),
            egui::Stroke::new(
                1.0_f32,
                if floating {
                    palette.border_strong
                } else if status.update.is_some() {
                    palette.accent_border
                } else {
                    palette.border
                },
            ),
            egui::StrokeKind::Inside,
        );
        let grip_rect = Rect::from_min_size(rect.min + egui::vec2(6.0, 6.0), Vec2::splat(24.0));
        if !floating {
            let grip = ui
                .interact(
                    grip_rect,
                    ui.id().with(("home-grip", name)),
                    if state.busy {
                        Sense::hover()
                    } else {
                        Sense::drag()
                    },
                )
                .on_hover_cursor(egui::CursorIcon::Grab)
                .on_hover_text("Drag to reorder on Home");
            let offset = ui
                .ctx()
                .input(|i| i.pointer.press_origin())
                .unwrap_or(grip_rect.center())
                - rect.min;
            grip.dnd_set_drag_payload(HomeTileDrag {
                name: name.into(),
                installed,
                pointer_offset: offset,
            });
        }
        for x in [0.0, 5.0] {
            for y in [0.0, 5.0, 10.0] {
                ui.painter().circle_filled(
                    grip_rect.center() + egui::vec2(x - 2.5, y - 5.0),
                    1.2,
                    palette.muted,
                );
            }
        }
        if status.update.is_some() {
            ui.painter().circle_filled(
                egui::pos2(rect.right() - 12.0, rect.top() + 12.0),
                4.0,
                palette.link,
            );
        }
        if let Some(texture) = self.icons.get(name) {
            let image = Rect::from_min_size(
                egui::pos2(rect.center().x - 24.0, rect.top() + 16.0),
                Vec2::splat(48.0),
            );
            egui::Image::new((texture.id(), image.size()))
                .corner_radius(CornerRadius::same(10))
                .paint_at(ui, image);
        }
        let mut tile = ui.new_child(
            UiBuilder::new()
                .id_salt(("home-content", name, floating))
                .max_rect(rect.shrink2(egui::vec2(8.0, 8.0)))
                .layout(Layout::top_down(Align::Center)),
        );
        tile.add_space(64.0);
        tile.add(
            egui::Label::new(
                RichText::new(model::title(name))
                    .font(theme::medium(14.0))
                    .color(palette.text),
            )
            .truncate(),
        );
        let caption = match (&status.installed, &status.update) {
            (_, Some(version)) => format!("Update {version} available"),
            (Some(app), _) => format!("Installed {}", app.version),
            _ => "Available to install".into(),
        };
        tile.add(
            egui::Label::new(
                RichText::new(caption)
                    .size(11.0)
                    .color(if status.update.is_some() {
                        palette.link
                    } else {
                        palette.muted
                    }),
            )
            .truncate(),
        );
        tile.add_space(8.0);
        if floating {
            tile.disable();
        }
        if installed {
            if status.update.is_some() {
                tile.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let action_width =
                        btn("Update").desired_size(ui).x + btn("Open").desired_size(ui).x + 4.0;
                    ui.add_space(((ui.available_width() - action_width) / 2.0).max(0.0));
                    if btn("Update")
                        .primary()
                        .enabled(!state.busy && !status.checking)
                        .show(ui)
                        .clicked()
                    {
                        self.confirm_install = Some(name.into());
                    }
                    if btn("Open").enabled(!state.busy).show(ui).clicked() {
                        self.result(craft_apps_manager::apps::launch(&self.paths, name));
                    }
                });
            } else if btn("Open")
                .icon(Icon::Play)
                .enabled(!state.busy)
                .show(&mut tile)
                .clicked()
            {
                self.result(craft_apps_manager::apps::launch(&self.paths, name));
            }
        } else if btn("Install")
            .icon(Icon::Download)
            .enabled(!state.busy)
            .show(&mut tile)
            .clicked()
        {
            self.confirm_install = Some(name.into());
        }
        if !floating {
            self.app_visibility_menu(&response, name);
        }
        if !floating && response.clicked() {
            self.select(name);
        }
    }

    fn overview_side(&mut self, ui: &mut Ui, state: &State) {
        ui.spacing_mut().item_spacing = Vec2::ZERO;

        theme::group().show(ui, |ui| {
            ui.set_width(ui.available_width());

            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(14, 12))
                .show(ui, |ui| self.status_card(ui, state));
        });

        ui.add_space(20.0);

        // Section label on the left and "Open log" on the right, centred on one line.

        let link_font = FontId::proportional(13.0);

        let height = row_height(ui, &link_font).max(row_height(ui, &theme::medium(12.0)));

        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());

        let mut left = ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Center)),
        );

        theme::section_label(&mut left, "Activity");

        let mut right = ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::right_to_left(Align::Center)),
        );

        let exists = self.job.log_path.is_file();

        let response = right
            .add_enabled(
                exists,
                egui::Label::new(
                    RichText::new("Open log")
                        .font(link_font)
                        .color(theme::palette().link)
                        .underline(),
                )
                .sense(Sense::click()),
            )
            .on_hover_text(self.job.log_path.display().to_string())
            .on_disabled_hover_text("No log file yet. Run an operation to create one.");

        if response.clicked() {
            let result = craft_apps_manager::platform::open(&self.job.log_path);

            self.result(result);
        }

        ui.add_space(8.0);

        theme::log_view(
            ui,
            if state.log.trim().is_empty() {
                "Ready. Craft app checks run only when requested or scheduled."
            } else {
                state.log.trim_start_matches(['\n', '\r'])
            },
            true,
        );
    }

    /// Dot, stage and detail, with Cancel on the right and the progress below while busy.
    fn status_card(&mut self, ui: &mut Ui, state: &State) {
        let failed = !state.busy && state.stage == "Failed";

        let width = ui.available_width();

        let cancel_width = if state.busy {
            let label = if self.job.cancel.load(Ordering::Relaxed) {
                "Cancel requested…"
            } else {
                "Cancel"
            };

            text_width(ui, label, FontId::proportional(13.0)) + 24.0 + 10.0
        } else {
            0.0
        };

        let text_left = 8.0 + 10.0;

        let text_width = (width - text_left - cancel_width).max(40.0);

        let title = if state.stage.is_empty() {
            "Ready"
        } else {
            &state.stage
        };

        let title_height = ui.fonts(|f| {
            f.layout(
                title.to_owned(),
                FontId::proportional(14.0),
                theme::palette().text,
                text_width,
            )
            .size()
            .y
        });

        let detail_height = if state.detail.is_empty() {
            0.0
        } else {
            ui.fonts(|f| {
                f.layout(
                    state.detail.clone(),
                    FontId::proportional(12.0),
                    theme::palette().muted,
                    text_width,
                )
                .size()
                .y
            })
        };

        let detail_gap = if state.detail.is_empty() {
            0.0
        } else {
            ui.spacing().item_spacing.y
        };

        let text_height = title_height + detail_gap + detail_height;

        let height = text_height.max(if state.busy { 32.0 } else { 0.0 });

        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());

        ui.painter().circle_filled(
            egui::pos2(rect.left() + 4.0, rect.center().y),
            4.0,
            if state.busy {
                BUSY_DOT
            } else if failed {
                theme::palette().red
            } else {
                theme::palette().green
            },
        );

        let mut text = ui.new_child(
            UiBuilder::new()
                .max_rect(Rect::from_min_size(
                    egui::pos2(rect.left() + text_left, rect.center().y - text_height / 2.0),
                    egui::vec2(text_width, text_height),
                ))
                .layout(Layout::top_down(Align::Min)),
        );

        text.add(
            egui::Label::new(RichText::new(title).size(14.0).color(theme::palette().text)).wrap(),
        );

        if !state.detail.is_empty() {
            text.add(
                egui::Label::new(RichText::new(&state.detail).size(12.0).color(if failed {
                    theme::palette().red_text
                } else {
                    theme::palette().muted
                }))
                .wrap(),
            );
        }

        if state.busy {
            let mut action = ui.new_child(
                UiBuilder::new()
                    .max_rect(rect)
                    .layout(Layout::right_to_left(Align::Center)),
            );

            self.cancel_button(&mut action, &state.stage);
        }
        let opacity = if state.busy {
            1.0
        } else if state.stage == "Complete" {
            self.completed_progress_opacity(ui.ctx())
        } else {
            0.0
        };
        if opacity > 0.0 {
            ui.add_space(10.0);
            ui.scope(|ui| {
                ui.multiply_opacity(opacity);
                theme::progress(
                    ui,
                    if state.busy {
                        state.progress
                    } else {
                        Some(1.0)
                    },
                    4.0,
                );
            });
        }
    }
}

/// The height egui gives one line of text in `font`.
fn row_height(ui: &Ui, font: &FontId) -> f32 {
    ui.fonts(|f| f.row_height(font))
}

fn text_width(ui: &Ui, text: &str, font: FontId) -> f32 {
    ui.fonts(|f| {
        f.layout_no_wrap(text.to_owned(), font, Color32::PLACEHOLDER)
            .size()
            .x
    })
}

/// One line of text in a line box of `line_height`, centred like CSS half-leading.
fn text_line(ui: &mut Ui, text: &str, font: FontId, color: Color32, line_height: f32) {
    let font_height = row_height(ui, &font);

    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), line_height),
        Sense::hover(),
    );

    // egui sets glyphs about a point lower in their row than the browser does.

    let top = rect.top() + (line_height - font_height) / 2.0 - 1.0;

    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(Rect::from_min_max(
                egui::pos2(rect.left(), top),
                egui::pos2(rect.right(), top + font_height),
            ))
            .layout(Layout::top_down(Align::Min)),
    );

    child.add(egui::Label::new(RichText::new(text).font(font).color(color)).truncate());
}

pub(super) fn release_format_label(format: &str) -> &'static str {
    if format == "installer" {
        "Installer"
    } else if cfg!(target_os = "linux") {
        "AppImage"
    } else if cfg!(target_os = "macos") {
        "Portable app"
    } else {
        "Portable ZIP"
    }
}
