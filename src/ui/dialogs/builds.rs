use super::*;

pub(super) fn build_choice_label(info: &model::BuildInfo) -> String {
    let date = chrono::DateTime::parse_from_rfc3339(&info.built_at)
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%b %d, %Y · %I:%M %p")
                .to_string()
        })
        .unwrap_or_else(|_| info.built_at.clone());
    format!(
        "{} · {}",
        date,
        info.commit.chars().take(7).collect::<String>()
    )
}
impl App {
    pub(super) fn render_build_dialogs(&mut self, ctx: &egui::Context, busy: bool) {
        if let Some(app) = self.confirm_build.clone() {
            if let Some(receiver) = &self.build_folder_receiver {
                match receiver.try_recv() {
                    Ok(result) => {
                        self.build_folder_receiver = None;
                        match result {
                            Ok(Some(path)) => self.build_parent = path.display().to_string(),
                            Ok(None) => {}
                            Err(error) => self.error = Some(error),
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        self.build_folder_receiver = None
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                }
            }
            let source = self.display_sources.get(&app).cloned();
            let archive = self.paths.at(format!("sources/{app}-source.zip"));
            let has_source = source.is_some() && archive.is_file();
            let state = self.job.snapshot();
            let source_job = self.job_action == "source-app"
                && self
                    .job_target
                    .as_ref()
                    .is_some_and(|(target, _)| target == &app);
            let downloading = state.busy && source_job;
            let completed_alpha =
                if source_job && self.build_source_fresh && state.stage == "Complete" {
                    self.completed_progress_opacity(ctx)
                } else {
                    0.0
                };
            let progress_visible = downloading || completed_alpha > 0.0;
            let reveal = ctx.animate_bool_with_time(
                egui::Id::new(("build-source-progress", &app)),
                progress_visible,
                0.18,
            );
            if self.build_source_fresh && !state.busy && state.stage == "Failed" {
                self.build_source_fresh = false;
            }
            let busy = state.busy || self.build_job.snapshot().busy;
            let art = self
                .icons
                .get(&app)
                .map(|t| Art::Texture(t.id()))
                .unwrap_or(Art::None);
            let response = modal(ctx, "Build app", 520.0, |ui| {
                band(
                    ui,
                    egui::Margin {
                        left: 24,
                        right: 24,
                        top: 28,
                        bottom: 26,
                    },
                    0.0,
                    |ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 16.0;
                            if let Art::Texture(texture) = art {
                                ui.add(
                                    egui::Image::new((texture, egui::vec2(48.0, 48.0)))
                                        .corner_radius(CornerRadius::same(10)),
                                );
                            }
                            ui.vertical(|ui| {
                                theme::heading(ui, format!("Build {}", model::title(&app)), 18.0);
                                ui.add_space(4.0);
                                let os = if cfg!(target_os = "windows") {
                                    "Windows"
                                } else if cfg!(target_os = "macos") {
                                    "macOS"
                                } else {
                                    "Linux"
                                };
                                theme::text(
                                    ui,
                                    format!("Local build · {os} · {}", model::MANAGER_ARCH),
                                    13.0,
                                    theme::palette().muted,
                                );
                            });
                        });
                        ui.add_space(26.0);
                        if has_source {
                            theme::text(ui, "Source location", 12.0, theme::palette().muted);
                            ui.add_space(6.0);
                            folder_path_row(ui, &archive, |path| {
                                self.result(craft_apps_manager::platform::open(path))
                            });
                            if let Some(source) = &source {
                                note(
                                    ui,
                                    format!(
                                        "Downloaded source · {} · {}",
                                        source.branch,
                                        &source.sha[..7.min(source.sha.len())]
                                    ),
                                );
                            }
                            if !self.build_source_fresh {
                                ui.add_space(12.0);
                                ui.add_enabled(
                                    !busy,
                                    egui::Checkbox::new(
                                        &mut self.latest,
                                        "Download latest source first",
                                    ),
                                );
                            }
                        } else {
                            theme::text(ui, "Source", 12.0, theme::palette().muted);
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                theme::text(ui, "Not downloaded", 14.0, theme::palette().text);
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if btn("Download source")
                                            .kind(Kind::Secondary)
                                            .medium()
                                            .enabled(!busy)
                                            .show(ui)
                                            .clicked()
                                        {
                                            self.app = app.clone();
                                            self.start("source-app");
                                            self.build_source_fresh = true;
                                            self.latest = false;
                                        }
                                    },
                                );
                            });
                            ui.add_space(6.0);
                            if theme::link(ui, format!("{} · main", model::repository(&app)))
                                .clicked()
                            {
                                ui.ctx().open_url(egui::OpenUrl::new_tab(format!(
                                    "https://github.com/{}/tree/main",
                                    model::repository(&app)
                                )));
                            }
                        }
                        if reveal > 0.001 {
                            // Animate the reserved height too, so the section closes without a jump.
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 62.0 * reveal),
                                Sense::hover(),
                            );
                            let mut progress_ui = ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(egui::Rect::from_min_size(
                                        rect.min,
                                        egui::vec2(rect.width(), 62.0),
                                    ))
                                    .layout(egui::Layout::top_down(egui::Align::Min)),
                            );
                            progress_ui.set_clip_rect(rect.intersect(ui.clip_rect()));
                            progress_ui.multiply_opacity(if downloading {
                                reveal
                            } else {
                                completed_alpha * reveal
                            });
                            progress_ui.spacing_mut().item_spacing.y = 0.0;
                            progress_ui.add_space(12.0);
                            theme::text(
                                &mut progress_ui,
                                if downloading {
                                    &state.stage
                                } else {
                                    "Source downloaded"
                                },
                                12.0,
                                theme::palette().muted,
                            );
                            progress_ui.add_space(6.0);
                            theme::progress(
                                &mut progress_ui,
                                if downloading {
                                    state.progress
                                } else {
                                    Some(1.0)
                                },
                                4.0,
                            );
                            progress_ui.add_space(8.0);
                            progress_ui.add(
                                egui::Label::new(
                                    RichText::new(if downloading {
                                        state.detail.as_str()
                                    } else {
                                        "Ready to build."
                                    })
                                    .size(12.0)
                                    .color(theme::palette().muted),
                                )
                                .truncate(),
                            );
                        }
                        ui.add_space(20.0);
                        rule(ui);
                        ui.add_space(12.0);
                        theme::text(ui, "Build location", 12.0, theme::palette().muted);
                        ui.add_space(8.0);
                        field_style(ui);
                        ui.horizontal(|ui| {
                            ui.add_enabled(
                                !busy,
                                egui::TextEdit::singleline(&mut self.build_parent)
                                    .font(FontId::proportional(14.0))
                                    .margin(egui::vec2(10.0, 9.0))
                                    .desired_width(ui.available_width() - 114.0),
                            );
                            if btn("Browse…")
                                .kind(Kind::Secondary)
                                .medium()
                                .min_width(92.0)
                                .enabled(!busy && self.build_folder_receiver.is_none())
                                .show(ui)
                                .clicked()
                            {
                                let initial = std::path::PathBuf::from(&self.build_parent);
                                let (tx, rx) = std::sync::mpsc::channel();
                                self.build_folder_receiver = Some(rx);
                                let ctx = ctx.clone();
                                std::thread::spawn(move || {
                                    let _ = tx.send(
                                        craft_apps_manager::portable::pick_folder(&initial)
                                            .map_err(|e| format!("{e:#}")),
                                    );
                                    ctx.request_repaint();
                                });
                            }
                        });
                        ui.add_space(20.0);
                        rule(ui);
                        ui.add_space(12.0);
                        theme::text(ui, "Build folder", 12.0, theme::palette().muted);
                        let folder = std::path::PathBuf::from(&self.build_parent).join(&app);
                        ui.add_space(6.0);
                        folder_path_row(ui, &folder, |path| {
                            self.result(craft_apps_manager::platform::open(path))
                        });
                        ui.add_space(20.0);
                        rule(ui);
                        ui.add_space(12.0);
                        desktop_shortcut_option(ui, &mut self.build_shortcut);
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.add_space(25.0);
                            note(ui, "Available after the build finishes successfully.");
                        });
                        ui.add_space(18.0);
                        note(
                            ui,
                            "Compiles a separate local copy. Your installed app stays unchanged.",
                        );
                    },
                );
                footer(ui, |ui| {
                    let valid = craft_apps_manager::portable::destination(
                        std::path::Path::new(&self.build_parent),
                        &app,
                    )
                    .is_ok();
                    if action(
                        ui,
                        "Build",
                        Kind::Primary,
                        has_source && valid && !busy && self.build_folder_receiver.is_none(),
                    )
                    .clicked()
                    {
                        self.app = app.clone();
                        if self.build_source_fresh {
                            self.latest = false;
                        }
                        if self.builder {
                            self.start("build");
                        } else {
                            self.start_build("build");
                        }
                        self.confirm_build = None;
                    }
                    if action(ui, "Cancel", Kind::Secondary, true).clicked() {
                        self.confirm_build = None;
                    }
                });
            });
            if response.should_close() {
                self.confirm_build = None;
            }
        }

        if self.build_options_open {
            let app = self.app.clone();
            let build_busy = self.build_job.snapshot().busy
                || busy
                || self
                    .operation_records
                    .iter()
                    .any(|operation| Job::builder_operation(operation, &app));
            let response = modal(ctx, "Local build options", 560.0, |ui| {
                title_bar(ui, &format!("{} build options", model::title(&app)), |ui| {
                    if close_button(ui).clicked() {
                        self.build_options_open = false;
                    }
                });
                let body_height = (ctx.screen_rect().height() - 200.0).clamp(180.0, 650.0);
                ui.scope(|ui| {
                    // Keep the scroll area's paint margin inside the body, away
                    // from the fixed header and footer (including during hover).
                    let body_rect = egui::Rect::from_min_size(
                        ui.cursor().min,
                        egui::vec2(ui.available_width(), body_height),
                    );
                    ui.set_clip_rect(ui.clip_rect().intersect(body_rect));
                    egui::ScrollArea::vertical()
                    .id_salt("build-options-scroll")
                    .auto_shrink([false, false])
                    .max_height(body_height)
                    .show(ui, |ui| {
                        band(ui, egui::Margin::same(24), 0.0, |ui| {
                            ui.spacing_mut().item_spacing.y = 9.0;
                            ui.add(egui::Label::new(RichText::new("Configure your compiled copy. Installed releases use their own launch settings.").size(13.0).color(theme::palette().muted)).wrap());
                            ui.add_space(11.0);
                            ui.label(RichText::new("Launch options").font(theme::bold(14.0)).color(theme::palette().text));
                            theme::text(ui, "Build to launch", 13.0, theme::palette().text_2);
                            let label = self.build_choices.iter().find(|(folder, _)| Some(folder) == self.build_choice.as_ref())
                                .map(|(_, info)| build_choice_label(info))
                                .unwrap_or_else(|| "Latest completed build (automatic)".into());
                            ui.scope(|ui| {
                                field_style(ui);
                                ui.spacing_mut().button_padding = egui::vec2(10.0, 9.0);
                                let previous = self.build_choice.clone();
                                ui.add_enabled_ui(!build_busy && !self.build_choices.is_empty(), |ui| {
                                    egui::ComboBox::from_id_salt(("build-to-launch", &app))
                                        .width(ui.available_width()).selected_text(label)
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(&mut self.build_choice, None, "Latest completed build (automatic)");
                                            for (folder, info) in &self.build_choices {
                                                ui.selectable_value(&mut self.build_choice, Some(folder.clone()), build_choice_label(info));
                                            }
                                        });
                                });
                                if previous != self.build_choice {
                                    self.delete_build_confirm = false;
                                    self.clean_build_confirm = false;
                                }
                            });
                            let chosen = self.build_choices.iter().find(|(folder, _)| Some(folder) == self.build_choice.as_ref())
                                .or_else(|| self.build_choices.first()).cloned();
                            theme::group().show(ui, |ui| {
                                band(ui, egui::Margin::same(12), 0.0, |ui| {
                                    if let Some((folder, info)) = &chosen {
                                        let description = if self.build_choice.is_none() {
                                            format!("Latest completed build · {}", info.commit.chars().take(7).collect::<String>())
                                        } else { build_choice_label(info) };
                                        theme::text(ui, &description, 14.0, theme::palette().text);
                                        ui.add(egui::Label::new(RichText::new(folder.display().to_string()).size(12.0).color(theme::palette().muted)).wrap());
                                    } else {
                                        theme::text(ui, "No completed builds yet", 13.0, theme::palette().muted);
                                    }
                                });
                            });
                            ui.add(egui::Label::new(RichText::new(if self.build_choice.is_none() {
                                "Launch automatically uses your newest completed build."
                            } else { "Launch uses this build until you choose another one." }).size(13.0).color(theme::palette().muted)).wrap());
                            theme::text(ui, "Arguments", 13.0, theme::palette().text_2);
                            ui.scope(|ui| {
                                field_style(ui);
                                ui.add(egui::TextEdit::multiline(&mut self.launch_arguments)
                                    .font(FontId::monospace(13.0)).margin(egui::Margin::same(10))
                                    .hint_text("--example-flag").desired_width(ui.available_width()).desired_rows(3));
                            });
                            ui.add(egui::Label::new(RichText::new("One argument per line. Passed directly to the app; no shell quotes needed.").size(12.0).color(theme::palette().muted)).wrap());
                            ui.add_space(11.0);
                            ui.label(RichText::new("Build files").font(theme::bold(14.0)).color(theme::palette().text));
                            let log = self.paths.at(format!("logs/{app}.log"));
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 10.0;
                                if btn("Open build folder").kind(Kind::Secondary).medium().icon(Icon::Folder).enabled(chosen.is_some()).show(ui).clicked() {
                                    if let Some((folder, _)) = &chosen { self.result(craft_apps_manager::platform::open(folder)); }
                                }
                                if action(ui, "View build log", Kind::Secondary, log.is_file()).clicked() {
                                    self.result(craft_apps_manager::platform::open(&log));
                                }
                            });
                            ui.add_space(11.0);
                            ui.label(RichText::new("Remove local build").font(theme::bold(14.0)).color(theme::palette().text));
                            ui.add(egui::Label::new(RichText::new("Delete the selected compiled build. Other builds and installed releases are kept.").size(13.0).color(theme::palette().muted)).wrap());
                            if self.delete_build_confirm {
                                ui.label("Delete this compiled build?");
                                ui.horizontal(|ui| {
                                    if action(ui, "Yes", Kind::Danger, !build_busy && chosen.is_some()).clicked() {
                                        if let Some((folder, _)) = &chosen {
                                            let result = craft_apps_manager::builder::delete_local(&self.paths, &app, folder);
                                            if result.is_ok() {
                                                self.build_choices.retain(|(path, _)| path != folder);
                                                if self.build_choice.as_ref() == Some(folder) { self.build_choice = None; }
                                                self.config_refresh_at = std::time::Instant::now();
                                                self.delete_build_confirm = false;
                                                if self.build_app == app {
                                                    self.build_job.state.lock().unwrap().output = craft_apps_manager::builder::history(&self.paths, &app);
                                                }
                                            }
                                            self.result(result);
                                        }
                                    }
                                    if action(ui, "No", Kind::Secondary, true).clicked() { self.delete_build_confirm = false; }
                                });
                            } else if action(ui, "Delete compiled build…", Kind::DangerOutline, !build_busy && chosen.is_some()).clicked() {
                                self.delete_build_confirm = true;
                                self.clean_build_confirm = false;
                            }
                            ui.add_space(3.0);
                            rule(ui);
                            ui.add_space(3.0);
                            theme::text(ui, "Build cache", 13.0, theme::palette().text_2);
                            ui.add(egui::Label::new(RichText::new("Clear temporary build files. Keeps completed builds and source ZIPs.").size(12.0).color(theme::palette().muted)).wrap());
                            if self.clean_build_confirm {
                                ui.label("Delete this app's build cache and temporary workspace?");
                                ui.horizontal(|ui| {
                                    if action(ui, "Yes", Kind::Primary, !build_busy).clicked() {
                                        {
                                            let paths = self.paths.clone();
                                            let app = app.clone();
                                            self.job = Job::interactive(self.paths.at("logs/updates.log"), &self.build_preferences);
                                            self.job.describe("Delete cache", &app);
                                            self.job_target = Some((app.clone(), "clean-build".into()));
                                            self.job_action = "clean-build".into();
                                            self.failure_dismissed = false;
                                            self.job.spawn(move |job| {
                                                job.stage("Cleaning build files", None, "Keeping compiled builds and downloaded sources");
                                                craft_apps_manager::builder::clean_app_cache(&paths, &app)
                                            });
                                        }
                                        self.clean_build_confirm = false;
                                        self.display_build_cache.remove(&app);
                                        self.build_app_cache = None;
                                        self.build_cache_receiver = None;
                                    }
                                    if action(ui, "No", Kind::Secondary, true).clicked() { self.clean_build_confirm = false; }
                                });
                            } else if action(ui, "Delete cache files…", Kind::Secondary, !build_busy && self.build_app_cache == Some(true))
                                .on_disabled_hover_text("No cache files to delete, or an operation is still running.").clicked() {
                                self.clean_build_confirm = true;
                                self.delete_build_confirm = false;
                            }
                        });
                    });
                });
                footer(ui, |ui| {
                    if action(ui, "Save", Kind::Primary, !build_busy).clicked() {
                        self.launch_draft.executable.clear();
                        self.launch_draft.arguments = self
                            .launch_arguments
                            .lines()
                            .filter(|line| !line.is_empty())
                            .map(String::from)
                            .collect();
                        let options = craft_apps_manager::builder::BuildLaunchOptions {
                            launch: self.launch_draft.clone(),
                            selected_build: self.build_choice.clone(),
                        };
                        let result = craft_apps_manager::builder::save_build_options(
                            &self.paths,
                            &app,
                            &options,
                        );
                        if result.is_ok() {
                            if let Ok(Some(folder)) =
                                craft_apps_manager::builder::selected_build(&self.paths, &app)
                            {
                                let info = craft_apps_manager::files::read_json(
                                    &folder.join("build-info.json"),
                                )
                                .ok();
                                self.display_builds.insert(app.clone(), (folder, info));
                            }
                            self.config_refresh_at = std::time::Instant::now();
                            self.build_options_open = false;
                        }
                        self.result(result);
                    }
                    if action(ui, "Cancel", Kind::Secondary, true).clicked() {
                        self.build_options_open = false;
                    }
                });
            });
            if response.should_close() {
                self.build_options_open = false;
            }
        }

        if let Some(app) = self.remove_source_confirm.clone() {
            let response = modal(ctx, "Remove downloaded source", 460.0, |ui| {
                band(ui, BODY, 0.0, |ui| {
                    header(ui, Art::Icon(Icon::Trash, theme::palette().red_bg, theme::palette().red, 40.0),
                        &format!("Remove {} source?", model::title(&app)),
                        "Deletes this app’s downloaded source ZIP. Compiled builds, extracted workspaces, installed apps, and backups are kept.", |_| {});
                });
                footer(ui, |ui| {
                    if action(ui, "No", Kind::Secondary, true).clicked() {
                        self.remove_source_confirm = None;
                    }
                    if action(
                        ui,
                        "Yes",
                        Kind::Danger,
                        !busy && !self.build_job.snapshot().busy,
                    )
                    .clicked()
                    {
                        self.app = app.clone();
                        self.start("remove-source");
                        self.remove_source_confirm = None;
                    }
                });
            });
            if response.should_close() {
                self.remove_source_confirm = None;
            }
        }
    }
}
