//! Settings: one scrolling page with a jump list, saved together with one Save.
use crate::ui::dialogs;
use crate::ui::theme::{self, btn, Kind};
use crate::ui::{modal, App};
use craft_apps_manager::{catalog, model, platform, self_update};
use eframe::egui::{self, CornerRadius, FontId, RichText, Sense};
use std::{path::PathBuf, time::Duration};

const MANAGER_SECTIONS: [&str; 6] = ["General", "Updates", "Apps", "Builds", "Folders", "About"];
const BUILDER_SECTIONS: [&str; 3] = ["Builds", "Folders", "About"];
const BUILDER_TOKEN_SECTIONS: [&str; 4] = ["Updates", "Builds", "Folders", "About"];
/// Width of the section list, its right hairline included.
const NAV_WIDTH: f32 = 176.0;

pub(in crate::ui) mod cache_cleanup;
pub(in crate::ui) mod edit_session;
mod lifecycle;
pub(in crate::ui) mod save;
pub(in crate::ui) use lifecycle::Loaded;
mod token_draft;
use edit_session::{CloseRoute, SelectionOrigin};
pub(in crate::ui) use token_draft::verification_allows_close;
pub(in crate::ui) use token_draft::TokenDraft;

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
    theme::group()
        .inner_margin(egui::Margin::symmetric(0, 8))
        .show(ui, |ui| {
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
            // Reserve wrapped text height before centering the row. Growing a
            // centered child afterwards can otherwise paint above its band.
            let wrap_width = (ui.available_width() - 160.0).max(1.0);
            let measured = |text: &str, size: f32| {
                ui.fonts(|f| {
                    f.layout(
                        text.to_owned(),
                        FontId::proportional(size),
                        color,
                        wrap_width,
                    )
                    .size()
                    .y
                })
            };
            let height = height
                .max(measured(title, 14.0) + detail.map_or(0.0, |text| 2.0 + measured(text, 12.0)));
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), height),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    ui.set_min_height(height);
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let out = control(ui);
                    ui.add_space(8.0);
                    // Size the text block up front so the row centres it as one piece.
                    let text_width = ui.available_width().max(1.0);
                    let measured_height = |text: &str, size: f32| {
                        ui.fonts(|f| {
                            f.layout(
                                text.to_owned(),
                                FontId::proportional(size),
                                color,
                                text_width,
                            )
                            .size()
                            .y
                        })
                    };
                    let text_height = measured_height(title, 14.0)
                        + detail.map_or(0.0, |text| 2.0 + measured_height(text, 12.0));
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
    pub(in crate::ui) fn settings_ui(&mut self, ctx: &egui::Context) {
        self.poll_api_cache_cleanup(ctx);
        self.poll_rollback_cleanup(ctx);
        if !self.settings {
            return;
        }
        if self.settings_load.is_some() {
            let response = modal(ctx, "Settings", 480.0, |ui| {
                dialogs::title_bar(ui, "Settings", |ui| {
                    if dialogs::close_button(ui).clicked() {
                        self.discard_settings(CloseRoute::TitleBar);
                    }
                });
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Loading settings…");
                });
            });
            if response.should_close() {
                self.discard_settings(if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    CloseRoute::Escape
                } else {
                    CloseRoute::Outside
                });
            }
            ctx.request_repaint_after(Duration::from_millis(50));
            return;
        }
        let sections: &[&str] = if self.builder && self.settings_token_entry {
            &BUILDER_TOKEN_SECTIONS
        } else if self.builder {
            &BUILDER_SECTIONS
        } else {
            &MANAGER_SECTIONS
        };
        let saving_token = self.settings_save.is_some();
        let busy = self.release_pending()
            || self.job.snapshot().busy
            || saving_token
            || self
                .rollback_dialog
                .as_ref()
                .is_some_and(dialogs::rollback::RollbackDialog::save_pending);
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
                    if ui
                        .add_enabled_ui(!saving_token, dialogs::close_button)
                        .inner
                        .clicked()
                    {
                        self.discard_settings(CloseRoute::TitleBar);
                    }
                });
                // The dialog is 600pt tall: title bar, scrolling body, footer.
                let total = (ctx.screen_rect().height() - 48.0).min(600.0);
                let height = (total - 2.0 - 2.0 * dialogs::BAR_HEIGHT).max(220.0);
                ui.add_enabled_ui(!saving_token, |ui| {
                    ui.horizontal_top(|ui| {
                        // Scroll areas allow a paint margin for widget shadows.
                        // Bound the entire body so that margin cannot bleed into
                        // the header or footer when settings are scrolled.
                        let body_rect = egui::Rect::from_min_size(
                            ui.cursor().min,
                            egui::vec2(ui.available_width(), height),
                        );
                        ui.set_clip_rect(ui.clip_rect().intersect(body_rect));
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
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(1.0, height), Sense::hover());
                        ui.painter().rect_filled(rect, 0.0, theme::palette().border);
                        egui::ScrollArea::vertical()
                            .id_salt("settings-scroll")
                            .auto_shrink([false, false])
                            .max_height(height)
                            .min_scrolled_height(height)
                            .show(ui, |ui| {
                                if saving_token {
                                    ui.disable();
                                }
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
                                                    "Updates" => {
                                                        if self.builder {
                                                            self.github_token_settings(ui);
                                                        } else {
                                                            self.settings_updates(ui, ctx, busy);
                                                        }
                                                    }
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
                });
                dialogs::footer(ui, |ui| {
                    if saving_token {
                        ui.spinner();
                        ctx.request_repaint_after(Duration::from_millis(50));
                    }
                    if dialogs::action(
                        ui,
                        if saving_token { "Saving…" } else { "Save" },
                        Kind::Primary,
                        !busy
                            && self.custom_receiver.is_none()
                            && self.custom_download_choice.is_none()
                            && self.custom_setup.is_none(),
                    )
                    .on_disabled_hover_text("Wait for the current operation to finish.")
                    .clicked()
                    {
                        self.save_settings(ctx);
                    }
                    if dialogs::action(ui, "Cancel", Kind::Secondary, !saving_token).clicked() {
                        self.discard_settings(CloseRoute::Cancel);
                    }
                });
            },
        );
        if self.visibility_open {
            let dialog = crate::ui::modal(ctx, "App visibility", 480.0, |ui| {
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
        let api_cache_dialog_open = self.api_cache_cleanup.confirm || self.rollback_cleanup.confirm;
        self.api_cache_cleanup_dialog(ctx);
        self.rollback_cleanup_dialog(ctx);
        if !api_cache_dialog_open
            && !self.visibility_open
            && !self.selection
            && !self.confirm_clean
            && !self.confirm_self_update
            && self.error.is_none()
            && self.selection_notice.is_none()
            && self.custom_download_choice.is_none()
            && self.custom_setup.is_none()
            && !saving_token
            && modal.should_close()
        {
            self.discard_settings(if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                CloseRoute::Escape
            } else {
                CloseRoute::Outside
            });
        }
    }

    pub(in crate::ui) fn app_visibility_menu(&mut self, response: &egui::Response, name: &str) {
        // The single rounded action is the whole menu, without an outer popup frame.
        let original_style = response.ctx.style();
        let mut menu_style = (*original_style).clone();
        menu_style.spacing.menu_margin = egui::Margin::ZERO;
        menu_style.spacing.menu_width = 0.0;
        menu_style.visuals.window_fill = egui::Color32::TRANSPARENT;
        menu_style.visuals.window_stroke = egui::Stroke::NONE;
        menu_style.visuals.popup_shadow = egui::epaint::Shadow::NONE;
        response.ctx.set_style(menu_style);
        response.context_menu(|ui| {
            if btn("Hide")
                .kind(Kind::Secondary)
                .icon(theme::Icon::EyeOff)
                .show(ui)
                .clicked()
            {
                let result = (|| -> anyhow::Result<()> {
                    let mut preferences = self.paths.read_preferences()?;
                    let base = preferences.clone();
                    let first_hide = !self.hide_notice_seen;
                    if !preferences.hidden_apps.iter().any(|app| app == name) {
                        preferences.hidden_apps.push(name.to_owned());
                    }
                    let preferences = craft_apps_manager::settings::save_preferences_patch(
                        &self.paths,
                        &base,
                        &preferences,
                    )?;
                    if first_hide {
                        self.hide_notice_open = true;
                    }
                    self.hide_notice_seen = true;
                    self.preferences.hidden_apps = preferences.hidden_apps.clone();
                    self.settings_draft.hidden_apps = preferences.hidden_apps;
                    if self.app == name {
                        self.page = crate::ui::Page::Overview;
                    }
                    Ok(())
                })();
                self.result(result);
                ui.close_menu();
            }
        });
        response.ctx.set_style(original_style);
    }

    fn settings_apps(&mut self, ui: &mut egui::Ui) {
        let busy = self.release_pending() || self.job.snapshot().busy;
        let visible_order = model::ordered_release_apps(
            &self.settings_draft.app_order,
            &self.settings_draft.release_format,
        );
        let shown = visible_order
            .iter()
            .filter(|name| !self.settings_draft.hidden_apps.contains(name))
            .count();
        group(ui, "App visibility", |ui| {
            if row(
                ui,
                "Apps",
                Some(&format!(
                    "{shown} of {} shown in sidebar and Home",
                    visible_order.len()
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
            // Link entry, shaped like the Folders path rows: pinned to the
            // available width so the field can never stretch the card.
            dialogs::band(ui, egui::Margin::symmetric(14, 12), 0.0, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.add(egui::Label::new(egui::RichText::new(
                    "Experimental: Some apps may not install, update, or uninstall correctly. Compatibility varies by publisher and package."
                ).size(12.0).color(theme::palette().amber)).wrap());
                ui.add_space(4.0);
                theme::text(ui, "GitHub repository link", 14.0, theme::palette().text);
                add_pressed = ui
                    .allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), 32.0),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            let adding = self.custom_receiver.is_some();
                            let enabled = !busy
                                && !adding
                                && self.custom_download_choice.is_none()
                                && self.custom_setup.is_none()
                                && !self.custom_link.trim().is_empty();
                            let pressed = btn("Add")
                                .enabled(enabled)
                                .show(ui)
                                .on_disabled_hover_text(if busy {
                                    "Wait for the current operation to finish."
                                } else if adding {
                                    "Checking release downloads. Please wait."
                                } else if self.custom_download_choice.is_some() {
                                    "Choose or cancel the current download selection first."
                                } else {
                                    "Type or paste a GitHub repository link first."
                                })
                                .clicked();
                            if adding {
                                ui.spinner();
                            }
                            dialogs::field_style(ui);
                            let field = ui.add(
                                egui::TextEdit::singleline(&mut self.custom_link)
                                    .hint_text("https://github.com/owner/repo")
                                    .font(FontId::monospace(12.0))
                                    .margin(egui::Margin::symmetric(10, 8))
                                    .min_size(egui::vec2(0.0, 32.0))
                                    .desired_width(ui.available_width()),
                            );
                            pressed
                                || (field.lost_focus()
                                    && ui.input(|input| input.key_pressed(egui::Key::Enter))
                                    && enabled)
                        },
                    )
                    .inner;
            });
            if !self.custom_message.is_empty() {
                dialogs::rule(ui);
                dialogs::band(ui, egui::Margin::symmetric(14, 10), 0.0, |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&self.custom_message).size(13.0).color(
                                if self.custom_failed {
                                    theme::palette().red
                                } else {
                                    theme::palette().text_2
                                },
                            ),
                        )
                        .wrap(),
                    );
                });
            }
            let customs: Vec<_> = self.custom_apps_draft.clone();
            if customs.is_empty() {
                dialogs::rule(ui);
                dialogs::band(ui, egui::Margin::symmetric(14, 10), 0.0, |ui| {
                    theme::text(ui, "No custom apps added.", 13.0, theme::palette().muted);
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
            "Add a GitHub repository, then save Settings. Installation options are chosen separately for each app.",
        );
        if add_pressed {
            self.add_custom_app(ui.ctx());
        }
        if let Some(key) = remove {
            self.remove_custom_confirm = Some(key);
        }
    }

    fn add_custom_app(&mut self, ctx: &egui::Context) {
        let Some(session) = &self.settings_edit else {
            return;
        };
        self.custom_settings_session = Some(session.id());
        let link = self.custom_link.trim().to_string();
        let slug = match catalog::parse_github_slug(&link) {
            Ok(slug) => slug,
            Err(error) => {
                self.custom_message = format!("{error:#}");
                self.custom_failed = true;
                return;
            }
        };
        if self
            .custom_apps_draft
            .iter()
            .any(|entry| entry.repository.eq_ignore_ascii_case(&slug))
        {
            self.custom_message = "This repository is already added.".into();
            self.custom_failed = true;
            return;
        }
        let root = self.paths.root.clone();
        let ctx = ctx.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.custom_receiver = Some(rx);
        self.custom_message = format!("Adding {slug}…");
        self.custom_failed = false;
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<crate::ui::CustomAppAddition> {
                let (entry, release_format, architecture) =
                    catalog::inspect_custom_app_for_system(&root, &slug)?;
                let release: model::Release = craft_apps_manager::network::Network::new(&root)?
                    .json(&format!(
                        "https://api.github.com/repos/{slug}/releases/latest"
                    ))?;
                let preferences = model::Preferences {
                    release_format,
                    architecture,
                    ..Default::default()
                };
                let candidates = craft_apps_manager::downloads::candidates(&release, &preferences);
                let selected = if candidates.len() == 1 {
                    candidates[0].name.clone()
                } else {
                    String::new()
                };
                let architecture_confirmed = !selected.is_empty()
                    && !craft_apps_manager::downloads::architecture_unspecified(&selected);
                Ok(crate::ui::CustomAppAddition::Setup(Box::new(
                    crate::ui::CustomAppSetup {
                        entry,
                        release,
                        preferences,
                        selected,
                        architecture_confirmed,
                        choosing: false,
                    },
                )))
            })()
            .map_err(|error| format!("{error:#}"));
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }

    pub(in crate::ui) fn custom_app_added(&mut self, entry: &catalog::Entry) {
        let Some(session) = &mut self.settings_edit else {
            return;
        };
        let Some(id) = self.custom_settings_session else {
            return;
        };
        if session.id() != id || self.settings_save.is_some() {
            return;
        }
        match catalog::merge_custom_registration(&mut self.custom_apps_draft, entry.clone()) {
            Ok(()) => {
                session.accept_custom_result(id, self.custom_apps_draft.clone());
                self.custom_message = format!(
                    "{} added for {}. Save Settings to keep this change.",
                    entry.title,
                    entry.registered_formats.join(", ")
                );
                self.custom_failed = false;
            }
            Err(error) => {
                self.custom_message = format!("{error:#}");
                self.custom_failed = true;
            }
        }
    }

    pub(in crate::ui) fn remove_custom_app(&mut self, key: &str) {
        self.custom_apps_draft.retain(|entry| entry.key != key);
        self.custom_message =
            "App removed from this manager. Save Settings to keep this change.".into();
        self.custom_failed = false;
    }

    fn settings_visibility(&mut self, ui: &mut egui::Ui) {
        let names = model::ordered_release_apps(
            &self.settings_draft.app_order,
            &self.settings_draft.release_format,
        );
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

    fn save_settings(&mut self, ctx: &egui::Context) {
        if self.settings_save.is_some()
            || self.settings_load.is_some()
            || self
                .rollback_dialog
                .as_ref()
                .is_some_and(dialogs::rollback::RollbackDialog::save_pending)
        {
            return;
        }
        let Some(session) = &mut self.settings_edit else {
            return;
        };
        let Some(draft) = session.drafts_mut() else {
            return;
        };
        if !self.builder {
            draft.manager = self.settings_draft.clone();
            draft.custom_apps = self.custom_apps_draft.clone();
            draft.scheduler_enabled = self.auto_draft;
        }
        draft.token = std::mem::take(&mut self.token_draft);
        draft.builder = self.build_draft.clone();
        draft.locations = edit_session::Locations {
            root: PathBuf::from(self.root_text.trim()),
            tools: PathBuf::from(self.tools_text.trim()),
        };
        match session.freeze_for_save() {
            Ok(edit) => {
                self.token_reveal = false;
                self.settings_save = Some(save::spawn(
                    save::SaveRequest {
                        paths: self.paths.clone(),
                        home: self.home.clone(),
                        edit,
                    },
                    save::SystemBackend,
                    ctx.clone(),
                ));
            }
            Err(error) => self.error = Some(format!("{error:#}")),
        }
    }

    fn settings_about(&self, ui: &mut egui::Ui) {
        let icon_id = egui::Id::new("about-manager-icon");
        let mut icon = ui
            .ctx()
            .data(|data| data.get_temp::<egui::TextureHandle>(icon_id));
        if icon.is_none() {
            if let Ok(image) = image::load_from_memory(include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/icon.png"
            ))) {
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
                            format!(
                                "Version {}",
                                craft_apps_manager::update_revisions::display_version(
                                    env!("CARGO_PKG_VERSION"),
                                    craft_apps_manager::update_revisions::BUILD_REVISION
                                )
                            ),
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
                ui.add(egui::Label::new("Install and launch Craft apps, check for new releases, manage rollback versions and launch settings, or download source code and build apps yourself.").wrap());
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
            ui.add_enabled_ui(self.tray_wake.can_hide(), |ui| {
                check_row(
                ui,
                &mut self.settings_draft.close_to_tray,
                "Keep running in the system tray when closed",
                Some(
                    "Closing hides the window. Update checks continue. Use the tray menu to exit.",
                ),
            );
            });
            if !self.tray_wake.can_hide() {
                note(ui, "Close-to-tray needs X11 or XWayland. Closing will quit normally on this desktop.");
            }
            if let Some(message) = &self.tray_error {
                note(ui, message);
            }
        });
        ui.add_space(24.0);
        group(ui, "Activity", |ui| {
            check_row(
                ui,
                &mut self.settings_draft.hide_activity_badge,
                "Hide Activity badge",
                Some("Unread activities stay highlighted in the Activity menu."),
            );
        });
    }

    fn github_token_settings(&mut self, ui: &mut egui::Ui) {
        use craft_apps_manager::network;
        let pending = self.settings_save.is_some() || self.token_receiver.is_some();
        let mut add = false;
        let mut remove = false;
        let token_top = ui.cursor().top();
        group(ui, "GitHub token", |ui| {
            dialogs::band(ui, egui::Margin::symmetric(14, 12), 0.0, |ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), 34.0),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        add = btn(if network::saved_token_present() {
                            "Replace"
                        } else {
                            "Add"
                        })
                        .enabled(!pending && !self.token_draft.input.trim().is_empty())
                        .show(ui)
                        .clicked();
                        if theme::icon_button(
                            ui,
                            if self.token_reveal {
                                theme::Icon::EyeOff
                            } else {
                                theme::Icon::Eye
                            },
                            if self.token_reveal {
                                "Hide token"
                            } else {
                                "Show token"
                            },
                            theme::palette().muted,
                        )
                        .clicked()
                        {
                            self.token_reveal = !self.token_reveal;
                        }
                        dialogs::field_style(ui);
                        let token_input_id = ui.make_persistent_id("github-token-input");
                        token_draft::prepare_token_paste(ui.ctx(), token_input_id);
                        let input = ui.add(
                            egui::TextEdit::singleline(&mut self.token_draft.input)
                                .id(token_input_id)
                                .password(!self.token_reveal)
                                .hint_text("Paste your GitHub token")
                                .margin(egui::Margin::symmetric(10, 8))
                                .min_size(egui::vec2(0.0, 34.0))
                                .desired_width(ui.available_width()),
                        );
                        if self.settings_token_focus && !pending && !ui.is_sizing_pass() {
                            input.request_focus();
                        }
                    },
                );
                ui.add_space(10.0);
                ui.add(
                    egui::Label::new(
                        RichText::new("Optional. Raises the GitHub API request limit. Changes apply with Save.")
                            .size(12.0)
                            .color(theme::palette().muted),
                    )
                    .wrap(),
                );
                ui.add_space(8.0);
                ui.hyperlink_to(
                    RichText::new("Create a classic token ↗").size(12.0),
                    crate::ui::widgets::github_activity::CLASSIC_TOKEN_URL,
                );
                let guide =
                    egui::CollapsingHeader::new(RichText::new("How to create one").size(12.0))
                        .id_salt("github-token-help")
                        .open(Some(self.token_help_open))
                        .show(ui, |ui| {
                            for (index, step) in crate::ui::widgets::github_activity::TOKEN_STEPS
                                .iter()
                                .enumerate()
                            {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(format!("{}. {step}", index + 1))
                                            .size(12.0)
                                            .color(theme::palette().muted),
                                    )
                                    .wrap(),
                                );
                                ui.add_space(5.0);
                            }
                        });
                if guide.header_response.clicked() {
                    self.token_help_open = !self.token_help_open;
                }
                if network::token_source().is_some_and(|source| source != "System credential store")
                {
                    ui.add_space(6.0);
                    ui.add(egui::Label::new(RichText::new("An environment token is active and takes priority over a saved token.").size(12.0).color(theme::palette().muted)).wrap());
                }
                if !self.token_message.is_empty() {
                    ui.add_space(6.0);
                    ui.add(
                        egui::Label::new(
                            RichText::new(&self.token_message)
                                .size(12.0)
                                .color(theme::palette().muted),
                        )
                        .wrap(),
                    );
                }
            });
            if let Some((title, detail)) = self.token_draft.row(network::saved_token_present()) {
                dialogs::rule(ui);
                remove = row(ui, title, Some(detail), 56.0, |ui| {
                    btn(self.token_draft.row_action_label())
                        .enabled(!pending)
                        .show(ui)
                        .clicked()
                });
            }
        });
        if self.settings_token_focus && !pending && !ui.is_sizing_pass() {
            // Measure the complete group, including help and the saved-token row,
            // after the modal's invisible sizing pass has established its viewport.
            ui.scroll_to_rect(
                egui::Rect::from_min_max(
                    egui::pos2(ui.max_rect().left(), token_top),
                    egui::pos2(ui.max_rect().right(), ui.cursor().top()),
                ),
                Some(egui::Align::Center),
            );
            self.settings_token_focus = false;
        }
        if add || remove {
            self.token_reveal = false;
            if remove {
                if self.token_draft.cancel_pending_set() {
                    self.token_message = if network::saved_token_present() {
                        "Replacement cancelled. Your saved token is unchanged."
                    } else {
                        "Token addition cancelled."
                    }
                    .into();
                } else if self.token_draft.undo_removal() {
                    self.token_message = "Token removal cancelled.".into();
                } else if network::saved_token_present() {
                    self.token_draft.remove();
                    self.token_message = "Token removal pending. Click Save to apply.".into();
                } else {
                    self.token_draft.discard();
                    self.token_message = "Pending token discarded.".into();
                }
            } else {
                self.token_draft.stage();
                self.token_message = "Token change pending. Click Save to apply.".into();
            }
        }
    }

    fn settings_updates(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, busy: bool) {
        let building = self.build_job.snapshot().busy;
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
                &format!(
                    "Version {}",
                    craft_apps_manager::update_revisions::display_version(
                        env!("CARGO_PKG_VERSION"),
                        craft_apps_manager::update_revisions::BUILD_REVISION
                    )
                ),
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
                self.selection_origin = self
                    .settings_edit
                    .as_ref()
                    .map(|s| SelectionOrigin::Settings(s.id()));
                self.selection_draft = self.settings_draft.selected_apps.clone();
                self.selection = true;
            }
            dialogs::rule(ui);
            check_row(
                ui,
                &mut self.settings_draft.check_catalog_on_startup,
                "Look for new Craft apps on startup",
                None,
            )
            .on_hover_text("Reuses recent discovery results for up to a minute. Discovery never downloads or installs apps.");
            dialogs::rule(ui);
            check_row(
                ui,
                &mut self.settings_draft.check_catalog_with_app_updates,
                "Look for new Craft apps during automatic update checks",
                None,
            )
            .on_hover_text("Uses periodic or background update checks at your configured interval. Recent simultaneous checks share their result.");
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
                Some("Check the manager and installed apps. Waits until downloads and builds finish."),
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
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        {
            ui.add_space(16.0);
            group(ui, "Batch updates", |ui| {
                check_row(ui, &mut self.settings_draft.batch_admin,
                    "Approve administrator access once per batch",
                    Some("Use one administrator prompt for multiple installer updates. Package or installer questions may still appear."));
            });
        }
        ui.add_space(16.0);
        group(ui, "Rollback", |ui| {
            let mut enabled = self.settings_draft.rollback_enabled();
            if check_row(
                ui,
                &mut enabled,
                "Keep the previous version for rollback",
                Some("Restore an app’s previous version if an update causes problems."),
            )
            .changed()
            {
                self.settings_draft.rollback_enabled = Some(enabled);
            }
            ui.add_enabled_ui(enabled, |ui| {
                check_row(ui, &mut self.settings_draft.compress_rollbacks,
                    "Compress rollback versions", Some("Saves space for future portable rollback versions. Creating and restoring them takes longer."));
            });
            self.rollback_cleanup_row(ui);
        });
        ui.add_space(16.0);
        self.github_token_settings(ui);
        ui.add_space(16.0);
        self.api_cache_cleanup_row(ui);
    }

    fn settings_builds(&mut self, ui: &mut egui::Ui, busy: bool) {
        let building = self.build_job.snapshot().busy;
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
        let package_install =
            self_update::installed_with_msi() || self_update::installed_with_linux_package();
        let portable = !package_install && craft_apps_manager::portable_mode::enabled(&self.home);
        if portable {
            group(ui, "Portable storage", |ui| {
                dialogs::band(ui, egui::Margin::symmetric(14, 12), 0.0, |ui| {
                    note(ui, "Data inside this folder moves with the manager.");
                    note(ui, "Finish active operations and close Manager and Builder before moving the whole folder.");
                    if !self.paths.root.starts_with(&self.home)
                        || !self.paths.tools.starts_with(&self.home)
                    {
                        note(ui, "Your library or tools are outside this folder and will not move with the manager.");
                    }
                });
            });
            ui.add_space(16.0);
        }
        group(ui, "Folders", |ui| {
            let block = egui::Margin::symmetric(14, 12);
            dialogs::band(ui, block, 0.0, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                theme::text(
                    ui,
                    "Library (releases, sources, builds, logs, rollback versions)",
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
