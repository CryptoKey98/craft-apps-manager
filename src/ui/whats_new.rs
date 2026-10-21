//! Local, versioned release highlights. Dismissal is independent of settings drafts.
use super::{dialogs, modal, theme, App};
use craft_apps_manager::files;
use eframe::egui::{self, Color32, CornerRadius, RichText, Sense};
use std::path::Path;

const SEEN_FILE: &str = "whats-new-seen.json";

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
enum Seen {
    Legacy(String),
    Build { version: String, revision: u32 },
}

pub(super) fn should_show(root: &Path, version: &str, revision: u32) -> bool {
    let Ok(seen) = files::read_json::<Seen>(&root.join(SEEN_FILE)) else {
        return true;
    };
    let (seen_version, seen_revision) = match seen {
        Seen::Legacy(version) => (version, 1),
        Seen::Build { version, revision } => (version, revision),
    };
    seen_version != version || seen_revision < revision
}

fn remember(root: &Path, version: &str, revision: u32) -> anyhow::Result<()> {
    // Baseline dismissals remain readable by managers released before revisions.
    let seen = if revision == 1 {
        Seen::Legacy(version.to_owned())
    } else {
        Seen::Build {
            version: version.to_owned(),
            revision,
        }
    };
    files::write_json(&root.join(SEEN_FILE), &seen)
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

fn github_mask(size: u32) -> egui::ColorImage {
    // GitHub's official 16px Octicon, rendered directly at common UI/DPI sizes.
    // These PNGs have transparent backgrounds; their alpha is the coverage mask.
    let bytes: &[u8] = match size {
        13 => include_bytes!("../../assets/github-mark-ui-13.png"),
        19 => include_bytes!("../../assets/github-mark-ui-19.png"),
        26 => include_bytes!("../../assets/github-mark-ui-26.png"),
        38 => include_bytes!("../../assets/github-mark-ui-38.png"),
        _ => include_bytes!("../../assets/github-mark-ui-64.png"),
    };
    let image = image::load_from_memory(bytes)
        .expect("bundled GitHub UI icon")
        .into_rgba8();
    let image = if image.width() == size {
        image
    } else {
        image::imageops::resize(&image, size, size, image::imageops::FilterType::Lanczos3)
    };
    egui::ColorImage {
        size: [size as usize, size as usize],
        pixels: image
            .pixels()
            .map(|pixel| Color32::from_white_alpha(pixel[3]))
            .collect(),
    }
}

pub(super) fn github(ui: &mut egui::Ui, rect: egui::Rect) {
    let pixels_per_point = ui.ctx().pixels_per_point();
    let size = (rect.width().min(rect.height()) * pixels_per_point)
        .round()
        .clamp(1.0, 256.0) as u32;
    let side = size as f32 / pixels_per_point;
    let origin = rect.center() - egui::Vec2::splat(side * 0.5);
    let origin = egui::pos2(
        (origin.x * pixels_per_point).round() / pixels_per_point,
        (origin.y * pixels_per_point).round() / pixels_per_point,
    );
    let rect = egui::Rect::from_min_size(origin, egui::Vec2::splat(side));
    let id = egui::Id::new(("github-octicon-small", size));
    let cached = ui
        .ctx()
        .data(|data| data.get_temp::<egui::TextureHandle>(id));
    let texture = cached.unwrap_or_else(|| {
        let texture = ui.ctx().load_texture(
            format!("github-mark-{size}"),
            github_mask(size),
            egui::TextureOptions::LINEAR,
        );
        ui.ctx()
            .data_mut(|data| data.insert_temp(id, texture.clone()));
        texture
    });
    ui.painter().image(
        texture.id(),
        rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        if ui.visuals().dark_mode {
            Color32::WHITE
        } else {
            Color32::BLACK
        },
    );
}

fn feature(ui: &mut egui::Ui, icon: theme::Icon, title: &str, text: &str) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let (slot, _) = ui.allocate_exact_size(egui::Vec2::splat(19.0), Sense::hover());
        let title_rect = ui
            .vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                let title = theme::heading(ui, title, 14.0);
                body_text(ui, text);
                title.rect
            })
            .inner;
        let rect = egui::Rect::from_center_size(
            egui::pos2(slot.center().x, title_rect.center().y),
            egui::Vec2::splat(19.0),
        );
        theme::paint_icon_weight(ui.painter(), rect, icon, theme::palette().muted, 1.65);
    });
    // Account for the parent layout's built-in gap so the visible gap is 20px.
    ui.add_space((20.0 - ui.spacing().item_spacing.y).max(0.0));
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
                        body_text(ui, "A smoother way to stay up to date.");
                        ui.add_space(22.0);
                        feature(ui, theme::Icon::ShieldCheck, "Smarter manager updates", "Get compatible updates even when download names or supported formats change.");
                        feature(ui, theme::Icon::Route, "Fewer update prompts", "Approve once and let the manager complete any required update steps.");
                        feature(ui, theme::Icon::PackageOpen, "More download options", "New Linux tar.gz and macOS DMG downloads, with ZIP and installer options still available.");
                        feature(ui, theme::Icon::Layers, "Multiple installations", "Keep multiple copies and choose which one to open, update or remove.");
                        feature(ui, theme::Icon::Search, "Discover new Craft apps", "Find new Craft apps automatically. Recent additions get a New badge until you view them.");
                        let notes = egui::CollapsingHeader::new(RichText::new(format!("Patch notes for {}", env!("CARGO_PKG_VERSION"))).size(13.0).color(p.link))
                            .id_salt("whats-new-notes").show(ui, |ui| {
                                theme::heading(ui, "New in 1.0.0", 13.0);
                                ui.add_space(10.0);
                                for note in [
                                    "Add an optional update service that keeps updates working when download names or supported package formats change. Every update is verified before installation.",
                                    "Add Linux tar.gz and macOS DMG downloads alongside the existing ZIP packages.",
                                    "Support Windows ZIP and MSI; Linux ZIP, tar.gz, DEB, RPM and Arch packages; and macOS ZIP and DMG in the new update service.",
                                    "Approve updates that need an intermediate version once, then let the manager finish the remaining steps automatically. Supported installer updates can also reuse one administrator approval.",
                                    "Set up macOS build tools without installing 7-Zip. It remains optional for compressed .7z backups.",
                                    "Improve the macOS DMG with a clearer drag-to-Applications layout.",
                                    "Fix Windows updates with intermediate steps failing when the installation folder contains spaces.",
                                    "Prevent manager updates from interrupting builds running in either Manager or Builder.",
                                    "Preserve macOS library locations when updating a manager installed in a shared Applications folder.",
                                    "Correct revision numbers in Arch Linux installer packages.",
                                    "Fix broken help links in portable Linux downloads.",
                                    "Reorganize the codebase and separate large test sections to make future fixes and improvements easier to maintain, while preserving existing behavior.",
                                ] {
                                    body_text(ui, &format!("• {note}"));
                                    ui.add_space(8.0);
                                }
                                ui.add_space(16.0);
                                theme::heading(ui, "Also included from 0.9.0", 13.0);
                                ui.add_space(10.0);
                                for note in [
                                    "Periodic update checks now include the manager alongside installed apps, using your chosen interval. Downloads and installations still require your approval.",
                                    "New badges expire 14 days after a Craft app's repository was created, including on fresh manager installs. Viewing the app clears its badge sooner; app updates do not reset the timer.",
                                    "Recognize custom GitHub apps using verified installer, package, and bundle identities, even when their names differ from the repository.",
                                    "Remember the selected executable for portable copies and packages with multiple applications. Keep portable choices working after moving their installation folder.",
                                    "Target the selected installation when verifying installs and uninstalling, and preserve existing copies when identity cannot be confirmed.",
                                    "Reject unsupported download architectures and recognize more Windows installation version formats.",
                                    #[cfg(target_os = "windows")]
                                    "Let users explicitly identify an existing Windows app when its installer makes no registration change.",
                                    #[cfg(target_os = "windows")]
                                    "Avoid duplicate application choices for per-user Windows installations.",
                                    #[cfg(target_os = "linux")]
                                    "Preserve supported Linux desktop launcher arguments and environment settings, and keep the previous app identity when installation fails or is cancelled.",
                                    #[cfg(target_os = "macos")]
                                    "Fix macOS manager updates failing because another application in the same folder contains symbolic links, while preserving update security checks.",
                                    #[cfg(target_os = "macos")]
                                    "Show saved macOS update failures once instead of repeating them on every launch, and retain their diagnostic details for troubleshooting.",
                                ] {
                                    body_text(ui, &format!("• {note}"));
                                    ui.add_space(8.0);
                                }
                                #[cfg(target_os = "linux")]
                                {
                                    ui.add_space(16.0);
                                    theme::heading(ui, "Also included from 0.8.3 Rev. 2 · Linux", 13.0);
                                    ui.add_space(10.0);
                                    for note in [
                                        "Close Manager to the tray on X11 or XWayland. On native Wayland, the unsupported setting is disabled and closing quits normally.",
                                        "Start normally when XWayland is unavailable, using native Wayland when supported by your desktop.",
                                        "See AppImage labels in installation selectors and package details, including for existing installations. Your selected copy and installation letters stay the same.",
                                        "Manager and Builder now identify themselves to Linux desktops so their windows can match the correct launcher, name and icon.",
                                        "What's New opens once after a revision update, while keeping your existing dismissal history.",
                                    ] {
                                        body_text(ui, &format!("• {note}"));
                                        ui.add_space(8.0);
                                    }
                                }
                                ui.add_space(16.0);
                                theme::heading(ui, "Also included from 0.8.3", 13.0);
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
            match remember(
                &self.paths.root,
                env!("CARGO_PKG_VERSION"),
                craft_apps_manager::update_revisions::BUILD_REVISION,
            ) {
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
    #[test]
    fn small_github_mark_uses_the_icon_area_instead_of_source_padding() {
        for size in [16, 19, 24, 38] {
            let mask = super::github_mask(size);
            let width = size as usize;
            let occupied_columns = (0..width)
                .filter(|&x| (0..width).any(|y| mask.pixels[y * width + x].a() > 128))
                .count();
            assert!(
                occupied_columns >= width - 2,
                "source padding shrank the {size}px logo"
            );
            assert!(mask
                .pixels
                .iter()
                .any(|pixel| (1..255).contains(&pixel.a())));
        }
    }
    use super::*;
    #[test]
    fn highlights_show_on_first_launch_and_version_change_only() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        assert!(should_show(&root, "0.8.0", 1));
        remember(&root, "0.8.0", 1).unwrap();
        assert!(!should_show(&root, "0.8.0", 1));
        assert!(should_show(&root, "0.9.0", 1));
        assert!(!root.join("manager-settings.json").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_dismissal_counts_as_revision_one_without_rewriting_it() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        files::write_json(&root.join(SEEN_FILE), &"0.8.3").unwrap();
        let original = std::fs::read(root.join(SEEN_FILE)).unwrap();
        assert!(!should_show(&root, "0.8.3", 1));
        assert!(should_show(&root, "0.8.3", 2));
        assert_eq!(std::fs::read(root.join(SEEN_FILE)).unwrap(), original);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn revision_dismissal_survives_restart_and_only_newer_revisions_show_again() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        assert!(should_show(&root, "0.8.3", 2));
        remember(&root, "0.8.3", 2).unwrap();
        assert!(!should_show(&root, "0.8.3", 2));
        assert!(should_show(&root, "0.8.3", 3));
        remember(&root, "0.8.3", 3).unwrap();
        assert!(!should_show(&root, "0.8.3", 3));
        assert!(!should_show(&root, "0.8.3", 2));
        assert!(should_show(&root, "0.8.4", 1));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn baseline_dismissal_stays_readable_by_older_managers() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        remember(&root, "0.8.3", 1).unwrap();
        assert_eq!(
            files::read_json::<String>(&root.join(SEEN_FILE)).unwrap(),
            "0.8.3"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
