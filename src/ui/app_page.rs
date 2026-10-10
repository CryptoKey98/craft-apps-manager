//! One app's page: what it is and its main actions in the middle, its tools on the right.
use super::overview::release_format_label;
use super::theme::{self, btn, Icon, Size};
use super::App;
use craft_apps_manager::{apps, builder, jobs::State, model, platform};
use eframe::egui::{
    self, pos2, vec2, Align, Color32, CornerRadius, FontId, Layout, Rect, Response, RichText,
    Sense, Stroke, Ui, UiBuilder,
};
use std::{path::PathBuf, process::Command};

/// Where the app's files live, opened by "Show in Finder" and its equivalents.
fn folder_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "Show in Finder"
    } else if cfg!(target_os = "windows") {
        "Show in Explorer"
    } else {
        "Open app folder"
    }
}

fn location_label(installed: &model::Installed) -> &'static str {
    if installed.install_kind == "installer" {
        if cfg!(target_os = "linux") {
            "Installed Linux package"
        } else if cfg!(target_os = "macos") {
            "Installed in Applications"
        } else {
            "Installed with Windows installer"
        }
    } else {
        "Installed portable release"
    }
}

fn short_date(rfc3339: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%b %-d, %Y %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

/// The height of one line of IBM Plex at `size`: the browser's `line-height: normal`.
fn line(size: f32) -> f32 {
    size * 1.3
}

/// Space between the blocks of the main column, and between the tools column's sections.
const MAIN_GAP: f32 = 24.0;
/// The height of a row of the tools group.
const TOOL_ROW: f32 = 63.75;
/// The left edge of a tool row's text, and of what sits below it, inside the group.
const TOOL_TEXT_X: f32 = 38.0;
impl App {
    pub(super) fn app_page(&mut self, ctx: &egui::Context, state: &State) {
        egui::SidePanel::right("app-tools")
            .resizable(false)
            .exact_width(320.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::palette().panel)
                    .inner_margin(egui::Margin {
                        // The panel's 1-point border sits inside the mockup's 20-point padding.
                        left: 21,
                        right: 20,
                        top: 20,
                        bottom: 20,
                    }),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("app-tools-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.tools_column(ui, state));
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::palette().bg))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("app-page-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Frame::new()
                            .inner_margin(egui::Margin::symmetric(40, 32))
                            .show(ui, |ui| {
                                ui.set_max_width(ui.available_width().min(720.0));
                                self.app_main(ui, state);
                            });
                    });
            });
    }

    fn app_main(&mut self, ui: &mut Ui, state: &State) {
        let app = self.app.clone();
        let title = model::title(&app);
        let status = self.status(&app);
        let format = self.preferences.release_format.clone();
        let mine = self
            .job_target
            .as_ref()
            .filter(|(target, _)| *target == app)
            .map(|(_, action)| action.clone());
        ui.spacing_mut().item_spacing.y = 0.0;
        // Header: artwork, name, then category · state · repository.
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 20.0;
            let icon = self.icons.get(&app).map(|texture| texture.id());
            let (rect, _) = ui.allocate_exact_size(
                vec2(if icon.is_some() { 72.0 } else { 0.0 }, 72.0),
                Sense::hover(),
            );
            if let Some(texture) = icon {
                egui::Image::new((texture, vec2(72.0, 72.0)))
                    .corner_radius(CornerRadius::same(16))
                    .paint_at(ui, rect);
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.spacing_mut().interact_size.y = 0.0;
                // The name (line height 1.2) and the meta row, 4 points apart and
                // centred on the artwork.
                let name_line = 26.0 * 1.2;
                let block = name_line + 4.0 + line(13.0);
                let half_leading = (line(26.0) - name_line) / 2.0;
                // egui sets 26-point Plex a point lower in its line than the browser.
                ui.add_space((72.0 - block) / 2.0 - half_leading - 1.0);
                theme::heading(ui, &title, 26.0);
                ui.add_space(4.0 - half_leading + 0.5);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 4.0);
                    let category = model::category(&app);
                    if !category.is_empty() {
                        theme::text(ui, category, 13.0, theme::palette().text_3);
                        theme::text(ui, "·", 13.0, theme::palette().text_3);
                    }
                    match &status.installed {
                        Some(installed) => {
                            theme::text(
                                ui,
                                format!("Installed {}", installed.version),
                                13.0,
                                theme::palette().green,
                            );
                        }
                        None if status.alternate.is_some() => {
                            theme::text(
                                ui,
                                format!("No {format} copy"),
                                13.0,
                                theme::palette().text_3,
                            );
                        }
                        None if !status.loaded => {
                            theme::text(ui, "Checking…", 13.0, theme::palette().text_3);
                        }
                        None => {
                            theme::text(ui, "Not installed", 13.0, theme::palette().text_3);
                        }
                    }
                    theme::text(ui, "·", 13.0, theme::palette().text_3);
                    let repository = model::repository(&app);
                    theme::hyperlink(
                        ui,
                        format!("github.com/{repository}"),
                        format!("https://github.com/{repository}"),
                        13.0,
                    )
                    .on_hover_text("View repository");
                });
            });
        });
        ui.add_space(MAIN_GAP);

        // A failed install, update, uninstall or source download for this app.
        if !state.busy && state.stage == "Failed" && !self.failure_dismissed {
            if let Some(action) = mine.clone() {
                let verb = match action.as_str() {
                    "uninstall-app" => "uninstall",
                    "move-app" => "move",
                    "source-app" => "download the source for",
                    "restore" => "restore a backup of",
                    "delete-backups" => "delete backups of",
                    _ if status.installed.is_some() => "update",
                    _ => "install",
                };
                egui::Frame::new()
                    .fill(theme::palette().red_bg)
                    .stroke(Stroke::new(1.0_f32, theme::palette().red_border))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(egui::Margin::symmetric(14, 12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().interact_size.y = 0.0;
                        ui.spacing_mut().item_spacing = vec2(12.0, 0.0);
                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                            if theme::icon_button(
                                ui,
                                Icon::Close,
                                "Dismiss",
                                theme::palette().red_text,
                            )
                            .clicked()
                            {
                                self.failure_dismissed = true;
                            }
                            let retry = match action.as_str() {
                                "install-app" => Some("Try again"),
                                "source-app" => Some("Try again"),
                                _ => None,
                            };
                            if let Some(retry) = retry {
                                if btn(retry).show(ui).clicked() {
                                    if action == "install-app" {
                                        self.confirm_install = Some(app.clone());
                                    } else {
                                        self.start(&action);
                                    }
                                }
                            }
                            ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                                ui.vertical(|ui| {
                                    ui.add_space(1.0);
                                    let (rect, _) =
                                        ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                                    theme::paint_icon(
                                        ui.painter(),
                                        rect,
                                        Icon::Alert,
                                        theme::palette().red,
                                    );
                                });
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 2.0;
                                    ui.label(
                                        RichText::new(format!("Couldn't {verb} {title}"))
                                            .font(theme::bold(14.0))
                                            .color(theme::palette().red_text),
                                    );
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&state.outcome)
                                                .size(13.0)
                                                .color(theme::palette().red_text),
                                        )
                                        .wrap(),
                                    );
                                });
                            });
                        });
                    });
                ui.add_space(MAIN_GAP);
            }
        }

        let description = model::description(&app);
        if !description.is_empty() {
            // CSS centres the 1.55 line height around the text; egui puts the extra
            // space below it.
            let line_height = 15.0 * 1.55;
            let shift = (line_height - line(15.0)) / 2.0;
            ui.add_space(shift);
            ui.scope(|ui| {
                // Capped at the design's measure, but never wider than the column.
                ui.set_max_width(ui.available_width().min(620.0));
                ui.add(
                    egui::Label::new(
                        RichText::new(description)
                            .size(15.0)
                            .color(theme::palette().text_2)
                            .line_height(Some(line_height)),
                    )
                    .wrap(),
                );
            });
            ui.add_space(MAIN_GAP - shift);
        }

        // Progress for an operation running on this app, in place of the buttons.
        let completed_opacity = if state.stage == "Complete" {
            self.completed_progress_opacity(ui.ctx())
        } else {
            0.0
        };
        let show_progress = mine.is_some() && (state.busy || completed_opacity > 0.0);
        if show_progress {
            let verb = match mine.as_deref() {
                Some("uninstall-app") => "Uninstalling",
                Some("move-app") => "Moving",
                Some("source-app") => "Downloading source for",
                Some("restore") => "Restoring",
                Some("delete-backups") => "Deleting backups of",
                _ if status.installed.is_some() => "Updating",
                _ => "Installing",
            };
            // 560 points of content inside the padding and border, as in the design.
            let width = ui.available_width().min(560.0 + 30.0);
            ui.scope(|ui| {
                if !state.busy {
                    ui.multiply_opacity(completed_opacity);
                }
                egui::Frame::new()
                    .fill(theme::palette().progress_bg)
                    .stroke(Stroke::new(1.0_f32, theme::palette().accent_border))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(egui::Margin::symmetric(14, 12))
                    .show(ui, |ui| {
                        let inner = width - 30.0;
                        ui.set_width(inner);
                        // Title row, 8 points, then the bar; Cancel centred beside them.
                        let height = line(14.0) + 8.0 + 6.0;
                        let (rect, _) = ui.allocate_exact_size(vec2(inner, height), Sense::hover());
                        let mut cancel = child(ui, rect, Layout::right_to_left(Align::Center));
                        if state.busy {
                            self.cancel_button(&mut cancel, &state.stage);
                        } else {
                            theme::text(&mut cancel, "Done", 13.0, theme::palette().text_3);
                        }
                        let right = cancel.min_rect().left() - 16.0;
                        let column = Rect::from_min_max(rect.min, pos2(right, rect.max.y));
                        let mut text = child(
                            ui,
                            Rect::from_min_size(column.min, vec2(column.width(), line(14.0))),
                            Layout::left_to_right(Align::Min),
                        );
                        text.spacing_mut().item_spacing.x = 12.0;
                        theme::text(
                            &mut text,
                            if state.busy {
                                format!("{verb} {title}…")
                            } else if mine.as_deref() == Some("install-app") {
                                "Installed successfully".into()
                            } else {
                                format!("{title} · Complete")
                            },
                            14.0,
                            theme::palette().text,
                        );
                        text.with_layout(Layout::right_to_left(Align::Min), |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(if state.detail.is_empty() {
                                        state.stage.clone()
                                    } else {
                                        format!("{} · {}", state.stage, state.detail)
                                    })
                                    .size(14.0)
                                    .color(theme::palette().text_3),
                                )
                                .truncate(),
                            );
                        });
                        let mut bar = child(
                            ui,
                            Rect::from_min_size(
                                pos2(column.left(), column.top() + line(14.0) + 8.0),
                                vec2(column.width(), 6.0),
                            ),
                            Layout::top_down(Align::Min),
                        );
                        theme::progress(
                            &mut bar,
                            if state.busy {
                                state.progress
                            } else {
                                Some(1.0)
                            },
                            6.0,
                        );
                    });
            });
        }
        // Source downloads and backup deletion leave the installed copy alone,
        // so its actions (Open in particular) stay available beside the progress.
        let replaces_actions = state.busy
            && matches!(
                mine.as_deref(),
                Some("install-app" | "uninstall-app" | "restore" | "move-app")
            );
        if !replaces_actions {
            if show_progress {
                ui.add_space(10.0);
            }
            self.app_actions(ui, state, &status);
        }
        ui.add_space(MAIN_GAP);
        self.facts(ui, &status);
        if let Some(alternate) = &status.alternate {
            ui.add_space(12.0);
            let location = if alternate.install_kind == "installer" {
                if cfg!(target_os = "macos") {
                    "Applications"
                } else {
                    "a system installation"
                }
            } else {
                "the portable library"
            };
            ui.spacing_mut().item_spacing.y = 4.0;
            theme::text(
                ui,
                format!(
                    "Also installed in {location} ({}){}",
                    alternate.version,
                    if status.installed.is_none() {
                        format!(". Switch the release format in Settings to manage it, or install a {format} copy.")
                    } else {
                        String::new()
                    }
                ),
                13.0,
                theme::palette().text_3,
            );
            ui.add(
                egui::Label::new(
                    RichText::new(&alternate.path)
                        .size(12.0)
                        .color(theme::palette().muted),
                )
                .truncate(),
            );
        }
    }

    fn version_split(
        &mut self,
        ui: &mut Ui,
        button: theme::Btn<'_>,
        status: &super::AppStatus,
    ) -> bool {
        let app = self.app.clone();
        let popup_id = ui.make_persistent_id((
            "app-version-menu",
            &app,
            &self.preferences.release_format,
            &self.preferences.architecture,
        ));
        let (main, arrow) = button.split(ui, &format!("Choose {} version", model::title(&app)));
        if arrow.clicked() {
            let opening = !ui.memory(|memory| memory.is_popup_open(popup_id));
            ui.memory_mut(|memory| memory.toggle_popup(popup_id));
            if opening {
                self.open_versions(ui.ctx());
            }
        }
        let mut anchor = main.clone().union(arrow.clone());
        anchor.rect = anchor.rect.translate(egui::vec2(0.0, 6.0));
        egui::popup::popup_below_widget(
            ui,
            popup_id,
            &anchor,
            egui::popup::PopupCloseBehavior::CloseOnClickOutside,
            |ui| {
                ui.set_width(278.0);
                ui.spacing_mut().item_spacing.y = 4.0;
                theme::text(ui, "CHOOSE A VERSION", 12.0, theme::palette().muted);
                ui.add_space(6.0);
                if self.versions_app.as_deref() != Some(app.as_str())
                    || self.versions_result.is_none()
                {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        theme::text(ui, "Loading versions…", 13.0, theme::palette().text_2);
                    });
                } else {
                    match self.versions_result.clone().unwrap() {
                        Err(error) => {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(error).size(12.0).color(theme::palette().red),
                                )
                                .wrap(),
                            );
                            if btn("Retry").show(ui).clicked() {
                                self.open_versions(ui.ctx());
                            }
                        }
                        Ok(versions) => {
                            if versions.is_empty() {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(
                                            "No compatible releases with published checksums.",
                                        )
                                        .size(13.0)
                                        .color(theme::palette().muted),
                                    )
                                    .wrap(),
                                );
                            }
                            egui::ScrollArea::vertical()
                                .id_salt((&app, "version-menu-scroll"))
                                .max_height(288.0)
                                .show(ui, |ui| {
                                    for (index, release) in versions.into_iter().enumerate() {
                                        let version = craft_apps_manager::updates::release_version(
                                            &release.tag_name,
                                        )
                                        .unwrap_or_default();
                                        let current = status
                                            .installed
                                            .as_ref()
                                            .is_some_and(|i| i.version == version);
                                        let (rect, response) = ui.allocate_exact_size(
                                            vec2(ui.available_width(), 40.0),
                                            Sense::click(),
                                        );
                                        if response.hovered() && !current {
                                            ui.painter().rect_filled(
                                                rect,
                                                CornerRadius::same(5),
                                                theme::palette().button_hover,
                                            );
                                            ui.ctx()
                                                .set_cursor_icon(egui::CursorIcon::PointingHand);
                                        }
                                        response.widget_info(|| {
                                            egui::WidgetInfo::labeled(
                                                egui::WidgetType::Button,
                                                !current,
                                                format!(
                                                    "Install {} {}",
                                                    model::title(&app),
                                                    version
                                                ),
                                            )
                                        });
                                        let color = if current {
                                            theme::palette().muted
                                        } else {
                                            theme::palette().text
                                        };
                                        ui.painter().text(
                                            pos2(rect.left() + 10.0, rect.center().y),
                                            egui::Align2::LEFT_CENTER,
                                            format!("Version {version}"),
                                            FontId::proportional(14.0),
                                            color,
                                        );
                                        let tag = if current {
                                            "Installed"
                                        } else if index == 0 {
                                            "Latest"
                                        } else {
                                            ""
                                        };
                                        ui.painter().text(
                                            pos2(rect.right() - 10.0, rect.center().y),
                                            egui::Align2::RIGHT_CENTER,
                                            tag,
                                            FontId::proportional(12.0),
                                            if current {
                                                theme::palette().muted
                                            } else {
                                                theme::palette().link
                                            },
                                        );
                                        if response.clicked() && !current {
                                            ui.memory_mut(|memory| memory.close_popup());
                                            self.review_version(app.clone(), release.tag_name);
                                        }
                                    }
                                });
                        }
                    }
                }
                ui.add_space(6.0);
                super::dialogs::rule(ui);
                ui.add_space(6.0);
                theme::text(
                    ui,
                    format!(
                        "{} · {} · {}",
                        model::release_os(),
                        release_format_label(&self.preferences.release_format),
                        model::architecture_label(&self.preferences.architecture)
                    ),
                    12.0,
                    theme::palette().muted,
                );
            },
        );
        main.clicked()
    }
    fn app_actions(&mut self, ui: &mut Ui, state: &State, status: &super::AppStatus) {
        let app = self.app.clone();
        let title = model::title(&app);
        let installed = status.installed.clone();
        // Only an installed copy in the active format can be updated in place.
        let matching_format = installed.as_ref().is_some_and(|i| {
            (i.install_kind == "installer") == (self.preferences.release_format == "installer")
        });
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
            ui.spacing_mut().interact_size.y = 0.0;
            match &installed {
                None => {
                    if self.version_split(
                        ui,
                        btn(&format!("Install {title}"))
                            .primary()
                            .size(Size::Card)
                            .icon(Icon::Download)
                            .enabled(!state.busy && !self.release_pending()),
                        status,
                    ) {
                        self.confirm_install = Some(app.clone());
                    }
                }
                Some(installed) => {
                    let update = status.update.clone().filter(|_| matching_format);
                    if update.is_some() {
                        if self.version_split(
                            ui,
                            btn(&format!("Update {title}"))
                                .primary()
                                .size(Size::Card)
                                .icon(Icon::Refresh)
                                .enabled(
                                    !state.busy && !status.checking && !self.release_pending(),
                                ),
                            status,
                        ) {
                            self.confirm_install = Some(app.clone());
                        }
                        if btn("Open")
                            .size(Size::Card)
                            .icon(Icon::Play)
                            .show(ui)
                            .clicked()
                        {
                            self.result(apps::launch(&self.paths, &app));
                        }
                    } else {
                        if btn(&format!("Open {title}"))
                            .primary()
                            .size(Size::Card)
                            .icon(Icon::Play)
                            .show(ui)
                            .clicked()
                        {
                            self.result(apps::launch(&self.paths, &app));
                        }
                        let label = if status.checking {
                            "Checking…"
                        } else if matching_format {
                            "Check for updates"
                        } else {
                            "Install (latest release)"
                        };
                        if self.version_split(
                            ui,
                            btn(label).size(Size::Card).icon(Icon::Refresh).enabled(
                                !state.busy && !status.checking && !self.release_pending(),
                            ),
                            status,
                        ) {
                            if matching_format {
                                self.check_selected_app(ui.ctx(), installed.version.clone());
                            } else {
                                self.confirm_install = Some(app.clone());
                            }
                        }
                    }
                    if btn("Move…")
                        .size(Size::Card)
                        .icon(Icon::Folder)
                        .enabled(!state.busy && !self.release_pending())
                        .show(ui)
                        .clicked()
                    {
                        self.confirm_move = Some(installed.clone());
                        self.move_parent = PathBuf::from(&installed.path)
                            .parent()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default();
                        self.move_running = None;
                        self.move_probe = None;
                        self.move_probe_at = std::time::Instant::now();
                    }
                    if btn("Uninstall…")
                        .size(Size::Card)
                        .icon_colored(Icon::Trash, theme::palette().red)
                        .enabled(!state.busy)
                        .show(ui)
                        .on_disabled_hover_text("Wait for the current operation to finish.")
                        .clicked()
                    {
                        self.confirm_uninstall = true;
                        self.delete_profile = false;
                    }
                }
            }
        });
        if status.checking {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.spinner();
                theme::text(ui, "Checking for updates…", 13.0, theme::palette().text_3);
            });
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(33));
        } else if let Some(check) = &status.check {
            ui.add_space(8.0);
            match check {
                Ok(None) => {
                    theme::text(
                        ui,
                        "You’re running the latest version.",
                        13.0,
                        theme::palette().text_3,
                    );
                }
                Ok(Some(version)) => {
                    theme::text(
                        ui,
                        format!("Version {version} is available."),
                        13.0,
                        theme::palette().link,
                    );
                }
                Err(error) => {
                    ui.add(
                        egui::Label::new(
                            RichText::new(error).size(13.0).color(theme::palette().red),
                        )
                        .wrap(),
                    );
                }
            }
        }
        if cfg!(target_os = "windows")
            && installed
                .as_ref()
                .is_some_and(|app| app.install_kind == "installer" && app.product_code.is_empty())
        {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                theme::text(
                    ui,
                    "Use Windows Installed apps to remove this installer version.",
                    13.0,
                    theme::palette().text_3,
                );
                if theme::link(ui, "Windows Installed apps").clicked() {
                    self.result(
                        Command::new("explorer.exe")
                            .arg("ms-settings:appsfeatures")
                            .spawn()
                            .map(|_| ())
                            .map_err(Into::into),
                    );
                }
            });
        }
    }

    /// Four facts about the app: installed copy, latest release, package and location.
    fn facts(&mut self, ui: &mut Ui, status: &super::AppStatus) {
        let app = self.app.clone();
        let installed = status.installed.clone();
        let latest = match (&status.update, &status.check, &installed) {
            (Some(version), _, _) => (version.clone(), "Newer than installed".to_owned()),
            (None, Some(Ok(None)), Some(installed)) => {
                (installed.version.clone(), "Up to date".to_owned())
            }
            (None, Some(Err(_)), _) => ("—".into(), "Check failed".into()),
            (None, _, Some(_)) => ("—".into(), "Not checked yet".into()),
            _ => ("—".into(), "At install".into()),
        };
        let location = installed
            .as_ref()
            .map(|i| {
                let path = PathBuf::from(&i.path);
                // The folder holding the executable or the macOS app bundle.
                let bundle = path.extension().is_some_and(|e| e == "app");
                let folder = if bundle || path.is_file() {
                    path.parent().map(|p| p.to_path_buf()).unwrap_or(path)
                } else {
                    path
                };
                folder.display().to_string()
            })
            .unwrap_or_else(|| "—".into());
        // Each cell is a label over one line, "value · detail" as in the design; a
        // value it cannot show in full shows it on hover.
        let joined = |value: &str, detail: &str| match (value, detail) {
            (_, "") => value.to_owned(),
            ("—", _) => detail.to_owned(),
            _ => format!("{value} · {detail}"),
        };
        let cells = [
            (
                "Installed",
                installed
                    .as_ref()
                    .map(|i| i.version.clone())
                    .unwrap_or_else(|| "Not installed".into()),
                installed.as_ref().map(|i| {
                    format!(
                        "{} · {}",
                        location_label(i),
                        model::architecture_label(&i.architecture)
                    )
                }),
            ),
            ("Latest release", joined(&latest.0, &latest.1), None),
            (
                "Package",
                joined(
                    release_format_label(&self.preferences.release_format),
                    if cfg!(target_os = "macos") {
                        "Universal"
                    } else {
                        model::architecture_label(&self.preferences.architecture)
                    },
                ),
                None,
            ),
            (
                "Location",
                location.clone(),
                installed.is_some().then(|| location.clone()),
            ),
        ];
        // A 1-point grid of hairlines: the border colour shows between the cells.
        // Four cells in a row, or two by two when the column is too narrow for them.
        let width = ui.available_width().min(720.0);
        let columns = if width >= 560.0 { 4 } else { 2 };
        let rows = cells.len() / columns;
        let row_height = 12.0 + line(12.0) + 4.0 + line(14.0) + 12.0;
        let (rect, _) = ui.allocate_exact_size(
            vec2(width, row_height * rows as f32 + (rows + 1) as f32),
            Sense::hover(),
        );
        ui.painter()
            .rect_filled(rect, CornerRadius::same(10), theme::palette().border);
        let inner = rect.shrink(1.0);
        let cell = (inner.width() - (columns - 1) as f32)
            / if columns == 4 { 4.7 } else { columns as f32 };
        for (index, (label, value, hover)) in cells.iter().enumerate() {
            let (row, column) = (index / columns, index % columns);
            let cell_rect = Rect::from_min_size(
                pos2(
                    inner.left() + column as f32 * (cell + 1.0),
                    inner.top() + row as f32 * (row_height + 1.0),
                ),
                vec2(
                    if columns == 4 && column == 3 {
                        cell * 1.7
                    } else {
                        cell
                    },
                    row_height,
                ),
            );
            // Only the grid's outer corners are rounded.
            let corner = |at_row: bool, at_column: bool| if at_row && at_column { 9 } else { 0 };
            let (first_row, last_row) = (row == 0, row == rows - 1);
            let (first_column, last_column) = (column == 0, column == columns - 1);
            let radius = CornerRadius {
                nw: corner(first_row, first_column),
                ne: corner(first_row, last_column),
                sw: corner(last_row, first_column),
                se: corner(last_row, last_column),
            };
            ui.painter()
                .rect_filled(cell_rect, radius, theme::palette().panel);
            let mut ui = child(
                ui,
                cell_rect.shrink2(vec2(14.0, 12.0)),
                Layout::top_down(Align::Min),
            );
            ui.spacing_mut().item_spacing.y = 4.0;
            theme::text(&mut ui, *label, 12.0, theme::palette().muted);
            let openable = *label == "Location"
                && installed.is_some()
                && !location.is_empty()
                && location != "—"
                && PathBuf::from(&location).is_dir();
            if openable {
                let response = ui
                    .horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let (rect, icon_response) =
                            ui.allocate_exact_size(vec2(16.0, line(14.0)), Sense::click());
                        theme::paint_icon(
                            ui.painter(),
                            Rect::from_center_size(rect.center(), vec2(14.0, 14.0)),
                            Icon::Folder,
                            theme::palette().link,
                        );
                        // Paint the truncated label directly: Label::ui adds its
                        // own tooltip, which overlaps our full-path tooltip.
                        let (text_pos, galley, text_response) = egui::Label::new(
                            RichText::new(value).size(14.0).color(theme::palette().link),
                        )
                        .truncate()
                        .sense(Sense::click())
                        .layout_in_ui(ui);
                        ui.painter().galley(text_pos, galley, theme::palette().link);
                        icon_response.union(text_response)
                    })
                    .inner
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(format!("Open folder\n{location}"));
                if response.clicked() {
                    self.result(craft_apps_manager::platform::open(std::path::Path::new(
                        &location,
                    )));
                }
            } else {
                let response = ui.add(
                    egui::Label::new(RichText::new(value).size(14.0).color(theme::palette().text))
                        .truncate(),
                );
                if let Some(hover) = hover {
                    response.on_hover_text(hover);
                }
            }
        }
        ui.add_space(MAIN_GAP);
        theme::hyperlink(
            ui,
            "Release notes",
            format!(
                "https://github.com/{}/releases/latest",
                model::repository(&app)
            ),
            13.0,
        );
    }

    fn tools_column(&mut self, ui: &mut Ui, state: &State) {
        let app = self.app.clone();
        let status = self.status(&app);
        let installed = status.installed.clone();
        let build = self.build_job.state.lock().unwrap().clone();
        let building_here = self.build_app == app;
        ui.spacing_mut().item_spacing.y = 0.0;
        theme::section_label(ui, &format!("{} tools", model::title(&app)));
        ui.add_space(12.0);
        theme::group().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            // Launch options
            let launch = self.display_launch.get(&app).cloned().unwrap_or_default();
            let summary = match (launch.executable.is_empty(), launch.arguments.len()) {
                (true, 0) => "No extra arguments".to_owned(),
                (true, 1) => "1 argument".to_owned(),
                (true, n) => format!("{n} arguments"),
                (false, 0) => launch.executable.clone(),
                (false, n) => format!("{} · {n} argument(s)", launch.executable),
            };
            let edit = tool_row(
                ui,
                Row {
                    icon: Icon::Sliders,
                    title: "Launch options",
                    detail: &summary,
                    color: theme::palette().muted,
                    gap: 0.0,
                    progress: None,
                },
                btn("Edit…").enabled(installed.is_some()),
            );
            if edit
                .on_disabled_hover_text("Install the app first.")
                .clicked()
            {
                match apps::settings(&self.paths, &app) {
                    Ok(settings) => {
                        self.launch_arguments = settings.arguments.join("\n");
                        self.launch_draft = settings;
                        self.launch_build_options = false;
                        self.launch_settings_open = true;
                    }
                    Err(error) => self.result(Err(error)),
                }
            }
            theme::divider(ui);
            // Backups
            let backups = self.display_backups.get(&app).cloned().unwrap_or_default();
            let summary = match backups.len() {
                0 => "None yet".to_owned(),
                1 => "1 backup".to_owned(),
                n => format!("{n} backups"),
            };
            let manage = tool_row(
                ui,
                Row {
                    icon: Icon::Archive,
                    title: "Backups",
                    detail: &summary,
                    color: theme::palette().muted,
                    gap: 0.0,
                    progress: None,
                },
                btn("Manage…").enabled(!state.busy && !backups.is_empty()),
            );
            if manage
                .on_disabled_hover_text(if state.busy {
                    "Wait for the current operation to finish."
                } else {
                    "No backups are available for this app."
                })
                .clicked()
            {
                self.restore_backups = backups;
                self.restore_selected = None;
                self.confirm_restore = false;
                self.backup_delete_mode = false;
                self.backup_delete_selected.clear();
                self.restore_app = Some(app.clone());
            }
            theme::divider(ui);
            // Source
            let source = self.display_sources.get(&app).cloned();
            let summary = match &source {
                Some(source) => format!(
                    "{} · {} · {}",
                    &source.sha[..7.min(source.sha.len())],
                    source.branch,
                    short_date(&source.downloaded_at)
                ),
                None => "Not downloaded".to_owned(),
            };
            let download = tool_row(
                ui,
                Row {
                    icon: Icon::Code,
                    title: "Source",
                    detail: &summary,
                    color: theme::palette().muted,
                    gap: 0.0,
                    progress: None,
                },
                btn(if source.is_some() {
                    "Update"
                } else {
                    "Download"
                })
                .enabled(!state.busy),
            );
            if download
                .on_hover_text("Downloads the latest source ZIP from GitHub")
                .on_disabled_hover_text("Wait for the current operation to finish.")
                .clicked()
            {
                self.start("source-app");
            }
            if source.is_some() {
                egui::Frame::new()
                    .inner_margin(egui::Margin {
                        left: 14,
                        right: 14,
                        top: 0,
                        bottom: 12,
                    })
                    .show(ui, |ui| {
                        if theme::link_enabled(
                            ui,
                            "Remove downloaded source…",
                            !state.busy && !build.busy,
                        )
                        .on_hover_text(
                            "Remove this app’s downloaded source ZIP. Builds and backups are kept.",
                        )
                        .clicked()
                        {
                            self.remove_source_confirm = Some(app.clone());
                        }
                    });
            }
            theme::divider(ui);
            // Build
            let last = self.display_builds.get(&app).cloned();
            let running = build.busy && building_here;
            let failed = building_here && !build.busy && build.stage == "Failed";
            let setup = self.build_action == "setup";
            let cleaning = self.cleaning();
            let (summary, color) = if running {
                (
                    if build.detail.is_empty() {
                        build.stage.clone()
                    } else {
                        format!("{} · {}", build.stage, build.detail)
                    },
                    theme::palette().link,
                )
            } else if failed {
                (
                    if setup {
                        "Tool setup failed · see the log"
                    } else {
                        "Build failed · see the log"
                    }
                    .to_owned(),
                    theme::palette().red,
                )
            } else if let Some((_, info)) = &last {
                (
                    info.as_ref()
                        .map(|info| {
                            format!(
                                "Ready · {}",
                                short_date(&info.built_at).split(", ").next().unwrap_or(""),
                            )
                        })
                        .unwrap_or_else(|| "Built".into()),
                    theme::palette().green,
                )
            } else {
                ("Not built yet".to_owned(), theme::palette().muted)
            };
            egui::Frame::new()
                .inner_margin(egui::Margin::same(14))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
                    ui.horizontal_top(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                            ui.allocate_ui_with_layout(
                                vec2(76.0, 0.0),
                                Layout::top_down(Align::Min),
                                |ui| {
                                    let label = if running {
                                        "Cancel"
                                    } else if failed {
                                        "Try again"
                                    } else if last.is_some() {
                                        "Rebuild"
                                    } else {
                                        "Build"
                                    };
                                    if btn(label)
                                        .kind(theme::Kind::Tinted)
                                        .min_width(72.0)
                                        .enabled(!cleaning && (running || !build.busy))
                                        .show(ui)
                                        .clicked()
                                    {
                                        if running {
                                            self.build_job
                                                .cancel
                                                .store(true, std::sync::atomic::Ordering::Relaxed);
                                        } else {
                                            self.start_build(if failed && setup {
                                                "setup"
                                            } else {
                                                "build"
                                            });
                                        }
                                    }
                                    if last.is_some() {
                                        ui.add_space(6.0);
                                        let launch = btn("Launch")
                                            .kind(theme::Kind::Tinted)
                                            .min_width(72.0)
                                            .enabled(!cleaning)
                                            .show(ui);
                                        if !cleaning {
                                            let pulse = ((ui.input(|i| i.time) * 2.5).sin() as f32
                                                + 1.0)
                                                * 0.5;
                                            let blue = theme::palette().accent;
                                            let alpha = if launch.hovered() {
                                                255
                                            } else {
                                                (100.0 + pulse * 155.0) as u8
                                            };
                                            ui.painter().rect_stroke(
                                                launch.rect,
                                                CornerRadius::same(8),
                                                Stroke::new(
                                                    1.0_f32,
                                                    Color32::from_rgba_unmultiplied(
                                                        blue.r(),
                                                        blue.g(),
                                                        blue.b(),
                                                        alpha,
                                                    ),
                                                ),
                                                egui::StrokeKind::Inside,
                                            );
                                            ui.ctx().request_repaint_after(
                                                std::time::Duration::from_millis(33),
                                            );
                                        }
                                        if launch.clicked() {
                                            self.result(builder::launch_local(&self.paths, &app));
                                        }
                                    }
                                },
                            );
                            ui.allocate_ui_with_layout(
                                vec2(ui.available_width(), 0.0),
                                Layout::top_down(Align::Min),
                                |ui| {
                                    theme::text(ui, "Local build", 14.0, theme::palette().text);
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&summary).size(12.0).color(color),
                                        )
                                        .wrap(),
                                    );
                                    if let Some((_, Some(info))) = &last {
                                        theme::text(
                                            ui,
                                            format!(
                                                "Source {}",
                                                &info.commit[..7.min(info.commit.len())]
                                            ),
                                            12.0,
                                            theme::palette().muted,
                                        );
                                    }
                                },
                            );
                        });
                    });
                    if running {
                        theme::progress(ui, build.progress, 4.0);
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if theme::link(ui, "Build options…").clicked() {
                            match builder::launch_options(&self.paths, &app) {
                                Ok(options) => {
                                    self.launch_arguments = options.arguments.join("\n");
                                    self.launch_draft = options;
                                }
                                Err(error) => {
                                    self.result(Err(error));
                                    return;
                                }
                            }
                            self.delete_build_confirm = false;
                            self.clean_build_confirm = false;
                            self.build_options_open = true;
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let cleaning = self.cleaning();
            let setup = theme::link_enabled(ui, "Set up build tools", !build.busy && !cleaning)
                .on_hover_text(
                    "Installs or checks the Rust toolchain and other build prerequisites",
                )
                .on_disabled_hover_text("Wait for the current build or cleanup to finish.");
            if setup.clicked() {
                self.start_build("setup");
            }
                        });
                    });
                });
        });
        if installed.is_some() {
            ui.add_space(8.0);
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            ui.spacing_mut().interact_size.y = 0.0;
            if installed.is_some() && theme::link(ui, folder_label()).clicked() {
                if let Some(installed) = &installed {
                    let path = PathBuf::from(&installed.path);
                    let folder = if path.is_file() {
                        path.parent().unwrap_or(&path)
                    } else {
                        &path
                    };
                    // On macOS, select the app bundle in Finder instead of only
                    // opening its folder.
                    let bundle = cfg!(target_os = "macos")
                        .then(|| model::installed_executable(folder, &app))
                        .flatten();
                    let result = match bundle {
                        Some(bundle) => Command::new("open")
                            .arg("-R")
                            .arg(bundle)
                            .spawn()
                            .map(|_| ())
                            .map_err(Into::into),
                        None => platform::open(folder),
                    };
                    self.result(result);
                }
            }
        });
        if building_here && (build.busy || !build.stage.is_empty()) {
            ui.add_space(8.0);
            theme::section_label(ui, "Build log");
            ui.add_space(8.0);
            theme::log_view(ui, &build.log, false);
        }
    }
}

/// A child Ui laid out in `rect`, leaving the parent's cursor where it is.
fn child(ui: &mut Ui, rect: Rect, layout: Layout) -> Ui {
    ui.new_child(UiBuilder::new().max_rect(rect).layout(layout))
}

/// What a row of the tools group shows left of its button.
struct Row<'a> {
    icon: Icon,
    title: &'a str,
    detail: &'a str,
    color: Color32,
    /// Space between the title and the detail.
    gap: f32,
    /// A build's progress, shown under the detail.
    progress: Option<Option<f32>>,
}

/// A row of the tools group: icon, title over detail, and a button on the right,
/// all centred on the row. Returns the button's response.
fn tool_row(ui: &mut Ui, row: Row, button: theme::Btn) -> Response {
    let width = ui.available_width();
    let button_size = button.desired_size(ui);
    let text_width = width - TOOL_TEXT_X - 10.0 - button_size.x - 12.0;
    let detail = ui.fonts(|fonts| {
        fonts.layout(
            row.detail.to_owned(),
            FontId::proportional(12.0),
            row.color,
            text_width,
        )
    });
    let progress = if row.progress.is_some() { 8.0 } else { 0.0 };
    let column = line(14.0) + row.gap + detail.size().y + progress;
    let (rect, _) =
        ui.allocate_exact_size(vec2(width, TOOL_ROW.max(column + 12.0)), Sense::hover());
    let icon = Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), vec2(16.0, 16.0));
    theme::paint_icon(ui.painter(), icon, row.icon, theme::palette().muted);
    let top = rect.center().y - column / 2.0;
    let mut text = child(
        ui,
        Rect::from_min_size(
            pos2(rect.left() + TOOL_TEXT_X, top),
            vec2(text_width, column),
        ),
        Layout::top_down(Align::Min),
    );
    text.spacing_mut().item_spacing.y = row.gap;
    theme::text(&mut text, row.title, 14.0, theme::palette().text);
    text.add(egui::Label::new(RichText::new(row.detail).size(12.0).color(row.color)).wrap());
    if let Some(value) = row.progress {
        text.add_space(4.0 - row.gap);
        theme::progress(&mut text, value, 4.0);
    }
    let mut action = child(
        ui,
        Rect::from_min_size(
            pos2(
                rect.right() - 12.0 - button_size.x,
                rect.center().y - button_size.y / 2.0,
            ),
            button_size,
        ),
        Layout::left_to_right(Align::Min),
    );
    button.show(&mut action)
}
