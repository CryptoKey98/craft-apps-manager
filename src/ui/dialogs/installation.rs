use super::*;

impl App {
    pub(super) fn install_options_controls(
        &mut self,
        ui: &mut egui::Ui,
        app: &str,
        plan: Option<&updates::ReleasePlan>,
        ctx: &egui::Context,
    ) {
        use craft_apps_manager::install_options::{self, InstallOptions};
        let pending = self.install_receiver.is_some()
            || self.custom_receiver.is_some()
            || self.job.snapshot().busy;

        let tag = plan.and_then(|plan| plan.requested_tag.clone());
        let endpoint = tag
            .as_ref()
            .map(|tag| {
                format!(
                    "https://api.github.com/repos/{}/releases/tags/{tag}",
                    model::repository(app)
                )
            })
            .unwrap_or_else(|| {
                format!(
                    "https://api.github.com/repos/{}/releases/latest",
                    model::repository(app)
                )
            });
        let Some(release) = craft_apps_manager::network::Network::cached_json::<model::Release>(
            &self.paths.root,
            &endpoint,
        ) else {
            ui.columns(2, |columns| {
                for (column, title) in columns.iter_mut().zip(["Release format", "Architecture"]) {
                    theme::text(column, title, 12.0, theme::palette().muted);
                    column.add_space(6.0);
                    column.add_enabled(
                        false,
                        egui::Button::new("Loading…")
                            .min_size(egui::vec2(column.available_width(), 32.0)),
                    );
                }
            });
            return;
        };
        let available = install_options::available(&release, app, &self.preferences);
        // Version changes must keep the verified installation configuration.
        // Preferences alone do not prove how an existing copy was installed.
        let version_change_locked = !self.additional_installation
            && tag.is_some()
            && self.display_config.as_ref().is_some_and(|config| {
                config
                    .apps
                    .iter()
                    .any(|copy| copy.name == app && !copy.path.is_empty())
            });
        let prefs = self.app_preferences(app);
        let original = InstallOptions {
            release_format: prefs.release_format,
            architecture: prefs.architecture,
        };
        let selection = if version_change_locked {
            self.display_config
                .as_ref()
                .and_then(|config| {
                    config
                        .apps
                        .iter()
                        .find(|copy| copy.name == app && !copy.path.is_empty())
                })
                .and_then(InstallOptions::installed)
        } else {
            install_options::preserve_or_default(&available, &original)
        };
        let Some(mut selected) = selection else {
            note(ui, "No confirmed compatible package. Choose a download to confirm incomplete release metadata.");
            return;
        };
        let mut changed = selected != original;
        field_style(ui);
        ui.add_enabled_ui(!pending && !version_change_locked, |ui| {
            ui.columns(2, |columns| {
                theme::text(
                    &mut columns[0],
                    "Release format",
                    12.0,
                    theme::palette().muted,
                );
                columns[0].add_space(6.0);
                egui::ComboBox::from_id_salt(("app-format", app))
                    .width(columns[0].available_width())
                    .selected_text(crate::ui::screens::overview::release_format_label(
                        &selected.release_format,
                    ))
                    .show_ui(&mut columns[0], |ui| {
                        for format in ["installer", "portable"] {
                            if available
                                .iter()
                                .any(|option| option.release_format == format)
                                && ui
                                    .selectable_value(
                                        &mut selected.release_format,
                                        format.into(),
                                        crate::ui::screens::overview::release_format_label(format),
                                    )
                                    .changed()
                            {
                                changed = true;
                            }
                        }
                    });
                if !version_change_locked && !available.contains(&selected) {
                    if let Some(option) = available
                        .iter()
                        .find(|option| {
                            option.release_format == selected.release_format
                                && option.architecture == original.architecture
                        })
                        .or_else(|| {
                            available
                                .iter()
                                .find(|option| option.release_format == selected.release_format)
                        })
                    {
                        selected = option.clone();
                        changed = true;
                    }
                }
                theme::text(
                    &mut columns[1],
                    "Architecture",
                    12.0,
                    theme::palette().muted,
                );
                columns[1].add_space(6.0);
                egui::ComboBox::from_id_salt(("app-architecture", app))
                    .width(columns[1].available_width())
                    .selected_text(model::architecture_label(&selected.architecture))
                    .show_ui(&mut columns[1], |ui| {
                        for option in available
                            .iter()
                            .filter(|option| option.release_format == selected.release_format)
                        {
                            if ui
                                .selectable_value(
                                    &mut selected.architecture,
                                    option.architecture.clone(),
                                    model::architecture_label(&option.architecture),
                                )
                                .changed()
                            {
                                changed = true;
                            }
                        }
                    });
            });
        });
        if version_change_locked {
            ui.add_space(8.0);
            note(
                ui,
                "Uninstall the current installation before changing its format or architecture.",
            );
        }
        if changed && !pending && !version_change_locked {
            let format_changed = selected.release_format != original.release_format;
            match self.save_install_options(app, selected) {
                Ok(()) => {
                    if format_changed {
                        // Reinitialize the destination for the new package type.
                        // Installer defaults (such as Program Files) must not leak
                        // into portable installations.
                        self.install_draft_app = None;
                    }
                    self.install_error = None;
                    self.install_receiver = None;
                    if let Some(tag) = tag.filter(|_| !self.additional_installation) {
                        let previous = self.install_result.clone();
                        self.review_version(app.into(), tag, ctx);
                        self.install_result = previous;
                    } else {
                        self.load_install(app, ctx);
                    }
                }
                Err(error) => self.install_error = Some(format!("{error:#}")),
            }
        }
    }

    pub(super) fn install_version_control(
        &mut self,
        ui: &mut egui::Ui,
        app: &str,
        plan: Option<&updates::ReleasePlan>,
        ctx: &egui::Context,
    ) {
        if self.versions_app.as_deref() != Some(app) {
            self.versions_app = Some(app.into());
            self.versions_result = None;
            let paths = self.paths.clone();
            let app = app.to_string();
            let ctx = ctx.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            self.versions_receiver = Some(rx);
            std::thread::spawn(move || {
                let result =
                    updates::available_versions(&paths, &app).map_err(|error| format!("{error:#}"));
                let _ = tx.send(result);
                ctx.request_repaint();
            });
        }
        theme::text(ui, "Version", 12.0, theme::palette().muted);
        ui.add_space(6.0);
        field_style(ui);
        let current = self
            .install_selected_tag
            .clone()
            .or_else(|| plan.and_then(|plan| plan.requested_tag.clone()));
        let latest_label = plan
            .and_then(|plan| plan.entries.first())
            .filter(|_| current.is_none())
            .map(|entry| format!("{} · Latest release", entry.version))
            .unwrap_or_else(|| "Latest release".into());
        let mut selected = current.clone();
        ui.add_enabled_ui(
            self.install_receiver.is_none() && self.custom_receiver.is_none(),
            |ui| {
                egui::ComboBox::from_id_salt(("install-version", app))
                    .width(ui.available_width())
                    .selected_text(current.as_deref().unwrap_or(&latest_label))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut selected, None, "Latest release");
                        match &self.versions_result {
                            Some(Ok(releases)) => {
                                for release in releases {
                                    ui.selectable_value(
                                        &mut selected,
                                        Some(release.tag_name.clone()),
                                        &release.tag_name,
                                    );
                                }
                            }
                            Some(Err(error)) => {
                                ui.label(error);
                            }
                            None => {
                                ui.label("Loading versions…");
                            }
                        }
                    });
            },
        );
        if selected != current {
            self.install_selected_tag = selected;
            self.load_install(app, ctx);
        }
    }

    pub(super) fn render_install_dialog(&mut self, ctx: &egui::Context) {
        if let Some(app) = self
            .confirm_install
            .clone()
            .filter(|_| self.custom_download_choice.is_none())
        {
            let install_preferences = self.app_preferences(&app);
            let installer = install_preferences.release_format == "installer";
            let current = self
                .display_config
                .as_ref()
                .and_then(|c| c.apps.iter().find(|a| a.name == app))
                .filter(|a| model::installed_architecture_matches(a, &install_preferences))
                .filter(|_| !self.additional_installation)
                .cloned();
            let installed_portable = !installer
                && current
                    .as_ref()
                    .is_some_and(|a| !a.path.is_empty() && a.install_kind != "installer");
            let new_draft = self.install_draft_app.as_deref() != Some(&app);
            if new_draft {
                self.install_draft_app = Some(app.clone());
                self.install_shortcut = true;
                self.load_install(&app, ctx);
            }
            let location_context = (
                app.clone(),
                install_preferences.release_format.clone(),
                install_preferences.architecture.clone(),
                self.additional_installation,
            );
            if new_draft || self.install_location_context.as_ref() != Some(&location_context) {
                self.install_destination_preview = DestinationPreview::default();
                self.install_parent = default_install_parent(
                    &self.paths,
                    current.as_ref(),
                    &install_preferences,
                    self.additional_installation,
                );
                self.install_location_context = Some(location_context);
                // A folder-picker result for the previous package type cannot
                // replace this package type's destination.
                self.folder_receiver = None;
            }
            let plan = self
                .install_result
                .as_ref()
                .and_then(|r| r.as_ref().ok())
                .filter(|plan| plan.entries.iter().all(|entry| entry.app == app))
                .cloned();
            let entry = plan.as_ref().and_then(|p| p.entries.first());
            let update = entry.is_some_and(|e| e.action == "Update");
            let downgrade = entry.zip(current.as_ref()).is_some_and(|(entry, current)| {
                entry.action == "Update"
                    && updates::version(&current.version)
                        .ok()
                        .zip(updates::version(&entry.version).ok())
                        .is_some_and(|(installed, selected)| selected < installed)
            });
            let install_action = if downgrade {
                "Downgrade"
            } else if update {
                "Update"
            } else {
                "Install"
            };
            #[cfg(target_os = "windows")]
            let custom_installer = installer
                && craft_apps_manager::installers::custom_location_supported(&app)
                && !current
                    .as_ref()
                    .is_some_and(|a| a.install_kind == "installer" && !a.path.is_empty());
            #[cfg(not(target_os = "windows"))]
            let custom_installer = false;
            let title = format!("{} {}", install_action, model::title(&app));
            let details = entry
                .filter(|e| !e.version.is_empty())
                .map(|e| {
                    format!(
                        "Version {} · {} · {}",
                        e.version,
                        if installer {
                            craft_apps_manager::installers::installer_label()
                        } else {
                            "Portable"
                        },
                        model::architecture_label(&install_preferences.architecture)
                    )
                })
                .unwrap_or_else(|| "Loading verified release details…".into());
            let native_location = current
                .as_ref()
                .filter(|a| a.install_kind == "installer" && !a.path.is_empty())
                .map(|a| a.path.clone())
                .unwrap_or_else(|| {
                    if cfg!(target_os = "linux") {
                        "System-managed location".into()
                    } else if cfg!(target_os = "macos") {
                        "/Applications (or ~/Applications)".into()
                    } else {
                        "Set by the installer".into()
                    }
                });
            let art = self
                .icons
                .get(&app)
                .map(|t| Art::Texture(t.id()))
                .unwrap_or(Art::None);
            let response = modal(ctx, "Install app", 520.0, |ui| {
                band(ui, BODY, 0.0, |ui| {
                    // Fade newly resolved content without hiding the controls or
                    // introducing another release request. The footer stays stable.
                    let transition_id = egui::Id::new(("install-content-transition", &app));
                    let signature = format!(
                        "{}|{}|{}|{:?}|{:?}",
                        install_preferences.release_format,
                        install_preferences.architecture,
                        details,
                        entry
                            .and_then(|entry| entry.asset.as_ref())
                            .map(|asset| (&asset.name, asset.size)),
                        entry.and_then(|entry| entry.error.as_ref())
                    );
                    let now = ui.input(|input| input.time);
                    let started = ui.ctx().data_mut(|data| {
                        let previous = data.get_temp::<(String, f64)>(transition_id);
                        let started = match previous {
                            Some((previous, started)) if previous == signature => started,
                            Some(_) => now,
                            None => now - 1.0,
                        };
                        data.insert_temp(transition_id, (signature, started));
                        started
                    });
                    let progress = ((now - started) / 0.16).clamp(0.0, 1.0) as f32;
                    let eased = progress * progress * (3.0 - 2.0 * progress);
                    ui.set_opacity(0.35 + 0.65 * eased);
                    if progress < 1.0 {
                        ctx.request_repaint();
                    }
                    header(ui, art, &title, "", |ui| {
                        install_detail(
                            ui,
                            egui::Id::new(("install-details", &app)),
                            &details,
                            14.0,
                            theme::palette().text_2,
                        );
                    });
                    ui.add_space(8.0);
                    let download_size = entry
                        .and_then(|e| e.asset.as_ref())
                        .map(|a| format!("Download size: {:.1} MB", a.size as f64 / 1_000_000.0))
                        .unwrap_or_else(|| "Download size: checking…".into());
                    install_detail(
                        ui,
                        egui::Id::new(("install-size", &app)),
                        &download_size,
                        12.0,
                        theme::palette().muted,
                    );
                    ui.add_space(18.0);
                    self.install_version_control(ui, &app, plan.as_ref(), ctx);
                    ui.add_space(18.0);
                    self.install_options_controls(ui, &app, plan.as_ref(), ctx);
                    if self.additional_installation {
                        ui.add_space(8.0);
                        note(ui, "This adds a separate copy. Some apps share user settings between installations. Only one native installer copy is supported.");
                    }
                    ui.add_space(18.0);
                    field_style(ui);
                    if installer && !custom_installer {
                        ui.horizontal(|ui| {
                            theme::text(ui, "App folder", 12.0, theme::palette().muted);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if open_folder_button(ui).clicked() {
                                        let folder = current
                                            .as_ref()
                                            .filter(|a| !a.path.is_empty())
                                            .map(|a| std::path::PathBuf::from(&a.path))
                                            .unwrap_or_else(|| {
                                                if cfg!(target_os = "linux") {
                                                    std::path::PathBuf::from("/usr")
                                                } else if cfg!(target_os = "macos") {
                                                    std::path::PathBuf::from("/Applications")
                                                } else {
                                                    std::path::PathBuf::from(
                                                        std::env::var_os("ProgramFiles")
                                                            .unwrap_or_default(),
                                                    )
                                                }
                                            });
                                        self.result(craft_apps_manager::platform::open(&folder));
                                    }
                                },
                            );
                        });
                        ui.add_space(6.0);
                        ui.add(egui::Label::new(compact_path(&native_location)).truncate())
                            .on_hover_text(&native_location);
                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(16.0);
                        desktop_shortcut_option(ui, &mut self.install_shortcut);
                        ui.add_space(10.0);
                        note(
                            ui,
                            if cfg!(target_os = "linux") {
                                "The package defines its install paths. Administrator approval is required."
                            } else if cfg!(target_os = "windows") {
                                "Windows may ask for administrator approval. Updates keep the existing app folder."
                            } else {
                                "The manager installs the app in an Applications folder."
                            },
                        );
                    } else {
                        theme::text(ui, "Install location", 13.0, theme::palette().text_3);
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            let width = (ui.available_width() - 98.0).max(80.0);
                            let response = ui.add_enabled(
                                !installed_portable,
                                egui::TextEdit::singleline(&mut self.install_parent)
                                    .desired_width(width)
                                    .margin(egui::Margin::symmetric(10, 8)),
                            );
                            if response.changed() {
                                self.install_error = None;
                            }
                            if action(
                                ui,
                                "Browse…",
                                Kind::Secondary,
                                !installed_portable && self.folder_receiver.is_none(),
                            )
                            .clicked()
                            {
                                let initial = std::path::PathBuf::from(&self.install_parent);
                                let ctx = ctx.clone();
                                let (tx, rx) = std::sync::mpsc::channel();
                                self.folder_receiver = Some(rx);
                                std::thread::spawn(move || {
                                    let result =
                                        craft_apps_manager::portable::pick_folder(&initial)
                                            .map_err(|e| format!("{e:#}"));
                                    let _ = tx.send(result);
                                    ctx.request_repaint();
                                });
                            }
                        });
                        ui.add_space(16.0);
                        ui.separator();
                        ui.add_space(8.0);
                        let target = if installed_portable {
                            Ok(std::path::PathBuf::from(&current.as_ref().unwrap().path))
                        } else if self.additional_installation && !installer {
                            self.install_destination_preview.get(
                                &self.paths,
                                &self.install_parent,
                                &app,
                                || {
                                    craft_apps_manager::portable::additional_destination(
                                        &self.paths,
                                        std::path::Path::new(&self.install_parent),
                                        &app,
                                    )
                                },
                            )
                        } else {
                            craft_apps_manager::portable::destination(
                                std::path::Path::new(&self.install_parent),
                                &app,
                            )
                        };
                        ui.horizontal(|ui| {
                            theme::text(ui, "App folder", 12.0, theme::palette().muted);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if open_folder_button(ui).clicked() {
                                        let mut folder =
                                            std::path::PathBuf::from(&self.install_parent);
                                        if let Ok(target) = &target {
                                            if target.is_dir() {
                                                folder = target.clone();
                                            }
                                        }
                                        while !folder.is_dir() && folder.pop() {}
                                        if folder.is_dir() {
                                            self.result(craft_apps_manager::platform::open(
                                                &folder,
                                            ));
                                        }
                                    }
                                },
                            );
                        });
                        ui.add_space(6.0);
                        if let Ok(target) = &target {
                            let full_path = target.display().to_string();
                            ui.add(egui::Label::new(compact_path(&full_path)).truncate())
                                .on_hover_text(full_path);
                        } else {
                            note(ui, "Enter an absolute folder path.");
                        }
                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(16.0);
                        desktop_shortcut_option(ui, &mut self.install_shortcut);
                        ui.add_space(10.0);
                        note(
                            ui,
                            if custom_installer {
                                "Windows may ask for administrator approval. Updates keep the selected app folder."
                            } else if installed_portable {
                                "Updates keep the existing app folder. Uninstall first to change its location."
                            } else {
                                "The app gets its own folder. Other files in this location stay untouched."
                            },
                        );
                    }
                    let error = self
                        .install_result
                        .as_ref()
                        .and_then(|r| r.as_ref().err())
                        .cloned()
                        .or_else(|| entry.and_then(|e| e.error.clone()))
                        .or_else(|| self.install_error.clone());
                    if let Some(error) = error {
                        ui.add_space(14.0);
                        ui.add(
                            egui::Label::new(RichText::new(error).color(theme::palette().red))
                                .wrap(),
                        );
                        if plan.is_none()
                            && action(
                                ui,
                                "Retry",
                                Kind::Secondary,
                                self.install_receiver.is_none(),
                            )
                            .clicked()
                        {
                            self.load_install(&app, ctx);
                        }
                    }
                    if entry.is_some_and(|e| e.action == "Skip" && e.error.is_none()) {
                        ui.add_space(12.0);
                        note(ui, "This version is already installed.");
                    }
                });
                footer(ui, |ui| {
                    let ready = entry.is_some_and(|e| {
                        e.error.is_none()
                            && matches!(e.action.as_str(), "Install" | "Update")
                            && e.preferences.release_format == install_preferences.release_format
                            && e.preferences.architecture == install_preferences.architecture
                    }) && self.install_receiver.is_none()
                        && self.custom_receiver.is_none()
                        && self.install_result.is_some()
                        && self.folder_receiver.is_none()
                        && !self.job.snapshot().busy;
                    if action(ui, install_action, Kind::Primary, ready).clicked() {
                        if let Some(mut plan) = plan.clone() {
                            plan.desktop_shortcut = self.install_shortcut;
                            let result = if custom_installer {
                                #[cfg(target_os = "windows")]
                                {
                                    updates::choose_installer_destination(
                                        &self.paths,
                                        &mut plan,
                                        std::path::Path::new(&self.install_parent),
                                    )
                                }
                                #[cfg(not(target_os = "windows"))]
                                {
                                    Ok(())
                                }
                            } else if installer {
                                Ok(())
                            } else {
                                updates::choose_destination(
                                    &self.paths,
                                    &mut plan,
                                    std::path::Path::new(&self.install_parent),
                                    self.install_shortcut,
                                )
                            };
                            match result {
                                Ok(()) => self.run_install_plan(app.clone(), plan),
                                Err(e) => self.install_error = Some(format!("{e:#}")),
                            }
                        }
                    }
                    if action(ui, "Cancel", Kind::Secondary, true).clicked() {
                        self.confirm_install = None;
                    }
                });
            });
            if response.should_close() {
                self.confirm_install = None;
            }
            if self.confirm_install.is_none() {
                self.additional_installation = false;
                self.install_draft_app = None;
                self.install_location_context = None;
                self.install_destination_preview = DestinationPreview::default();
                self.install_receiver = None;
                self.folder_receiver = None;
                self.install_result = None;
            }
        }
    }
}
