//! One app's page: what it is and its main actions in the middle, its tools on the right.
use super::overview::{log_view, release_format_label};
use super::theme::{self, btn, Icon, Kind};
use super::App;
use craft_apps_manager::{apps, jobs::State, model, platform};
use eframe::egui::{self, CornerRadius, RichText};
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

impl App {
    pub(super) fn app_page(&mut self, ctx: &egui::Context, state: &State) {
        egui::SidePanel::right("app-tools")
            .resizable(false)
            .exact_width(320.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .inner_margin(egui::Margin::same(20)),
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

    fn app_main(&mut self, ui: &mut egui::Ui, state: &State) {
        let app = self.app.clone();
        let title = model::title(&app);
        let status = self.status(&app);
        let format = self.preferences.release_format.clone();
        let mine = self
            .job_target
            .as_ref()
            .filter(|(target, _)| *target == app)
            .map(|(_, action)| action.clone());
        // Header: artwork, name, then category · state · repository.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 20.0;
            if let Some(texture) = self.icons.get(&app) {
                ui.add(
                    egui::Image::new((texture.id(), egui::vec2(72.0, 72.0)))
                        .corner_radius(CornerRadius::same(16)),
                );
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.add_space(6.0);
                theme::heading(ui, &title, 26.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
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
                    ui.hyperlink_to(
                        RichText::new(format!("github.com/storytold/{repository}")).size(13.0),
                        format!("https://github.com/storytold/{repository}"),
                    )
                    .on_hover_text("View repository");
                });
            });
        });
        ui.add_space(20.0);

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
                    .stroke(egui::Stroke::new(1.0, theme::RED_BORDER))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(egui::Margin::symmetric(14, 12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            let (rect, _) = ui
                                .allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
                            theme::paint_icon(ui.painter(), rect, Icon::Alert, theme::RED);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if theme::icon_button(
                                        ui,
                                        Icon::Close,
                                        "Dismiss",
                                        theme::RED_TEXT,
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
                                    ui.with_layout(
                                        egui::Layout::top_down(egui::Align::Min),
                                        |ui| {
                                            ui.spacing_mut().item_spacing.y = 2.0;
                                            ui.label(
                                                RichText::new(format!("Couldn't {verb} {title}"))
                                                    .font(theme::bold(14.0))
                                                    .color(egui::Color32::from_rgb(
                                                        0xff, 0xd6, 0xd8,
                                                    )),
                                            );
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(&state.outcome)
                                                        .size(13.0)
                                                        .color(theme::RED_TEXT),
                                                )
                                                .wrap(),
                                            );
                                        },
                                    );
                                },
                            );
                        });
                    });
                ui.add_space(16.0);
            }
        }

        let description = model::description(&app);
        if !description.is_empty() {
            ui.add(
                egui::Label::new(
                    RichText::new(description)
                        .size(15.0)
                        .color(theme::TEXT_2)
                        .line_height(Some(23.0)),
                )
                .wrap(),
            );
            ui.add_space(20.0);
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
            egui::Frame::new()
                .fill(theme::PROGRESS_BG)
                .stroke(egui::Stroke::new(1.0, theme::ACCENT_BORDER))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(egui::Margin::symmetric(14, 12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width().min(560.0));
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            self.cancel_button(ui, &state.stage);
                            ui.add_space(8.0);
                            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                                ui.horizontal(|ui| {
                                    theme::text(ui, format!("{verb} {title}…"), 14.0, theme::TEXT);
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(if state.detail.is_empty() {
                                                        state.stage.clone()
                                                    } else {
                                                        format!(
                                                            "{} · {}",
                                                            state.stage, state.detail
                                                        )
                                                    })
                                                    .size(13.0)
                                                    .color(theme::TEXT_3),
                                                )
                                                .truncate(),
                                            );
                                        },
                                    );
                                });
                                theme::progress(ui, state.progress, 6.0);
                            });
                        });
                    });
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
                ui.add_space(12.0);
            }
            self.app_actions(ui, state, &status);
        }
        ui.add_space(24.0);
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

    fn app_actions(&mut self, ui: &mut egui::Ui, state: &State, status: &super::AppStatus) {
        let app = self.app.clone();
        let title = model::title(&app);
        let installed = status.installed.clone();
        // Only an installed copy in the active format can be updated in place.
        let matching_format = installed.as_ref().is_some_and(|i| {
            (i.install_kind == "installer") == (self.preferences.release_format == "installer")
        });
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            match &installed {
                None => {
                    if btn(&format!("Install {title}"))
                        .primary()
                        .large()
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
                            .large()
                            .icon(Icon::Refresh)
                            .enabled(!state.busy && !status.checking)
                            .show(ui)
                            .clicked()
                        {
                            self.confirm_install = Some(app.clone());
                        }
                        if btn("Open").large().icon(Icon::Play).show(ui).clicked() {
                            self.result(apps::launch(&self.paths, &app));
                        }
                    } else {
                        if btn(&format!("Open {title}"))
                            .primary()
                            .large()
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
                            .large()
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
                        .large()
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
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spinner();
                theme::text(ui, "Checking for updates…", 13.0, theme::TEXT_3);
            });
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(33));
        } else if let Some(check) = &status.check {
            ui.add_space(6.0);
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
            ui.add_space(6.0);
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
    fn facts(&mut self, ui: &mut egui::Ui, status: &super::AppStatus) {
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
        let cells = [
            (
                "Installed",
                installed
                    .as_ref()
                    .map(|i| i.version.clone())
                    .unwrap_or_else(|| "Not installed".into()),
                installed
                    .as_ref()
                    .map(|i| {
                        format!(
                            "{} · {}",
                            location_label(i),
                            model::architecture_label(&i.architecture)
                        )
                    })
                    .unwrap_or_default(),
            ),
            ("Latest release", latest.0, latest.1),
            (
                "Package",
                release_format_label(&self.preferences.release_format).to_owned(),
                if cfg!(target_os = "macos") {
                    "Universal".to_owned()
                } else {
                    model::architecture_label(&self.preferences.architecture).to_owned()
                },
            ),
            ("Location", location.clone(), String::new()),
        ];
        theme::group().show(ui, |ui| {
            let width = ui.available_width();
            ui.set_width(width);
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
            ui.horizontal_top(|ui| {
                let cell = (width / 4.0).floor();
                for (index, (label, value, detail)) in cells.iter().enumerate() {
                    if index > 0 {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(1.0, 64.0), egui::Sense::hover());
                        ui.painter().rect_filled(rect, 0.0, theme::BORDER);
                    }
                    ui.allocate_ui_with_layout(
                        egui::vec2(cell - 1.0, 64.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            egui::Frame::new()
                                .inner_margin(egui::Margin::symmetric(14, 12))
                                .show(ui, |ui| {
                                    ui.set_width(cell - 29.0);
                                    ui.spacing_mut().item_spacing.y = 3.0;
                                    theme::text(ui, *label, 12.0, theme::MUTED);
                                    let response = ui.add(
                                        egui::Label::new(
                                            RichText::new(value).size(14.0).color(theme::TEXT),
                                        )
                                        .truncate(),
                                    );
                                    if *label == "Location" && installed.is_some() {
                                        response.on_hover_text(&location);
                                    }
                                    if !detail.is_empty() {
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(detail)
                                                    .size(12.0)
                                                    .color(theme::MUTED),
                                            )
                                            .truncate(),
                                        );
                                    }
                                });
                        },
                    );
                }
            });
        });
        ui.add_space(10.0);
        ui.hyperlink_to(
            RichText::new("Release notes").size(13.0),
            format!(
                "https://github.com/storytold/{}/releases/latest",
                model::repository(&app)
            ),
        );
    }

    fn tools_column(&mut self, ui: &mut egui::Ui, state: &State) {
        let app = self.app.clone();
        let status = self.status(&app);
        let installed = status.installed.clone();
        let build = self.build_job.state.lock().unwrap().clone();
        let building_here = self.build_app == app;
        ui.spacing_mut().item_spacing.y = 12.0;
        theme::section_label(ui, &format!("{} tools", model::title(&app)));
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
            if tool_row(
                ui,
                Icon::Sliders,
                "Launch options",
                &summary,
                theme::MUTED,
                |ui| {
                    btn("Edit…")
                        .enabled(installed.is_some())
                        .show(ui)
                        .on_disabled_hover_text("Install the app first.")
                        .clicked()
                },
            ) {
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
            if tool_row(ui, Icon::Archive, "Backups", &summary, theme::MUTED, |ui| {
                btn("Manage…")
                    .enabled(!state.busy && !backups.is_empty())
                    .show(ui)
                    .on_disabled_hover_text(if state.busy {
                        "Wait for the current operation to finish."
                    } else {
                        "No backups are available for this app."
                    })
                    .clicked()
            }) {
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
            if tool_row(ui, Icon::Code, "Source", &summary, theme::MUTED, |ui| {
                btn(if source.is_some() {
                    "Update"
                } else {
                    "Download"
                })
                .enabled(!state.busy)
                .show(ui)
                .on_hover_text("Downloads the latest source ZIP from GitHub")
                .on_disabled_hover_text("Wait for the current operation to finish.")
                .clicked()
            }) {
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
                    theme::RED,
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
            let clicked = tool_row(ui, Icon::Wrench, "Build", &summary, color, |ui| {
                if running {
                    let requested = self
                        .build_job
                        .cancel
                        .load(std::sync::atomic::Ordering::Relaxed);
                    if btn(if requested { "Cancelling…" } else { "Cancel" })
                        .enabled(!requested)
                        .show(ui)
                        .clicked()
                    {
                        self.build_job
                            .cancel
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    false
                } else {
                    btn(if failed {
                        "Try again"
                    } else if last.is_some() {
                        "Rebuild"
                    } else {
                        "Build"
                    })
                    .kind(Kind::Soft)
                    .enabled(!build.busy && !cleaning)
                    .show(ui)
                    .on_disabled_hover_text(if cleaning {
                        "Wait for the build file cleanup to finish.".to_owned()
                    } else {
                        format!(
                            "Building {}. One build runs at a time.",
                            model::title(&self.build_app)
                        )
                    })
                    .clicked()
                }
            });
            if clicked {
                // Try again repeats whatever failed, which may be tool setup.
                self.start_build(if failed && setup { "setup" } else { "build" });
            }
            egui::Frame::new()
                .inner_margin(egui::Margin {
                    left: 40,
                    right: 14,
                    top: 0,
                    bottom: 12,
                })
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    if running {
                        theme::progress(ui, build.progress, 4.0);
                    }
                    ui.add_enabled(
                        !build.busy,
                        egui::Checkbox::new(&mut self.latest, "Download the latest source first"),
                    )
                    .on_hover_text(
                        "Unchecked builds from your local source ZIP without contacting GitHub.",
                    );
                    let output = if building_here {
                        build.output.clone()
                    } else {
                        None
                    }
                    .or_else(|| last.as_ref().map(|(folder, _)| folder.clone()));
                    ui.horizontal(|ui| {
                        if let Some(output) = &output {
                            if btn("Open folder").show(ui).clicked() {
                                self.result(platform::open(output));
                            }
                        }
                        let log = self.paths.at(format!("logs/{app}.log"));
                        if log.is_file() && btn("View log").show(ui).clicked() {
                            self.result(platform::open(&log));
                        }
                    });
                });
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
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
            ui.add_space(4.0);
            theme::section_label(ui, "Build log");
            log_view(ui, &build.log);
        }
    }
}

/// A row of the tools group. Returns whether its action was clicked.
fn tool_row(
    ui: &mut egui::Ui,
    icon: Icon,
    title: &str,
    detail: &str,
    detail_color: egui::Color32,
    action: impl FnOnce(&mut egui::Ui) -> bool,
) -> bool {
    let mut clicked = false;
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.set_min_height(36.0);
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                theme::paint_icon(ui.painter(), rect, icon, theme::MUTED);
                ui.add_space(2.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    clicked = action(ui);
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        theme::text(ui, title, 14.0, theme::TEXT);
                        ui.add(
                            egui::Label::new(RichText::new(detail).size(12.0).color(detail_color))
                                .truncate(),
                        );
                    });
                });
            });
        });
    clicked
}
