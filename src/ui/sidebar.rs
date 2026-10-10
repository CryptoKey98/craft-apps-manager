//! Window chrome: the top bar, the app list on the left and the status bar.
use super::theme::{self, btn, Icon, Kind, Size};
use super::{App, Page};
use craft_apps_manager::{jobs::State, model, settings};
use eframe::egui::{self, Color32, CornerRadius, FontId, Rect, Sense, Stroke};

/// Height of one CSS text line in the mockups: IBM Plex Sans renders at 1.327 em.
const LINE: f32 = 1.327;
/// Placeholder colour of the mockup's search field.
const PLACEHOLDER: Color32 = Color32::from_rgb(0x75, 0x75, 0x75);
/// Border of the "Get" chip.
/// Status-bar dot while an operation or a build runs.
const BUSY_DOT: Color32 = Color32::from_rgb(0x5b, 0x8c, 0xff);
// The top bar draws its own pill now, which leaves `Size::Pill` without a caller.
// Remove this line together with that variant in theme.rs.

#[derive(Clone)]
struct SidebarDrag {
    name: String,
    installed: bool,
    offset: egui::Vec2,
}

impl App {
    pub(super) fn top_bar(&mut self, ctx: &egui::Context, state: &State) {
        let building = self.build_job.state.lock().unwrap().busy;
        egui::TopBottomPanel::top("header")
            .frame(
                egui::Frame::new()
                    .fill(theme::palette().panel)
                    // The bottom point holds the separator line egui draws inside the panel.
                    .inner_margin(egui::Margin {
                        left: 20,
                        right: 16,
                        top: 10,
                        bottom: 11,
                    }),
            )
            .show(ctx, |ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), 36.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_min_height(36.0);
                        ui.spacing_mut().item_spacing.x = 10.0;
                        ui.label(
                            egui::RichText::new("Craft Apps Manager")
                                .font(theme::bold(15.0))
                                .color(theme::palette().text),
                        );
                        theme::text(
                            ui,
                            format!("Version {}", env!("CARGO_PKG_VERSION")),
                            12.0,
                            theme::palette().muted,
                        );
                        if self.manager_plan.is_some() {
                            ui.spinner();
                            theme::text(
                                ui,
                                "Downloading update…",
                                12.0,
                                theme::palette().accent_text,
                            );
                        } else if let Some(available) = &self.manager_available {
                            let version = available.version.clone();
                            if btn("Update available")
                                .kind(Kind::Soft)
                                .size(Size::Pill)
                                .icon(Icon::ArrowUp)
                                .icon_weight(2.5)
                                .enabled(!state.busy && !building)
                                .show(ui)
                                .on_hover_text(format!("Craft Apps Manager {version} is available"))
                                .on_disabled_hover_text(if building {
                                    "Wait for the build to finish."
                                } else {
                                    "Wait for the current operation to finish."
                                })
                                .clicked()
                            {
                                self.confirm_self_update = true;
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if btn("Settings")
                                .size(Size::Toolbar)
                                .icon(Icon::Gear)
                                .show(ui)
                                .clicked()
                            {
                                self.open_settings();
                            }
                            if theme::theme_toggle(ui, self.preferences.theme).clicked() {
                                let result = (|| -> anyhow::Result<()> {
                                    let mut preferences = self.paths.read_preferences()?;
                                    preferences.theme =
                                        theme::resolved(ctx, self.preferences.theme).toggled();
                                    craft_apps_manager::files::write_json(
                                        &self.paths.at("manager-settings.json"),
                                        &preferences,
                                    )?;
                                    self.preferences.theme = preferences.theme;
                                    self.settings_draft.theme = preferences.theme;
                                    theme::apply(ctx, preferences.theme);
                                    ctx.request_repaint();
                                    Ok(())
                                })();
                                self.result(result);
                            }
                        });
                    },
                );
            });
    }

    pub(super) fn status_bar(&mut self, ctx: &egui::Context, state: &State) {
        let build = self.build_job.state.lock().unwrap().clone();
        egui::TopBottomPanel::bottom("footer")
            .frame(
                egui::Frame::new()
                    .fill(theme::palette().panel)
                    // The top point holds the separator line egui draws inside the panel.
                    .inner_margin(egui::Margin {
                        left: 16,
                        right: 16,
                        top: 1,
                        bottom: 0,
                    }),
            )
            .show(ctx, |ui| {
                let (color, text) = if state.busy {
                    (
                        BUSY_DOT,
                        if state.stage.is_empty() || state.stage == "Working" {
                            "Working".to_owned()
                        } else {
                            format!("Working · {}", state.stage)
                        },
                    )
                } else if build.busy {
                    (
                        BUSY_DOT,
                        format!(
                            "Building {} · {}",
                            model::title(&self.build_app),
                            build.stage
                        ),
                    )
                } else if state.stage == "Failed" {
                    (theme::palette().red, "Last operation failed".to_owned())
                } else {
                    (theme::palette().green, "Ready".to_owned())
                };
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), 30.0),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        ui.set_min_height(30.0);
                        ui.spacing_mut().item_spacing.x = 8.0;
                        theme::text(ui, text, 12.0, theme::palette().text_3);
                        theme::dot(ui, color);
                        ui.add_space(8.0);
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(format!(
                                        "Data: {}",
                                        self.paths.root.display()
                                    ))
                                    .size(12.0)
                                    .color(theme::palette().text_3),
                                )
                                .truncate(),
                            );
                        });
                    },
                );
            });
    }

    pub(super) fn sidebar(&mut self, ctx: &egui::Context, state: &State) {
        egui::SidePanel::left("sidebar")
            .resizable(false)
            // 264 points of list plus the separator line egui draws inside the panel.
            .exact_width(265.0)
            .frame(egui::Frame::new().fill(theme::palette().panel))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                egui::Frame::new()
                    .inner_margin(egui::Margin {
                        left: 12,
                        right: 13,
                        top: 14,
                        bottom: 6,
                    })
                    .show(ui, |ui| self.search_box(ui));
                egui::ScrollArea::vertical()
                    .id_salt("creative-app-list")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Frame::new()
                            .inner_margin(egui::Margin {
                                left: 8,
                                right: 9,
                                top: 4,
                                bottom: 12,
                            })
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                self.app_list(ui, state)
                            });
                    });
            });
    }

    fn search_box(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new()
            .fill(theme::palette().field)
            .stroke(Stroke::new(1.0_f32, theme::palette().border_strong))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(egui::Margin::symmetric(10, 0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.set_min_height(36.0);
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), Sense::hover());
                    theme::paint_icon(ui.painter(), rect, Icon::Search, theme::palette().muted);
                    // Lay out right to left so the clear button keeps its room.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let clear = !self.search.is_empty()
                            && theme::icon_button(
                                ui,
                                Icon::Close,
                                "Clear search",
                                theme::palette().muted,
                            )
                            .clicked();
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut self.search)
                                .hint_text(
                                    egui::RichText::new("Search apps")
                                        .size(13.0)
                                        .color(PLACEHOLDER),
                                )
                                .frame(false)
                                .margin(egui::Margin::ZERO)
                                .desired_width(ui.available_width())
                                .text_color(theme::palette().text)
                                .font(FontId::proportional(13.0)),
                        );
                        if clear {
                            self.search.clear();
                            response.request_focus();
                        }
                    });
                });
            });
    }

    fn app_list(&mut self, ui: &mut egui::Ui, state: &State) {
        let needle = self.search.trim().to_lowercase();
        let matches = |name: &str| {
            needle.is_empty()
                || format!("{} {}", model::title(name), model::category(name))
                    .to_lowercase()
                    .contains(&needle)
        };
        let order = self.preferences.app_order.clone();
        let updates = order
            .iter()
            .filter(|name| self.status(name).update.is_some())
            .count();
        if self.overview_row(ui, updates) {
            self.page = Page::Overview;
        }
        let mut reorder = None;
        let (installed, available): (Vec<_>, Vec<_>) = order
            .iter()
            .filter(|name| matches(name))
            .filter(|name| !self.preferences.hidden_apps.contains(name))
            .cloned()
            .partition(|name| self.status(name).installed.is_some());
        // Until the first snapshot loads nothing is known to be installed or not.
        let groups = if self.display_config.is_some() {
            vec![
                ("Installed", installed.clone(), true),
                ("Not installed", available.clone(), false),
            ]
        } else {
            vec![("Apps", available.clone(), false)]
        };
        for (label, group, is_installed) in groups {
            if group.is_empty() {
                continue;
            }
            ui.add_space(14.0);
            group_header(ui, label, group.len());
            let height = if is_installed { 48.0 } else { 47.5 };
            let step = height + 2.0;
            ui.add_space(2.0);
            let (area, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), group.len() as f32 * step - 2.0),
                Sense::hover(),
            );
            let pointer = ui.ctx().input(|i| i.pointer.interact_pos());
            let drag = egui::DragAndDrop::payload::<SidebarDrag>(ui.ctx()).filter(|drag| {
                drag.installed == is_installed && group.contains(&drag.name) && !state.busy
            });
            let motion_id = ui.id().with(("sidebar-drag-motion", is_installed));
            let now = ui.ctx().input(|input| input.time);
            if drag.is_some() {
                ui.ctx().data_mut(|data| data.insert_temp(motion_id, now));
            }
            let animate = ui
                .ctx()
                .data_mut(|data| data.get_temp::<f64>(motion_id))
                .is_some_and(|time| now - time < 0.22);
            let duration = if animate { 0.16 } else { 0.0 };
            let mut preview = group.clone();
            let drop_id = ui.id().with(("sidebar-drop-position", is_installed));
            if let Some(drag) = &drag {
                let mut destination = ui
                    .ctx()
                    .data_mut(|data| data.get_temp::<usize>(drop_id))
                    .unwrap_or_else(|| group.iter().position(|name| *name == drag.name).unwrap());
                if let Some(pos) = pointer.filter(|pos| area.contains(*pos)) {
                    destination = (((pos.y - area.top()) / step).floor().max(0.0) as usize)
                        .min(group.len() - 1);
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(drop_id, destination));
                }
                preview.retain(|name| *name != drag.name);
                preview.insert(destination.min(preview.len()), drag.name.clone());
            } else {
                ui.ctx().data_mut(|data| data.remove::<usize>(drop_id));
            }
            for (index, name) in preview.iter().enumerate() {
                let y = ui.ctx().animate_value_with_time(
                    ui.id().with(("sidebar-row-position", is_installed, name)),
                    index as f32 * step,
                    duration,
                );
                let rect = Rect::from_min_size(
                    area.min + egui::vec2(0.0, y),
                    egui::vec2(area.width(), height),
                );
                if drag.as_ref().is_some_and(|drag| drag.name == *name) {
                    let placeholder = Rect::from_min_size(
                        area.min + egui::vec2(0.0, index as f32 * step),
                        rect.size(),
                    );
                    ui.painter().rect_filled(
                        placeholder,
                        CornerRadius::same(8),
                        theme::palette().field,
                    );
                    ui.painter().rect_stroke(
                        placeholder.shrink(1.0),
                        CornerRadius::same(8),
                        Stroke::new(1.0_f32, theme::palette().accent_border),
                        egui::StrokeKind::Inside,
                    );
                    ui.interact(
                        Rect::from_center_size(
                            egui::pos2(
                                rect.left() + (16.0 + if is_installed { 32.0 } else { 28.0 }) / 2.0,
                                rect.center().y,
                            ),
                            egui::vec2(
                                16.0 + if is_installed { 32.0 } else { 28.0 },
                                rect.height() - 4.0,
                            ),
                        ),
                        ui.id().with(("sidebar-grip", name)),
                        Sense::drag(),
                    );
                    continue;
                }
                ui.ctx()
                    .animate_bool_with_time(ui.id().with(("sidebar-lift", name)), false, 0.12);
                self.app_row(ui, state, name, is_installed, rect, false);
            }
            if let (Some(drag), Some(pos)) = (&drag, pointer) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                let lift = ui.ctx().animate_bool_with_time(
                    ui.id().with(("sidebar-lift", &drag.name)),
                    true,
                    0.12,
                );
                let rect = Rect::from_min_size(
                    pos - drag.offset - egui::vec2(0.0, 3.0 * lift),
                    egui::vec2(area.width(), height),
                )
                .expand(lift);
                let layer =
                    egui::LayerId::new(egui::Order::Tooltip, ui.id().with("sidebar-floating-row"));
                let mut floating =
                    ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(rect));
                floating.set_clip_rect(ui.ctx().screen_rect());
                for spread in (1..=7).rev() {
                    floating.painter().rect_filled(
                        rect.translate(egui::vec2(0.0, 4.0)).expand(spread as f32),
                        CornerRadius::same(10),
                        Color32::from_black_alpha(5),
                    );
                }
                floating
                    .painter()
                    .rect_filled(rect, CornerRadius::same(8), theme::palette().card);
                floating.disable();
                self.app_row(&mut floating, state, &drag.name, is_installed, rect, true);
                ui.ctx().request_repaint();
            }
            if ui.ctx().input(|i| i.pointer.any_released()) {
                if let Some(drag) = drag {
                    if pointer.is_some_and(|pos| area.contains(pos)) && preview != group {
                        let index = preview.iter().position(|name| *name == drag.name).unwrap();
                        let (target, before) = if index + 1 < preview.len() {
                            (preview[index + 1].clone(), true)
                        } else {
                            (preview[index - 1].clone(), false)
                        };
                        reorder = Some((drag.name.clone(), target, before));
                    }
                    egui::DragAndDrop::take_payload::<SidebarDrag>(ui.ctx());
                    ui.ctx().data_mut(|data| data.remove::<usize>(drop_id));
                }
            }
        }
        if installed.is_empty() && available.is_empty() {
            ui.add_space(14.0 + 8.0);
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                theme::text(
                    ui,
                    format!("No apps match “{}”.", self.search.trim()),
                    13.0,
                    theme::palette().muted,
                );
            });
        }
        if let Some((dragged, target, before)) = reorder {
            match settings::reorder(&self.paths, &dragged, &target, before) {
                Ok(preferences) => {
                    if self.preferences.release_format != preferences.release_format
                        || self.preferences.architecture != preferences.architecture
                    {
                        self.check_generation += 1;
                        self.checking_apps.clear();
                        self.release_checks.clear();
                        self.display_config = None;
                        self.config_receiver = None;
                        self.config_refresh_at = std::time::Instant::now();
                    }
                    self.settings_draft = preferences.clone();
                    self.preferences = preferences;
                }
                Err(error) => self.error = Some(format!("Could not save app order: {error:#}")),
            }
        }
    }

    fn overview_row(&mut self, ui: &mut egui::Ui, updates: usize) -> bool {
        let selected = self.page == Page::Overview;
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 44.0), Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::SelectableLabel,
                true,
                selected,
                if updates > 0 {
                    format!("Home · {updates} update(s) available")
                } else {
                    "Home".into()
                },
            )
        });
        paint_row_background(ui, rect, selected, response.hovered());
        let tile = Rect::from_min_size(
            egui::pos2(rect.left() + 10.0, rect.center().y - 16.0),
            egui::vec2(32.0, 32.0),
        );
        ui.painter()
            .rect_filled(tile, CornerRadius::same(8), theme::palette().button);
        theme::paint_icon(
            ui.painter(),
            Rect::from_center_size(tile.center(), egui::vec2(16.0, 16.0)),
            Icon::Grid,
            theme::palette().text_2,
        );
        ui.painter().text(
            egui::pos2(tile.right() + 12.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            "Home",
            theme::medium(14.0),
            theme::palette().text,
        );
        if updates > 0 {
            let galley = ui.painter().layout_no_wrap(
                updates.to_string(),
                theme::bold(11.0),
                theme::palette().accent_text,
            );
            let width = (galley.size().x + 12.0).max(20.0);
            let badge = Rect::from_min_size(
                egui::pos2(rect.right() - 10.0 - width, rect.center().y - 10.0),
                egui::vec2(width, 20.0),
            );
            ui.painter()
                .rect_filled(badge, CornerRadius::same(10), theme::palette().accent_soft);
            ui.painter().galley(
                badge.center() - galley.size() / 2.0,
                galley,
                theme::palette().accent_text,
            );
        }
        response.clicked()
    }

    /// One app in the sidebar. Returns a reorder request when another row is dropped on it.
    fn app_row(
        &mut self,
        ui: &mut egui::Ui,
        state: &State,
        name: &str,
        is_installed: bool,
        rect: Rect,
        floating: bool,
    ) {
        let status = self.status(name);
        let selected = self.page == Page::App && self.app == name;
        let response = ui.interact(rect, ui.id().with(("sidebar-row", name)), Sense::click());
        let installing = state.busy
            && self
                .job_target
                .as_ref()
                .is_some_and(|(app, action)| app == name && action == "install-app");
        let format = &self.preferences.release_format;
        let other = status.alternate.as_ref().map(|a| {
            (
                if a.install_kind == "installer" {
                    if cfg!(target_os = "macos") {
                        "Applications"
                    } else {
                        "System install"
                    }
                } else {
                    "Portable"
                },
                a.version.clone(),
            )
        });
        // Full status for hover and screen readers, as the old list showed it.
        let hover = if let Some(installed) = &status.installed {
            format!(
                "Installed · {} · {format}{}",
                installed.version,
                other
                    .as_ref()
                    .map(|(label, version)| format!(" · also {label} {version}"))
                    .unwrap_or_default()
            )
        } else if let Some((label, version)) = &other {
            format!("{label} {version} · no {format} copy")
        } else if status.loaded {
            "Not installed".into()
        } else {
            "Checking…".into()
        };
        let response = response.on_hover_text(&hover);
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::SelectableLabel,
                true,
                selected,
                format!("{} · {hover}", model::title(name)),
            )
        });
        paint_row_background(ui, rect, selected, response.hovered());
        let grip_rect = Rect::from_center_size(
            egui::pos2(rect.left() + 7.0, rect.center().y),
            egui::vec2(12.0, 26.0),
        );
        let grab_rect = Rect::from_min_max(
            rect.min + egui::vec2(0.0, 2.0),
            egui::pos2(
                rect.left() + 16.0 + if is_installed { 32.0 } else { 28.0 },
                rect.bottom() - 2.0,
            ),
        );
        let mut grip_hot = floating;
        if !floating && !state.busy {
            let grip = ui
                .interact(
                    grab_rect,
                    ui.id().with(("sidebar-grip", name)),
                    Sense::drag(),
                )
                .on_hover_cursor(egui::CursorIcon::Grab)
                .on_hover_text("Drag to arrange sidebar apps");
            let offset = ui
                .ctx()
                .input(|i| i.pointer.press_origin())
                .unwrap_or(grip_rect.center())
                - rect.min;
            grip_hot = grip.hovered() || grip.dragged();
            grip.dnd_set_drag_payload(SidebarDrag {
                name: name.into(),
                installed: is_installed,
                offset,
            });
        }
        let hovered = floating
            || ui
                .ctx()
                .input(|i| i.pointer.hover_pos())
                .is_some_and(|pos| rect.contains(pos));
        let reveal = ui.ctx().animate_bool_with_time(
            ui.id().with(("sidebar-grip-reveal", name)),
            hovered && !state.busy,
            0.20,
        );
        // Ease the grip into its narrow gutter without shifting app content.
        let reveal = if floating {
            1.0
        } else {
            reveal * reveal * (3.0 - 2.0 * reveal)
        };
        let highlight = ui.ctx().animate_bool_with_time(
            ui.id().with(("sidebar-grip-highlight", name)),
            grip_hot && !state.busy,
            0.12,
        );
        let grip_color = Color32::from(egui::lerp(
            egui::Rgba::from(theme::palette().muted)..=egui::Rgba::from(theme::palette().link),
            highlight,
        ))
        .gamma_multiply((0.75 + 0.25 * highlight) * reveal);
        let grip_painter =
            ui.painter()
                .with_clip_rect(ui.clip_rect().intersect(Rect::from_min_max(
                    egui::pos2(rect.left(), rect.top()),
                    egui::pos2(rect.left() + 14.0, rect.bottom()),
                )));
        for x in [0.0, 3.0] {
            for y in [0.0, 3.0, 6.0] {
                grip_painter.circle_filled(
                    grip_rect.center() + egui::vec2(x - 1.5 - 9.0 * (1.0 - reveal), y - 3.0),
                    0.8,
                    grip_color,
                );
            }
        }
        let icon_size = if is_installed { 32.0 } else { 28.0 };
        let icon = Rect::from_min_size(
            egui::pos2(rect.left() + 16.0, rect.center().y - icon_size / 2.0),
            egui::vec2(icon_size, icon_size),
        );
        let radius = CornerRadius::same(if is_installed { 8 } else { 7 });
        if is_installed {
            if let Some(texture) = self.icons.get(name) {
                egui::Image::new((texture.id(), icon.size()))
                    .corner_radius(radius)
                    .paint_at(ui, icon);
            }
        } else if let Some(texture) = dimmed_icon(ui.ctx(), name) {
            egui::Image::new((texture.id(), icon.size()))
                .corner_radius(radius)
                .paint_at(ui, icon);
        } else if let Some(texture) = self.icons.get(name) {
            egui::Image::new((texture.id(), icon.size()))
                .corner_radius(radius)
                .tint(Color32::from_rgba_unmultiplied(170, 170, 170, 150))
                .paint_at(ui, icon);
        }
        let text_left = icon.right() + 12.0;
        let chip = if status.update.is_some() {
            Some("Update")
        } else if !is_installed && status.loaded && !installing {
            Some("Get")
        } else {
            None
        };
        // Update is a filled 20-point tag, Get an outlined 26-point chip.
        let chip_galley = chip.map(|chip| {
            ui.painter()
                .layout_no_wrap(chip.to_owned(), theme::bold(11.0), Color32::PLACEHOLDER)
        });
        let chip_rect = chip.zip(chip_galley.as_ref()).map(|(chip, galley)| {
            let (height, width) = if chip == "Update" {
                (20.0, galley.size().x + 16.0)
            } else {
                (26.0, galley.size().x + 22.0)
            };
            Rect::from_min_size(
                egui::pos2(rect.right() - 10.0 - width, rect.center().y - height / 2.0),
                egui::vec2(width, height),
            )
        });
        let text_right = chip_rect.map_or(rect.right() - 10.0, |chip| chip.left() - 12.0);
        let max_text = (text_right - text_left).max(1.0);
        let title = ui.painter().layout(
            model::title(name),
            if is_installed {
                theme::medium(14.0)
            } else {
                FontId::proportional(14.0)
            },
            if is_installed {
                theme::palette().text
            } else {
                theme::palette().text_3
            },
            max_text,
        );
        let (detail, detail_color) = if installing {
            (
                match state.progress {
                    Some(progress) => format!("Installing… {:.0}%", progress * 100.0),
                    None => "Installing…".into(),
                },
                theme::palette().link,
            )
        } else if let Some(installed) = &status.installed {
            if let Some(update) = &status.update {
                (
                    format!("{} {} {update}", installed.version, theme::arrow()),
                    theme::palette().link,
                )
            } else if status.checking {
                (
                    format!("{} · checking…", installed.version),
                    theme::palette().muted,
                )
            } else if matches!(status.check, Some(Ok(None))) {
                (
                    format!("{} · up to date", installed.version),
                    theme::palette().green,
                )
            } else {
                (
                    format!("Installed {}", installed.version),
                    theme::palette().green,
                )
            }
        } else if let Some((label, version)) = &other {
            (
                format!("{label} {version} · no {format} copy"),
                theme::palette().muted,
            )
        } else if !status.loaded {
            ("Checking…".into(), theme::palette().muted)
        } else {
            (model::category(name).to_owned(), theme::palette().muted)
        };
        let mut detail_job = egui::text::LayoutJob::simple_singleline(
            detail,
            FontId::proportional(12.0),
            detail_color,
        );
        detail_job.wrap.max_width = max_text;
        detail_job.wrap.max_rows = 1;
        let detail = ui.painter().layout_job(detail_job);
        // Two CSS lines 1 point apart, centred in the row, each glyph run centred in its line.
        let (title_line, detail_line) = (14.0 * LINE, 12.0 * LINE);
        let top = rect.center().y - (title_line + 1.0 + detail_line) / 2.0;
        let title_top = top + (title_line - title.size().y) / 2.0;
        let detail_top = top + title_line + 1.0 + (detail_line - detail.size().y) / 2.0;
        ui.painter().galley(
            egui::pos2(text_left, title_top),
            title,
            theme::palette().text,
        );
        ui.painter()
            .galley(egui::pos2(text_left, detail_top), detail, detail_color);
        let mut chip_clicked = false;
        if let (Some(chip), Some(chip_rect), Some(galley)) = (chip, chip_rect, chip_galley) {
            let label = format!(
                "{} {}",
                if chip == "Get" { "Install" } else { "Update" },
                model::title(name)
            );
            let id = ui.id().with(("install-app", name));
            let chip_response = ui
                .add_enabled_ui(!state.busy, |ui| ui.interact(chip_rect, id, Sense::click()))
                .inner
                .on_hover_text(format!("{label} (latest release)"))
                .on_disabled_hover_text("Wait for the current operation to finish.");
            chip_response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, !state.busy, &label)
            });
            let hovered = chip_response.hovered() && !state.busy;
            let (fill, stroke, color, radius) = if chip == "Update" {
                (
                    if hovered {
                        theme::palette().selected
                    } else {
                        theme::palette().accent_soft
                    },
                    Stroke::NONE,
                    theme::palette().accent_text,
                    10,
                )
            } else {
                (
                    if hovered {
                        theme::palette().button_hover
                    } else {
                        Color32::TRANSPARENT
                    },
                    Stroke::new(1.0_f32, theme::palette().border_strong),
                    theme::palette().text_2,
                    13,
                )
            };
            let alpha = if state.busy { 0.45 } else { 1.0 };
            ui.painter().rect(
                chip_rect,
                CornerRadius::same(radius),
                fill,
                stroke,
                egui::StrokeKind::Inside,
            );
            ui.painter().galley(
                chip_rect.center() - galley.size() / 2.0,
                galley,
                color.gamma_multiply(alpha),
            );
            if hovered {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            chip_clicked = chip_response.clicked();
        } else if status.checking {
            let spinner = Rect::from_center_size(
                egui::pos2(rect.right() - 20.0, rect.center().y),
                egui::vec2(14.0, 14.0),
            );
            // Not `ui.put`: allocating the spinner would move the next row up to its bottom.
            ui.new_child(egui::UiBuilder::new().max_rect(spinner))
                .add(egui::Spinner::new().size(12.0))
                .on_hover_text("Checking for updates…");
        }
        if !floating {
            self.app_visibility_menu(&response, name);
        }
        if !floating && chip_clicked {
            self.confirm_install = Some(name.into());
        } else if !floating && response.clicked() {
            response.request_focus();
            self.select(name);
        }
    }
}

fn paint_row_background(ui: &egui::Ui, rect: Rect, selected: bool, hovered: bool) {
    if selected || hovered {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(8),
            if selected {
                theme::palette().selected
            } else {
                theme::palette().hover
            },
        );
    }
}

/// "INSTALLED 3": an uppercase, letter-spaced group title with its count on the right.
fn group_header(ui: &mut egui::Ui, label: &str, count: usize) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 8.0 + 12.0 * LINE),
        Sense::hover(),
    );
    let text = |text: String| {
        egui::RichText::new(text)
            .font(theme::medium(12.0))
            .color(theme::palette().muted)
            .extra_letter_spacing(0.72)
    };
    // Padding 4 × 8 around one 12-point line.
    let inner = Rect::from_min_max(
        rect.min + egui::vec2(8.0, 4.0 + (12.0 * LINE - 12.0 * 1.3) / 2.0),
        rect.max - egui::vec2(8.0, 4.0),
    );
    // A child that does not allocate again: the row above already holds the space.
    let mut ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(egui::Align::Min)),
    );
    ui.label(text(label.to_uppercase()));
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        ui.label(text(count.to_string()));
    });
}

/// The not-installed look of an app icon: CSS `grayscale(0.7)` and `opacity(0.6)`,
/// made once per icon and kept in egui's memory.
fn dimmed_icon(ctx: &egui::Context, name: &str) -> Option<egui::TextureHandle> {
    let id = egui::Id::new(("dimmed-app-icon", name));
    if let Some(texture) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return Some(texture);
    }
    let image = image::load_from_memory(super::app_icon(name)?)
        .ok()?
        .into_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let mut pixels = image.into_raw();
    let k = 1.0 - 0.7;
    for px in pixels.as_chunks_mut::<4>().0 {
        let (r, g, b) = (px[0] as f32, px[1] as f32, px[2] as f32);
        let out = [
            (0.2126 + 0.7874 * k) * r + (0.7152 - 0.7152 * k) * g + (0.0722 - 0.0722 * k) * b,
            (0.2126 - 0.2126 * k) * r + (0.7152 + 0.2848 * k) * g + (0.0722 - 0.0722 * k) * b,
            (0.2126 - 0.2126 * k) * r + (0.7152 - 0.7152 * k) * g + (0.0722 + 0.9278 * k) * b,
        ];
        // Premultiply in sRGB, as the browser composites; egui's own
        // premultiplication is linear and would leave the icon brighter.
        let alpha = px[3] as f32 / 255.0 * 0.6;
        for (channel, value) in px.iter_mut().zip(out) {
            *channel = (value * alpha).round().clamp(0.0, 255.0) as u8;
        }
        px[3] = (alpha * 255.0).round() as u8;
    }
    let texture = ctx.load_texture(
        format!("{name}-dimmed"),
        egui::ColorImage::from_rgba_premultiplied(size, &pixels),
        egui::TextureOptions::LINEAR,
    );
    ctx.data_mut(|d| d.insert_temp(id, texture.clone()));
    Some(texture)
}
