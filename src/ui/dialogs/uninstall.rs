use super::*;

impl App {
    pub(super) fn render_uninstall_dialog(&mut self, ctx: &egui::Context) {
        if self.confirm_uninstall {
            let modal = modal(
                ctx,
                format!("Uninstall {}?", model::title(&self.app)),
                520.0,
                |ui| {
                    let installed = self
                        .display_config
                        .as_ref()
                        .and_then(|c| c.apps.iter().find(|a| a.name == self.app))
                        .map(|a| a.path.clone())
                        .filter(|path| !path.is_empty());
                    let art = match self.icons.get(&self.app) {
                        Some(texture) => Art::Texture(texture.id()),
                        None => Art::None,
                    };
                    band(ui, BODY, 0.0, |ui| {
                        header(
                            ui,
                            art,
                            &format!("Uninstall {}?", model::title(&self.app)),
                            &self
                                .display_config
                                .as_ref()
                                .and_then(|c| c.apps.iter().find(|a| a.name == self.app))
                                .map(|a| {
                                    format!(
                                        "Version {} · {} · {}",
                                        a.version,
                                        if a.install_kind == "installer" {
                                            "Installer"
                                        } else {
                                            "Portable"
                                        },
                                        model::architecture_label(&a.architecture)
                                    )
                                })
                                .unwrap_or_else(|| "Remove the installed app".into()),
                            |_| {},
                        );
                        ui.add_space(22.0);
                        theme::text(ui, "Installed location", 13.0, theme::palette().text_3);
                        ui.add_space(8.0);
                        if let Some(path) = &installed {
                            ui.horizontal(|ui| {
                                let button_width = 112.0;
                                ui.allocate_ui_with_layout(
                                    egui::vec2(
                                        (ui.available_width()
                                            - button_width
                                            - ui.spacing().item_spacing.x)
                                            .max(40.0),
                                        32.0,
                                    ),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.add(egui::Label::new(compact_path(path)).truncate())
                                            .on_hover_text(path);
                                    },
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if open_folder_button(ui).clicked() {
                                            self.result(craft_apps_manager::platform::open(
                                                std::path::Path::new(path),
                                            ));
                                        }
                                    },
                                );
                            });
                        }
                        ui.add_space(12.0);
                        note(ui, "Projects and images saved outside the app folder are kept. Move personal files stored directly inside that folder before uninstalling.");
                        ui.add_space(8.0);
                        note(ui, "Source ZIPs, compiled builds, backups and launch settings are also kept.");
                        ui.add_space(18.0);
                        let targets = profiles::targets(&self.paths, &self.app);
                        theme::group().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            let label = format!(
                                "Also delete {} settings and caches",
                                model::title(&self.app)
                            );
                            band(ui, egui::Margin::symmetric(14, 12), 0.0, |ui| {
                                ui.horizontal_top(|ui| {
                                    ui.spacing_mut().item_spacing.x = 12.0;
                                    ui.vertical(|ui| {
                                        ui.add_space(1.0);
                                        theme::checkbox(
                                            ui,
                                            &mut self.delete_profile,
                                            18.0,
                                            CHECK_RED,
                                            &label,
                                            targets.is_ok(),
                                        );
                                    });
                                    ui.vertical(|ui| {
                                        ui.spacing_mut().item_spacing.y = 2.0;
                                        let enabled = targets.is_ok();
                                        let title = ui.add_enabled(
                                            enabled,
                                            egui::Label::new(
                                                RichText::new(&label).size(14.0).color(theme::palette().text),
                                            )
                                            .sense(Sense::click()),
                                        );
                                        if enabled && title.clicked() {
                                            self.delete_profile = !self.delete_profile;
                                        }
                                        note(ui, "Includes plug-ins and recovery copies. Other copies of the app may share them.");
                                    });
                                });
                            });
                            rule(ui);
                            let list_margin = egui::Margin {
                                left: 44,
                                right: 14,
                                top: 10,
                                bottom: 12,
                            };
                            match &targets {
                                Ok(targets) => {
                                    let existing: Vec<_> =
                                        targets.iter().filter(|t| t.path.exists()).collect();
                                    band(ui, list_margin, 0.0, |ui| {
                                        ui.spacing_mut().item_spacing.y = 4.0;
                                        if existing.is_empty() {
                                            note(ui, "No existing profile folders found.");
                                        }
                                        egui::ScrollArea::vertical().max_height(100.0).show(
                                            ui,
                                            |ui| {
                                                ui.spacing_mut().item_spacing.y = 4.0;
                                                for target in existing {
                                                    let response = ui.label(
                                                        RichText::new(
                                                            target.path.display().to_string(),
                                                        )
                                                        .font(FontId::monospace(12.0))
                                                        .color(theme::palette().text_2),
                                                    );
                                                    ui.painter().circle_filled(
                                                        egui::pos2(
                                                            response.rect.left() - 12.0,
                                                            response.rect.center().y,
                                                        ),
                                                        2.5,
                                                        theme::palette().text_2,
                                                    );
                                                }
                                            },
                                        );
                                    });
                                    band(
                                        ui,
                                        egui::Margin {
                                            top: 0,
                                            ..list_margin
                                        },
                                        0.0,
                                        |ui| {
                                            note(ui, "Custom profile locations outside these folders are kept.");
                                        },
                                    );
                                }
                                Err(error) => {
                                    band(ui, list_margin, 0.0, |ui| {
                                        note(ui, format!("Not available: {error}"));
                                    });
                                }
                            }
                        });
                    });
                    footer(ui, |ui| {
                        if action(ui, "Uninstall", Kind::Danger, true).clicked() {
                            self.start("uninstall-app");
                            self.confirm_uninstall = false;
                        }
                        if action(ui, "Cancel", Kind::Secondary, true).clicked() {
                            self.confirm_uninstall = false;
                        }
                    });
                },
            );
            if modal.should_close() {
                self.confirm_uninstall = false;
            }
        }
    }
}
