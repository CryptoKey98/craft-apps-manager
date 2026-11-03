use super::*;

impl App {
    pub(in crate::ui) fn open_compatibility_info(
        &mut self,
        app: &str,
        tag: &str,
        reason: &str,
        ctx: &egui::Context,
    ) {
        self.compatibility_info = Some(CompatibilityInfo {
            app: model::title(app),
            version: tag.into(),
            reason: reason.into(),
            opened: ctx.input(|input| input.time),
        });
        ctx.memory_mut(|memory| memory.close_popup());
    }
    pub(super) fn render_compatibility_info(&mut self, ctx: &egui::Context) {
        let Some(info) = self.compatibility_info.clone() else {
            return;
        };
        let progress = if ctx.style().animation_time > 0.0 {
            ((ctx.input(|input| input.time) - info.opened) / 0.18).clamp(0.0, 1.0) as f32
        } else {
            1.0
        };
        let opacity = progress * progress * (3.0 - 2.0 * progress);
        if progress < 1.0 {
            ctx.request_repaint();
        }
        let response = modal_faded(ctx, "Incompatible version", 400.0, opacity, |ui| {
            band(ui, BODY, 0.0, |ui| {
                header(
                    ui,
                    Art::Icon(
                        Icon::Info,
                        theme::palette().accent_soft,
                        theme::palette().accent_text,
                        40.0,
                    ),
                    "Incompatible version",
                    "",
                    |ui| {
                        note(ui, format!("{} · {}", info.app, info.version));
                        ui.add_space(8.0);
                        paragraph(ui, &info.reason);
                    },
                );
            });
            footer(ui, |ui| {
                if action(ui, "OK", Kind::Primary, true).clicked() {
                    self.compatibility_info = None;
                }
            });
        });
        if response.should_close() {
            self.compatibility_info = None;
        }
    }
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
