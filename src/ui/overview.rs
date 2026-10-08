//! The Overview page: bulk updates in the middle, status, hourly checks and activity on the right.
use super::theme::{self, btn, Icon};
use super::App;
use craft_apps_manager::{
    jobs::State,
    model::{self, APPS, SOURCES},
    scheduler,
};
use eframe::egui::{self, CornerRadius, RichText};

impl App {
    pub(super) fn overview(&mut self, ctx: &egui::Context, state: &State) {
        egui::SidePanel::right("overview-side")
            .resizable(false)
            .exact_width(320.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .inner_margin(egui::Margin::same(20)),
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
                                ui.set_max_width(ui.available_width().min(760.0));
                                self.overview_main(ui, state);
                            });
                    });
            });
    }

    fn overview_main(&mut self, ui: &mut egui::Ui, state: &State) {
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
        ui.spacing_mut().item_spacing.y = 4.0;
        theme::heading(ui, "Overview", 26.0);
        theme::text(
            ui,
            format!(
                "{} apps · {installed} installed · {} · {}, {}",
                APPS.len(),
                match updates.len() {
                    0 => "no updates found".to_owned(),
                    1 => "1 update available".to_owned(),
                    n => format!("{n} updates available"),
                },
                release_format_label(&self.preferences.release_format),
                model::architecture_label(&self.preferences.architecture),
            ),
            14.0,
            theme::TEXT_3,
        );
        ui.add_space(24.0);
        ui.spacing_mut().item_spacing.y = 8.0;
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(18, 16))
                .show(ui, |ui| {
                    let subtitle = format!(
                        "Installs or updates the {} of {} apps chosen in Settings. You review each step first.",
                        self.preferences.selected_apps.len(),
                        APPS.len()
                    );
                    card_header(ui, "App updates", &subtitle, |ui| {
                        let label = if updates.is_empty() {
                            "Update all".to_owned()
                        } else {
                            format!("Update all · {}", updates.len())
                        };
                        if btn(&label)
                            .primary()
                            .medium()
                            .icon(Icon::Refresh)
                            .enabled(!state.busy)
                            .show(ui)
                            .on_hover_text("Plans installs and updates for the apps chosen in Settings and asks you to confirm them")
                            .on_disabled_hover_text("Wait for the current operation to finish.")
                            .clicked()
                        {
                            self.start("releases");
                        }
                    });
                });
            for (name, installed, latest) in &updates {
                theme::divider(ui);
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(18, 10))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if let Some(texture) = self.icons.get(name) {
                                ui.add(
                                    egui::Image::new((texture.id(), egui::vec2(36.0, 36.0)))
                                        .corner_radius(CornerRadius::same(9)),
                                );
                            }
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                theme::text(ui, model::title(name), 14.0, theme::TEXT);
                                theme::text(
                                    ui,
                                    format!("{installed} {} {latest}", theme::arrow()),
                                    12.0,
                                    theme::LINK,
                                );
                            });
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if btn("Details").show(ui).clicked() {
                                        self.select(name);
                                    }
                                },
                            );
                        });
                    });
            }
        });
        ui.add_space(8.0);
        let downloaded = SOURCES
            .iter()
            .filter(|name| self.display_sources.contains_key(**name))
            .count();
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(18, 16))
                .show(ui, |ui| {
                    let subtitle = format!(
                        "{} · {} of {} sources chosen in Settings",
                        match downloaded {
                            0 => "None downloaded yet".to_owned(),
                            n => format!("{n} downloaded"),
                        },
                        self.preferences.selected_sources.len(),
                        SOURCES.len()
                    );
                    card_header(ui, "Sources", &subtitle, |ui| {
                        if btn("Update all sources")
                            .medium()
                            .icon(Icon::Code)
                            .enabled(!state.busy)
                            .show(ui)
                            .on_disabled_hover_text("Wait for the current operation to finish.")
                            .clicked()
                        {
                            self.start("sources");
                        }
                    });
                });
        });
    }

    fn overview_side(&mut self, ui: &mut egui::Ui, state: &State) {
        ui.spacing_mut().item_spacing.y = 10.0;
        theme::group().show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(14, 12))
                .show(ui, |ui| {
                    let failed = !state.busy && state.stage == "Failed";
                    ui.horizontal(|ui| {
                        theme::dot(
                            ui,
                            if state.busy {
                                theme::LINK
                            } else if failed {
                                theme::RED
                            } else {
                                theme::GREEN
                            },
                        );
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            theme::text(
                                ui,
                                if state.stage.is_empty() {
                                    "Ready"
                                } else {
                                    &state.stage
                                },
                                14.0,
                                theme::TEXT,
                            );
                            if !state.detail.is_empty() {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&state.detail).size(12.0).color(if failed {
                                            theme::RED_TEXT
                                        } else {
                                            theme::MUTED
                                        }),
                                    )
                                    .wrap(),
                                );
                            }
                        });
                    });
                    if state.busy {
                        ui.add_space(4.0);
                        theme::progress(ui, state.progress, 6.0);
                        ui.add_space(4.0);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                            self.cancel_button(ui, &state.stage);
                        });
                    }
                });
        });
        ui.add_space(8.0);
        theme::section_label(ui, "Hourly checks");
        if ui
            .add_enabled(
                !state.busy,
                egui::Checkbox::new(&mut self.auto, "App updates"),
            )
            .on_hover_text("Checks selected installed apps and notifies you when a new version is available. Supports installers and portable ZIPs; nothing downloads automatically.")
            .changed()
        {
            let result = scheduler::set(&self.paths, false, self.auto);
            if result.is_err() {
                self.auto = !self.auto;
            }
            self.result(result)
        }
        if ui
            .add_enabled(
                !state.busy,
                egui::Checkbox::new(&mut self.auto_source, "Source updates"),
            )
            .changed()
        {
            let result = scheduler::set(&self.paths, true, self.auto_source);
            if result.is_err() {
                self.auto_source = !self.auto_source;
            }
            self.result(result)
        }
        theme::text(
            ui,
            "Runs hourly and after sign-in for the apps and sources chosen in Settings. Downloads always ask first.",
            12.0,
            theme::MUTED,
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            theme::section_label(ui, "Activity");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let exists = self.job.log_path.is_file();
                let response = ui
                    .add_enabled(
                        exists,
                        egui::Label::new(RichText::new("Open log").size(13.0).color(theme::LINK))
                            .sense(egui::Sense::click()),
                    )
                    .on_hover_text(self.job.log_path.display().to_string())
                    .on_disabled_hover_text("No log file yet. Run an operation to create one.");
                if response.clicked() {
                    let result = craft_apps_manager::platform::open(&self.job.log_path);
                    self.result(result);
                }
            });
        });
        log_view(
            ui,
            if state.log.is_empty() {
                "Ready. Craft app checks run only when requested or scheduled."
            } else {
                &state.log
            },
        );
    }
}

/// Card title and wrapped subtitle on the left, the card's action on the right.
fn card_header(ui: &mut egui::Ui, title: &str, subtitle: &str, action: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            action(ui);
            ui.add_space(16.0);
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                theme::heading(ui, title, 16.0);
                ui.add(
                    egui::Label::new(RichText::new(subtitle).size(13.0).color(theme::MUTED)).wrap(),
                );
            });
        });
    });
}

/// Monospace, selectable log text that follows new lines.
pub(super) fn log_view(ui: &mut egui::Ui, text: &str) {
    egui::Frame::new()
        .fill(theme::LOG)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .max_height((ui.available_height() - 4.0).max(80.0))
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(text)
                                .monospace()
                                .size(12.0)
                                .color(theme::TEXT_3),
                        )
                        .selectable(true)
                        .wrap_mode(egui::TextWrapMode::Extend),
                    );
                });
        });
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
