use super::*;

impl App {
    pub(super) fn render_self_update_dialog(&mut self, ctx: &egui::Context) {
        if self.confirm_self_update {
            let packaged = manager_update::installed_with_linux_package();
            let msi = manager_update::installed_with_msi() || packaged;
            let modal = modal(ctx, "Update Craft Apps Manager", 480.0, |ui| {
                band(ui, BODY, 280.0 - 2.0 - BAR_HEIGHT, |ui| {
                    header(
                        ui,
                        Art::Icon(
                            Icon::ArrowUp,
                            theme::palette().accent_soft,
                            theme::palette().accent_text,
                            48.0,
                        ),
                        "Update Craft Apps Manager?",
                        &format!(
                            "Version {} is available. {} Your library and settings are kept.",
                            self.manager_available
                                .as_ref()
                                .map(|a| a.display_version())
                                .unwrap_or_else(|| "?".into()),
                            if packaged {
                                "The system package manager installs it after administrator approval."
                            } else if msi {
                                "The manager downloads and installs it."
                            } else {
                                "The manager downloads it, checks it and restarts."
                            }
                        ),
                        |ui| {
                            theme::text(
                                ui,
                                self.manager_available
                                    .as_ref()
                                    .map(|a| a.confirmation_hint())
                                    .unwrap_or_else(|| {
                                        "Close other manager and builder windows first.".into()
                                    }),
                                13.0,
                                theme::palette().muted,
                            );
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                theme::text(ui, "From ", 13.0, theme::palette().muted);
                                theme::hyperlink(
                                    ui,
                                    manager_update::REPOSITORY_NAME,
                                    manager_update::REPOSITORY,
                                    13.0,
                                );
                            });
                        },
                    );
                });
                footer(ui, |ui| {
                    if action(
                        ui,
                        if msi {
                            "Download and install"
                        } else {
                            "Download and restart"
                        },
                        Kind::Primary,
                        true,
                    )
                    .clicked()
                    {
                        if let Some(available) = self.manager_available.clone() {
                            self.start_manager_update(ctx, available, None);
                        }
                        self.confirm_self_update = false;
                    }
                    if action(ui, "Later", Kind::Secondary, true).clicked() {
                        self.confirm_self_update = false;
                    }
                });
            });
            if modal.should_close() {
                self.confirm_self_update = false;
            }
        }
    }
}
