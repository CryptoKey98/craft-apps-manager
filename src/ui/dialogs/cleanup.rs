use super::*;

impl App {
    pub(super) fn render_cleanup_dialog(&mut self, ctx: &egui::Context) {
        if self.confirm_clear || self.confirm_clean {
            let modal = modal(ctx, "Confirm cleanup", 440.0, |ui| {
                let (icon, fill, color) = danger();
                band(ui, BODY, 220.0 - 2.0 - BAR_HEIGHT, |ui| {
                    header(
                        ui,
                        Art::Icon(icon, fill, color, 40.0),
                        if self.confirm_clear {
                            "Delete all backups?"
                        } else {
                            "Delete temporary build files?"
                        },
                        if self.confirm_clear {
                            "Deletes managed release and source backups. Installed apps, finished builds, source ZIPs and tools are kept."
                        } else {
                            "Deletes compilation caches and extracted source workspaces. Installed apps, finished builds, source ZIPs and tools are kept."
                        },
                        |_| {},
                    );
                });
                footer(ui, |ui| {
                    if action(ui, "Delete", Kind::Danger, true).clicked() {
                        self.start(if self.confirm_clear { "clear" } else { "clean" });
                        self.confirm_clear = false;
                        self.confirm_clean = false;
                    }
                    if action(ui, "Cancel", Kind::Secondary, true).clicked() {
                        self.confirm_clear = false;
                        self.confirm_clean = false;
                    }
                });
            });
            if modal.should_close() {
                self.confirm_clear = false;
                self.confirm_clean = false;
            }
        }
    }
}
