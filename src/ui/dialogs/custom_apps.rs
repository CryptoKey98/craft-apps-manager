use super::*;

pub(super) fn setup_chevron(
    ui: &egui::Ui,
    rect: egui::Rect,
    visuals: &egui::style::WidgetVisuals,
    _open: bool,
    _placement: egui::AboveOrBelow,
) {
    theme::paint_icon(
        ui.painter(),
        egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(14.0)),
        Icon::ChevronDown,
        visuals.fg_stroke.color,
    );
}

pub(super) fn paint_setup_file(painter: &egui::Painter, rect: egui::Rect, color: Color32) {
    let r = egui::Rect::from_center_size(rect.center(), egui::vec2(14.0, 18.0));
    let fold = 4.0;
    let points = vec![
        r.left_top(),
        egui::pos2(r.right() - fold, r.top()),
        egui::pos2(r.right(), r.top() + fold),
        r.right_bottom(),
        r.left_bottom(),
        r.left_top(),
    ];
    painter.add(egui::Shape::line(points, Stroke::new(1.3_f32, color)));
    painter.add(egui::Shape::line(
        vec![
            egui::pos2(r.right() - fold, r.top()),
            egui::pos2(r.right() - fold, r.top() + fold),
            egui::pos2(r.right(), r.top() + fold),
        ],
        Stroke::new(1.3_f32, color),
    ));
}
impl App {
    pub(super) fn custom_setup_dialog(&mut self, ctx: &egui::Context) {
        if self.custom_download_choice.is_some() {
            return;
        }
        let Some(mut setup) = self.custom_setup.take() else {
            return;
        };
        let mut add = false;
        let mut cancel = false;
        let response = modal(ctx, "Set up custom app", 600.0, |ui| {
            band(ui, egui::Margin::symmetric(24, 22), 0.0, |ui| {
                header(
                    ui,
                    Art::Icon(
                        Icon::Download,
                        theme::palette().accent_soft,
                        theme::palette().link,
                        32.0,
                    ),
                    "Set up custom app",
                    "Review the detected download and make changes if needed.",
                    |_| {},
                );
                ui.add_space(24.0);
                theme::heading(ui, &setup.entry.title, 17.0);
                note(ui, &setup.entry.repository);
                ui.add_space(18.0);
                theme::text(ui, "Download", 13.0, theme::palette().text_2);
                ui.add_space(6.0);
                egui::Frame::new()
                    .fill(theme::palette().field)
                    .stroke(Stroke::new(1.0_f32, theme::palette().border_strong))
                    .corner_radius(CornerRadius::same(8))
                    .show(ui, |ui| {
                        band(ui, egui::Margin::symmetric(12, 8), 0.0, |ui| {
                            let width = ui.available_width();
                            ui.allocate_ui_with_layout(
                                egui::vec2(width, 36.0),
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.spacing_mut().item_spacing.x = 12.0;
                                    if btn("Choose…").medium().show(ui).clicked() {
                                        setup.choosing = true;
                                    }
                                    let remaining = ui.available_width();
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(remaining, 36.0),
                                        egui::Layout::left_to_right(egui::Align::Center),
                                        |ui| {
                                            let (icon, _) = ui.allocate_exact_size(
                                                egui::vec2(18.0, 24.0),
                                                Sense::hover(),
                                            );
                                            paint_setup_file(
                                                ui.painter(),
                                                icon,
                                                theme::palette().text_3,
                                            );
                                            ui.add(
                                                egui::Label::new(if setup.selected.is_empty() {
                                                    "Choose a compatible download"
                                                } else {
                                                    &setup.selected
                                                })
                                                .truncate(),
                                            )
                                            .on_hover_text(&setup.selected);
                                        },
                                    );
                                },
                            );
                        });
                    });
                let candidates =
                    craft_apps_manager::downloads::candidates(&setup.release, &setup.preferences);
                let asset = candidates.iter().find(|asset| asset.name == setup.selected);
                ui.add_space(6.0);
                note(
                    ui,
                    match asset {
                        Some(asset) => format!(
                            "Version {} · {:.1} MB",
                            setup.release.tag_name,
                            asset.size as f64 / 1_000_000.0
                        ),
                        None => format!("Version {}", setup.release.tag_name),
                    },
                );
                ui.add_space(22.0);
                let old_format = setup.preferences.release_format.clone();
                let old_arch = setup.preferences.architecture.clone();
                ui.columns(2, |columns| {
                    for column in columns.iter_mut() {
                        field_style(column);
                        column.spacing_mut().interact_size.y = 40.0;
                        column.spacing_mut().button_padding = egui::vec2(12.0, 9.0);
                    }
                    theme::text(
                        &mut columns[0],
                        "Package type",
                        13.0,
                        theme::palette().text_2,
                    );
                    columns[0].add_space(6.0);
                    egui::ComboBox::from_id_salt("setup-package")
                        .icon(setup_chevron)
                        .width(columns[0].available_width())
                        .selected_text(if old_format == "installer" {
                            "Installer"
                        } else {
                            "Portable"
                        })
                        .show_ui(&mut columns[0], |ui| {
                            ui.selectable_value(
                                &mut setup.preferences.release_format,
                                "installer".into(),
                                "Installer",
                            );
                            ui.selectable_value(
                                &mut setup.preferences.release_format,
                                "portable".into(),
                                "Portable",
                            );
                        });
                    theme::text(
                        &mut columns[1],
                        "Architecture",
                        13.0,
                        theme::palette().text_2,
                    );
                    columns[1].add_space(6.0);
                    egui::ComboBox::from_id_salt("setup-architecture")
                        .icon(setup_chevron)
                        .width(columns[1].available_width())
                        .selected_text(if setup.architecture_confirmed {
                            model::architecture_label(&old_arch)
                        } else {
                            "Choose architecture…"
                        })
                        .show_ui(&mut columns[1], |ui| {
                            for architecture in ["x64", "x86", "arm64"] {
                                if ui
                                    .selectable_value(
                                        &mut setup.preferences.architecture,
                                        architecture.into(),
                                        model::architecture_label(architecture),
                                    )
                                    .clicked()
                                {
                                    setup.architecture_confirmed = true;
                                }
                            }
                        });
                    if !setup.architecture_confirmed {
                        columns[1].add_space(6.0);
                        theme::text(
                            &mut columns[1],
                            "Not specified by publisher",
                            12.0,
                            theme::palette().amber,
                        );
                    }
                });
                if old_format != setup.preferences.release_format
                    || old_arch != setup.preferences.architecture
                {
                    let still_compatible = craft_apps_manager::downloads::candidates(
                        &setup.release,
                        &setup.preferences,
                    )
                    .iter()
                    .any(|asset| asset.name == setup.selected);
                    if !still_compatible {
                        setup.selected.clear();
                        setup.choosing = true;
                    }
                }
                ui.add_space(22.0);
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), Sense::hover());
                    ui.painter().circle_stroke(rect.center(), 7.0, Stroke::new(1.3_f32, theme::palette().muted));
                    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "i", FontId::proportional(12.0), theme::palette().muted);
                    note(
                    ui,
                    if setup.preferences.release_format == "installer" {
                        if cfg!(target_os = "windows") {
                            "The app’s setup wizard handles installation.\nThe manager detects the installed app afterward."
                        } else if cfg!(target_os = "linux") {
                            "Your system package manager handles installation.\nThe manager detects the installed app afterward."
                        } else {
                            "The manager installs the application from its disk image."
                        }
                    } else {
                        "The manager extracts the portable app into your chosen folder."
                    },
                    );
                });
            });
            footer(ui, |ui| {
                let candidates =
                    craft_apps_manager::downloads::candidates(&setup.release, &setup.preferences);
                let ready = setup.architecture_confirmed
                    && candidates.iter().any(|asset| asset.name == setup.selected);
                add = action(ui, "Add app", Kind::Primary, ready).clicked();
                cancel = action(ui, "Cancel", Kind::Secondary, true).clicked();
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    note(ui, "Adding an app does not install it.");
                });
            });
        });
        if add {
            let candidates =
                craft_apps_manager::downloads::candidates(&setup.release, &setup.preferences);
            if let Some(asset) = candidates
                .into_iter()
                .find(|asset| asset.name == setup.selected)
            {
                let choice = craft_apps_manager::catalog::DownloadChoice {
                    entry: setup.entry.clone(),
                    assets: vec![asset.clone()],
                    version: updates::release_version(&setup.release.tag_name).unwrap_or_default(),
                    preferences: setup.preferences.clone(),
                };
                match craft_apps_manager::catalog::selected_custom_download(&choice, 0) {
                    Ok(entry) => {
                        self.settings_draft.app_install_options.insert(
                            entry.repository.to_ascii_lowercase(),
                            craft_apps_manager::install_options::InstallOptions {
                                release_format: setup.preferences.release_format.clone(),
                                architecture: setup.preferences.architecture.clone(),
                            },
                        );
                        self.custom_app_added(&entry);
                    }
                    Err(error) => {
                        self.custom_message = format!("{error:#}");
                        self.custom_failed = true;
                        self.custom_setup = Some(setup);
                    }
                }
            }
        } else if !cancel && !response.should_close() {
            if setup.choosing {
                setup.choosing = false;
                let assets =
                    craft_apps_manager::downloads::candidates(&setup.release, &setup.preferences);
                self.custom_download_index = assets
                    .iter()
                    .position(|asset| asset.name == setup.selected)
                    .unwrap_or(0);
                self.custom_download_confirmed = false;
                self.custom_download_choice =
                    Some(Box::new(craft_apps_manager::catalog::DownloadChoice {
                        entry: setup.entry.clone(),
                        assets: assets.into_iter().cloned().collect(),
                        version: updates::release_version(&setup.release.tag_name)
                            .unwrap_or_default(),
                        preferences: setup.preferences.clone(),
                    }));
            }
            self.custom_setup = Some(setup);
        } else {
            self.custom_message = "App setup cancelled.".into();
            self.custom_failed = false;
        }
    }

    pub(super) fn render_custom_choices(&mut self, ctx: &egui::Context) {
        if let Some(key) = self.remove_custom_confirm.clone() {
            if self.settings {
                let title = self
                    .custom_apps_draft
                    .iter()
                    .find(|entry| entry.key == key)
                    .map(|entry| entry.title.clone())
                    .unwrap_or_else(|| "custom app".into());
                let response = modal(ctx, "Remove custom app", 460.0, |ui| {
                    band(ui, BODY, 0.0, |ui| {
                        header(ui,
                            Art::Icon(Icon::Trash, theme::palette().accent_soft, theme::palette().accent, 40.0),
                            &format!("Remove {title} from the app list?"),
                            "This only removes the app from the manager’s list for this release format. It does not uninstall the application or delete its files or settings from your system.",
                            |_| {});
                        ui.add_space(12.0);
                        theme::text(
                            ui,
                            "Save Settings to apply this change.",
                            13.0,
                            theme::palette().muted,
                        );
                    });
                    footer(ui, |ui| {
                        if action(ui, "Cancel", Kind::Secondary, true).clicked() {
                            self.remove_custom_confirm = None;
                        }
                        if action(ui, "OK", Kind::Primary, true).clicked() {
                            self.remove_custom_app(&key);
                            self.remove_custom_confirm = None;
                        }
                    });
                });
                if response.should_close() {
                    self.remove_custom_confirm = None;
                }
            } else {
                self.remove_custom_confirm = None;
            }
        }

        if let Some(choice) = self.custom_download_choice.clone() {
            let mut confirm = false;

            let mut cancel = false;

            let response = modal(ctx, "Choose download", 560.0, |ui| {
                band(ui, BODY, 0.0, |ui| {
                    header(
                        ui,
                        Art::Icon(
                            Icon::Download,
                            theme::palette().accent_soft,
                            theme::palette().link,
                            48.0,
                        ),
                        &format!("Choose download for {}", choice.entry.title),
                        "Select the package you want the manager to use.",
                        |ui| {
                            note(
                                ui,
                                format!(
                                    "Version {} · {} · {}",
                                    choice.version,
                                    choice.preferences.architecture,
                                    choice.preferences.release_format
                                ),
                            );
                        },
                    );

                    ui.add_space(20.0);

                    theme::group()
                        .inner_margin(egui::Margin::symmetric(0, 8))
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .max_height(260.0)
                                .show(ui, |ui| {
                                    for (index, asset) in choice.assets.iter().enumerate() {
                                        if index > 0 {
                                            rule(ui);
                                        }

                                        band(ui, egui::Margin::symmetric(12, 10), 0.0, |ui| {
                                            ui.horizontal(|ui| {
                                                if ui
                                                    .radio_value(
                                                        &mut self.custom_download_index,
                                                        index,
                                                        "",
                                                    )
                                                    .changed()
                                                {
                                                    self.custom_download_confirmed = false;
                                                }

                                                ui.vertical(|ui| {
                                                    ui.add(
                                                        egui::Label::new(
                                                            RichText::new(&asset.name).size(14.0),
                                                        )
                                                        .truncate(),
                                                    )
                                                    .on_hover_text(&asset.name);

                                                    note(
                                                        ui,
                                                        format!(
                                                            "{} · {:.1} MB",
                                                            asset
                                                                .name
                                                                .rsplit('.')
                                                                .next()
                                                                .unwrap_or("Package")
                                                                .to_uppercase(),
                                                            asset.size as f64 / 1_000_000.0
                                                        ),
                                                    );
                                                });
                                            });
                                        });
                                    }
                                });
                        });

                    ui.add_space(12.0);

                    if choice
                        .assets
                        .get(self.custom_download_index)
                        .is_some_and(|asset| {
                            craft_apps_manager::downloads::architecture_unspecified(&asset.name)
                        })
                    {
                        note(ui, format!("Architecture is not labeled. Choose this file only if the publisher confirms it supports {}.", choice.preferences.architecture));
                    }
                    if choice.preferences.release_format == "installer" {
                        note(ui, "EXE downloads are checked for a supported installer before they are run. Standalone programs are rejected.");
                    }
                    if choice
                        .assets
                        .get(self.custom_download_index)
                        .is_some_and(|asset| {
                            craft_apps_manager::downloads::needs_confirmation(
                                asset,
                                &choice.preferences,
                            )
                        })
                    {
                        ui.add_space(8.0);
                        ui.checkbox(
                            &mut self.custom_download_confirmed,
                            format!(
                                "I confirmed this is a desktop {} package for {}.",
                                choice.preferences.release_format, choice.preferences.architecture
                            ),
                        );
                    }
                    note(ui, "Your choice is remembered for future updates. The package will be verified before installation.");
                });

                footer(ui, |ui| {
                    confirm = action(
                        ui,
                        "Choose",
                        Kind::Primary,
                        !self.job.snapshot().busy
                            && choice
                                .assets
                                .get(self.custom_download_index)
                                .is_some_and(|asset| {
                                    !craft_apps_manager::downloads::needs_confirmation(
                                        asset,
                                        &choice.preferences,
                                    ) || self.custom_download_confirmed
                                }),
                    )
                    .clicked();

                    cancel = action(ui, "Cancel", Kind::Secondary, true).clicked();
                });
            });

            if confirm && self.custom_setup.is_some() {
                if let (Some(setup), Some(asset)) = (
                    self.custom_setup.as_mut(),
                    choice.assets.get(self.custom_download_index),
                ) {
                    setup.selected = asset.name.clone();
                    setup.architecture_confirmed =
                        !craft_apps_manager::downloads::architecture_unspecified(&asset.name)
                            || self.custom_download_confirmed;
                }
                self.custom_download_choice = None;
            } else if confirm {
                let selected = if self.custom_resume_install.is_some()
                    || self.custom_resume_versions.is_some()
                {
                    craft_apps_manager::catalog::choose_custom_download(
                        &self.paths.root,
                        &choice,
                        self.custom_download_index,
                    )
                } else {
                    craft_apps_manager::catalog::selected_custom_download(
                        &choice,
                        self.custom_download_index,
                    )
                };
                match selected {
                    Ok(entry) => self.finish_custom_selection(&entry, ctx),

                    Err(error) => {
                        self.custom_message = format!("{error:#}");
                        self.custom_failed = true;
                    }
                }

                self.custom_download_choice = None;
            } else if cancel || response.should_close() {
                self.custom_download_choice = None;

                self.custom_message = if self.custom_resume_versions.take().is_some() {
                    self.versions_result = Some(Err(
                        "Download selection cancelled. Try again to choose a package.".into(),
                    ));
                    "Download selection cancelled.".into()
                } else if self.custom_resume_install.take().is_some() {
                    self.confirm_install = None;
                    self.additional_installation = false;
                    self.install_draft_app = None;
                    "Download selection cancelled.".into()
                } else {
                    "App was not added.".into()
                };

                self.custom_failed = false;
            }
        }
    }
}
