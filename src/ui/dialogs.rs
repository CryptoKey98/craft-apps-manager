//! Every modal the manager shows. Each keeps the behaviour of the dialog it replaces.
//!
//! `modal` draws the bare dialog card; the helpers here draw its bands the way the
//! mockups do: a title bar or an icon header, a padded body, and a footer with a
//! hairline above right-aligned buttons.
use super::theme::{self, btn, Icon, Kind};
use super::{modal, result_for_display, App};
use craft_apps_manager::{apps, backups, jobs::Job, model, profiles, self_update, updates};
use eframe::egui::{self, Color32, CornerRadius, FontId, RichText, Sense, Stroke, StrokeKind};

/// Accent of the destructive checkboxes (`accent-color: #d9534f` in the mockups).
pub(super) const CHECK_RED: Color32 = Color32::from_rgb(0xd9, 0x53, 0x4f);
/// Footer, title bar and the dialog's own height, from the mockups.
pub(super) const BAR_HEIGHT: f32 = 61.0;

/// A padded band of a dialog that is at least `height` tall, borders included.
pub(super) fn band<R>(
    ui: &mut egui::Ui,
    margin: egui::Margin,
    height: f32,
    content: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    egui::Frame::new()
        .inner_margin(margin)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height((height - margin.sum().y).max(0.0));
            content(ui)
        })
        .inner
}

/// A settings-style card with a section heading and consistent padding.
fn options_section(ui: &mut egui::Ui, title: &str, content: impl FnOnce(&mut egui::Ui)) {
    theme::text(ui, title, 13.0, theme::palette().text_3);
    ui.add_space(4.0);
    theme::group().show(ui, |ui| {
        band(ui, egui::Margin::same(16), 0.0, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            content(ui);
        });
    });
}
/// A full-width 1pt hairline.
pub(super) fn rule(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::ZERO, theme::palette().border);
}

/// Title bar: 16pt title on the left, `right` (a close button or a control) on the
/// right, padding 14/16/14/20 and a hairline below.
pub(super) fn title_bar(ui: &mut egui::Ui, title: &str, right: impl FnOnce(&mut egui::Ui)) {
    band(
        ui,
        egui::Margin {
            left: 20,
            right: 16,
            top: 14,
            bottom: 14,
        },
        0.0,
        |ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), 32.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    right(ui);
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        theme::heading(ui, title, 16.0);
                    });
                },
            );
        },
    );
    rule(ui);
}

/// The 32pt close button of a title bar with an 18pt cross.
pub(super) fn close_button(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(32.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Close"));
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(8), theme::palette().button_hover);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(8),
            Stroke::new(2.0_f32, theme::palette().link),
            StrokeKind::Inside,
        );
    }
    theme::paint_icon(
        ui.painter(),
        egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(18.0)),
        Icon::Close,
        theme::palette().text_3,
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.on_hover_text("Close")
}

/// Footer: hairline, then right-aligned 36pt buttons with padding 12/16 and 8pt gaps.
/// Add the primary action first: the layout runs right to left.
pub(super) fn footer(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    rule(ui);
    band(ui, egui::Margin::symmetric(16, 12), 0.0, |ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 36.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                content(ui);
            },
        );
    });
}

/// A 36pt footer button.
pub(super) fn action(ui: &mut egui::Ui, text: &str, kind: Kind, enabled: bool) -> egui::Response {
    btn(text).kind(kind).medium().enabled(enabled).show(ui)
}

/// Body text of a dialog: 14pt with a 1.5 line height.
fn paragraph(ui: &mut egui::Ui, text: &str) {
    ui.add(
        egui::Label::new(
            RichText::new(text)
                .size(14.0)
                .color(theme::palette().text_2)
                .line_height(Some(21.0)),
        )
        .wrap(),
    );
}

/// Icon or app artwork, title and supporting text at the top of a dialog body.
/// `more` adds lines below the text in the same column, 8pt apart.
fn header(ui: &mut egui::Ui, art: Art, title: &str, text: &str, more: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        match art {
            Art::Texture(texture) => {
                ui.add(
                    egui::Image::new((texture, egui::vec2(48.0, 48.0)))
                        .corner_radius(CornerRadius::same(11)),
                );
            }
            Art::Icon(icon, fill, color, size) => {
                let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(size), Sense::hover());
                let (radius, glyph) = if size >= 48.0 { (11, 22.0) } else { (10, 20.0) };
                ui.painter()
                    .rect_filled(rect, CornerRadius::same(radius), fill);
                theme::paint_icon(
                    ui.painter(),
                    egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(glyph)),
                    icon,
                    color,
                );
            }
            Art::None => {}
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            theme::heading(ui, title, 17.0);
            if !text.is_empty() {
                paragraph(ui, text);
            }
            more(ui);
        });
    });
}

enum Art {
    Texture(egui::TextureId),
    /// Icon, tile fill, icon colour and tile size (40 or 48).
    Icon(Icon, Color32, Color32, f32),
    None,
}

/// Padding of a dialog body without a title bar.
const BODY: egui::Margin = egui::Margin::same(24);

fn note(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.add(egui::Label::new(RichText::new(text).size(12.0).color(theme::palette().muted)).wrap());
}

fn danger() -> (Icon, Color32, Color32) {
    (Icon::Trash, theme::palette().red_bg, theme::palette().red)
}

impl App {
    pub(super) fn dialogs(&mut self, ctx: &egui::Context) {
        let busy = self.job.state.lock().unwrap().busy;
        if self.build_options_open {
            let app = self.app.clone();
            let last = self.display_builds.get(&app).cloned();
            let build_busy = self.build_job.state.lock().unwrap().busy || self.cleaning();
            let response = modal(ctx, "Local build options", 560.0, |ui| {
                title_bar(ui, &format!("{} build options", model::title(&app)), |ui| {
                    if close_button(ui).clicked() {
                        self.build_options_open = false;
                    }
                });
                egui::ScrollArea::vertical()
                    .id_salt("build-options-scroll")
                    .max_height((ctx.screen_rect().height() - 200.0).clamp(180.0, 560.0))
                    .show(ui, |ui| {
                        band(ui, egui::Margin::same(24), 0.0, |ui| {
                            ui.spacing_mut().item_spacing.y = 8.0;
                            ui.add(egui::Label::new(RichText::new("Configure your compiled copy. Installed releases use their own launch settings.").size(13.0).color(theme::palette().muted)).wrap());
                            ui.add_space(12.0);
                            options_section(ui, "Launch options", |ui| {
                                theme::text(ui, "Executable", 12.0, theme::palette().muted);
                                theme::text(ui, model::build_executable_name(&app), 14.0, theme::palette().text);
                                ui.add_space(8.0);
                                theme::text(ui, "Arguments", 13.0, theme::palette().text_2);
                                ui.scope(|ui| {
                                    field_style(ui);
                                    ui.add(egui::TextEdit::multiline(&mut self.launch_arguments)
                                        .font(FontId::monospace(13.0))
                                        .margin(egui::Margin::same(10))
                                        .hint_text("--example-flag")
                                        .desired_width(ui.available_width()).desired_rows(4));
                                });
                                ui.add(egui::Label::new(RichText::new("One argument per line. Passed directly to the app; no shell quotes needed.").size(12.0).color(theme::palette().muted)).wrap());
                            });
                            ui.add_space(16.0);
                            options_section(ui, "Build files", |ui| {
                                ui.add(egui::Label::new(RichText::new("Browse the compiled files or review the output from your last build.").size(13.0).color(theme::palette().text_2)).wrap());
                                ui.add_space(6.0);
                                let log = self.paths.at(format!("logs/{app}.log"));
                                ui.horizontal_wrapped(|ui| {
                                    ui.spacing_mut().item_spacing.x = 10.0;
                                    if action(ui, "Open build folder", Kind::Secondary, last.is_some()).clicked() {
                                        if let Some((folder, _)) = &last { self.result(craft_apps_manager::platform::open(folder)); }
                                    }
                                    if action(ui, "View build log", Kind::Secondary, log.is_file()).clicked() {
                                        self.result(craft_apps_manager::platform::open(&log));
                                    }
                                });
                            });
                            ui.add_space(16.0);
                            options_section(ui, "Remove local build", |ui| {
                                ui.add(egui::Label::new(RichText::new("Delete the compiled copy to free up space. Source files, installed releases, and other builds are kept.").size(13.0).color(theme::palette().text_2)).wrap());
                                ui.add_space(6.0);
                    if self.delete_build_confirm {
                        ui.add(egui::Label::new("Delete this local build?").wrap());
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;
                        if action(
                            ui,
                            "Yes",
                            Kind::Danger,
                            !build_busy && last.is_some(),
                        )
                        .clicked()
                        {
                            if let Some((folder, _)) = &last {
                                let result = craft_apps_manager::builder::delete_local(
                                    &self.paths,
                                    &app,
                                    folder,
                                );
                                if result.is_ok() {
                                    self.display_builds.remove(&app);
                                    if self.build_app == app {
                                        self.build_job.state.lock().unwrap().output = None;
                                    }
                                    self.config_refresh_at = std::time::Instant::now();
                                    self.build_options_open = false;
                                }
                                self.result(result);
                            }
                        }
                        if action(ui, "No", Kind::Secondary, true).clicked() {
                            self.delete_build_confirm = false;
                        }
                        });
                    } else if action(
                        ui,
                        "Delete local build…",
                        Kind::DangerOutline,
                        !build_busy && last.is_some(),
                    )
                    .clicked()
                    {
                        self.delete_build_confirm = true;
                    }

                            });
                        });
                    });
                footer(ui, |ui| {
                    if action(ui, "Cancel", Kind::Secondary, true).clicked() {
                        self.build_options_open = false;
                    }
                    if action(ui, "Save", Kind::Primary, true).clicked() {
                        self.launch_draft.executable.clear();
                        self.launch_draft.arguments = self
                            .launch_arguments
                            .lines()
                            .filter(|line| !line.is_empty())
                            .map(String::from)
                            .collect();
                        let result = craft_apps_manager::builder::save_launch_options(
                            &self.paths,
                            &app,
                            &self.launch_draft,
                        );
                        if result.is_ok() {
                            self.build_options_open = false;
                        }
                        self.result(result);
                    }
                });
            });
            if response.should_close() {
                self.build_options_open = false;
            }
        }
        if let Some(plan) = self
            .release_plan
            .clone()
            .filter(|_| !self.selection_from_review)
        {
            let response = modal(ctx, "Review release operations", 600.0, |ui| {
                band(ui, BODY, 0.0, |ui| {
                    header(
                        ui,
                        Art::Icon(
                            Icon::Refresh,
                            theme::palette().accent_soft,
                            theme::palette().accent_text,
                            40.0,
                        ),
                        "Review install and update",
                        &format!(
                            "{} · {}. Only the Install and Update entries below will run.",
                            plan.preferences.release_format,
                            model::architecture_label(&plan.preferences.architecture)
                        ),
                        |_| {},
                    );
                    ui.add_space(18.0);
                    theme::group().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                            for (index, entry) in plan.entries.iter().enumerate() {
                                if index > 0 {
                                    rule(ui);
                                }
                                egui::Frame::new()
                                    .inner_margin(egui::Margin::symmetric(14, 10))
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.spacing_mut().item_spacing.y = 4.0;
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = 12.0;
                                            if let Some(texture) = self.icons.get(&entry.app) {
                                                ui.add(
                                                    egui::Image::new((
                                                        texture.id(),
                                                        egui::vec2(24.0, 24.0),
                                                    ))
                                                    .corner_radius(CornerRadius::same(6)),
                                                );
                                            }
                                            theme::text(ui, model::title(&entry.app), 14.0, theme::palette().text);
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    let (fill, color) = match entry.action.as_str() {
                                                        "Install" | "Update" => {
                                                            (theme::palette().accent_soft, theme::palette().accent_text)
                                                        }
                                                        "Error" => (theme::palette().red_bg, theme::palette().red),
                                                        _ => (theme::palette().button, theme::palette().text_3),
                                                    };
                                                    theme::badge(
                                                        ui,
                                                        format!("{} {}", entry.action, entry.version).trim(),
                                                        fill,
                                                        color,
                                                    );
                                                },
                                            );
                                        });
                                        if let Some(error) = &entry.error {
                                            ui.label(RichText::new(error).size(12.0).color(theme::palette().red));
                                        } else {
                                            note(
                                                ui,
                                                entry
                                                    .destination
                                                    .as_ref()
                                                    .map(|p| format!("Destination: {}", p.display()))
                                                    .unwrap_or_else(|| {
                                                        "Destination: Applications / system installation (installer controlled)".into()
                                                    }),
                                            );
                                            if let Some(asset) = &entry.asset {
                                                theme::hyperlink(
                                                    ui,
                                                    &asset.name,
                                                    &asset.browser_download_url,
                                                    12.0,
                                                );
                                            }
                                        }
                                    });
                            }
                        });
                    });
                });
                footer(ui, |ui| {
                    let count = plan.executable_count();
                    if action(
                        ui,
                        &format!("Run {count} operation{}", if count == 1 { "" } else { "s" }),
                        Kind::Primary,
                        count > 0,
                    )
                    .clicked()
                    {
                        self.release_plan = None;
                        let paths = self.paths.clone();
                        self.job = Job::new(paths.at("logs/updates.log"), &self.build_preferences);
                        self.job_target = None;
                        self.job_action.clear();
                        self.failure_dismissed = false;
                        self.operation_was_busy = true;
                        let plan = plan.clone();
                        self.job
                            .spawn(move |job| updates::execute_plan(&paths, &plan, &job));
                    }
                    if action(ui, "Cancel", Kind::Secondary, true).clicked() {
                        self.release_plan = None;
                    }
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), 36.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            if action(ui, "Choose…", Kind::Secondary, true).clicked() {
                                self.selection_draft = plan.preferences.selected_apps.clone();
                                self.source_selection = false;
                                self.selection_from_review = true;
                                self.selection = true;
                            }
                        },
                    );
                });
            });
            if response.should_close() {
                self.release_plan = None;
            }
        }

        if let Some(app) = self.confirm_install.clone() {
            let installer = self.preferences.release_format == "installer";
            let update = self.status(&app).update;
            let modal = modal(ctx, "Confirm installation", 480.0, |ui| {
                let title = match &update {
                    Some(version) => format!("Update {} to {version}?", model::title(&app)),
                    None => format!("Install {}?", model::title(&app)),
                };
                let text = if installer {
                    if cfg!(target_os = "linux") {
                        format!(
                            "Downloads the latest {} {} package. You will be asked to authorize the installation.",
                            model::title(&app),
                            craft_apps_manager::installers::installer_label()
                        )
                    } else if cfg!(target_os = "macos") {
                        format!(
                            "Downloads the latest release and copies {} to your Applications folder.",
                            model::executable_name(&app)
                        )
                    } else {
                        format!(
                            "Downloads the latest {} {} and opens its installer wizard. Windows may ask for administrator permission.",
                            model::title(&app),
                            craft_apps_manager::installers::installer_label()
                        )
                    }
                } else {
                    format!(
                        "Downloads the latest release of {} into your library as a portable app.",
                        model::title(&app)
                    )
                };
                let art = match self.icons.get(&app) {
                    Some(texture) => Art::Texture(texture.id()),
                    None => Art::None,
                };
                band(ui, BODY, 240.0 - 2.0 - BAR_HEIGHT, |ui| {
                    header(ui, art, &title, &text, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(
                            RichText::new(format!(
                                "Latest stable release · {} · {}",
                                model::architecture_label(&self.preferences.architecture),
                                self.preferences.release_format
                            ))
                            .size(13.0)
                            .color(theme::palette().muted),
                        );
                        theme::hyperlink(
                            ui,
                            format!(
                                "github.com/storytold/{}/releases/latest",
                                model::repository(&app)
                            ),
                            format!(
                                "https://github.com/storytold/{}/releases/latest",
                                model::repository(&app)
                            ),
                            13.0,
                        );
                    });
                });
                footer(ui, |ui| {
                    if action(
                        ui,
                        if update.is_some() {
                            "Update"
                        } else {
                            "Install"
                        },
                        Kind::Primary,
                        true,
                    )
                    .clicked()
                    {
                        self.app = app.clone();
                        self.confirm_install = None;
                        self.start("install-app");
                    }
                    if action(ui, "Cancel", Kind::Secondary, true).clicked() {
                        self.confirm_install = None;
                    }
                });
            });
            if modal.should_close() {
                self.confirm_install = None;
            }
        }
        if self.launch_settings_open {
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
                                        for exe in apps::executables(&self.paths, &self.app)
                                            .unwrap_or_default()
                                        {
                                            ui.selectable_value(
                                                &mut self.launch_draft.executable,
                                                exe.clone(),
                                                &exe,
                                            );
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
        if self.confirm_uninstall {
            let modal = modal(
                ctx,
                format!("Uninstall {}?", model::title(&self.app)),
                520.0,
                |ui| {
                    let installed = self
                        .display_config
                        .as_ref()
                        .and_then(|c| c.apps.iter().find(|a| a.name == self.app))
                        .map(|a| a.path.clone())
                        .filter(|path| !path.is_empty());
                    let art = match self.icons.get(&self.app) {
                        Some(texture) => Art::Texture(texture.id()),
                        None => Art::None,
                    };
                    band(ui, BODY, 0.0, |ui| {
                        header(
                            ui,
                            art,
                            &format!("Uninstall {}?", model::title(&self.app)),
                            &format!(
                                "Removes {}. Source ZIPs, builds, backups and launch settings are kept.",
                                installed
                                    .as_deref()
                                    .map(|path| removal_target(path, &self.app))
                                    .unwrap_or_else(|| "the installed copy".into())
                            ),
                            |_| {},
                        );
                        ui.add_space(18.0);
                        let targets = profiles::targets(&self.paths, &self.app);
                        theme::group().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            let label = format!(
                                "Also delete {} settings and caches",
                                model::title(&self.app)
                            );
                            band(ui, egui::Margin::symmetric(14, 12), 0.0, |ui| {
                                ui.horizontal_top(|ui| {
                                    ui.spacing_mut().item_spacing.x = 12.0;
                                    ui.vertical(|ui| {
                                        ui.add_space(1.0);
                                        theme::checkbox(
                                            ui,
                                            &mut self.delete_profile,
                                            18.0,
                                            CHECK_RED,
                                            &label,
                                            targets.is_ok(),
                                        );
                                    });
                                    ui.vertical(|ui| {
                                        ui.spacing_mut().item_spacing.y = 2.0;
                                        let enabled = targets.is_ok();
                                        let title = ui.add_enabled(
                                            enabled,
                                            egui::Label::new(
                                                RichText::new(&label).size(14.0).color(theme::palette().text),
                                            )
                                            .sense(Sense::click()),
                                        );
                                        if enabled && title.clicked() {
                                            self.delete_profile = !self.delete_profile;
                                        }
                                        note(ui, "Includes plug-ins and recovery copies. Other copies of the app may share them.");
                                    });
                                });
                            });
                            rule(ui);
                            let list_margin = egui::Margin {
                                left: 44,
                                right: 14,
                                top: 10,
                                bottom: 12,
                            };
                            match &targets {
                                Ok(targets) => {
                                    let existing: Vec<_> =
                                        targets.iter().filter(|t| t.path.exists()).collect();
                                    band(ui, list_margin, 0.0, |ui| {
                                        ui.spacing_mut().item_spacing.y = 4.0;
                                        if existing.is_empty() {
                                            note(ui, "No existing profile folders found.");
                                        }
                                        egui::ScrollArea::vertical().max_height(100.0).show(
                                            ui,
                                            |ui| {
                                                ui.spacing_mut().item_spacing.y = 4.0;
                                                for target in existing {
                                                    let response = ui.label(
                                                        RichText::new(
                                                            target.path.display().to_string(),
                                                        )
                                                        .font(FontId::monospace(12.0))
                                                        .color(theme::palette().text_2),
                                                    );
                                                    ui.painter().circle_filled(
                                                        egui::pos2(
                                                            response.rect.left() - 12.0,
                                                            response.rect.center().y,
                                                        ),
                                                        2.5,
                                                        theme::palette().text_2,
                                                    );
                                                }
                                            },
                                        );
                                    });
                                    band(
                                        ui,
                                        egui::Margin {
                                            top: 0,
                                            ..list_margin
                                        },
                                        0.0,
                                        |ui| {
                                            note(ui, "Custom profile locations outside these folders are kept.");
                                        },
                                    );
                                }
                                Err(error) => {
                                    band(ui, list_margin, 0.0, |ui| {
                                        note(ui, format!("Not available: {error}"));
                                    });
                                }
                            }
                        });
                    });
                    footer(ui, |ui| {
                        if action(ui, "Uninstall", Kind::Danger, true).clicked() {
                            self.start("uninstall-app");
                            self.confirm_uninstall = false;
                        }
                        if action(ui, "Cancel", Kind::Secondary, true).clicked() {
                            self.confirm_uninstall = false;
                        }
                    });
                },
            );
            if modal.should_close() {
                self.confirm_uninstall = false;
            }
        }
        if self.selection {
            let choices = if self.source_selection {
                model::sources()
            } else {
                model::apps()
            };
            let modal = modal(
                ctx,
                if self.source_selection {
                    "Choose source apps"
                } else {
                    "Choose release apps"
                },
                480.0,
                |ui| {
                    let top = ui.cursor().top();
                    band(ui, egui::Margin::symmetric(20, 16), 0.0, |ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        ui.spacing_mut().interact_size.y = 0.0;
                        theme::heading(
                            ui,
                            if self.source_selection {
                                "Sources in Update all sources"
                            } else {
                                "Apps in Update all"
                            },
                            16.0,
                        );
                        if self.source_selection {
                            theme::text(
                                ui,
                                "Choose which source ZIPs to update. ArtCraft X is source only.",
                                13.0,
                                theme::palette().text_3,
                            );
                        }
                        ui.horizontal(|ui| {
                            theme::text(
                                ui,
                                format!(
                                    "{} of {} selected",
                                    self.selection_draft.len(),
                                    choices.len()
                                ),
                                13.0,
                                theme::palette().muted,
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.spacing_mut().item_spacing.x = 14.0;
                                    if theme::link(ui, "Deselect all").clicked() {
                                        self.selection_draft.clear();
                                    }
                                    if theme::link(ui, "Select all").clicked() {
                                        self.selection_draft = choices.clone();
                                    }
                                },
                            );
                        });
                    });
                    rule(ui);
                    let header = ui.cursor().top() - top;
                    let total = (ctx.screen_rect().height() - 48.0).min(600.0);
                    let height = (total - 2.0 - header - BAR_HEIGHT).max(120.0);
                    egui::ScrollArea::vertical()
                        .id_salt("selection-scroll")
                        .auto_shrink([false, false])
                        .max_height(height)
                        .min_scrolled_height(height)
                        .show(ui, |ui| {
                            band(ui, egui::Margin::symmetric(12, 8), height, |ui| {
                                for name in &choices {
                                    let name = name.as_str();
                                    let checked = self.selection_draft.iter().any(|s| s == name);
                                    let (rect, response) = ui.allocate_exact_size(
                                        egui::vec2(ui.available_width(), 48.0),
                                        Sense::click(),
                                    );
                                    response.widget_info(|| {
                                        egui::WidgetInfo::selected(
                                            egui::WidgetType::Checkbox,
                                            true,
                                            checked,
                                            model::title(name),
                                        )
                                    });
                                    if response.hovered() {
                                        ui.painter().rect_filled(
                                            rect,
                                            CornerRadius::same(8),
                                            theme::palette().hover,
                                        );
                                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                    }
                                    if response.has_focus() {
                                        ui.painter().rect_stroke(
                                            rect,
                                            CornerRadius::same(8),
                                            Stroke::new(2.0_f32, theme::palette().link),
                                            StrokeKind::Inside,
                                        );
                                    }
                                    let mark = egui::Rect::from_min_size(
                                        egui::pos2(rect.left() + 12.0, rect.center().y - 8.0),
                                        egui::Vec2::splat(16.0),
                                    );
                                    theme::paint_check(
                                        ui,
                                        mark,
                                        checked,
                                        theme::palette().accent,
                                        true,
                                    );
                                    if let Some(texture) = self.icons.get(name) {
                                        ui.painter().image(
                                            texture.id(),
                                            egui::Rect::from_min_size(
                                                egui::pos2(
                                                    mark.right() + 15.0,
                                                    rect.center().y - 12.0,
                                                ),
                                                egui::Vec2::splat(24.0),
                                            ),
                                            egui::Rect::from_min_max(
                                                egui::pos2(0.0, 0.0),
                                                egui::pos2(1.0, 1.0),
                                            ),
                                            Color32::WHITE,
                                        );
                                    }
                                    theme::painter_text(
                                        ui.painter(),
                                        egui::pos2(mark.right() + 51.0, rect.center().y),
                                        egui::Align2::LEFT_CENTER,
                                        model::title(name),
                                        14.0,
                                        theme::palette().text,
                                    );
                                    if response.clicked() {
                                        if checked {
                                            self.selection_draft.retain(|s| s != name)
                                        } else {
                                            self.selection_draft.push(name.into())
                                        }
                                    }
                                }
                            });
                        });
                    footer(ui, |ui| {
                        if action(ui, "Save", Kind::Primary, true).clicked() {
                            if self.selection_from_review {
                                match craft_apps_manager::settings::select_release_apps(
                                    &self.paths,
                                    &self.selection_draft,
                                ) {
                                    Ok(preferences) => {
                                        self.preferences = preferences.clone();
                                        self.settings_draft = preferences;
                                        self.selection = false;
                                        self.selection_from_review = false;
                                        self.release_plan = None;
                                        self.begin_release_review();
                                    }
                                    Err(error) => self.error = Some(format!("{error:#}")),
                                }
                            } else if self.source_selection {
                                self.settings_draft.selected_sources = self.selection_draft.clone();
                                self.selection = false;
                            } else {
                                self.settings_draft.selected_apps = self.selection_draft.clone();
                                self.selection = false;
                            }
                        }
                        if action(ui, "Cancel", Kind::Secondary, true).clicked() {
                            self.selection = false;
                            self.selection_from_review = false;
                        }
                    });
                },
            );
            if modal.should_close() {
                self.selection = false;
                self.selection_from_review = false;
            }
        }
        if let Some(app) = self.restore_app.clone() {
            let modal = modal(
                ctx,
                format!("{} backups", model::title(&app)),
                560.0,
                |ui| {
                    let top = ui.cursor().top();
                    title_bar(ui, &format!("{} backups", model::title(&app)), |ui| {
                        let mut mode = self.backup_delete_mode;
                        if theme::segmented(
                            ui,
                            28.0,
                            12.0,
                            &mut mode,
                            &[(false, "Restore"), (true, "Delete")],
                        ) {
                            self.backup_delete_mode = mode;
                            self.confirm_restore = false;
                            self.restore_selected = None;
                            self.backup_delete_selected.clear();
                        }
                    });
                    band(
                        ui,
                        egui::Margin::symmetric(20, 16),
                        520.0 - 2.0 - BAR_HEIGHT - (ui.cursor().top() - top),
                        |ui| {
                            ui.spacing_mut().item_spacing.y = 12.0;
                            ui.add(
                                egui::Label::new(
                                    RichText::new(if self.backup_delete_mode {
                                        "Choose the backups to delete. Installed apps and source files are kept."
                                    } else {
                                        "Choose one backup to restore. It replaces the current managed copy and the backup is kept."
                                    })
                                    .size(13.0)
                                    .color(theme::palette().text_3),
                                )
                                .wrap(),
                            );
                            if self.restore_backups.is_empty() {
                                theme::text(
                                    ui,
                                    "No backups are available for this app.",
                                    14.0,
                                    theme::palette().text_2,
                                );
                            }
                            self.backup_rows(ui, &app);
                            if self.backup_delete_mode {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 16.0;
                                    if theme::link(ui, "Select all").clicked() {
                                        self.backup_delete_selected =
                                            (0..self.restore_backups.len()).collect();
                                        self.confirm_restore = false;
                                    }
                                    if theme::link(ui, "Deselect all").clicked() {
                                        self.backup_delete_selected.clear();
                                        self.confirm_restore = false;
                                    }
                                    theme::text(
                                        ui,
                                        format!("{} selected", self.backup_delete_selected.len()),
                                        13.0,
                                        theme::palette().muted,
                                    );
                                });
                            }
                            if self.confirm_restore {
                                ui.label(
                                    RichText::new(if self.backup_delete_mode {
                                        "Permanently delete the selected backups?"
                                    } else {
                                        "This replaces the current managed copy. Continue?"
                                    })
                                    .color(theme::palette().amber),
                                );
                            }
                        },
                    );
                    footer(ui, |ui| {
                        let ready = (if self.backup_delete_mode {
                            !self.backup_delete_selected.is_empty()
                        } else {
                            self.restore_selected.is_some()
                        }) && !busy;
                        if action(
                            ui,
                            if self.backup_delete_mode && self.confirm_restore {
                                "Confirm deletion"
                            } else if self.backup_delete_mode {
                                "Delete selected…"
                            } else if self.confirm_restore {
                                "Confirm restore"
                            } else {
                                "Restore…"
                            },
                            if self.backup_delete_mode {
                                Kind::Danger
                            } else {
                                Kind::Primary
                            },
                            ready,
                        )
                        .clicked()
                        {
                            if self.confirm_restore {
                                let selected: Vec<_> = if self.backup_delete_mode {
                                    self.backup_delete_selected
                                        .iter()
                                        .map(|i| self.restore_backups[*i].clone())
                                        .collect()
                                } else {
                                    vec![self.restore_backups[self.restore_selected.unwrap()]
                                        .clone()]
                                };
                                let deleting = self.backup_delete_mode;
                                let paths = self.paths.clone();
                                let app = app.clone();
                                self.release_checks.remove(&app);
                                self.job =
                                    Job::new(paths.at("logs/updates.log"), &self.build_preferences);
                                self.job_action.clear();
                                self.failure_dismissed = false;
                                self.job_target = Some((
                                    app.clone(),
                                    if deleting {
                                        "delete-backups"
                                    } else {
                                        "restore"
                                    }
                                    .into(),
                                ));
                                self.operation_was_busy = true;
                                self.job.spawn(move |job| {
                                    if deleting {
                                        backups::delete_selected(&paths, &app, &selected)?;
                                        job.log(&format!(
                                            "Deleted {} backup(s) for {}",
                                            selected.len(),
                                            model::title(&app)
                                        ));
                                        Ok(())
                                    } else {
                                        backups::restore(&paths, &app, &selected[0], &job)
                                    }
                                });
                                self.restore_app = None;
                            } else {
                                self.confirm_restore = true;
                            }
                        }
                        if action(ui, "Close", Kind::Secondary, true).clicked() {
                            self.restore_app = None;
                        }
                    });
                },
            );
            if modal.should_close() {
                self.restore_app = None;
            }
        }
        if self.confirm_self_update {
            let packaged = self_update::installed_with_linux_package();
            let msi = self_update::installed_with_msi() || packaged;
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
                                .map(|a| a.version.as_str())
                                .unwrap_or("?"),
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
                                "Close other manager and builder windows first.",
                                13.0,
                                theme::palette().muted,
                            );
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                theme::text(ui, "From ", 13.0, theme::palette().muted);
                                theme::hyperlink(
                                    ui,
                                    self_update::REPOSITORY_NAME,
                                    self_update::REPOSITORY,
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
                            let paths = self.paths.clone();
                            let ctx = ctx.clone();
                            let (tx, rx) = std::sync::mpsc::channel();
                            self.manager_plan = Some(rx);
                            self.job = Job::new(
                                self.paths.at("logs/updates.log"),
                                &self.build_preferences,
                            );
                            self.job_target = None;
                            self.job_action.clear();
                            self.failure_dismissed = false;
                            self.job.spawn(move |job| {
                                let result = self_update::prepare(&paths, &available, &job);
                                let _ = tx.send(result_for_display(&result));
                                ctx.request_repaint();
                                result.map(|_| ())
                            });
                            self.manager_message = "Downloading and verifying manager…".into();
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

    /// The list of backups: radio buttons to restore one, checkboxes to delete several.
    fn backup_rows(&mut self, ui: &mut egui::Ui, app: &str) {
        if self.restore_backups.is_empty() {
            return;
        }
        theme::group().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::ScrollArea::vertical()
                .max_height(300.0)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let count = self.restore_backups.len();
                    for (i, backup) in self.restore_backups.iter().enumerate() {
                        if i > 0 {
                            rule(ui);
                        }
                        let (title, detail, kind) = backup_labels(app, backup);
                        let selected = if self.backup_delete_mode {
                            self.backup_delete_selected.contains(&i)
                        } else {
                            self.restore_selected == Some(i)
                        };
                        let (rect, response) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), 68.0),
                            Sense::click(),
                        );
                        response.widget_info(|| {
                            egui::WidgetInfo::selected(
                                if self.backup_delete_mode {
                                    egui::WidgetType::Checkbox
                                } else {
                                    egui::WidgetType::RadioButton
                                },
                                true,
                                selected,
                                format!("{title} · {detail}"),
                            )
                        });
                        if response.hovered() {
                            let round = |on: bool| if on { 9 } else { 0 };
                            ui.painter().rect_filled(
                                rect,
                                CornerRadius {
                                    nw: round(i == 0),
                                    ne: round(i == 0),
                                    sw: round(i + 1 == count),
                                    se: round(i + 1 == count),
                                },
                                theme::palette().hover,
                            );
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if response.has_focus() {
                            ui.painter().rect_stroke(
                                rect.shrink(1.0),
                                CornerRadius::same(6),
                                Stroke::new(2.0_f32, theme::palette().link),
                                StrokeKind::Inside,
                            );
                        }
                        let mark = egui::Rect::from_min_size(
                            egui::pos2(
                                rect.left() + if self.backup_delete_mode { 18.0 } else { 19.0 },
                                rect.center().y - 8.0,
                            ),
                            egui::Vec2::splat(16.0),
                        );
                        if self.backup_delete_mode {
                            theme::paint_check(ui, mark, selected, CHECK_RED, true);
                        } else {
                            theme::paint_radio(ui, mark.center(), selected);
                        }
                        theme::painter_text(
                            ui.painter(),
                            egui::pos2(rect.left() + 50.0, rect.center().y - 1.0),
                            egui::Align2::LEFT_BOTTOM,
                            &title,
                            14.0,
                            theme::palette().text,
                        );
                        theme::painter_text(
                            ui.painter(),
                            egui::pos2(rect.left() + 50.0, rect.center().y + 1.0),
                            egui::Align2::LEFT_TOP,
                            &detail,
                            12.0,
                            theme::palette().muted,
                        );
                        let galley = ui.painter().layout_no_wrap(
                            kind.to_owned(),
                            theme::bold(11.0),
                            theme::palette().text_3,
                        );
                        let chip = egui::Rect::from_min_size(
                            egui::pos2(
                                rect.right() - 14.0 - galley.size().x - 16.0,
                                rect.center().y - 11.0,
                            ),
                            egui::vec2(galley.size().x + 16.0, 22.0),
                        );
                        ui.painter().rect_filled(
                            chip,
                            CornerRadius::same(11),
                            theme::palette().button,
                        );
                        ui.painter().galley(
                            chip.center() - galley.size() / 2.0,
                            galley,
                            theme::palette().text_3,
                        );
                        if response.clicked() {
                            if self.backup_delete_mode {
                                if !self.backup_delete_selected.insert(i) {
                                    self.backup_delete_selected.remove(&i);
                                }
                            } else {
                                self.restore_selected = Some(i);
                            }
                            self.confirm_restore = false;
                        }
                    }
                });
        });
    }
}

/// Field look of the mockups' inputs and selects: dark fill, 1pt #33363d outline,
/// 8pt corners.
pub(super) fn field_style(ui: &mut egui::Ui) {
    let visuals = &mut ui.style_mut().visuals;
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.bg_fill = theme::palette().field;
        widget.weak_bg_fill = theme::palette().field;
        widget.corner_radius = CornerRadius::same(8);
        widget.expansion = 0.0;
    }
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, theme::palette().border_strong);
    visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, theme::palette().border_strong);
}

/// What an uninstall removes: the app inside its install folder, or the
/// recorded path itself when no app is found there.
fn removal_target(path: &str, app: &str) -> String {
    let folder = std::path::Path::new(path);
    match model::installed_executable(folder, app) {
        Some(found) if folder.is_dir() => found
            .file_name()
            .map(|name| format!("{} from {path}", name.to_string_lossy()))
            .unwrap_or_else(|| path.to_owned()),
        _ => path.to_owned(),
    }
}

/// Title, date and kind for one backup row.
fn backup_labels(app: &str, backup: &backups::Backup) -> (String, String, &'static str) {
    let filename = backup.path.file_name().unwrap().to_string_lossy();
    let version = filename
        .strip_prefix(&if backup.source {
            format!("{app}-source-")
        } else {
            format!("{app}-")
        })
        .unwrap_or(&filename)
        .split('-')
        .take(1)
        .collect::<Vec<_>>()
        .join(" ");
    let date = std::fs::metadata(&backup.path)
        .and_then(|m| m.modified())
        .ok()
        .map(|t| {
            chrono::DateTime::<chrono::Local>::from(t)
                .format("%b %d, %Y · %I:%M %p")
                .to_string()
        })
        .unwrap_or_default();
    let format = if backup.path.is_dir() {
        "Folder"
    } else if backup.path.extension().is_some_and(|e| e == "7z") {
        "7-Zip archive"
    } else {
        "ZIP archive"
    };
    (
        if backup.source {
            format!("Source {version}")
        } else {
            format!("Version {version}")
        },
        format!("{date} · {format}"),
        if backup.source {
            "Source"
        } else {
            "Portable app"
        },
    )
}
