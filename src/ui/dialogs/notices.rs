use super::*;

impl App {
    pub(super) fn render_notices(&mut self, ctx: &egui::Context) {
        if self.hide_notice_open {
            let notice = modal(ctx, "App hidden", 400.0, |ui| {
                band(ui, BODY, 0.0, |ui| {
                    header(ui, Art::Icon(Icon::EyeOff, theme::palette().accent_soft, theme::palette().accent_text, 36.0),
                        "App hidden", "To show hidden apps again, go to Settings → Apps → App visibility → Choose… and check the apps you want to see.", |_| {});
                });
                footer(ui, |ui| {
                    if action(ui, "OK", Kind::Primary, true).clicked() {
                        self.hide_notice_open = false;
                    }
                });
            });
            if notice.should_close() {
                self.hide_notice_open = false;
            }
        }

        if let Some(message) = self.selection_notice.clone() {
            let modal = modal(ctx, "No apps selected", 440.0, |ui| {
                band(ui, BODY, 220.0 - 2.0 - BAR_HEIGHT, |ui| {
                    header(
                        ui,
                        Art::Icon(
                            Icon::Alert,
                            theme::palette().accent_soft,
                            theme::palette().accent_text,
                            40.0,
                        ),
                        "No apps selected",
                        &message,
                        |_| {},
                    );
                });
                footer(ui, |ui| {
                    if action(ui, "OK", Kind::Primary, true).clicked() {
                        self.selection_notice = None;
                    }
                });
            });
            if modal.should_close() {
                self.selection_notice = None;
            }
        }

        if let Some(error) = self.error.clone() {
            let modal = modal(ctx, "Craft Apps Manager", 480.0, |ui| {
                band(ui, BODY, 220.0 - 2.0 - BAR_HEIGHT, |ui| {
                    header(
                        ui,
                        Art::Icon(
                            Icon::Alert,
                            theme::palette().red_bg,
                            theme::palette().red,
                            40.0,
                        ),
                        "Something went wrong",
                        "",
                        |ui| {
                            egui::ScrollArea::vertical()
                                .max_height(260.0)
                                .show(ui, |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(error)
                                                .size(14.0)
                                                .color(theme::palette().red_text)
                                                .line_height(Some(21.0)),
                                        )
                                        .selectable(true),
                                    );
                                });
                        },
                    );
                });
                footer(ui, |ui| {
                    if action(ui, "OK", Kind::Primary, true).clicked() {
                        self.error = None;
                    }
                });
            });
            if modal.should_close() {
                self.error = None;
            }
        }
    }
}
