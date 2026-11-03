use super::*;

impl App {
    pub(super) fn render_identity_choice(&mut self, ctx: &egui::Context) -> DialogFlow {
        if let Some(choice) = self.job.identity_choice() {
            draw_choice(ctx, &choice);
            return DialogFlow::Stop;
        }
        DialogFlow::Continue
    }
}

fn draw_choice(ctx: &egui::Context, choice: &craft_apps_manager::jobs::IdentityChoice) {
    let title = "Choose application";
    modal(ctx, title, 520.0, |ui| {
        let mut close = false;
        title_bar(ui, title, |ui| {
            close = close_button(ui).clicked();
        });
        band(ui, egui::Margin::same(20), 0.0, |ui| {
            paragraph(ui, &choice.title);
            ui.add_space(12.0);
            egui::ScrollArea::vertical()
                .max_height(260.0)
                .show(ui, |ui| {
                    for (index, candidate) in choice.candidates.iter().enumerate() {
                        if ui.selectable_label(false, candidate).clicked() {
                            choice.answer(Some(index));
                        }
                    }
                });
        });
        footer(ui, |ui| {
            if action(ui, "Cancel", Kind::Secondary, true).clicked() {
                choice.answer(None);
            }
        });
        if close {
            choice.answer(None);
        }
    });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        choice.answer(None);
    }
}
