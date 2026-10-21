use super::*;

impl App {
    pub(super) fn render_launch_dialogs(&mut self, ctx: &egui::Context) {
        if self.launch_settings_open {
            if !self.launch_build_options {
                let installed = self
                    .display_config
                    .as_ref()
                    .and_then(|config| {
                        config
                            .apps
                            .iter()
                            .find(|copy| copy.name == self.app && !copy.path.is_empty())
                    })
                    .cloned();
                let key = (
                    self.app.clone(),
                    installed
                        .as_ref()
                        .map(|copy| copy.path.clone())
                        .unwrap_or_default(),
                );
                if self.launch_executables_key.as_ref() != Some(&key) {
                    self.launch_executables_key = Some(key);
                    self.launch_executables = None;
                    let paths = self.paths.clone();
                    let app = self.app.clone();
                    let ctx = ctx.clone();
                    let (tx, rx) = std::sync::mpsc::channel();
                    self.launch_executables_receiver = Some(rx);
                    std::thread::spawn(move || {
                        let result = match installed {
                            Some(copy) => apps::executables_for_installed(&paths, &app, &copy),
                            None => apps::executables(&paths, &app),
                        }
                        .map_err(|error| format!("{error:#}"));
                        let _ = tx.send(result);
                        ctx.request_repaint();
                    });
                }
                if let Some(receiver) = &self.launch_executables_receiver {
                    let result = match receiver.try_recv() {
                        Ok(result) => Some(result),
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err("Executable discovery stopped unexpectedly. Reopen Launch Options to retry.".into())),
                        Err(std::sync::mpsc::TryRecvError::Empty) => None,
                    };
                    if let Some(result) = result {
                        self.launch_executables = Some(result);
                        self.launch_executables_receiver = None;
                    }
                }
            }
            let modal = modal(
                ctx,
                format!(
                    "{}{} launch settings",
                    model::title(&self.app),
                    if self.launch_build_options {
                        " local build"
                    } else {
                        ""
                    }
                ),
                520.0,
                |ui| {
                    title_bar(
                        ui,
                        &format!(
                            "{}{} launch options",
                            model::title(&self.app),
                            if self.launch_build_options {
                                " local build"
                            } else {
                                ""
                            }
                        ),
                        |ui| {
                            if close_button(ui).clicked() {
                                self.launch_settings_open = false;
                            }
                        },
                    );
                    band(
                        ui,
                        egui::Margin::same(20),
                        400.0 - 2.0 - 2.0 * BAR_HEIGHT,
                        |ui| {
                            field_style(ui);
                            ui.spacing_mut().item_spacing.y = 6.0;
                            theme::text(ui, "Open", 13.0, theme::palette().text_3);
                            ui.spacing_mut().button_padding = egui::vec2(10.0, 9.0);
                            if self.launch_build_options {
                                ui.label(model::build_executable_name(&self.app));
                            } else {
                                egui::ComboBox::from_id_salt("launch-executable")
                                    .width(ui.available_width())
                                    .icon(|ui, rect, _, _, _| {
                                        let c = rect.center() + egui::vec2(2.0, 0.0);
                                        ui.painter().add(egui::Shape::line(
                                            vec![
                                                c + egui::vec2(-4.5, -2.25),
                                                c + egui::vec2(0.0, 2.25),
                                                c + egui::vec2(4.5, -2.25),
                                            ],
                                            Stroke::new(1.5_f32, theme::palette().text),
                                        ));
                                    })
                                    .selected_text(
                                        RichText::new(if self.launch_draft.executable.is_empty() {
                                            "Default executable"
                                        } else {
                                            &self.launch_draft.executable
                                        })
                                        .size(14.0)
                                        .color(theme::palette().text),
                                    )
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut self.launch_draft.executable,
                                            String::new(),
                                            "Default executable",
                                        );
                                        match &self.launch_executables {
                                            Some(Ok(executables)) => {
                                                for exe in executables {
                                                    ui.selectable_value(
                                                        &mut self.launch_draft.executable,
                                                        exe.clone(),
                                                        exe,
                                                    );
                                                }
                                            }
                                            Some(Err(error)) => {
                                                ui.label(error);
                                            }
                                            None => {
                                                ui.label("Loading executables…");
                                            }
                                        }
                                    });
                            }
                            ui.add_space(12.0);
                            theme::text(
                                ui,
                                "Arguments, one per line",
                                13.0,
                                theme::palette().text_3,
                            );
                            ui.add(
                                egui::TextEdit::multiline(&mut self.launch_arguments)
                                    .font(FontId::monospace(13.0))
                                    .hint_text("--example-flag")
                                    .margin(egui::Margin::symmetric(10, 8))
                                    .desired_width(ui.available_width())
                                    .desired_rows(5)
                                    .min_size(egui::vec2(0.0, 101.5)),
                            );
                            note(
                                ui,
                                "Passed to the app exactly as written. Don't add shell quotes.",
                            );
                        },
                    );
                    footer(ui, |ui| {
                        if action(ui, "Save", Kind::Primary, true).clicked() {
                            self.launch_draft.arguments = self
                                .launch_arguments
                                .lines()
                                .filter(|s| !s.is_empty())
                                .map(String::from)
                                .collect();
                            let result = if self.launch_build_options {
                                craft_apps_manager::builder::save_launch_options(
                                    &self.paths,
                                    &self.app,
                                    &self.launch_draft,
                                )
                            } else {
                                apps::save(&self.paths, &self.app, &self.launch_draft)
                            };
                            if result.is_ok() {
                                self.launch_settings_open = false;
                            }
                            self.result(result);
                        }
                        if action(ui, "Cancel", Kind::Secondary, true).clicked() {
                            self.launch_settings_open = false;
                        }
                    });
                },
            );
            if modal.should_close() {
                self.launch_settings_open = false;
            }
        }

        if !self.launch_settings_open {
            self.launch_executables_key = None;
            self.launch_executables = None;
            self.launch_executables_receiver = None;
        }
    }
}
