//! Local, versioned release highlights. Dismissal is independent of settings drafts.
use super::{dialogs, modal, theme, App};
use craft_apps_manager::files;
use eframe::egui::{self, Color32, CornerRadius, RichText, Sense};
use std::path::Path;

const SEEN_FILE: &str = "whats-new-seen.json";

pub(super) fn should_show(root: &Path, version: &str) -> bool {
    files::read_json::<String>(&root.join(SEEN_FILE))
        .ok()
        .as_deref()
        != Some(version)
}

fn remember(root: &Path, version: &str) -> anyhow::Result<()> {
    files::write_json(&root.join(SEEN_FILE), &version)
}

fn body_text(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(13.0).color(theme::palette().muted)).wrap());
}

pub(super) fn github(ui: &mut egui::Ui, rect: egui::Rect) {
    let id = egui::Id::new("whats-new-github-mark");
    let cached = ui
        .ctx()
        .data(|data| data.get_temp::<egui::TextureHandle>(id));
    let texture = cached.unwrap_or_else(|| {
        let image = image::load_from_memory(include_bytes!("../../assets/github-mark.png"))
            .expect("bundled GitHub logo")
            .into_rgba8();
        let size = [image.width() as usize, image.height() as usize];
        // Publisher's mark has a white background: turn its ink into a tintable mask.
        let pixels: Vec<Color32> = image
            .pixels()
            .map(|pixel| {
                Color32::from_white_alpha(((255 - pixel[0]) as u16 * pixel[3] as u16 / 255) as u8)
            })
            .collect();
        let texture = ui.ctx().load_texture(
            "github-mark",
            egui::ColorImage { size, pixels },
            Default::default(),
        );
        ui.ctx()
            .data_mut(|data| data.insert_temp(id, texture.clone()));
        texture
    });
    ui.painter().image(
        texture.id(),
        rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        theme::palette().muted,
    );
}

fn feature(ui: &mut egui::Ui, icon: Option<theme::Icon>, title: &str, text: &str) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 22.0), Sense::hover());
        let rect = egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(19.0));
        if let Some(icon) = icon {
            theme::paint_icon(ui.painter(), rect, icon, theme::palette().muted);
        } else {
            github(ui, rect);
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            theme::heading(ui, title, 14.0);
            body_text(ui, text);
        });
    });
    ui.add_space(20.0);
}

impl App {
    pub(super) fn whats_new_dialog(&mut self, ctx: &egui::Context) {
        if !self.whats_new_open || self.builder {
            return;
        }
        let mut dismiss = false;
        let p = theme::palette();
        let response = modal(ctx, "What's new", 548.0, |ui| {
            dialogs::band(ui, egui::Margin::same(24), 0.0, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 16.0;
                    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(40.0), Sense::hover());
                    ui.painter()
                        .rect_filled(rect, CornerRadius::same(9), p.accent_soft);
                    theme::paint_icon(
                        ui.painter(),
                        rect.shrink(9.0),
                        theme::Icon::Info,
                        p.accent_text,
                    );
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 3.0;
                        theme::heading(ui, "What's new", 19.0);
                        body_text(
                            ui,
                            &format!("Craft Apps Manager · Version {}", env!("CARGO_PKG_VERSION")),
                        );
                    });
                });
                ui.add_space(18.0);
                egui::ScrollArea::vertical().id_salt("whats-new-content")
                    .max_height((ctx.screen_rect().height() - 260.0).clamp(160.0, 490.0))
                    .show(ui, |ui| {
                        body_text(ui, "More control over your apps. A clearer view of every operation.");
                        ui.add_space(22.0);
                        feature(ui, Some(theme::Icon::Sliders), "Install your way", "Choose the package format and architecture for each app, using the downloads available for that release.");
                        feature(ui, Some(theme::Icon::Layers), "Multiple installations", "Keep installer and portable copies side by side. Use the Installation dropdown to choose which copy to open, update, move or remove.");
                        feature(ui, Some(theme::Icon::Activity), "Follow every operation", "See downloads, installs and builds in Activity, with progress, cancellation and detailed results.");
                        feature(ui, None, "Add your own GitHub apps", "Review detected downloads before adding an app. Custom app support is experimental.");
                        #[cfg(target_os = "windows")]
                        {
                        egui::Frame::new().fill(p.card).corner_radius(8).inner_margin(egui::Margin::symmetric(14, 12)).show(ui, |ui| {
                            ui.horizontal_top(|ui| {
                                ui.spacing_mut().item_spacing.x = 10.0;
                                let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(17.0), Sense::hover());
                                theme::paint_icon(ui.painter(), rect, theme::Icon::Archive, p.muted);
                                body_text(ui, "Supported Windows MSI downgrades prepare a verified recovery installer before replacing the current version.");
                            });
                        });
                        ui.add_space(18.0);
                        }
                        egui::CollapsingHeader::new(RichText::new(format!("Patch notes for {}", env!("CARGO_PKG_VERSION"))).size(13.0).color(p.link))
                            .id_salt("whats-new-notes").show(ui, |ui| {
                                for note in [
                                    "Choose which completed build to launch in Build Options. The manager and builder use the same selection.",
                                    "Builder progress, logs and cancellation stay synchronized between windows, including when reopening the manager with the same data folder.",
                                    "Activity notifications and theme changes stay synchronized between the manager and builder.",
                                    "Each build gets its own clearly named shortcut. Removing a build also removes its unchanged manager-created shortcut.",
                                    "Configure an optional GitHub token in Settings → Updates. The compact GitHub status icon shows connection details on hover.",
                                    "Portable manager downloads keep managed paths connected when you move the whole folder. See Portable storage in Settings → Folders.",
                                    "Updates from older portable versions preserve your existing library and tools locations automatically.",
                                    "Downloaded setup files are cleaned up after successful installation, with cleanup included in installation progress.",
                                    "Improve Build Options layout and build-cache controls, and remove the redundant Show in Explorer link from Tools.",
                                ] {
                                    body_text(ui, &format!("• {note}"));
                                    ui.add_space(8.0);
                                }
                            });
                    });
            });
            ui.separator();
            dialogs::band(ui, egui::Margin::symmetric(24, 16), 0.0, |ui| {
                ui.horizontal(|ui| {
                    let note_width = (ui.available_width() - 80.0).max(0.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(note_width, 36.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_min_width(note_width);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(
                                        "Reopen using the info icon next to the version.",
                                    )
                                    .size(12.0)
                                    .color(p.muted),
                                )
                                .wrap(),
                            );
                        },
                    );
                    dismiss = theme::btn("Got it").primary().medium().show(ui).clicked();
                });
            });
        });
        if dismiss || response.should_close() {
            match remember(&self.paths.root, env!("CARGO_PKG_VERSION")) {
                Ok(()) => self.whats_new_open = false,
                Err(error) => {
                    self.whats_new_open = false;
                    self.error = Some(format!("Could not save What's New dismissal: {error:#}"));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn highlights_show_on_first_launch_and_version_change_only() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        assert!(should_show(&root, "0.8.0"));
        remember(&root, "0.8.0").unwrap();
        assert!(!should_show(&root, "0.8.0"));
        assert!(should_show(&root, "0.9.0"));
        assert!(!root.join("manager-settings.json").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
