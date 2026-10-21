use super::*;

impl App {
    pub(super) fn render_move_dialog(&mut self, ctx: &egui::Context) {
        if let Some(installed) = self.confirm_move.clone() {
            if let Some(receiver) = &self.move_folder {
                if let Some(result) = crate::ui::poll_worker_result(receiver, || {
                    "Folder picker stopped unexpectedly.".into()
                }) {
                    self.move_folder = None;
                    match result {
                        Ok(Some(path)) => self.move_parent = path.display().to_string(),
                        Ok(None) => {}
                        Err(error) => self.error = Some(error),
                    }
                }
            }
            if let Some(receiver) = &self.move_probe {
                if let Some(result) = crate::ui::poll_worker_result(receiver, || {
                    "Application status check stopped unexpectedly.".into()
                }) {
                    self.move_probe = None;
                    self.move_running = Some(result.unwrap_or(true));
                    self.move_probe_at =
                        std::time::Instant::now() + std::time::Duration::from_secs(1);
                }
            }
            if self.move_probe.is_none() && std::time::Instant::now() >= self.move_probe_at {
                let (tx, rx) = std::sync::mpsc::channel();
                self.move_probe = Some(rx);
                let app = installed.name.clone();
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(
                        craft_apps_manager::platform::running_app(&app)
                            .map_err(|e| format!("{e:#}")),
                    );
                    ctx.request_repaint();
                });
            }
            let supported = craft_apps_manager::relocation::supported(&installed);
            let target = craft_apps_manager::relocation::destination(
                &self.paths,
                &installed,
                std::path::Path::new(&self.move_parent),
            );
            let response = modal(ctx, "Move app", 570.0, |ui| {
                band(ui, BODY, 0.0, |ui| {
                    header(
                        ui,
                        self.icons
                            .get(&installed.name)
                            .map(|texture| Art::Texture(texture.id()))
                            .unwrap_or(Art::None),
                        &format!("Move {}", model::title(&installed.name)),
                        "Choose a new location for this app.",
                        |_| {},
                    );
                    ui.add_space(24.0);
                    theme::text(ui, "Current location", 13.0, theme::palette().text);
                    ui.add_space(8.0);
                    folder_path_row(ui, std::path::Path::new(&installed.path), |path| {
                        self.result(craft_apps_manager::platform::open(path));
                    });
                    ui.add_space(20.0);
                    theme::text(ui, "New install location", 13.0, theme::palette().text);
                    ui.add_space(8.0);
                    ui.add_enabled_ui(supported, |ui| {
                        ui.horizontal(|ui| {
                            let width = (ui.available_width() - 106.0).max(40.0);
                            ui.add_sized(
                                egui::vec2(width, 36.0),
                                egui::TextEdit::singleline(&mut self.move_parent)
                                    .font(egui::TextStyle::Body)
                                    .vertical_align(egui::Align::Center)
                                    .margin(egui::Margin::symmetric(10, 8)),
                            );
                            if action(ui, "Browse…", Kind::Secondary, self.move_folder.is_none())
                                .clicked()
                            {
                                let initial = std::path::PathBuf::from(&self.move_parent);
                                let (tx, rx) = std::sync::mpsc::channel();
                                self.move_folder = Some(rx);
                                let ctx = ctx.clone();
                                std::thread::spawn(move || {
                                    let result =
                                        craft_apps_manager::portable::pick_folder(&initial)
                                            .map_err(|e| format!("{e:#}"));
                                    let _ = tx.send(result);
                                    ctx.request_repaint();
                                });
                            }
                        });
                    });
                    ui.add_space(8.0);
                    if let Ok(target) = &target {
                        note(
                            ui,
                            format!(
                                "App folder: {}",
                                compact_path(&target.display().to_string())
                            ),
                        );
                    } else if supported {
                        note(ui, target.as_ref().unwrap_err().to_string());
                    }
                    ui.add_space(24.0);
                    if !supported {
                        note(ui, "This system-managed package has a fixed install location. Portable apps can be moved.");
                    } else if self.move_running == Some(true) {
                        note(
                            ui,
                            format!("Close {} to enable Move.", model::title(&installed.name)),
                        );
                    } else if self.move_running.is_none() {
                        note(ui, "Checking whether the app is running…");
                    } else {
                        note(ui, "The manager will update its location and shortcuts. Project files outside the app folder stay where they are.");
                    }
                    if installed.install_kind == "installer" && supported {
                        ui.add_space(8.0);
                        note(ui, "Uses a verified installer of the same version. Windows may ask for administrator permission.");
                    }
                });
                footer(ui, |ui| {
                    if action(
                        ui,
                        "Move",
                        Kind::Primary,
                        supported
                            && target.is_ok()
                            && self.move_running == Some(false)
                            && self.move_folder.is_none()
                            && !self.job.snapshot().busy,
                    )
                    .clicked()
                    {
                        self.confirm_move = None;
                        self.job =
                            Job::new(self.paths.at("logs/updates.log"), &self.build_preferences);
                        self.job_target = Some((installed.name.clone(), "move-app".into()));
                        self.job_action = "move-app".into();
                        self.operation_was_busy = true;
                        self.failure_dismissed = false;
                        let paths = self.paths.clone();
                        let installed = installed.clone();
                        let parent = std::path::PathBuf::from(&self.move_parent);
                        self.job.describe("Move", &installed.name);
                        self.job.spawn(move |job| {
                            craft_apps_manager::relocation::move_app(
                                &paths, &installed, &parent, &job,
                            )
                        });
                    }
                    if action(ui, "Cancel", Kind::Secondary, true).clicked() {
                        self.confirm_move = None;
                    }
                });
            });
            if response.should_close() {
                self.confirm_move = None;
            }
        }
    }
}
