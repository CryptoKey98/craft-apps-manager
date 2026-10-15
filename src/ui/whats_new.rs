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

/// A narrow accent moves gently back and forth below the stationary label.
fn patch_notes_underline(ui: &egui::Ui, header: egui::Rect, color: Color32, elapsed: f64) {
    let line = egui::Rect::from_min_max(
        egui::pos2(header.left() + 18.0, header.bottom() - 2.0),
        egui::pos2(header.right() - 3.0, header.bottom()),
    );
    if line.width() <= 0.0 || !ui.is_rect_visible(line) {
        return;
    }
    // Six seconds for a full round trip, slowing naturally at either end.
    let phase = (elapsed / 6.0).fract() as f32;
    let center = 0.5 - 0.5 * (phase * std::f32::consts::TAU).cos();
    let mut mesh = egui::Mesh::default();
    for index in 0..=32 {
        let position = index as f32 / 32.0;
        // Keep the accent short while its center travels the full label width.
        let strength = (1.0 - ((position - center) / 0.19).abs()).max(0.0);
        let ink = color.gamma_multiply(0.85 * strength);
        let x = line.left() + line.width() * position;
        mesh.colored_vertex(egui::pos2(x, line.top()), ink);
        mesh.colored_vertex(egui::pos2(x, line.bottom()), ink);
        if index < 32 {
            let vertex = index * 2;
            mesh.add_triangle(vertex, vertex + 1, vertex + 2);
            mesh.add_triangle(vertex + 1, vertex + 3, vertex + 2);
        }
    }
    ui.painter().add(egui::Shape::mesh(mesh));
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(33));
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
            self.whats_new_animation_started = None;
            return;
        }
        let now = ctx.input(|input| input.time);
        let elapsed = now - *self.whats_new_animation_started.get_or_insert(now);
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
                        feature(ui, Some(theme::Icon::Sliders), "Install your way", "Choose the package type and architecture for each app.");
                        feature(ui, Some(theme::Icon::Layers), "Multiple installations", "Keep multiple copies and choose which one to open, update or remove.");
                        feature(ui, Some(theme::Icon::Activity), "Follow every operation", "Track installs, updates and builds, with progress and cancellation.");
                        feature(ui, None, "Add your own GitHub apps", "Add apps from GitHub releases. Custom app support is experimental.");
                        feature(ui, Some(theme::Icon::Search), "Discover new Craft apps", "New Craft apps appear automatically, marked New so they're easy to find.");
                        let notes = egui::CollapsingHeader::new(RichText::new(format!("Patch notes for {}", env!("CARGO_PKG_VERSION"))).size(13.0).color(p.link))
                            .id_salt("whats-new-notes").show(ui, |ui| {
                                theme::heading(ui, "New in 0.8.3", 13.0);
                                ui.add_space(10.0);
                                body_text(ui, "• Manager updates now support platform-specific build revisions. Minor fixes can reach only the systems that need them.");
                                ui.add_space(8.0);
                                body_text(ui, "• Update packages use signed revision metadata and verified checksums. Older managers retain a compatible update path.");
                                ui.add_space(16.0);
                                theme::heading(ui, "Also included from 0.8.2", 13.0);
                                ui.add_space(10.0);
                                for note in [
                                    "See the selected installation on each Home tile. Use its compact switcher to choose another copy; the app page uses the same selection.",
                                    "Newly discovered Craft apps appear at the top of available apps with a New badge. The badge fades when you open the app, and its top position is remembered.",
                                    "New apps get a subtitle from their repository description. Discovery checks package names and versions more carefully, and keeps known apps if their repositories disappear.",
                                    "Hide the Activity button's notification badge in Settings → General. Unread activities still stay highlighted inside the menu.",
                                    "Portable installations use matching letters in shortcuts and installation selectors. Existing copies keep their letters; new copies reuse available letters. Installer shortcuts show just the architecture.",
                                    "Uninstalling an additional portable copy also removes its empty manager-created parent folder.",
                                    "Additional portable library copies use simple numbered folders. Older empty wrapper folders are cleaned up on uninstall, and extra shortcuts are no longer created in the releases folder.",
                                    "Installer cleanup also removes empty app download folders, while preserving files needed for repair or recovery.",
                                    "Choosing a version keeps your installation location. Windows installer defaults use the correct Program Files folder and cannot target manager storage folders.",
                                    "Clean temporary files left by interrupted builds, even before your first successful build. Completed builds and source ZIPs are kept; the next build recreates its cache.",
                                    "Saving settings keeps additional installations, including copies with the same architecture. Older portable copies are detected more reliably.",
                                    "Installation buttons transition more smoothly when switching copies or finishing an operation. Progress bars align with the installation details below.",
                                    "App descriptions appear above the installation selector. Package labels now say Portable, and New badges have softer light-theme styling.",
                                    "Background checks and dialogs report when their worker stops unexpectedly, instead of staying stuck loading.",
                                    "PDFCraft source builds use the correct package name after the upstream rename.",
                                    "Portable backup restoration uses the selected installation rather than old global settings, and refreshes architecture information from the restored executable where supported.",
                                    "Backup handling supports app names containing numbers. Failed downloads clean up temporary files more reliably.",
                                ] {
                                    body_text(ui, &format!("• {note}"));
                                    ui.add_space(8.0);
                                }
                                ui.add_space(8.0);
                                theme::heading(ui, "Also included from 0.8.1", 13.0);
                                ui.add_space(10.0);
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
                        if notes.body_returned.is_none() {
                            patch_notes_underline(ui, notes.header_response.rect, p.link, elapsed);
                        }
                        #[cfg(target_os = "linux")]
                        {
                            ui.add_space(12.0);
                            egui::CollapsingHeader::new(RichText::new("Patch notes for 0.8.3 Rev. 2 · Linux").size(13.0).color(p.link))
                                .id_salt("whats-new-linux-rev2-notes")
                                .default_open(true)
                                .show(ui, |ui| {
                                    ui.add_space(6.0);
                                    for note in [
                                        "Close Manager to the tray on X11 or XWayland. On native Wayland, the unsupported setting is disabled and closing quits normally.",
                                        "Start normally when XWayland is unavailable, using native Wayland when supported by your desktop.",
                                        "See AppImage labels in installation selectors and package details, including for existing installations. Your selected copy and installation letters stay the same.",
                                        "Manager and Builder now identify themselves to Linux desktops so their windows can match the correct launcher, name and icon.",
                                    ] {
                                        body_text(ui, &format!("• {note}"));
                                        ui.add_space(8.0);
                                    }
                                });
                        }
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
            self.whats_new_animation_started = None;
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
