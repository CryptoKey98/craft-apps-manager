//! Window chrome: the top bar, the app list on the left and the status bar.
use super::theme::{self, btn, Icon, Kind, Size};
use super::{App, Page};
use craft_apps_manager::{jobs::State, model, settings};
use eframe::egui::{self, Color32, CornerRadius, Rect, Sense, Stroke};

impl App {
    pub(super) fn top_bar(&mut self, ctx: &egui::Context, state: &State) {
        let building = self.build_job.state.lock().unwrap().busy;
        egui::TopBottomPanel::top("header")
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .inner_margin(egui::Margin {
                        left: 20,
                        right: 16,
                        top: 10,
                        bottom: 10,
                    }),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(36.0);
                    ui.label(
                        egui::RichText::new("Craft Apps Manager")
                            .font(theme::bold(15.0))
                            .color(theme::TEXT),
                    );
                    theme::text(
                        ui,
                        format!("Version {}", env!("CARGO_PKG_VERSION")),
                        12.0,
                        theme::MUTED,
                    );
                    if self.manager_plan.is_some() {
                        ui.spinner();
                        theme::text(ui, "Downloading update…", 12.0, theme::ACCENT_TEXT);
                    } else if let Some(available) = &self.manager_available {
                        let version = available.version.clone();
                        if btn("Update available")
                            .kind(Kind::Soft)
                            .size(Size::Pill)
                            .icon(Icon::ArrowUp)
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
                        if btn("Settings").medium().icon(Icon::Gear).show(ui).clicked() {
                            self.open_settings();
                        }
                    });
                });
            });
    }

    pub(super) fn status_bar(&mut self, ctx: &egui::Context, state: &State) {
        let build = self.build_job.state.lock().unwrap().clone();
        egui::TopBottomPanel::bottom("footer")
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .inner_margin(egui::Margin::symmetric(16, 6)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (color, text) = if state.busy {
                        (
                            theme::LINK,
                            if state.stage.is_empty() || state.stage == "Working" {
                                "Working".to_owned()
                            } else {
                                format!("Working · {}", state.stage)
                            },
                        )
                    } else if build.busy {
                        (
                            theme::LINK,
                            format!(
                                "Building {} · {}",
                                model::title(&self.build_app),
                                build.stage
                            ),
                        )
                    } else if state.stage == "Failed" {
                        (theme::RED, "Last operation failed".to_owned())
                    } else {
                        (theme::GREEN, "Ready".to_owned())
                    };
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        theme::text(ui, text, 12.0, theme::TEXT_3);
                        theme::dot(ui, color);
                        ui.add_space(12.0);
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(format!(
                                        "Data: {}",
                                        self.paths.root.display()
                                    ))
                                    .size(12.0)
                                    .color(theme::TEXT_3),
                                )
                                .truncate(),
                            );
                        });
                    });
                });
            });
    }

    pub(super) fn sidebar(&mut self, ctx: &egui::Context, state: &State) {
        egui::SidePanel::left("sidebar")
            .resizable(false)
            .exact_width(264.0)
            .frame(egui::Frame::new().fill(theme::PANEL))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin {
                        left: 12,
                        right: 12,
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
                                right: 8,
                                top: 4,
                                bottom: 12,
                            })
                            .show(ui, |ui| self.app_list(ui, state));
                    });
            });
    }

    fn search_box(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new()
            .fill(theme::FIELD)
            .stroke(Stroke::new(1.0, Color32::from_rgb(0x2f, 0x32, 0x38)))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(egui::Margin::symmetric(10, 0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.set_min_height(34.0);
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), Sense::hover());
                    theme::paint_icon(ui.painter(), rect, Icon::Search, theme::MUTED);
                    // Lay out right to left so the clear button keeps its room.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let clear = !self.search.is_empty()
                            && theme::icon_button(ui, Icon::Close, "Clear search", theme::MUTED)
                                .clicked();
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut self.search)
                                .hint_text("Search apps")
                                .frame(false)
                                .margin(egui::Margin::ZERO)
                                .desired_width(ui.available_width())
                                .font(egui::FontId::proportional(13.0)),
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
        ui.add_space(10.0);
        let mut reorder = None;
        let (installed, available): (Vec<_>, Vec<_>) = order
            .iter()
            .filter(|name| matches(name))
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
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                theme::section_label(ui, label);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(8.0);
                    theme::text(ui, group.len().to_string(), 11.5, theme::MUTED);
                });
            });
            ui.add_space(2.0);
            for name in &group {
                if let Some(drop) = self.app_row(ui, state, name, is_installed) {
                    reorder = Some(drop);
                }
            }
            ui.add_space(12.0);
        }
        if installed.is_empty() && available.is_empty() {
            ui.add_space(4.0);
            theme::text(
                ui,
                format!("No apps match “{}”.", self.search.trim()),
                13.0,
                theme::MUTED,
            );
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
                    format!("Overview · {updates} update(s) available")
                } else {
                    "Overview".into()
                },
            )
        });
        paint_row_background(ui, rect, selected, response.hovered());
        let tile = Rect::from_min_size(
            egui::pos2(rect.left() + 10.0, rect.center().y - 16.0),
            egui::vec2(32.0, 32.0),
        );
        ui.painter().rect_filled(
            tile,
            CornerRadius::same(8),
            Color32::from_rgb(0x23, 0x26, 0x2c),
        );
        theme::paint_icon(
            ui.painter(),
            Rect::from_center_size(tile.center(), egui::vec2(16.0, 16.0)),
            Icon::Grid,
            theme::TEXT_2,
        );
        ui.painter().text(
            egui::pos2(tile.right() + 12.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            "Overview",
            theme::medium(14.0),
            theme::TEXT,
        );
        if updates > 0 {
            let badge = Rect::from_center_size(
                egui::pos2(rect.right() - 22.0, rect.center().y),
                egui::vec2(20.0, 20.0),
            );
            ui.painter()
                .rect_filled(badge, CornerRadius::same(10), theme::ACCENT_SOFT);
            theme::painter_text(
                ui.painter(),
                badge.center(),
                egui::Align2::CENTER_CENTER,
                updates,
                11.0,
                theme::ACCENT_TEXT,
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
    ) -> Option<(String, String, bool)> {
        let status = self.status(name);
        let selected = self.page == Page::App && self.app == name;
        let height = if is_installed { 48.0 } else { 44.0 };
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), height),
            if state.busy {
                Sense::click()
            } else {
                Sense::click_and_drag()
            },
        );
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
        response.dnd_set_drag_payload(name.to_owned());
        let mut drop = None;
        if response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            ui.painter().rect_stroke(
                rect.shrink(1.0),
                CornerRadius::same(8),
                Stroke::new(1.0, theme::ACCENT),
                egui::StrokeKind::Inside,
            );
        }
        let before = ui
            .ctx()
            .input(|i| i.pointer.interact_pos())
            .is_none_or(|pos| pos.y < rect.center().y);
        if let Some(dragged) = response.dnd_hover_payload::<String>() {
            if dragged.as_str() != name {
                let y = if before { rect.top() } else { rect.bottom() };
                ui.painter().line_segment(
                    [
                        egui::pos2(rect.left() + 4.0, y),
                        egui::pos2(rect.right() - 4.0, y),
                    ],
                    Stroke::new(2.0, theme::ACCENT),
                );
            }
        }
        if let Some(dragged) = response.dnd_release_payload::<String>() {
            if dragged.as_str() != name {
                drop = Some((dragged.as_ref().clone(), name.to_owned(), before));
            }
        }
        paint_row_background(ui, rect, selected, response.hovered());
        let icon_size = if is_installed { 32.0 } else { 28.0 };
        let icon = Rect::from_min_size(
            egui::pos2(rect.left() + 10.0, rect.center().y - icon_size / 2.0),
            egui::vec2(icon_size, icon_size),
        );
        if let Some(texture) = self.icons.get(name) {
            egui::Image::new((texture.id(), icon.size()))
                .corner_radius(CornerRadius::same(if is_installed { 8 } else { 7 }))
                .tint(if is_installed {
                    Color32::WHITE
                } else {
                    Color32::from_rgba_unmultiplied(170, 170, 170, 150)
                })
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
        let chip_width = if chip.is_some() { 64.0 } else { 8.0 };
        let max_text = (rect.right() - chip_width - text_left).max(1.0);
        let title = ui.painter().layout(
            model::title(name),
            if is_installed {
                theme::medium(14.0)
            } else {
                egui::FontId::proportional(14.0)
            },
            if is_installed {
                theme::TEXT
            } else {
                theme::TEXT_3
            },
            max_text,
        );
        let (detail, detail_color) = if installing {
            (
                match state.progress {
                    Some(progress) => format!("Installing… {:.0}%", progress * 100.0),
                    None => "Installing…".into(),
                },
                theme::LINK,
            )
        } else if let Some(installed) = &status.installed {
            if let Some(update) = &status.update {
                (
                    format!("{} {} {update}", installed.version, theme::arrow()),
                    theme::LINK,
                )
            } else if status.checking {
                (format!("{} · checking…", installed.version), theme::MUTED)
            } else if matches!(status.check, Some(Ok(None))) {
                (format!("{} · up to date", installed.version), theme::GREEN)
            } else {
                (format!("Installed {}", installed.version), theme::GREEN)
            }
        } else if let Some((label, version)) = &other {
            (
                format!("{label} {version} · no {format} copy"),
                theme::MUTED,
            )
        } else if !status.loaded {
            ("Checking…".into(), theme::MUTED)
        } else {
            (model::category(name).to_owned(), theme::MUTED)
        };
        let mut detail_job = egui::text::LayoutJob::simple_singleline(
            detail,
            egui::FontId::proportional(12.0),
            detail_color,
        );
        detail_job.wrap.max_width = max_text;
        detail_job.wrap.max_rows = 1;
        let detail = ui.painter().layout_job(detail_job);
        let top = rect.center().y - (title.size().y + 1.0 + detail.size().y) / 2.0;
        let title_height = title.size().y;
        ui.painter()
            .galley(egui::pos2(text_left, top), title, theme::TEXT);
        ui.painter().galley(
            egui::pos2(text_left, top + title_height + 1.0),
            detail,
            detail_color,
        );
        let mut chip_clicked = false;
        if let Some(chip) = chip {
            let chip_rect = Rect::from_min_size(
                egui::pos2(rect.right() - 10.0 - 54.0, rect.center().y - 12.0),
                egui::vec2(54.0, 24.0),
            );
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
            let (fill, stroke, color) = if chip == "Update" {
                (
                    if hovered {
                        Color32::from_rgb(0x24, 0x38, 0x61)
                    } else {
                        theme::ACCENT_SOFT
                    },
                    Stroke::NONE,
                    theme::ACCENT_TEXT,
                )
            } else {
                (
                    if hovered {
                        theme::BUTTON_HOVER
                    } else {
                        Color32::TRANSPARENT
                    },
                    Stroke::new(1.0, Color32::from_rgb(0x3a, 0x3d, 0x44)),
                    theme::TEXT_2,
                )
            };
            let alpha = if state.busy { 0.45 } else { 1.0 };
            ui.painter().rect(
                chip_rect,
                CornerRadius::same(12),
                fill,
                stroke,
                egui::StrokeKind::Inside,
            );
            theme::painter_text(
                ui.painter(),
                chip_rect.center(),
                egui::Align2::CENTER_CENTER,
                chip,
                11.5,
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
            ui.put(spinner, egui::Spinner::new().size(12.0))
                .on_hover_text("Checking for updates…");
        }
        if chip_clicked {
            self.confirm_install = Some(name.into());
        } else if response.clicked() {
            response.request_focus();
            self.select(name);
        }
        drop
    }
}

fn paint_row_background(ui: &egui::Ui, rect: Rect, selected: bool, hovered: bool) {
    if selected || hovered {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(8),
            if selected {
                theme::SELECTED
            } else {
                theme::HOVER
            },
        );
    }
}
