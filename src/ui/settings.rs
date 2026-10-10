//! Settings: one scrolling page with a jump list, saved together with one Save.
use super::dialogs;
use super::overview::release_format_label;
use super::theme::{self, btn, Kind};
use super::{modal, App, Locations};
use craft_apps_manager::{catalog, model, platform, scheduler, self_update};
use eframe::egui::{self, CornerRadius, FontId, RichText, Sense};
use std::{path::PathBuf, time::Duration};

const MANAGER_SECTIONS: [&str; 7] = [
    "General", "Updates", "Apps", "Backups", "Builds", "Folders", "About",
];
const BUILDER_SECTIONS: [&str; 3] = ["Builds", "Folders", "About"];
/// Width of the section list, its right hairline included.
const NAV_WIDTH: f32 = 176.0;

/// A titled group of rows inside a section: 13pt semibold title, 10pt gap, card.
fn group(ui: &mut egui::Ui, title: &str, content: impl FnOnce(&mut egui::Ui)) {
    if !title.is_empty() {
        ui.label(
            RichText::new(title)
                .font(theme::bold(13.0))
                .color(theme::palette().text_3),
        );
        ui.add_space(10.0);
    }
    theme::group().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.0;
        content(ui);
    });
}

/// Small print below a group, 10pt under it.
fn note(ui: &mut egui::Ui, text: &str) {
    ui.add_space(10.0);
    ui.add(egui::Label::new(RichText::new(text).size(12.0).color(theme::palette().muted)).wrap());
}

/// One row of a group: title and optional detail on the left, the control on the
/// right, 16pt apart, vertically centred in at least `height` points plus padding.
fn row<R>(
    ui: &mut egui::Ui,
    title: &str,
    detail: Option<&str>,
    height: f32,
    control: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    row_with(ui, title, detail, height, false, control)
}

fn row_with<R>(
    ui: &mut egui::Ui,
    title: &str,
    detail: Option<&str>,
    height: f32,
    nested: bool,
    control: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let (left, vertical, color) = if nested {
        (32, 6, theme::palette().text_2)
    } else {
        (14, 8, theme::palette().text)
    };
    dialogs::band(
        ui,
        egui::Margin {
            left,
            right: 14,
            top: vertical,
            bottom: vertical,
        },
        0.0,
        |ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), height),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    ui.set_min_height(height);
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let out = control(ui);
                    ui.add_space(8.0);
                    // Size the text block up front so the row centres it as one piece.
                    let line = |size: f32| ui.fonts(|f| f.row_height(&FontId::proportional(size)));
                    let text_height = match detail {
                        Some(_) => line(14.0) + 2.0 + line(12.0),
                        None => line(14.0),
                    };
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), text_height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            theme::text(ui, title, 14.0, color);
                            if let Some(detail) = detail {
                                theme::text(ui, detail, 12.0, theme::palette().muted);
                            }
                        },
                    );
                    out
                },
            )
            .inner
        },
    )
}

/// A row with an 18pt checkbox on the right.
fn check_row(
    ui: &mut egui::Ui,
    value: &mut bool,
    title: &str,
    detail: Option<&str>,
) -> egui::Response {
    check_row_with(ui, value, title, detail, false)
}

fn check_row_with(
    ui: &mut egui::Ui,
    value: &mut bool,
    title: &str,
    detail: Option<&str>,
    nested: bool,
) -> egui::Response {
    let height = if nested { 44.0 } else { 48.0 };
    row_with(ui, title, detail, height, nested, |ui| {
        // Native checkboxes keep a 3pt margin on their right.
        ui.add_space(3.0);
        theme::checkbox(ui, value, 18.0, theme::palette().accent, title, true)
    })
}

/// A 64x32 number field.
fn number(ui: &mut egui::Ui, value: egui::DragValue) -> egui::Response {
    ui.allocate_ui_with_layout(
        egui::vec2(64.0, 32.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            dialogs::field_style(ui);
            ui.spacing_mut().interact_size = egui::vec2(64.0, 32.0);
            ui.spacing_mut().button_padding = egui::vec2(8.0, 0.0);
            ui.add(value)
        },
    )
    .inner
}

/// A full-width monospace path field with an Open button.
fn path_field(ui: &mut egui::Ui, text: &mut String) -> bool {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), 32.0),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let open = btn("Open").show(ui).clicked();
            dialogs::field_style(ui);
            ui.add(
                egui::TextEdit::singleline(text)
                    .font(FontId::monospace(12.0))
                    .margin(egui::Margin::symmetric(10, 8))
                    .min_size(egui::vec2(0.0, 32.0))
                    .desired_width(ui.available_width()),
            );
            open
        },
    )
    .inner
}

impl App {
    pub(super) fn settings_ui(&mut self, ctx: &egui::Context) {
        if !self.settings {
            return;
        }
        let sections: &[&str] = if self.builder {
            &BUILDER_SECTIONS
        } else {
            &MANAGER_SECTIONS
        };
        let busy = self.release_pending() || self.job.state.lock().unwrap().busy;
        let active_id = egui::Id::new("settings-active-section");
        let active = ctx.data(|d| d.get_temp::<usize>(active_id)).unwrap_or(0);
        let modal = modal(
            ctx,
            if self.builder {
                "Builder settings"
            } else {
                "Manager settings"
            },
            760.0,
            |ui| {
                dialogs::title_bar(ui, "Settings", |ui| {
                    if dialogs::close_button(ui).clicked() {
                        self.settings = false;
                    }
                });
                // The dialog is 600pt tall: title bar, scrolling body, footer.
                let total = (ctx.screen_rect().height() - 48.0).min(600.0);
                let height = (total - 2.0 - 2.0 * dialogs::BAR_HEIGHT).max(220.0);
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.allocate_ui_with_layout(
                        egui::vec2(NAV_WIDTH - 1.0, height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_min_size(egui::vec2(NAV_WIDTH - 1.0, height));
                            dialogs::band(ui, egui::Margin::symmetric(8, 12), 0.0, |ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                for (index, section) in sections.iter().enumerate() {
                                    let selected = index == active;
                                    let (rect, response) = ui.allocate_exact_size(
                                        egui::vec2(ui.available_width(), 36.0),
                                        Sense::click(),
                                    );
                                    response.widget_info(|| {
                                        egui::WidgetInfo::selected(
                                            egui::WidgetType::SelectableLabel,
                                            true,
                                            selected,
                                            *section,
                                        )
                                    });
                                    if selected || response.hovered() {
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
                                    theme::painter_text(
                                        ui.painter(),
                                        egui::pos2(rect.left() + 12.0, rect.center().y),
                                        egui::Align2::LEFT_CENTER,
                                        section,
                                        14.0,
                                        if selected {
                                            theme::palette().text
                                        } else {
                                            theme::palette().text_3
                                        },
                                    );
                                    if response.clicked() {
                                        self.settings_jump = Some(index);
                                    }
                                }
                            });
                        },
                    );
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, height), Sense::hover());
                    ui.painter().rect_filled(rect, 0.0, theme::palette().border);
                    egui::ScrollArea::vertical()
                        .id_salt("settings-scroll")
                        .auto_shrink([false, false])
                        .max_height(height)
                        .min_scrolled_height(height)
                        .show(ui, |ui| {
                            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                                let viewport_top = ui.clip_rect().top();
                                egui::Frame::new()
                                    .inner_margin(egui::Margin {
                                        left: 24,
                                        right: 24,
                                        top: 20,
                                        bottom: 40,
                                    })
                                    .show(ui, |ui| {
                                        ui.spacing_mut().item_spacing.y = 0.0;
                                        let mut current = 0;
                                        for (index, section) in sections.iter().enumerate() {
                                            let top = ui.cursor().top();
                                            if top <= viewport_top + 48.0 {
                                                current = index;
                                            }
                                            if index > 0 {
                                                ui.add_space(36.0);
                                            }
                                            let heading = theme::heading(ui, *section, 18.0);
                                            ui.add_space(16.0);
                                            if self.settings_jump == Some(index) {
                                                heading.scroll_to_me(Some(egui::Align::TOP));
                                                self.settings_jump = None;
                                            }
                                            match *section {
                                                "General" => self.settings_general(ui),
                                                "Apps" => self.settings_apps(ui),
                                                "Updates" => self.settings_updates(ui, ctx, busy),
                                                "Backups" => self.settings_backups(ui, busy),
                                                "Builds" => self.settings_builds(ui, busy),
                                                "About" => self.settings_about(ui),
                                                _ => self.settings_folders(ui),
                                            }
                                        }
                                        // Let the last section scroll to the top of the view.
                                        ui.add_space((height - 300.0).max(0.0));
                                        ctx.data_mut(|d| d.insert_temp(active_id, current));
                                    });
                            });
                        });
                });
                dialogs::footer(ui, |ui| {
                    if dialogs::action(ui, "Save", Kind::Primary, !busy)
                        .on_disabled_hover_text("Wait for the current operation to finish.")
                        .clicked()
                    {
                        self.save_settings();
                    }
                    if dialogs::action(ui, "Cancel", Kind::Secondary, true).clicked() {
                        self.settings = false;
                    }
                });
            },
        );
        if self.visibility_open {
            let dialog = super::modal(ctx, "App visibility", 480.0, |ui| {
                dialogs::title_bar(ui, "App visibility", |ui| {
                    if dialogs::close_button(ui).clicked() {
                        self.visibility_open = false;
                    }
                });
                dialogs::band(ui, egui::Margin::symmetric(20, 16), 0.0, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(420.0)
                        .show(ui, |ui| self.settings_visibility(ui));
                });
                dialogs::footer(ui, |ui| {
                    if dialogs::action(ui, "Save", Kind::Primary, true).clicked() {
                        self.settings_draft.hidden_apps = self.visibility_draft.clone();
                        self.visibility_open = false;
                    }
                    if dialogs::action(ui, "Cancel", Kind::Secondary, true).clicked() {
                        self.visibility_open = false;
                    }
                });
            });
            if dialog.should_close() {
                self.visibility_open = false;
            }
        }
        if !self.visibility_open
            && !self.selection
            && !self.confirm_clear
            && !self.confirm_clean
            && !self.confirm_self_update
            && self.error.is_none()
            && self.selection_notice.is_none()
            && modal.should_close()
        {
            self.settings = false;
        }
    }

    pub(super) fn app_visibility_menu(&mut self, response: &egui::Response, name: &str) {
        response.context_menu(|ui| {
            if btn("Hide")
                .kind(Kind::Ghost)
                .icon(theme::Icon::EyeOff)
                .show(ui)
                .clicked()
            {
                let result = (|| -> anyhow::Result<()> {
                    let mut preferences = self.paths.read_preferences()?;
                    let first_hide = !self.hide_notice_seen;
                    if !preferences.hidden_apps.iter().any(|app| app == name) {
                        preferences.hidden_apps.push(name.to_owned());
                    }
                    craft_apps_manager::files::write_json(
                        &self.paths.at("manager-settings.json"),
                        &preferences,
                    )?;
                    if first_hide {
                        self.hide_notice_open = true;
                    }
                    self.hide_notice_seen = true;
                    self.preferences.hidden_apps = preferences.hidden_apps.clone();
                    self.settings_draft.hidden_apps = preferences.hidden_apps;
                    if self.app == name {
                        self.page = super::Page::Overview;
                    }
                    Ok(())
                })();
                self.result(result);
                ui.close_menu();
            }
        });
    }

    fn settings_apps(&mut self, ui: &mut egui::Ui) {
        let busy = self.release_pending() || self.job.state.lock().unwrap().busy;
        let shown = self
            .settings_draft
            .app_order
            .iter()
            .filter(|name| !self.settings_draft.hidden_apps.contains(name))
            .count();
        group(ui, "App visibility", |ui| {
            if row(
                ui,
                "Apps",
                Some(&format!(
                    "{shown} of {} shown in sidebar and Home",
                    self.settings_draft.app_order.len()
                )),
                48.0,
                |ui| btn("Choose…").show(ui).clicked(),
            ) {
                self.visibility_draft = self.settings_draft.hidden_apps.clone();
                self.visibility_open = true;
            }
        });
        ui.add_space(16.0);
        let mut add_pressed = false;
        let mut remove: Option<String> = None;
        group(ui, "Custom apps", |ui| {
            // Link entry, shaped like the Folders path rows.
            dialogs::band(ui, egui::Margin::symmetric(14, 12), 0.0, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                theme::text(ui, "GitHub repository link", 14.0, theme::palette().text);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    dialogs::field_style(ui);
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut self.custom_link)
                            .hint_text("https://github.com/owner/repo")
                            .font(FontId::monospace(12.0))
                            .margin(egui::Margin::symmetric(10, 8))
                            .min_size(egui::vec2(0.0, 32.0))
                            .desired_width(f32::INFINITY),
                    );
                    let adding = self.custom_receiver.is_some();
                    let enabled = !busy && !adding && !self.custom_link.trim().is_empty();
                    add_pressed = btn("Add")
                        .enabled(enabled)
                        .show(ui)
                        .on_disabled_hover_text(if busy {
                            "Wait for the current operation to finish."
                        } else {
                            "Type or paste a GitHub repository link first."
                        })
                        .clicked()
                        || (field.lost_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter))
                            && enabled);
                    if adding {
                        ui.spinner();
                    }
                });
            });
            if !self.custom_message.is_empty() {
                dialogs::rule(ui);
                dialogs::band(ui, egui::Margin::symmetric(14, 10), 0.0, |ui| {
                    theme::text(
                        ui,
                        &self.custom_message,
                        13.0,
                        if self.custom_failed {
                            theme::palette().red
                        } else {
                            theme::palette().text_2
                        },
                    );
                });
            }
            let customs = catalog::custom_apps(&self.paths.root);
            if customs.is_empty() {
                dialogs::rule(ui);
                dialogs::band(ui, egui::Margin::symmetric(14, 10), 0.0, |ui| {
                    theme::text(ui, "No custom apps yet.", 13.0, theme::palette().muted);
                });
            } else {
                for entry in &customs {
                    dialogs::rule(ui);
                    if row(ui, &entry.title, Some(&entry.repository), 56.0, |ui| {
                        btn("Remove")
                            .enabled(!busy)
                            .show(ui)
                            .on_disabled_hover_text("Wait for the current operation to finish.")
                            .clicked()
                    }) {
                        remove = Some(entry.key.clone());
                    }
                }
            }
        });
        note(
            ui,
            "Track any other GitHub app: paste its repository link and the manager checks its latest stable release for an installable file, then updates it like a built-in app.",
        );
        if add_pressed {
            self.add_custom_app(ui.ctx());
        }
        if let Some(key) = remove {
            self.remove_custom_app(&key);
        }
    }

    fn add_custom_app(&mut self, ctx: &egui::Context) {
        let link = self.custom_link.trim().to_string();
        let slug = match catalog::parse_github_slug(&link) {
            Ok(slug) => slug,
            Err(error) => {
                self.custom_message = format!("{error:#}");
                self.custom_failed = true;
                return;
            }
        };
        let root = self.paths.root.clone();
        let release_format = self.settings_draft.release_format.clone();
        let architecture = self.settings_draft.architecture.clone();
        let ctx = ctx.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.custom_receiver = Some(rx);
        self.custom_message = format!("Adding {slug}…");
        self.custom_failed = false;
        std::thread::spawn(move || {
            let result = catalog::add_custom_app(&root, &slug, &release_format, &architecture)
                .map(|entry| {
                    (
                        entry.key.clone(),
                        format!("Added {} ({})", entry.title, entry.repository),
                    )
                })
                .map_err(|error| format!("{error:#}"));
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }

    fn remove_custom_app(&mut self, key: &str) {
        match catalog::remove_custom_app(&self.paths.root, key) {
            Ok(entry) => {
                for list in [
                    &mut self.settings_draft.selected_apps,
                    &mut self.settings_draft.selected_sources,
                    &mut self.settings_draft.app_order,
                    &mut self.settings_draft.home_app_order,
                    &mut self.settings_draft.known_apps,
                ] {
                    list.retain(|name| name != &entry.key);
                }
                self.catalog_changed();
                self.custom_message = format!("Removed {}", entry.title);
                self.custom_failed = false;
            }
            Err(error) => {
                self.custom_message = format!("{error:#}");
                self.custom_failed = true;
            }
        }
    }

    fn settings_visibility(&mut self, ui: &mut egui::Ui) {
        let names = self.settings_draft.app_order.clone();
        let shown = names
            .iter()
            .filter(|name| !self.visibility_draft.contains(name))
            .count();
        theme::text(
            ui,
            format!("{shown} of {} shown", names.len()),
            13.0,
            theme::palette().muted,
        );
        ui.horizontal(|ui| {
            if theme::link(ui, "Select all").clicked() {
                self.visibility_draft.clear();
            }
            if theme::link(ui, "Deselect all").clicked() {
                self.visibility_draft = names.clone();
            }
        });
        note(
            ui,
            "Hidden apps stay installed. Hiding an app does not change its update preferences.",
        );
        ui.add_space(12.0);
        dialogs::rule(ui);
        for name in names {
            let visible = !self.visibility_draft.contains(&name);
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 48.0), Sense::click());
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::Checkbox,
                    true,
                    visible,
                    model::title(&name),
                )
            });
            if response.hovered() {
                ui.painter()
                    .rect_filled(rect, CornerRadius::same(8), theme::palette().hover);
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if response.has_focus() {
                ui.painter().rect_stroke(
                    rect,
                    CornerRadius::same(8),
                    egui::Stroke::new(2.0_f32, theme::palette().link),
                    egui::StrokeKind::Inside,
                );
            }
            let mark = egui::Rect::from_min_size(
                egui::pos2(rect.left() + 12.0, rect.center().y - 8.0),
                egui::Vec2::splat(16.0),
            );
            theme::paint_check(ui, mark, visible, theme::palette().accent, true);
            if let Some(texture) = self.icons.get(&name) {
                ui.painter().image(
                    texture.id(),
                    egui::Rect::from_min_size(
                        egui::pos2(mark.right() + 15.0, rect.center().y - 12.0),
                        egui::Vec2::splat(24.0),
                    ),
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
            theme::painter_text(
                ui.painter(),
                egui::pos2(mark.right() + 51.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                model::title(&name),
                14.0,
                theme::palette().text,
            );
            if response.clicked() {
                self.visibility_draft.retain(|app| app != &name);
                if visible {
                    self.visibility_draft.push(name);
                }
            }
        }
    }

    fn save_settings(&mut self) {
        let root = PathBuf::from(self.root_text.trim());
        let tools = PathBuf::from(self.tools_text.trim());
        let locations = Locations {
            root: Some(root.clone()),
            tools: Some(tools.clone()),
        };
        let result = craft_apps_manager::settings::save(
            &self.paths,
            &self.home.join("data-root.json"),
            &locations,
            &root,
            &tools,
            if self.builder {
                None
            } else {
                Some(&self.settings_draft)
            },
            Some(&self.build_draft),
        );
        if result.is_ok() {
            self.build_preferences = self.build_draft.clone();
            if !self.builder {
                self.settings_draft.validate().expect("validated settings");
                let changed = self.preferences.release_format != self.settings_draft.release_format
                    || self.preferences.architecture != self.settings_draft.architecture;
                self.preferences = self.settings_draft.clone();
                if changed {
                    self.check_generation += 1;
                    self.checking_apps.clear();
                    self.release_checks.clear();
                }
                // Appearance/selection saves must not temporarily empty the app lists.
                if changed || root != self.paths.root {
                    self.display_config = None;
                    self.config_receiver = None;
                }
                self.config_refresh_at = std::time::Instant::now();
            }
            if !self.builder && self.auto_draft != self.auto {
                if let Err(error) = scheduler::set(&self.paths, false, self.auto_draft) {
                    self.auto = scheduler::enabled(false);
                    self.auto_draft = self.auto;
                    self.result(Err(error.context(
                        "Preferences saved, but the background scheduler could not be changed",
                    )));
                    return;
                }
                self.auto = self.auto_draft;
            }
            self.settings = false;
        }
        self.result(result);
    }

    fn settings_about(&self, ui: &mut egui::Ui) {
        let icon_id = egui::Id::new("about-manager-icon");
        let mut icon = ui
            .ctx()
            .data(|data| data.get_temp::<egui::TextureHandle>(icon_id));
        if icon.is_none() {
            if let Ok(image) = image::load_from_memory(include_bytes!("../../assets/icon.png")) {
                let image = image.into_rgba8();
                let pixels = egui::ColorImage::from_rgba_unmultiplied(
                    [image.width() as usize, image.height() as usize],
                    image.as_raw(),
                );
                let texture = ui.ctx().load_texture(
                    "about-manager-icon",
                    pixels,
                    egui::TextureOptions::LINEAR,
                );
                ui.ctx()
                    .data_mut(|data| data.insert_temp(icon_id, texture.clone()));
                icon = Some(texture);
            }
        }
        group(ui, "", |ui| {
            dialogs::band(ui, egui::Margin::same(20), 0.0, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 16.0;
                    if let Some(icon) = &icon {
                        ui.add(egui::Image::new(icon).fit_to_exact_size(egui::vec2(56.0, 56.0)));
                    }
                    ui.vertical(|ui| {
                        theme::heading(ui, "Craft Apps Manager", 21.0);
                        ui.add_space(6.0);
                        theme::text(
                            ui,
                            format!("Version {}", env!("CARGO_PKG_VERSION")),
                            14.0,
                            theme::palette().text_2,
                        );
                        ui.add_space(4.0);
                        theme::text(
                            ui,
                            "Your Craft apps, in one place.",
                            12.0,
                            theme::palette().muted,
                        );
                    });
                });
                ui.add_space(20.0);
                dialogs::rule(ui);
                ui.add_space(16.0);
                ui.add(egui::Label::new("Install and launch Craft apps, check for new releases, manage backups and launch settings, or download source code and build apps yourself.").wrap());
                ui.add_space(16.0);
                theme::hyperlink(ui, "Visit project on GitHub", self_update::REPOSITORY, 13.0);
                ui.add_space(16.0);
                let built = env!("CRAFT_BUILD_TIMESTAMP")
                    .parse::<i64>()
                    .ok()
                    .and_then(|timestamp| chrono::DateTime::from_timestamp(timestamp, 0))
                    .map(|date| date.format("%Y-%m-%d %H:%M UTC").to_string())
                    .unwrap_or_else(|| "Unknown".to_owned());
                theme::text(
                    ui,
                    format!(
                        "{} · {} · {}",
                        model::release_os(),
                        model::MANAGER_ARCH,
                        env!("CRAFT_BUILD_PROFILE")
                    ),
                    12.0,
                    theme::palette().muted,
                );
                ui.add_space(4.0);
                theme::text(ui, format!("Built {built}"), 12.0, theme::palette().muted);
            });
        });
        ui.add_space(24.0);
        group(ui, "Contributors", |ui| {
            dialogs::band(ui, egui::Margin::same(16), 0.0, |ui| {
                ui.add(egui::Label::new("Thank you to everyone who contributes, tests builds, reports bugs, and shares feedback.").wrap());
                ui.add_space(12.0);
                theme::hyperlink(
                    ui,
                    "View all contributors",
                    "https://github.com/CryptoKey98/craft-apps-manager/graphs/contributors",
                    13.0,
                );
            });
        });
        ui.add_space(24.0);
        group(ui, "License and acknowledgments", |ui| {
            dialogs::band(ui, egui::Margin::same(16), 0.0, |ui| {
                ui.add(egui::Label::new("Open source under the MIT license. Third-party licenses are included in THIRD-PARTY-NOTICES.txt.").wrap());
                note(ui, "An independent community project, not affiliated with Adobe or officially maintained by Storytold. Craft apps and their branding belong to their respective creators.");
            });
        });
    }
    fn settings_general(&mut self, ui: &mut egui::Ui) {
        group(ui, "Window behavior", |ui| {
            check_row(ui, &mut self.settings_draft.close_to_tray, "Keep running in the system tray when closed", Some("Closing hides the window. Periodic checks continue; choose Exit in the tray menu to quit."));
            if let Some(message) = &self.tray_error {
                note(ui, message);
            }
        });
        ui.add_space(16.0);
        group(ui, "Releases", |ui| {
            let hint = if self.settings_draft.release_format == "installer" {
                if cfg!(target_os = "macos") {
                    "Copies apps into Applications"
                } else if cfg!(target_os = "linux") {
                    "Installs system packages"
                } else {
                    "Opens each app's Windows installer"
                }
            } else {
                "Keeps apps in the library"
            };
            row(ui, "Release format", Some(hint), 56.0, |ui| {
                theme::segmented(
                    ui,
                    30.0,
                    14.0,
                    &mut self.settings_draft.release_format,
                    &[
                        ("installer".to_owned(), "Installer"),
                        ("portable".to_owned(), release_format_label("portable")),
                    ],
                );
            });
            dialogs::rule(ui);
            if cfg!(target_os = "macos") {
                row(ui, "Architecture", None, 52.0, |ui| {
                    theme::text(
                        ui,
                        "Universal · Apple silicon and Intel",
                        14.0,
                        theme::palette().text_3,
                    );
                });
            } else {
                row(ui, "Architecture", None, 52.0, |ui| {
                    egui::ComboBox::from_id_salt("arch")
                        .selected_text(model::architecture_label(&self.settings_draft.architecture))
                        .show_ui(ui, |ui| {
                            for (value, label) in [
                                ("x64", "64-bit (x64)"),
                                ("x86", "32-bit (x86)"),
                                ("arm64", "ARM64"),
                            ] {
                                ui.selectable_value(
                                    &mut self.settings_draft.architecture,
                                    value.into(),
                                    label,
                                );
                            }
                        });
                });
            }
        });
        note(
            ui,
            if cfg!(target_os = "linux") {
                "AppImage updates retain a rollback copy until successful. System packages require administrator authorization."
            } else if cfg!(target_os = "macos") {
                "A temporary rollback copy is kept until the update succeeds. Installer mode copies the signed app into Applications; portable mode keeps it in the library."
            } else {
                "A temporary rollback copy is kept until the update succeeds. Installer mode downloads and opens the Windows installer wizard."
            },
        );
    }

    fn settings_updates(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, busy: bool) {
        let building = self.build_job.state.lock().unwrap().busy;
        group(ui, "Craft Apps Manager", |ui| {
            let built = env!("CRAFT_BUILD_TIMESTAMP")
                .parse::<i64>()
                .ok()
                .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
                .map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
                .unwrap_or_default();
            let detail = format!(
                "Build {built} · {} · {} {}",
                env!("CRAFT_BUILD_PROFILE"),
                model::release_os(),
                model::MANAGER_ARCH
            );
            let checking = self.manager_receiver.is_some() || self.manager_plan.is_some();
            row(
                ui,
                &format!("Version {}", env!("CARGO_PKG_VERSION")),
                Some(&detail),
                56.0,
                |ui| {
                    if btn("Check for updates")
                        .enabled(!checking)
                        .show(ui)
                        .on_hover_text("Check for a newer Craft Apps Manager release")
                        .clicked()
                    {
                        self.check_manager(ctx);
                    }
                    if checking {
                        ui.spinner();
                        ctx.request_repaint_after(Duration::from_millis(100));
                    }
                },
            );
            if !self.manager_message.is_empty() || self.manager_available.is_some() {
                dialogs::band(
                    ui,
                    egui::Margin {
                        left: 14,
                        right: 14,
                        top: 0,
                        bottom: 10,
                    },
                    0.0,
                    |ui| {
                        ui.horizontal(|ui| {
                            theme::text(
                                ui,
                                &self.manager_message,
                                13.0,
                                if self.manager_available.is_some() {
                                    theme::palette().link
                                } else {
                                    theme::palette().text_3
                                },
                            );
                            if self.manager_available.is_some() {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if btn(
                                            if self_update::installed_with_msi()
                                                || self_update::installed_with_linux_package()
                                            {
                                                "Download and install…"
                                            } else {
                                                "Download and restart…"
                                            },
                                        )
                                        .primary()
                                        .enabled(!busy && !building && self.manager_plan.is_none())
                                        .show(ui)
                                        .clicked()
                                        {
                                            self.confirm_self_update = true;
                                        }
                                    },
                                );
                            }
                        });
                        if self.manager_available.is_some()
                            && self_update::installed_with_linux_package()
                        {
                            note(ui, "The system package manager will install this update after administrator approval. Your settings and library will be kept.");
                        }
                    },
                );
            }
            dialogs::rule(ui);
            check_row(
                ui,
                &mut self.settings_draft.check_manager_on_startup,
                "Check for a new version on startup",
                None,
            )
            .on_hover_text(
                "Checks for a new Craft Apps Manager release. Downloads require your confirmation.",
            );
            dialogs::rule(ui);
            dialogs::band(ui, egui::Margin::symmetric(14, 8), 0.0, |ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), 44.0),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        theme::hyperlink(
                            ui,
                            self_update::REPOSITORY_NAME,
                            self_update::REPOSITORY,
                            13.0,
                        );
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            theme::text(ui, "Release source", 13.0, theme::palette().text_3);
                        });
                    },
                );
            });
        });
        ui.add_space(16.0);
        group(ui, "Update all", |ui| {
            let eligible = self
                .display_config
                .as_ref()
                .map(|config| {
                    config
                        .apps
                        .iter()
                        .filter(|app| {
                            craft_apps_manager::updates::installed_for_updates(
                                app,
                                &self.settings_draft,
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let included = eligible
                .iter()
                .filter(|app| self.settings_draft.selected_apps.contains(&app.name))
                .count();
            let apps = format!("{} of {} installed apps included", included, eligible.len());
            if row(ui, "Apps", Some(&apps), 52.0, |ui| {
                btn("Choose…").show(ui).clicked()
            }) {
                self.source_selection = false;
                self.selection_draft = self.settings_draft.selected_apps.clone();
                self.selection = true;
            }
            dialogs::rule(ui);
            check_row(
                ui,
                &mut self.settings_draft.select_new_apps,
                "Include newly published Craft apps automatically",
                None,
            )
            .on_hover_text(
                "New Craft apps are included in Update All when they are first discovered.",
            );
            dialogs::rule(ui);
            check_row(
                ui,
                &mut self.settings_draft.check_catalog_on_startup,
                "Look for new Craft apps on startup",
                None,
            )
            .on_hover_text("Uses the saved app list for six hours between automatic checks. Discovery never downloads or installs apps.");
            dialogs::rule(ui);
            check_row(
                ui,
                &mut self.settings_draft.check_catalog_with_app_updates,
                "Look for new Craft apps during automatic update checks",
                None,
            )
            .on_hover_text("Uses periodic or background update checks. Shares the six-hour discovery cache with startup checks. Refresh below checks immediately.");
            dialogs::rule(ui);
            let known = catalog::all().len();
            let status = if self.catalog_message.is_empty() {
                let checked = catalog::age(&self.paths.root)
                    .map(|s| {
                        if s < 120 {
                            "just now".to_string()
                        } else if s < 7200 {
                            format!("{} min ago", s / 60)
                        } else {
                            format!("{} h ago", s / 3600)
                        }
                    })
                    .unwrap_or_else(|| "not yet".into());
                format!("{known} Craft apps known · checked {checked}")
            } else {
                self.catalog_message.clone()
            };
            let refreshing = self.catalog_receiver.is_some();
            if row(ui, "App list", Some(&status), 52.0, |ui| {
                ui.add_enabled_ui(!refreshing, |ui| btn("Refresh").show(ui).clicked())
                    .inner
            }) {
                let ctx = ui.ctx().clone();
                self.refresh_catalog(&ctx);
            }
        });
        ui.add_space(16.0);
        group(ui, "Automatic update checks", |ui| {
            check_row(
                ui,
                &mut self.settings_draft.check_installed_apps_on_startup,
                "When the manager opens",
                Some("Check installed apps for newer versions."),
            );
            dialogs::rule(ui);
            check_row(
                ui,
                &mut self.settings_draft.check_installed_apps_periodically,
                "Periodically while the manager is open",
                Some("Checks wait until downloads and builds finish."),
            );
            row_with(
                ui,
                "Check every",
                Some("Minutes · 10–60"),
                44.0,
                true,
                |ui| {
                    ui.add_enabled_ui(
                        self.settings_draft.check_installed_apps_periodically,
                        |ui| {
                            number(
                                ui,
                                egui::DragValue::new(
                                    &mut self.settings_draft.app_check_interval_minutes,
                                )
                                .range(10..=60),
                            )
                        },
                    )
                    .inner
                },
            );
            dialogs::rule(ui);
            check_row(
                ui,
                &mut self.auto_draft,
                "Check even when the manager is closed",
                Some("Every hour using your system’s background scheduler."),
            );
        });
        note(
            ui,
            "Checks only look for updates. Downloads and installations require your approval.",
        );
        ui.add_space(16.0);
        group(ui, "Notifications", |ui| {
            check_row(
                ui,
                &mut self.settings_draft.notify_updates,
                "Notify me when app updates are available",
                None,
            );
        });
    }

    fn settings_backups(&mut self, ui: &mut egui::Ui, busy: bool) {
        group(ui, "Backups", |ui| {
            check_row(
                ui,
                &mut self.settings_draft.keep_app_backups,
                "Back up portable apps",
                Some("Before each update. Portable releases only."),
            );
            dialogs::rule(ui);
            check_row_with(
                ui,
                &mut self.settings_draft.compress_backups,
                "Compress portable app backups",
                Some("7-Zip Ultra / LZMA2"),
                true,
            );
            dialogs::rule(ui);
            check_row(
                ui,
                &mut self.settings_draft.keep_source_backups,
                "Back up sources",
                Some("Before each source update"),
            );
            dialogs::rule(ui);
            check_row_with(
                ui,
                &mut self.settings_draft.compress_source_backups,
                "Recompress source backups",
                Some("7-Zip Ultra / LZMA2"),
                true,
            );
            dialogs::rule(ui);
            row(
                ui,
                "Previous versions to keep",
                Some("Per app and per source"),
                52.0,
                |ui| {
                    number(
                        ui,
                        egui::DragValue::new(&mut self.settings_draft.backup_versions)
                            .range(1..=10),
                    );
                },
            );
        });
        ui.add_space(10.0);
        if btn("Delete all backups…")
            .kind(Kind::DangerOutline)
            .height(34.0)
            .enabled(!busy)
            .show(ui)
            .on_disabled_hover_text("Wait for the current operation to finish.")
            .clicked()
        {
            self.confirm_clear = true;
        }
    }

    fn settings_builds(&mut self, ui: &mut egui::Ui, busy: bool) {
        let building = self.build_job.state.lock().unwrap().busy;
        group(ui, "After a successful build", |ui| {
            check_row(
                ui,
                &mut self.build_draft.delete_cache_after_success,
                "Delete the compilation cache",
                None,
            );
            dialogs::rule(ui);
            check_row(
                ui,
                &mut self.build_draft.delete_workspace_after_success,
                "Delete the extracted source and node_modules",
                None,
            );
        });
        note(ui, "Cancelled and failed builds keep their cache. Completed builds and tools are retained.");
        ui.add_space(16.0);
        group(ui, "Build logs", |ui| {
            row(
                ui,
                "Start a new log at",
                Some("Megabytes per app log"),
                52.0,
                |ui| {
                    theme::text(ui, "MB", 14.0, theme::palette().text_3);
                    number(
                        ui,
                        egui::DragValue::new(&mut self.build_draft.log_size_mb).range(1..=100),
                    );
                },
            );
            dialogs::rule(ui);
            row(ui, "Older logs to keep", None, 52.0, |ui| {
                number(
                    ui,
                    egui::DragValue::new(&mut self.build_draft.log_archives).range(0..=5),
                );
            });
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if btn("Clean temporary build files…")
                .height(34.0)
                .enabled(!busy && !building)
                .show(ui)
                .on_disabled_hover_text(if building {
                    "Wait for the build to finish."
                } else {
                    "Wait for the current operation to finish."
                })
                .clicked()
            {
                self.confirm_clean = true;
            }
            if !self.builder
                && btn("Open builder window")
                    .kind(Kind::Ghost)
                    .show(ui)
                    .on_hover_text("Builds any source, including ArtCraft X, in a separate window")
                    .clicked()
            {
                self.open_builder();
            }
        });
    }

    fn settings_folders(&mut self, ui: &mut egui::Ui) {
        group(ui, "Folders", |ui| {
            let block = egui::Margin::symmetric(14, 12);
            dialogs::band(ui, block, 0.0, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                theme::text(
                    ui,
                    "Library (releases, sources, builds, logs, backups)",
                    14.0,
                    theme::palette().text,
                );
                if path_field(ui, &mut self.root_text) {
                    self.result(platform::open(&self.paths.root));
                }
            });
            dialogs::rule(ui);
            dialogs::band(ui, block, 0.0, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                theme::text(ui, "Build tools", 14.0, theme::palette().text);
                if path_field(ui, &mut self.tools_text) {
                    self.result(platform::open(&self.paths.tools));
                }
            });
            dialogs::rule(ui);
            dialogs::band(ui, block, 0.0, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 16.0;
                    for (label, folder) in [
                        ("Open releases", "releases"),
                        ("Open sources", "sources"),
                        ("Open builds", "builds"),
                    ] {
                        if theme::link(ui, label).clicked() {
                            self.result(platform::open(&self.paths.at(folder)));
                        }
                    }
                });
            });
        });
        note(ui, "Folder changes apply after reopening the window.");
    }
}
