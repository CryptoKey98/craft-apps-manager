use super::*;

pub(in crate::ui) fn release_version_row(
    ui: &mut egui::Ui,
    label: &str,
    status: &str,
    selected: bool,
    disabled: bool,
    explain: bool,
) -> (egui::Response, Option<egui::Response>) {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 40.0),
        if disabled {
            Sense::hover()
        } else {
            Sense::click()
        },
    );
    if selected || (response.hovered() && !disabled) {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(5),
            if selected {
                theme::palette().accent_soft
            } else {
                theme::palette().button_hover
            },
        );
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, !disabled, label));
    let color = if disabled {
        theme::palette().muted
    } else {
        theme::palette().text
    };
    let status_right = rect.right() - if explain { 26.0 } else { 10.0 };
    let status_galley = ui.painter().layout_no_wrap(
        status.into(),
        FontId::proportional(12.0),
        if disabled {
            theme::palette().muted
        } else {
            theme::palette().link
        },
    );
    let label_clip = egui::Rect::from_min_max(
        rect.min,
        egui::pos2(
            (status_right - status_galley.size().x - 10.0).max(rect.left()),
            rect.bottom(),
        ),
    );
    ui.painter().with_clip_rect(label_clip).text(
        egui::pos2(rect.left() + 10.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        FontId::proportional(14.0),
        color,
    );
    ui.painter().galley(
        egui::pos2(
            status_right - status_galley.size().x,
            rect.center().y - status_galley.size().y / 2.0,
        ),
        status_galley,
        theme::palette().muted,
    );
    let information = explain.then(|| {
        let target = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 17.0, rect.center().y),
            egui::vec2(24.0, 28.0),
        );
        let info = ui.interact(
            target,
            response.id.with("compatibility-info"),
            Sense::click(),
        );
        info.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                true,
                format!("Why {label} is incompatible"),
            )
        });
        if info.hovered() || info.has_focus() {
            ui.painter()
                .rect_filled(target, CornerRadius::same(5), theme::palette().button_hover);
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if info.has_focus() {
            ui.painter().rect_stroke(
                target,
                CornerRadius::same(5),
                Stroke::new(1.0_f32, theme::palette().link),
                StrokeKind::Inside,
            );
        }
        theme::paint_icon(
            ui.painter(),
            egui::Rect::from_center_size(target.center(), egui::Vec2::splat(10.0)),
            Icon::Info,
            if info.hovered() {
                theme::palette().text
            } else {
                theme::palette().muted
            },
        );
        info
    });
    if response.hovered() && !disabled {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    (response, information)
}

fn install_version_row(
    ui: &mut egui::Ui,
    selected: &mut Option<String>,
    tag: &str,
    reason: Option<&str>,
    unknown: bool,
) -> (egui::Response, Option<egui::Response>) {
    let (response, information) = release_version_row(
        ui,
        tag,
        if reason.is_some() {
            "Incompatible"
        } else if unknown {
            "Unknown"
        } else {
            ""
        },
        selected.as_deref() == Some(tag),
        reason.is_some(),
        reason.is_some(),
    );
    if response.clicked() {
        *selected = Some(tag.into());
    }
    (response, information)
}

fn install_version_menu(
    ui: &mut egui::Ui,
    id: egui::Id,
    selected: &str,
    content: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    let popup_id = id.with("popup");
    let padding = ui.spacing().button_padding;
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(selected.into(), font, ui.visuals().text_color());
    let height = (galley.size().y.max(ui.spacing().icon_width) + padding.y * 2.0)
        .max(ui.spacing().interact_size.y);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, ui.is_enabled(), selected)
    });
    if response.clicked() {
        ui.memory_mut(|memory| memory.toggle_popup(popup_id));
    }
    let open = ui.memory(|memory| memory.is_popup_open(popup_id));
    let screen = ui.ctx().screen_rect();
    let global_rect = ui
        .ctx()
        .layer_transform_to_global(ui.layer_id())
        .map_or(rect, |transform| transform * rect);
    let below = (screen.bottom() - global_rect.bottom()).max(0.0);
    let above = (global_rect.top() - screen.top()).max(0.0);
    let margin = egui::Frame::popup(ui.style()).total_margin().sum().y;
    let show_above = below < ui.spacing().combo_height + margin && above > below;
    let viewport_height = ui
        .spacing()
        .combo_height
        .min((if show_above { above } else { below } - margin).max(0.0));
    let direction = if show_above {
        egui::AboveOrBelow::Above
    } else {
        egui::AboveOrBelow::Below
    };
    if ui.is_rect_visible(rect) {
        let visuals = if open {
            &ui.visuals().widgets.open
        } else {
            ui.style().interact(&response)
        };
        ui.painter().rect(
            rect,
            visuals.corner_radius,
            visuals.weak_bg_fill,
            visuals.bg_stroke,
            StrokeKind::Inside,
        );
        let inner = rect.shrink2(padding);
        let icon = egui::Rect::from_center_size(
            egui::pos2(
                inner.right() - ui.spacing().icon_width / 2.0,
                inner.center().y,
            ),
            egui::vec2(
                ui.spacing().icon_width * 0.7,
                ui.spacing().icon_width * 0.45,
            ),
        );
        let points = if show_above {
            vec![icon.left_bottom(), icon.right_bottom(), icon.center_top()]
        } else {
            vec![icon.left_top(), icon.right_top(), icon.center_bottom()]
        };
        ui.painter().add(egui::Shape::convex_polygon(
            points,
            visuals.fg_stroke.color,
            Stroke::NONE,
        ));
        let text_clip = egui::Rect::from_min_max(
            inner.min,
            egui::pos2(icon.left() - ui.spacing().icon_spacing, inner.bottom()),
        );
        ui.painter()
            .with_clip_rect(text_clip.intersect(ui.clip_rect()))
            .galley(
                egui::pos2(inner.left(), inner.center().y - galley.size().y / 2.0),
                galley,
                visuals.text_color(),
            );
    }
    egui::popup::popup_above_or_below_widget(
        ui,
        popup_id,
        &response,
        direction,
        egui::popup::PopupCloseBehavior::CloseOnClick,
        |ui| {
            // egui 0.31 reuses the popup Area's last size as its next max_rect.
            // Expand the viewport before creating the ScrollArea, so the initial
            // Latest/Loading rows cannot permanently cap the arriving history.
            ui.set_max_height(viewport_height);
            egui::ScrollArea::vertical()
                .max_height(viewport_height)
                .min_scrolled_height(0.0)
                .show(ui, |ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    content(ui);
                });
        },
    );
    response
}

fn native_install_location(current: Option<&model::Installed>) -> String {
    current
        .filter(|app| app.install_kind == "installer" && !app.path.is_empty())
        .map(|app| app.path.clone())
        .unwrap_or_else(|| {
            if cfg!(target_os = "linux") {
                "System-managed location".into()
            } else if cfg!(target_os = "macos") {
                "/Applications (or ~/Applications)".into()
            } else {
                // Package metadata may not be available during preview. Do not
                // promise a destination/scope chooser that older or custom
                // installers may never provide.
                "Set by installer".into()
            }
        })
}

fn managed_location_note(ui: &mut egui::Ui, id: egui::Id, visible: bool, setup: bool) {
    let progress = ui.ctx().animate_value_with_time(
        id,
        f32::from(visible),
        if ui.style().animation_time > 0.0 {
            0.28
        } else {
            0.0
        },
    );
    if progress <= 0.0 {
        return;
    }
    // Text is transparent while its space expands/collapses, preventing clipping.
    let height_fraction = (progress / 0.55).clamp(0.0, 1.0);
    let opacity = ((progress - 0.55) / 0.45).clamp(0.0, 1.0);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id)
            .max_rect(ui.available_rect_before_wrap()),
    );
    child.multiply_opacity(opacity);
    child.add_space(6.0);
    note(
        &mut child,
        if setup {
            "Location and permissions are handled by the installer."
        } else {
            "Managed by the system installer. This location can’t be changed."
        },
    );
    ui.allocate_space(egui::vec2(
        child.min_rect().width(),
        child.min_rect().height() * height_fraction,
    ));
}

fn release_generation_changed(
    ctx: &egui::Context,
    id: egui::Id,
    generation: Option<String>,
) -> bool {
    let Some(generation) = generation else {
        return false;
    };
    ctx.data_mut(|data| {
        let previous = data.get_temp::<String>(id);
        data.insert_temp(id, generation.clone());
        previous.is_some_and(|previous| previous != generation)
    })
}

fn install_plan_matches_selection(
    plan: &updates::ReleasePlan,
    app: &str,
    selected_tag: Option<&str>,
    preferences: &model::Preferences,
    additional: bool,
) -> bool {
    plan.requested_tag.as_deref() == selected_tag
        && plan.additional_installation == additional
        && plan.entries.len() == 1
        && plan.entries.first().is_some_and(|entry| {
            entry.app == app
                && entry.preferences.release_format == preferences.release_format
                && entry.preferences.architecture == preferences.architecture
        })
}

type InstallLocationContext = (String, String, String, bool);

fn initialize_install_location(
    context: &mut Option<InstallLocationContext>,
    parent: &mut String,
    next: InstallLocationContext,
    paths: &model::Paths,
    current: Option<&model::Installed>,
    preferences: &model::Preferences,
    new_draft: bool,
) -> bool {
    if !new_draft && context.as_ref() == Some(&next) {
        return false;
    }
    *parent = default_install_parent(paths, current, preferences, next.3);
    *context = Some(next);
    true
}

impl App {
    pub(super) fn install_options_controls(
        &mut self,
        ui: &mut egui::Ui,
        app: &str,
        plan: Option<&updates::ReleasePlan>,
        ctx: &egui::Context,
    ) {
        use craft_apps_manager::install_options::{self, InstallOptions};
        // Preparing a preview does not block choosing another package. Replacing
        // its receiver discards late results; execution still requires a matching plan.
        let pending = self.custom_receiver.is_some() || self.job.snapshot().busy;

        // The selected tag is authoritative even while the previous plan remains
        // visible. None explicitly means Latest, including after a version switch.
        let tag = self.install_selected_tag.clone();
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
        if release_generation_changed(
            ctx,
            egui::Id::new((
                "install-release-generation",
                &self.paths.root,
                app,
                &endpoint,
            )),
            craft_apps_manager::installations::release_catalog::provider::generation(
                &self.paths.root,
                &model::repository(app),
            )
            .or_else(|| {
                craft_apps_manager::network::Network::cached_generation(&self.paths.root, &endpoint)
            })
            .or_else(|| {
                tag.as_ref()?;
                let history = format!(
                    "https://api.github.com/repos/{}/releases?per_page=100",
                    model::repository(app)
                );
                craft_apps_manager::network::Network::cached_generation(&self.paths.root, &history)
            }),
        ) && !self.job.snapshot().busy
            && self.custom_receiver.is_none()
        {
            self.load_install(app, ctx);
        }
        // Updates and downgrades must keep the verified installation configuration.
        // Preferences alone do not prove how an existing copy was installed.
        let version_change_locked = !self.additional_installation
            && self.display_config.as_ref().is_some_and(|config| {
                config
                    .apps
                    .iter()
                    .any(|copy| copy.name == app && !copy.path.is_empty())
            });
        field_style(ui);
        let repaint = ctx.clone();
        let Some(release) =
            craft_apps_manager::installations::release_catalog::provider::peek_release(
                &self.paths.root,
                &model::repository(app),
                tag.as_deref(),
            )
            .or_else(|| {
                craft_apps_manager::network::Network::peek_json::<model::Release>(
                    &self.paths.root,
                    &endpoint,
                    move || repaint.request_repaint(),
                )
            })
            .or_else(|| {
                let selected = tag.as_deref()?;
                let history = format!(
                    "https://api.github.com/repos/{}/releases?per_page=100",
                    model::repository(app)
                );
                let repaint = ctx.clone();
                craft_apps_manager::network::Network::peek_json::<Vec<model::Release>>(
                    &self.paths.root,
                    &history,
                    move || repaint.request_repaint(),
                )?
                .iter()
                .find(|release| release.tag_name == selected)
                .cloned()
                .map(std::sync::Arc::new)
            })
        else {
            let installed = self
                .display_config
                .as_ref()
                .and_then(|config| {
                    config
                        .apps
                        .iter()
                        .find(|copy| copy.name == app && !copy.path.is_empty())
                })
                .filter(|_| version_change_locked)
                .and_then(InstallOptions::installed)
                .or_else(|| {
                    // Retain the chosen labels while the new release is being
                    // fetched; their disabled state makes availability explicit.
                    plan?.entries.iter().find(|entry| entry.app == app)?;
                    let preferences = self.app_preferences(app);
                    Some(InstallOptions {
                        release_format: preferences.release_format,
                        architecture: preferences.architecture,
                    })
                });
            ui.add_enabled_ui(false, |ui| {
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
                        .selected_text(
                            installed
                                .as_ref()
                                .map(|copy| {
                                    crate::ui::screens::overview::release_format_label(
                                        &copy.release_format,
                                    )
                                })
                                .unwrap_or("Loading\u{2026}"),
                        )
                        .show_ui(&mut columns[0], |_| {});
                    theme::text(
                        &mut columns[1],
                        "Architecture",
                        12.0,
                        theme::palette().muted,
                    );
                    columns[1].add_space(6.0);
                    egui::ComboBox::from_id_salt(("app-architecture", app))
                        .width(columns[1].available_width())
                        .selected_text(
                            installed
                                .as_ref()
                                .map(|copy| model::architecture_label(&copy.architecture))
                                .unwrap_or("Loading…"),
                        )
                        .show_ui(&mut columns[1], |_| {});
                });
            });
            if version_change_locked {
                ui.add_space(8.0);
                note(ui, "Uninstall the current installation before changing its format or architecture.");
            }
            return;
        };
        let available = install_options::available(&release, app, &self.preferences);
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
            match self.save_install_options(app, selected) {
                Ok(()) => {
                    // The format/architecture location context resets destination
                    // defaults on the next frame. Retain the dialog draft so that
                    // frame does not start a second preview worker.
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
        let latest = format!(
            "https://api.github.com/repos/{}/releases/latest",
            model::repository(app)
        );
        if release_generation_changed(
            ctx,
            egui::Id::new(("install-versions-generation", &self.paths.root, app)),
            craft_apps_manager::installations::release_catalog::provider::generation(
                &self.paths.root,
                &model::repository(app),
            )
            .or_else(|| {
                craft_apps_manager::network::Network::cached_generation(&self.paths.root, &latest)
            }),
        ) && self.versions_app.as_deref() == Some(app)
        {
            self.load_versions_for(app, ctx);
            // Package-generation changes are handled by install_options_controls.
            // Do not start a second preview or discard the usable version history here.
        }
        ui.horizontal(|ui| {
            theme::text(ui, "Version", 12.0, theme::palette().muted);
            if (self.versions_app.as_deref() == Some(app) && self.versions_receiver.is_some())
                || self.install_receiver.is_some()
                || self.custom_receiver.is_some()
            {
                ui.add(egui::Spinner::new().size(12.0));
            }
        });
        ui.add_space(6.0);
        field_style(ui);
        let current = self.install_selected_tag.clone();
        let latest_label = plan
            .and_then(|plan| plan.entries.first())
            .filter(|_| current.is_none())
            .map(|entry| format!("{} · Latest release", entry.version))
            .unwrap_or_else(|| "Latest release".into());
        let mut selected = current.clone();
        let mut explanation = None;
        ui.add_enabled_ui(
            self.custom_receiver.is_none() && !self.job.snapshot().busy,
            |ui| {
                let id = ui.make_persistent_id(("install-version", app));
                let was_open=ui.memory(|memory|memory.is_popup_open(id.with("popup")));
                install_version_menu(ui, id, current.as_deref().unwrap_or(&latest_label), |ui| {
                    if self.versions_app.as_deref() != Some(app) || (!was_open && craft_apps_manager::installations::release_catalog::provider::source().enabled) {
                        self.load_versions_for(app, ctx);
                    }

                    if !was_open {begin_version_list_transition(ctx,id.with("loaded-list"),self.versions_result.is_some());}
                    animated_version_list(ui,id.with("loaded-list"),self.versions_result.is_some(),|ui| {
                    ui.selectable_value(&mut selected, None, "Latest release");
                    match &self.versions_result {
                        Some(Ok(releases)) => {
                            for release in releases {
                                let reason = (!self.additional_installation)
                                    .then(|| {
                                        self.display_config
                                            .as_ref()?
                                            .apps
                                            .iter()
                                            .find(|copy| copy.name == app)
                                            .and_then(|copy| {
                                                updates::downgrade_scope_error(
                                                    &self.paths.root,
                                                    copy,
                                                    release,
                                                    &self.preferences,
                                                )
                                            })
                                    })
                                    .flatten();
                                let unknown=craft_apps_manager::catalog::get(app).is_some_and(|entry| !entry.repository.contains('/')) && !self.additional_installation && self.display_config.as_ref().and_then(|c|c.apps.iter().find(|copy|copy.name==app)).is_some_and(|copy| matches!(updates::downgrade_scope_decision(&self.paths.root,copy,release,&self.preferences),craft_apps_manager::installations::release_catalog::CompatibilityDecision::Unknown(_)));
                                let (_,information)=install_version_row(ui,&mut selected,&release.tag_name,reason.as_deref(),unknown);
                                if information.is_some_and(|response|response.clicked()) {
                                    if let Some(reason)=reason { explanation=Some((release.tag_name.clone(),reason)); }
                                }
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
                });
            },
        );
        if let Some((tag, reason)) = explanation {
            self.open_compatibility_info(app, &tag, &reason, ctx);
        }
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
            if initialize_install_location(
                &mut self.install_location_context,
                &mut self.install_parent,
                location_context,
                &self.paths,
                current.as_ref(),
                &install_preferences,
                new_draft,
            ) {
                self.install_destination_preview = DestinationPreview::default();
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
            let update = entry.is_some_and(|e| e.action == "Update")
                || entry.is_none() && current.as_ref().is_some_and(|copy| !copy.path.is_empty());
            let requested_downgrade = current
                .as_ref()
                .filter(|copy| !copy.path.is_empty())
                .zip(self.install_selected_tag.as_deref())
                .is_some_and(|(copy, tag)| {
                    updates::version(&copy.version)
                        .ok()
                        .zip(updates::version(tag).ok())
                        .is_some_and(|(installed, selected)| selected < installed)
                });
            let downgrade = (entry.is_none() && requested_downgrade)
                || entry.zip(current.as_ref()).is_some_and(|(entry, current)| {
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
            let setup_location = cfg!(target_os = "windows")
                && installer
                && !current
                    .as_ref()
                    .is_some_and(|copy| copy.install_kind == "installer" && !copy.path.is_empty());
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
            let art = self
                .icons
                .get(&app)
                .map(|t| Art::Texture(t.id()))
                .unwrap_or(Art::None);
            let response = modal(ctx, "Install app", 520.0, |ui| {
                animated_dialog_body(
                    ui,
                    egui::Id::new(("install-dialog-size", &app)),
                    entry.is_some(),
                    |ui| {
                        band(ui, BODY, 0.0, |ui| {
                            // Keep dialog controls stable while only changing metadata rows crossfade.
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
                                .map(|a| {
                                    format!("Download size: {:.1} MB", a.size as f64 / 1_000_000.0)
                                })
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
                            field_style(ui);
                            let managed_location = installer;
                            let mut native_location = native_install_location(current.as_ref());
                            {
                                ui.add_space(18.0);
                                theme::text(ui, "Install location", 13.0, theme::palette().text_3);
                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    let width = (ui.available_width() - 98.0).max(80.0);
                                    let response = ui.add_enabled(
                                        !installed_portable && !managed_location,
                                        egui::TextEdit::singleline(if managed_location {
                                            &mut native_location
                                        } else {
                                            &mut self.install_parent
                                        })
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
                                        !installed_portable
                                            && !managed_location
                                            && self.folder_receiver.is_none(),
                                    )
                                    .clicked()
                                    {
                                        let initial =
                                            std::path::PathBuf::from(&self.install_parent);
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
                                managed_location_note(
                                    ui,
                                    egui::Id::new(("managed-location-note", &app)),
                                    managed_location,
                                    setup_location,
                                );
                                ui.add_space(16.0);
                                ui.separator();
                                ui.add_space(8.0);
                                let target = if managed_location {
                                    Ok(std::path::PathBuf::from(&native_location))
                                } else if installed_portable {
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
                                            if ui
                                                .add_enabled_ui(!setup_location, open_folder_button)
                                                .inner
                                                .clicked()
                                            {
                                                let mut folder = if managed_location {
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
                                                } else {
                                                    std::path::PathBuf::from(&self.install_parent)
                                                };
                                                if let Ok(target) = &target {
                                                    if target.is_dir() {
                                                        folder = target.clone();
                                                    }
                                                }
                                                while !folder.is_dir() && folder.pop() {}
                                                if folder.is_dir() {
                                                    self.result(
                                                        craft_apps_manager::platform::open(&folder),
                                                    );
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
                            }
                            ui.add_space(16.0);
                            desktop_shortcut_option(ui, &mut self.install_shortcut);
                            ui.add_space(10.0);
                            note(
                                ui,
                                if installer {
                                    if cfg!(target_os = "linux") {
                                        "The package defines its install paths. Administrator approval is required."
                                    } else if cfg!(target_os = "windows") {
                                        "Windows may ask for administrator approval. Updates keep the existing app folder."
                                    } else {
                                        "The manager installs the app in an Applications folder."
                                    }
                                } else if installed_portable {
                                    "Updates keep the existing app folder. Uninstall first to change its location."
                                } else {
                                    "The app gets its own folder. Other files in this location stay untouched."
                                },
                            );
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
                                    egui::Label::new(
                                        RichText::new(error).color(theme::palette().red),
                                    )
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
                            let ready = plan.as_ref().is_some_and(|plan| {
                                install_plan_matches_selection(
                                    plan,
                                    &app,
                                    self.install_selected_tag.as_deref(),
                                    &self.app_preferences(&app),
                                    self.additional_installation,
                                )
                            }) && entry.is_some_and(|e| {
                                e.error.is_none()
                                    && matches!(e.action.as_str(), "Install" | "Update")
                                    && e.preferences.release_format
                                        == install_preferences.release_format
                                    && e.preferences.architecture
                                        == install_preferences.architecture
                            }) && self.install_receiver.is_none()
                                && self.custom_receiver.is_none()
                                && self.install_result.is_some()
                                && self.folder_receiver.is_none()
                                && !self.job.snapshot().busy;
                            if action(ui, install_action, Kind::Primary, ready).clicked() {
                                if let Some(mut plan) = plan.clone() {
                                    plan.desktop_shortcut = self.install_shortcut;
                                    self.run_install_plan(app.clone(), plan);
                                }
                            }
                            if action(ui, "Cancel", Kind::Secondary, true).clicked() {
                                self.confirm_install = None;
                            }
                        });
                    },
                );
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

#[cfg(test)]
#[path = "installation_tests.rs"]
mod tests;
