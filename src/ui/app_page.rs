//! One app's page: what it is and its main actions in the middle, its tools on the right.
use super::overview::release_format_label;
use super::theme::{self, btn, Icon, Kind, Size};
use super::App;
use craft_apps_manager::{apps, jobs::State, model, platform};
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
/// The failed build summary.
const RED_SOFT: Color32 = Color32::from_rgb(0xf2, 0xa0, 0xa4);
/// The error banner's title.
const RED_TITLE: Color32 = Color32::from_rgb(0xff, 0xd6, 0xd8);

impl App {
    pub(super) fn app_page(&mut self, ctx: &egui::Context, state: &State) {
        egui::SidePanel::right("app-tools")
            .resizable(false)
            .exact_width(320.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL)
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
            .frame(egui::Frame::new().fill(theme::BG))
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
                        theme::text(ui, category, 13.0, theme::TEXT_3);
                        theme::text(ui, "·", 13.0, theme::TEXT_3);
                    }
                    match &status.installed {
                        Some(installed) => {
                            theme::text(
                                ui,
                                format!("Installed {}", installed.version),
                                13.0,
                                theme::GREEN,
                            );
                        }
                        None if status.alternate.is_some() => {
                            theme::text(ui, format!("No {format} copy"), 13.0, theme::TEXT_3);
                        }
                        None if !status.loaded => {
                            theme::text(ui, "Checking…", 13.0, theme::TEXT_3);
                        }
                        None => {
                            theme::text(ui, "Not installed", 13.0, theme::TEXT_3);
                        }
                    }
                    theme::text(ui, "·", 13.0, theme::TEXT_3);
                    let repository = model::repository(&app);
                    theme::hyperlink(
                        ui,
                        format!("github.com/storytold/{repository}"),
                        format!("https://github.com/storytold/{repository}"),
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
                    "source-app" => "download the source for",
                    "restore" => "restore a backup of",
                    "delete-backups" => "delete backups of",
                    _ if status.installed.is_some() => "update",
                    _ => "install",
                };
                egui::Frame::new()
                    .fill(theme::RED_BG)
                    .stroke(Stroke::new(1.0, theme::RED_BORDER))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(egui::Margin::symmetric(14, 12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().interact_size.y = 0.0;
                        ui.spacing_mut().item_spacing = vec2(12.0, 0.0);
                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                            if theme::icon_button(ui, Icon::Close, "Dismiss", theme::RED_TEXT)
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
                                    theme::paint_icon(ui.painter(), rect, Icon::Alert, theme::RED);
                                });
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 2.0;
                                    ui.label(
                                        RichText::new(format!("Couldn't {verb} {title}"))
                                            .font(theme::bold(14.0))
                                            .color(RED_TITLE),
                                    );
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&state.outcome)
                                                .size(13.0)
                                                .color(theme::RED_TEXT),
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
                            .color(theme::TEXT_2)
                            .line_height(Some(line_height)),
                    )
                    .wrap(),
                );
            });
            ui.add_space(MAIN_GAP - shift);
        }

        // Progress for an operation running on this app, in place of the buttons.
        if state.busy && mine.is_some() {
            let verb = match mine.as_deref() {
                Some("uninstall-app") => "Uninstalling",
                Some("source-app") => "Downloading source for",
                Some("restore") => "Restoring",
                Some("delete-backups") => "Deleting backups of",
                _ if status.installed.is_some() => "Updating",
                _ => "Installing",
            };
            // 560 points of content inside the padding and border, as in the design.
            let width = ui.available_width().min(560.0 + 30.0);
            egui::Frame::new()
                .fill(theme::PROGRESS_BG)
                .stroke(Stroke::new(1.0, theme::ACCENT_BORDER))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(egui::Margin::symmetric(14, 12))
                .show(ui, |ui| {
                    let inner = width - 30.0;
                    ui.set_width(inner);
                    // Title row, 8 points, then the bar; Cancel centred beside them.
                    let height = line(14.0) + 8.0 + 6.0;
                    let (rect, _) = ui.allocate_exact_size(vec2(inner, height), Sense::hover());
                    let mut cancel = child(ui, rect, Layout::right_to_left(Align::Center));
                    self.cancel_button(&mut cancel, &state.stage);
                    let right = cancel.min_rect().left() - 16.0;
                    let column = Rect::from_min_max(rect.min, pos2(right, rect.max.y));
                    let mut text = child(
                        ui,
                        Rect::from_min_size(column.min, vec2(column.width(), line(14.0))),
                        Layout::left_to_right(Align::Min),
                    );
                    text.spacing_mut().item_spacing.x = 12.0;
                    theme::text(&mut text, format!("{verb} {title}…"), 14.0, theme::TEXT);
                    text.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(if state.detail.is_empty() {
                                    state.stage.clone()
                                } else {
                                    format!("{} · {}", state.stage, state.detail)
                                })
                                .size(14.0)
                                .color(theme::TEXT_3),
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
                    theme::progress(&mut bar, state.progress, 6.0);
                });
        }
        // Source downloads and backup deletion leave the installed copy alone,
        // so its actions (Open in particular) stay available beside the progress.
        let replaces_actions = state.busy
            && matches!(
                mine.as_deref(),
                Some("install-app" | "uninstall-app" | "restore")
            );
        if !replaces_actions {
            if state.busy && mine.is_some() {
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
                theme::TEXT_3,
            );
            ui.add(
                egui::Label::new(
                    RichText::new(&alternate.path)
                        .size(12.0)
                        .color(theme::MUTED),
                )
                .truncate(),
            );
        }
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
                    if btn(&format!("Install {title}"))
                        .primary()
                        .size(Size::Card)
                        .icon(Icon::Download)
                        .enabled(!state.busy)
                        .show(ui)
                        .on_disabled_hover_text("Wait for the current operation to finish.")
                        .clicked()
                    {
                        self.confirm_install = Some(app.clone());
                    }
                }
                Some(installed) => {
                    let update = status.update.clone().filter(|_| matching_format);
                    if let Some(version) = &update {
                        if btn(&format!("Update to {version}"))
                            .primary()
                            .size(Size::Card)
                            .icon(Icon::Refresh)
                            .enabled(!state.busy && !status.checking)
                            .show(ui)
                            .clicked()
                        {
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
                        if btn(label)
                            .size(Size::Card)
                            .icon(Icon::Refresh)
                            .enabled(!state.busy && !status.checking)
                            .show(ui)
                            .clicked()
                        {
                            if matching_format {
                                self.check_selected_app(ui.ctx(), installed.version.clone());
                            } else {
                                self.confirm_install = Some(app.clone());
                            }
                        }
                    }
                    if btn("Uninstall…")
                        .size(Size::Card)
                        .icon_colored(Icon::Trash, theme::RED)
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
                theme::text(ui, "Checking for updates…", 13.0, theme::TEXT_3);
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
                        theme::TEXT_3,
                    );
                }
                Ok(Some(version)) => {
                    theme::text(
                        ui,
                        format!("Version {version} is available."),
                        13.0,
                        theme::LINK,
                    );
                }
                Err(error) => {
                    ui.add(
                        egui::Label::new(RichText::new(error).size(13.0).color(theme::RED)).wrap(),
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
                    theme::TEXT_3,
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
            .rect_filled(rect, CornerRadius::same(10), theme::BORDER);
        let inner = rect.shrink(1.0);
        let cell = (inner.width() - (columns - 1) as f32) / columns as f32;
        for (index, (label, value, hover)) in cells.iter().enumerate() {
            let (row, column) = (index / columns, index % columns);
            let cell_rect = Rect::from_min_size(
                pos2(
                    inner.left() + column as f32 * (cell + 1.0),
                    inner.top() + row as f32 * (row_height + 1.0),
                ),
                vec2(cell, row_height),
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
            ui.painter().rect_filled(cell_rect, radius, theme::PANEL);
            let mut ui = child(
                ui,
                cell_rect.shrink2(vec2(14.0, 12.0)),
                Layout::top_down(Align::Min),
            );
            ui.spacing_mut().item_spacing.y = 4.0;
            theme::text(&mut ui, *label, 12.0, theme::MUTED);
            let response = ui.add(
                egui::Label::new(RichText::new(value).size(14.0).color(theme::TEXT)).truncate(),
            );
            if let Some(hover) = hover {
                response.on_hover_text(hover);
            }
        }
        ui.add_space(MAIN_GAP);
        theme::hyperlink(
            ui,
            "Release notes",
            format!(
                "https://github.com/storytold/{}/releases/latest",
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
                    color: theme::MUTED,
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
                    color: theme::MUTED,
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
                    color: theme::MUTED,
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
                    theme::LINK,
                )
            } else if failed {
                (
                    if setup {
                        "Tool setup failed · see the log"
                    } else {
                        "Build failed · see the log"
                    }
                    .to_owned(),
                    RED_SOFT,
                )
            } else if let Some((_, info)) = &last {
                (
                    info.as_ref()
                        .map(|info| {
                            format!(
                                "Built {} · {}",
                                short_date(&info.built_at),
                                &info.commit[..7.min(info.commit.len())]
                            )
                        })
                        .unwrap_or_else(|| "Built".into()),
                    theme::GREEN,
                )
            } else {
                ("No builds yet".to_owned(), theme::MUTED)
            };
            let row = Row {
                icon: Icon::Wrench,
                title: "Build",
                detail: &summary,
                color,
                gap: 2.0,
                progress: running.then_some(build.progress),
            };
            if running {
                let requested = self
                    .build_job
                    .cancel
                    .load(std::sync::atomic::Ordering::Relaxed);
                let cancel = tool_row(
                    ui,
                    row,
                    btn(if requested { "Cancelling…" } else { "Cancel" }).enabled(!requested),
                );
                if cancel.clicked() {
                    self.build_job
                        .cancel
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                }
            } else {
                let start = tool_row(
                    ui,
                    row,
                    btn(if failed {
                        "Try again"
                    } else if last.is_some() {
                        "Rebuild"
                    } else {
                        "Build"
                    })
                    .kind(Kind::Tinted)
                    .enabled(!build.busy && !cleaning),
                );
                if start
                    .on_disabled_hover_text(if cleaning {
                        "Wait for the build file cleanup to finish.".to_owned()
                    } else {
                        format!(
                            "Building {}. One build runs at a time.",
                            model::title(&self.build_app)
                        )
                    })
                    .clicked()
                {
                    // Try again repeats whatever failed, which may be tool setup.
                    self.start_build(if failed && setup { "setup" } else { "build" });
                }
            }
            egui::Frame::new()
                .inner_margin(egui::Margin {
                    left: TOOL_TEXT_X as i8,
                    right: 12,
                    top: 0,
                    bottom: 12,
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                    ui.spacing_mut().interact_size.y = 0.0;
                    let output = if building_here {
                        build.output.clone()
                    } else {
                        None
                    }
                    .or_else(|| last.as_ref().map(|(folder, _)| folder.clone()));
                    let log = self.paths.at(format!("logs/{app}.log"));
                    if output.is_some() || log.is_file() {
                        ui.horizontal(|ui| {
                            if let Some(output) = &output {
                                if btn("Open folder").show(ui).clicked() {
                                    self.result(platform::open(output));
                                }
                            }
                            if log.is_file() && btn("View log").show(ui).clicked() {
                                self.result(platform::open(&log));
                            }
                        });
                    }
                    ui.spacing_mut().icon_spacing = 10.0;
                    ui.add_enabled(
                        !build.busy,
                        egui::Checkbox::new(
                            &mut self.latest,
                            RichText::new("Download the latest source first")
                                .size(14.0)
                                .color(theme::TEXT_2),
                        ),
                    )
                    .on_hover_text(
                        "Unchecked builds from your local source ZIP without contacting GitHub.",
                    );
                });
        });
        ui.add_space(12.0);
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
        if building_here && (build.busy || !build.stage.is_empty()) {
            ui.add_space(20.0);
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
    theme::paint_icon(ui.painter(), icon, row.icon, theme::MUTED);
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
    theme::text(&mut text, row.title, 14.0, theme::TEXT);
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
