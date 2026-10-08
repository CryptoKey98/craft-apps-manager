//! Settings: one scrolling page with a jump list, saved together with one Save.
use super::overview::release_format_label;
use super::theme::{self, btn, Kind};
use super::{modal, App, Locations};
use craft_apps_manager::{
    model::{self, APPS, SOURCES},
    platform, self_update,
};
use eframe::egui::{self, RichText};
use std::{path::PathBuf, time::Duration};

const MANAGER_SECTIONS: [&str; 5] = ["General", "Updates", "Backups", "Builds", "Folders"];
const BUILDER_SECTIONS: [&str; 2] = ["Builds", "Folders"];

/// A titled group of rows inside a section.
fn group(ui: &mut egui::Ui, title: &str, content: impl FnOnce(&mut egui::Ui)) {
    if !title.is_empty() {
        theme::text(ui, title, 13.0, theme::TEXT_3);
    }
    theme::group().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.0;
        content(ui);
    });
}

fn note(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(12.0).color(theme::MUTED)).wrap());
}

fn check_row(
    ui: &mut egui::Ui,
    value: &mut bool,
    title: &str,
    detail: Option<&str>,
) -> egui::Response {
    theme::row(ui, title, detail, |ui| {
        theme::switch(ui, value, title, true)
    })
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
            780.0,
            |ui| {
                ui.horizontal(|ui| {
                    theme::heading(ui, "Settings", 17.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::icon_button(ui, theme::Icon::Close, "Close", theme::TEXT_3)
                            .clicked()
                        {
                            self.settings = false;
                        }
                    });
                });
                ui.add_space(12.0);
                theme::divider(ui);
                let height = (ctx.screen_rect().height() - 230.0).clamp(220.0, 540.0);
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(150.0, height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.add_space(12.0);
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for (index, section) in sections.iter().enumerate() {
                                let selected = index == active;
                                let (rect, response) = ui.allocate_exact_size(
                                    egui::vec2(140.0, 34.0),
                                    egui::Sense::click(),
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
                                        egui::CornerRadius::same(8),
                                        if selected {
                                            theme::SELECTED
                                        } else {
                                            theme::HOVER
                                        },
                                    );
                                }
                                theme::painter_text(
                                    ui.painter(),
                                    egui::pos2(rect.left() + 12.0, rect.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    section,
                                    14.0,
                                    if selected { theme::TEXT } else { theme::TEXT_3 },
                                );
                                if response.clicked() {
                                    self.settings_jump = Some(index);
                                }
                            }
                        },
                    );
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(1.0, height), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 0.0, theme::BORDER);
                    egui::ScrollArea::vertical()
                        .id_salt("settings-scroll")
                        .auto_shrink([false, false])
                        .max_height(height)
                        .show(ui, |ui| {
                            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                                let viewport_top = ui.clip_rect().top();
                                egui::Frame::new()
                                    .inner_margin(egui::Margin {
                                        left: 24,
                                        right: 12,
                                        top: 16,
                                        bottom: 16,
                                    })
                                    .show(ui, |ui| {
                                        ui.spacing_mut().item_spacing.y = 10.0;
                                        let mut current = 0;
                                        for (index, section) in sections.iter().enumerate() {
                                            let top = ui.cursor().top();
                                            if top <= viewport_top + 48.0 {
                                                current = index;
                                            }
                                            if index > 0 {
                                                ui.add_space(18.0);
                                            }
                                            let heading = theme::heading(ui, *section, 16.0);
                                            if self.settings_jump == Some(index) {
                                                heading.scroll_to_me(Some(egui::Align::TOP));
                                                self.settings_jump = None;
                                            }
                                            match *section {
                                                "General" => self.settings_general(ui),
                                                "Updates" => self.settings_updates(ui, ctx, busy),
                                                "Backups" => self.settings_backups(ui),
                                                "Builds" => self.settings_builds(ui),
                                                _ => self.settings_folders(ui),
                                            }
                                        }
                                        // Let the last section scroll to the top of the view.
                                        ui.add_space((height - 260.0).max(0.0));
                                        ctx.data_mut(|d| d.insert_temp(active_id, current));
                                    });
                            });
                        });
                });
                theme::divider(ui);
                ui.add_space(12.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if btn("Save")
                        .primary()
                        .medium()
                        .min_width(88.0)
                        .enabled(!busy)
                        .show(ui)
                        .on_disabled_hover_text("Wait for the current operation to finish.")
                        .clicked()
                    {
                        self.save_settings();
                    }
                    if btn("Cancel").medium().show(ui).clicked() {
                        self.settings = false;
                    }
                });
            },
        );
        if !self.selection
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
                self.display_config = None;
                self.config_receiver = None;
                self.config_refresh_at = std::time::Instant::now();
            }
            self.settings = false;
        }
        self.result(result);
    }

    fn settings_general(&mut self, ui: &mut egui::Ui) {
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
            theme::row(ui, "Release format", Some(hint), |ui| {
                theme::segmented(
                    ui,
                    &mut self.settings_draft.release_format,
                    &[
                        ("portable".to_owned(), release_format_label("portable")),
                        ("installer".to_owned(), "Installer"),
                    ],
                );
            });
            theme::divider(ui);
            if cfg!(target_os = "macos") {
                theme::row(ui, "Architecture", None, |ui| {
                    theme::text(
                        ui,
                        "Universal · Apple silicon and Intel",
                        13.0,
                        theme::TEXT_3,
                    );
                });
            } else {
                theme::row(ui, "Architecture", None, |ui| {
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
            theme::row(
                ui,
                &format!("Version {}", env!("CARGO_PKG_VERSION")),
                Some(&detail),
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
                egui::Frame::new()
                    .inner_margin(egui::Margin {
                        left: 14,
                        right: 14,
                        top: 0,
                        bottom: 10,
                    })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            theme::text(
                                ui,
                                &self.manager_message,
                                13.0,
                                if self.manager_available.is_some() {
                                    theme::LINK
                                } else {
                                    theme::TEXT_3
                                },
                            );
                            if self.manager_available.is_some() {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if btn(if self_update::installed_with_msi() {
                                            "Download and install…"
                                        } else {
                                            "Download and restart…"
                                        })
                                        .primary()
                                        .enabled(!busy && self.manager_plan.is_none())
                                        .show(ui)
                                        .clicked()
                                        {
                                            self.confirm_self_update = true;
                                        }
                                    },
                                );
                            }
                        });
                    });
            }
            theme::divider(ui);
            check_row(
                ui,
                &mut self.settings_draft.check_manager_on_startup,
                "Check for a new version on startup",
                None,
            )
            .on_hover_text(
                "Checks for a new Craft Apps Manager release. Downloads require your confirmation.",
            );
            theme::divider(ui);
            theme::row(ui, "Release source", None, |ui| {
                ui.hyperlink_to(
                    RichText::new(self_update::REPOSITORY_NAME).size(13.0),
                    self_update::REPOSITORY,
                );
            });
        });
        group(ui, "Update all", |ui| {
            let apps = format!(
                "{} of {} included",
                self.settings_draft.selected_apps.len(),
                APPS.len()
            );
            if theme::row(ui, "Apps", Some(&apps), |ui| {
                btn("Choose…").show(ui).clicked()
            }) {
                self.source_selection = false;
                self.selection_draft = self.settings_draft.selected_apps.clone();
                self.selection = true;
            }
            theme::divider(ui);
            let sources = format!(
                "{} of {} included",
                self.settings_draft.selected_sources.len(),
                SOURCES.len()
            );
            if theme::row(ui, "Sources", Some(&sources), |ui| {
                btn("Choose…").show(ui).clicked()
            }) {
                self.source_selection = true;
                self.selection_draft = self.settings_draft.selected_sources.clone();
                self.selection = true;
            }
        });
        group(ui, "Checks and notifications", |ui| {
            check_row(
                ui,
                &mut self.settings_draft.check_installed_apps_on_startup,
                "Check installed apps on startup",
                None,
            )
            .on_hover_text("Checks installed apps in the selected release format. Reports availability only; downloads and installation require confirmation.");
            theme::divider(ui);
            check_row(
                ui,
                &mut self.settings_draft.notify_updates,
                "Notify me when app or source updates are available",
                None,
            );
        });
    }

    fn settings_backups(&mut self, ui: &mut egui::Ui) {
        group(ui, "", |ui| {
            check_row(
                ui,
                &mut self.settings_draft.keep_app_backups,
                "Back up portable apps",
                Some("Before each update. Portable releases only."),
            );
            theme::divider(ui);
            check_row(
                ui,
                &mut self.settings_draft.compress_backups,
                "Compress portable app backups",
                Some("7-Zip Ultra / LZMA2"),
            );
            theme::divider(ui);
            check_row(
                ui,
                &mut self.settings_draft.keep_source_backups,
                "Back up sources",
                Some("Before each source update"),
            );
            theme::divider(ui);
            check_row(
                ui,
                &mut self.settings_draft.compress_source_backups,
                "Recompress source backups",
                Some("7-Zip Ultra / LZMA2"),
            );
            theme::divider(ui);
            theme::row(
                ui,
                "Previous versions to keep",
                Some("Per app and per source"),
                |ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.settings_draft.backup_versions)
                            .range(1..=10),
                    );
                },
            );
        });
        if btn("Delete all backups…")
            .icon_colored(theme::Icon::Trash, theme::RED)
            .show(ui)
            .clicked()
        {
            self.confirm_clear = true;
        }
    }

    fn settings_builds(&mut self, ui: &mut egui::Ui) {
        group(ui, "After a successful build", |ui| {
            check_row(
                ui,
                &mut self.build_draft.delete_cache_after_success,
                "Delete the compilation cache",
                None,
            );
            theme::divider(ui);
            check_row(
                ui,
                &mut self.build_draft.delete_workspace_after_success,
                "Delete the extracted source and node_modules",
                None,
            );
        });
        note(ui, "Cancelled and failed builds keep their cache. Completed builds and tools are retained.");
        group(ui, "Build logs", |ui| {
            theme::row(
                ui,
                "Start a new log at",
                Some("Megabytes per app log"),
                |ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.build_draft.log_size_mb)
                            .range(1..=100)
                            .suffix(" MB"),
                    );
                },
            );
            theme::divider(ui);
            theme::row(ui, "Older logs to keep", None, |ui| {
                ui.add(egui::DragValue::new(&mut self.build_draft.log_archives).range(0..=5));
            });
        });
        ui.horizontal(|ui| {
            if btn("Clean temporary build files…").show(ui).clicked() {
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
        group(ui, "", |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(14, 12))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    theme::text(
                        ui,
                        "Library (releases, sources, builds, logs, backups)",
                        13.0,
                        theme::TEXT_3,
                    );
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.root_text)
                                .desired_width(ui.available_width() - 70.0)
                                .margin(egui::Margin::symmetric(8, 6)),
                        );
                        if btn("Open").show(ui).clicked() {
                            self.result(platform::open(&self.paths.root));
                        }
                    });
                    ui.add_space(6.0);
                    theme::text(ui, "Build tools", 13.0, theme::TEXT_3);
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.tools_text)
                                .desired_width(ui.available_width() - 70.0)
                                .margin(egui::Margin::symmetric(8, 6)),
                        );
                        if btn("Open").show(ui).clicked() {
                            self.result(platform::open(&self.paths.tools));
                        }
                    });
                    ui.add_space(6.0);
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
