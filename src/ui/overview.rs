//! The Overview page: bulk updates in the middle, status, hourly checks and activity on the right.
use super::theme::{self, btn, Icon, Size};
use super::App;
use craft_apps_manager::{
    jobs::State,
    model::{self, APPS, SOURCES},
    scheduler,
};
use eframe::egui::{
    self, Align, Color32, CornerRadius, FontId, Layout, Rect, RichText, Sense, Ui, UiBuilder, Vec2,
};
use std::sync::atomic::Ordering;

/// The status dot while an operation runs.
const BUSY_DOT: Color32 = Color32::from_rgb(0x5b, 0x8c, 0xff);

impl App {
    pub(super) fn overview(&mut self, ctx: &egui::Context, state: &State) {
        egui::SidePanel::right("overview-side")
            .resizable(false)
            .exact_width(320.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL)
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
            .frame(egui::Frame::new().fill(theme::BG))
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
        let order = self.preferences.app_order.clone();
        let statuses: Vec<_> = order.iter().map(|name| (name, self.status(name))).collect();
        let installed = statuses
            .iter()
            .filter(|(_, s)| s.installed.is_some())
            .count();
        let updates: Vec<_> = statuses
            .iter()
            .filter_map(|(name, s)| {
                Some((
                    (*name).clone(),
                    s.installed.clone()?.version,
                    s.update.clone()?,
                ))
            })
            .collect();
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        // Heading block: 26pt title at line-height 1.2, then the summary 4pt below.
        text_line(ui, "Overview", theme::bold(26.0), theme::TEXT, 31.2);
        ui.add_space(4.0);
        ui.add(
            egui::Label::new(
                RichText::new(format!(
                    "{} apps · {installed} installed · {} · {}, {}",
                    APPS.len(),
                    match updates.len() {
                        0 => "no updates found".to_owned(),
                        1 => "1 update available".to_owned(),
                        n => format!("{n} updates available"),
                    },
                    release_format_label(&self.preferences.release_format),
                    model::architecture_label(&self.preferences.architecture),
                ))
                .size(14.0)
                .color(theme::TEXT_3),
            )
            .wrap(),
        );
        ui.add_space(24.0);
        let card_width = ui.available_width().min(760.0);
        theme::card().show(ui, |ui| {
            ui.set_width(card_width - 2.0);
            let subtitle = format!(
                "Installs or updates the {} of {} apps chosen in Settings. You review each step first.",
                self.preferences.selected_apps.len(),
                APPS.len()
            );
            let label = if updates.is_empty() {
                "Update all".to_owned()
            } else {
                format!("Update all · {}", updates.len())
            };
            let button = btn(&label)
                .primary()
                .size(Size::Card)
                .icon(Icon::Refresh)
                .enabled(!state.busy);
            card_header(ui, "App updates", &subtitle, button.desired_size(ui), |ui| {
                if button
                    .show(ui)
                    .on_hover_text("Plans installs and updates for the apps chosen in Settings and asks you to confirm them")
                    .on_disabled_hover_text("Wait for the current operation to finish.")
                    .clicked()
                {
                    self.start("releases");
                }
            });
            for (name, installed, latest) in &updates {
                theme::divider(ui);
                // 60pt of content plus 8pt above and below, like the mockup's rows.
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), 76.0), Sense::hover());
                let inner = rect.shrink2(egui::vec2(18.0, 8.0));
                if let Some(texture) = self.icons.get(name) {
                    let icon = Rect::from_min_size(
                        egui::pos2(inner.left(), inner.center().y - 18.0),
                        Vec2::splat(36.0),
                    );
                    egui::Image::new((texture.id(), icon.size()))
                        .corner_radius(CornerRadius::same(9))
                        .paint_at(ui, icon);
                }
                let mut action = ui.new_child(
                    UiBuilder::new()
                        .max_rect(inner)
                        .layout(Layout::right_to_left(Align::Center)),
                );
                if btn("Details").show(&mut action).clicked() {
                    self.select(name);
                }
                let text_left = inner.left() + 36.0 + 12.0;
                let text_right = action.min_rect().left() - 12.0;
                let name_height = row_height(ui, &theme::medium(14.0));
                let version_height = row_height(ui, &FontId::proportional(12.0));
                let text_height = name_height + 2.0 + version_height;
                let mut text = ui.new_child(
                    UiBuilder::new()
                        .max_rect(Rect::from_min_max(
                            egui::pos2(text_left, inner.center().y - text_height / 2.0),
                            egui::pos2(text_right.max(text_left), inner.bottom()),
                        ))
                        .layout(Layout::top_down(Align::Min)),
                );
                text.spacing_mut().item_spacing.y = 2.0;
                text.add(
                    egui::Label::new(
                        RichText::new(model::title(name))
                            .font(theme::medium(14.0))
                            .color(theme::TEXT),
                    )
                    .truncate(),
                );
                text.add(
                    egui::Label::new(
                        RichText::new(format!("{installed} {} {latest}", theme::arrow()))
                            .size(12.0)
                            .color(theme::LINK),
                    )
                    .truncate(),
                );
            }
        });
        ui.add_space(24.0);
        let downloaded = SOURCES
            .iter()
            .filter(|name| self.display_sources.contains_key(**name))
            .count();
        theme::card().show(ui, |ui| {
            ui.set_width(card_width - 2.0);
            let subtitle = format!(
                "{} · {} of {} sources chosen in Settings",
                match downloaded {
                    0 => "None downloaded yet".to_owned(),
                    n => format!("{n} downloaded"),
                },
                self.preferences.selected_sources.len(),
                SOURCES.len()
            );
            let button = btn("Update all sources")
                .size(Size::Card)
                .icon(Icon::Code)
                .enabled(!state.busy);
            card_header(ui, "Sources", &subtitle, button.desired_size(ui), |ui| {
                if button
                    .show(ui)
                    .on_disabled_hover_text("Wait for the current operation to finish.")
                    .clicked()
                {
                    self.start("sources");
                }
            });
        });
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
        theme::section_label(ui, "Hourly checks");
        ui.add_space(11.0);
        if theme::check_label(ui, &mut self.auto, "App updates", !state.busy)
            .on_hover_text("Checks selected installed apps and notifies you when a new version is available. Supports installers and portable ZIPs; nothing downloads automatically.")
            .changed()
        {
            let result = scheduler::set(&self.paths, false, self.auto);
            if result.is_err() {
                self.auto = !self.auto;
            }
            self.result(result)
        }
        ui.add_space(10.0);
        if theme::check_label(ui, &mut self.auto_source, "Source updates", !state.busy)
            .on_hover_text("Runs hourly and after sign-in for the apps and sources chosen in Settings. Downloads always ask first.")
            .changed()
        {
            let result = scheduler::set(&self.paths, true, self.auto_source);
            if result.is_err() {
                self.auto_source = !self.auto_source;
            }
            self.result(result)
        }
        ui.add_space(19.0);
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
                        .color(theme::LINK)
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
                theme::TEXT,
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
                    theme::MUTED,
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
                theme::RED
            } else {
                theme::GREEN
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
        text.add(egui::Label::new(RichText::new(title).size(14.0).color(theme::TEXT)).wrap());
        if !state.detail.is_empty() {
            text.add(
                egui::Label::new(RichText::new(&state.detail).size(12.0).color(if failed {
                    theme::RED_TEXT
                } else {
                    theme::MUTED
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
            ui.add_space(10.0);
            theme::progress(ui, state.progress, 4.0);
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

/// Card title and wrapped subtitle on the left, the card's 40pt action on the right,
/// both centred vertically inside 16pt by 18pt padding.
fn card_header(
    ui: &mut Ui,
    title: &str,
    subtitle: &str,
    action_size: Vec2,
    action: impl FnOnce(&mut Ui),
) {
    let width = ui.available_width();
    let text_width = (width - 36.0 - action_size.x - 12.0).max(80.0);
    let title_height = row_height(ui, &theme::bold(16.0));
    let subtitle_height = ui.fonts(|f| {
        f.layout(
            subtitle.to_owned(),
            FontId::proportional(13.0),
            theme::MUTED,
            text_width,
        )
        .size()
        .y
    });
    let text_height = title_height + 2.0 + subtitle_height;
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(width, text_height.max(action_size.y) + 32.0),
        Sense::hover(),
    );
    let inner = rect.shrink2(egui::vec2(18.0, 16.0));
    let mut text = ui.new_child(
        UiBuilder::new()
            .max_rect(Rect::from_min_size(
                egui::pos2(inner.left(), inner.center().y - text_height / 2.0),
                egui::vec2(text_width, text_height),
            ))
            .layout(Layout::top_down(Align::Min)),
    );
    text.spacing_mut().item_spacing.y = 2.0;
    text.add(egui::Label::new(
        RichText::new(title)
            .font(theme::bold(16.0))
            .color(theme::TEXT),
    ));
    text.add(egui::Label::new(RichText::new(subtitle).size(13.0).color(theme::MUTED)).wrap());
    let mut right = ui.new_child(
        UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::right_to_left(Align::Center)),
    );
    action(&mut right);
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
