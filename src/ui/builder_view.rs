//! The separate builder window (`--builder`): build any source, including ArtCraft X.
use super::theme::{self, btn, Icon};
use super::App;
use craft_apps_manager::{
    builder,
    jobs::{Job, State},
    model, platform,
};
use eframe::egui::{self, RichText};

impl App {
    pub(super) fn builder_view(&mut self, ctx: &egui::Context, state: &State) {
        egui::TopBottomPanel::top("header")
            .frame(
                egui::Frame::new()
                    .fill(theme::palette().panel)
                    .inner_margin(egui::Margin::symmetric(20, 12)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Craft Apps Builder")
                            .font(theme::bold(15.0))
                            .color(theme::palette().text),
                    );
                    theme::text(
                        ui,
                        format!("Version {}", env!("CARGO_PKG_VERSION")),
                        12.0,
                        theme::palette().muted,
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        self.activity_settings_buttons(ui);
                    });
                });
                theme::text(
                    ui,
                    "Build original source. Keep control of your tools and output.",
                    13.0,
                    theme::palette().text_3,
                );
            });
        egui::TopBottomPanel::bottom("footer")
            .frame(
                egui::Frame::new()
                    .fill(theme::palette().panel)
                    .inner_margin(egui::Margin::symmetric(16, 6)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        theme::text(
                            ui,
                            if state.busy { "Working" } else { "Ready" },
                            12.0,
                            theme::palette().text_3,
                        );
                        theme::dot(
                            ui,
                            if state.busy {
                                theme::palette().link
                            } else {
                                theme::palette().green
                            },
                        );
                        ui.add_space(12.0);
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(format!("Data: {}", self.paths.root.display()))
                                        .size(12.0)
                                        .color(theme::palette().text_3),
                                )
                                .truncate(),
                            );
                        });
                    });
                });
            });
        egui::SidePanel::left("sidebar")
            .resizable(false)
            .exact_width(240.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::palette().panel)
                    .inner_margin(egui::Margin::same(16)),
            )
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 10.0;
                theme::section_label(ui, "Build controls");
                let previous = self.app.clone();
                egui::ComboBox::from_id_salt("app")
                    .width(ui.available_width())
                    .selected_text(model::title(&self.app))
                    .show_ui(ui, |ui| {
                        for name in model::sources() {
                            ui.add_enabled_ui(!state.busy, |ui| {
                                let label = model::title(&name);
                                ui.selectable_value(&mut self.app, name, label);
                            });
                        }
                    });
                if previous != self.app {
                    self.job = Job::new(
                        self.paths.at(format!("logs/{}.log", self.app)),
                        &self.build_preferences,
                    );
                    self.job.history(&self.job.log_path);
                    self.job.state.lock().unwrap().output =
                        builder::history(&self.paths, &self.app);
                }
                let width = ui.available_width();
                let completed = self
                    .display_builds
                    .get(&self.app)
                    .map(|(folder, _)| folder.clone())
                    .or_else(|| self.job.snapshot().output);
                let has_build = completed.as_ref().is_some_and(|folder| {
                    folder
                        .join(model::build_executable_name(&self.app))
                        .is_file()
                });
                if btn(if has_build {
                    "Rebuild executable"
                } else {
                    "Build executable"
                })
                .primary()
                .medium()
                .icon(Icon::Wrench)
                .min_width(width)
                .enabled(!state.busy)
                .show(ui)
                .clicked()
                {
                    self.start("build")
                }
                if has_build
                    && btn("Launch")
                        .medium()
                        .icon(Icon::Play)
                        .min_width(width)
                        .enabled(!state.busy)
                        .show(ui)
                        .clicked()
                {
                    self.result(builder::launch_local(&self.paths, &self.app));
                }
                if btn("Set up build tools")
                    .medium()
                    .min_width(width)
                    .enabled(!state.busy)
                    .show(ui)
                    .clicked()
                {
                    self.start("setup")
                }
                if btn("Cancel build")
                    .medium()
                    .min_width(width)
                    .enabled(state.busy)
                    .show(ui)
                    .clicked()
                    && !self.job.request_cancel()
                {
                    if let Some(operation) = self.operation_records.iter().find(|op| {
                        craft_apps_manager::jobs::Job::builder_operation(op, &self.app)
                            && op.parent.is_none()
                    }) {
                        let _ = self.job.operations.request_cancel(&operation.id);
                    }
                }
                if btn("Open build folder")
                    .medium()
                    .icon(Icon::Folder)
                    .min_width(width)
                    .enabled(has_build)
                    .show(ui)
                    .clicked()
                {
                    let result =
                        builder::selected_build(&self.paths, &self.app).and_then(|folder| {
                            platform::open(&folder.ok_or_else(|| {
                                anyhow::anyhow!("No completed local build was found")
                            })?)
                        });
                    self.result(result);
                }
                if btn("Build options")
                    .medium()
                    .icon(Icon::Gear)
                    .min_width(width)
                    .enabled(!state.busy)
                    .show(ui)
                    .clicked()
                {
                    self.open_build_options();
                }
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::palette().bg)
                    .inner_margin(egui::Margin::symmetric(28, 24)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if state.busy {
                        ui.spinner();
                    }
                    theme::heading(
                        ui,
                        if state.stage.is_empty() {
                            if state.output.is_some() {
                                "Previous build available"
                            } else {
                                "Ready"
                            }
                        } else {
                            &state.stage
                        },
                        17.0,
                    );
                });
                if !state.detail.is_empty() {
                    theme::text(ui, &state.detail, 13.0, theme::palette().text_3);
                }
                if state.busy {
                    theme::progress(ui, state.progress, 6.0);
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    theme::section_label(ui, "Build log");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let exists = self.job.log_path.is_file();
                        if btn("Open log")
                            .enabled(exists)
                            .show(ui)
                            .on_hover_text(self.job.log_path.display().to_string())
                            .on_disabled_hover_text(
                                "No log file yet. Run an operation to create one.",
                            )
                            .clicked()
                        {
                            let result = platform::open(&self.job.log_path);
                            self.result(result);
                        }
                    });
                });
                theme::log_view(
                    ui,
                    if state.log.is_empty() {
                        "Ready. Craft app checks run only when requested or scheduled."
                    } else {
                        &state.log
                    },
                    false,
                );
            });
    }
}
