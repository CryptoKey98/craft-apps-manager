use super::{theme, App};
use craft_apps_manager::operations::{Lifecycle, Operation};
use eframe::egui;

#[derive(Default)]
pub(super) struct ActivityNotifications {
    shared: Option<craft_apps_manager::operations::Manager>,
    initialized: bool,
    seen: std::collections::HashMap<String, bool>,
    unread: std::collections::HashSet<String>,
    highlight_ids: std::collections::HashSet<egui::Id>,
}
impl ActivityNotifications {
    pub(super) fn new(root: &std::path::Path) -> Self {
        Self {
            shared: Some(craft_apps_manager::operations::Manager::new(root)),
            ..Default::default()
        }
    }
    pub(super) fn observe(&mut self, records: &[Operation]) {
        for record in records
            .iter()
            .filter(|op| op.parent.is_none() && !op.kind.starts_with("Review "))
        {
            let terminal = record.lifecycle.terminal();
            let previous = self.seen.insert(record.id.clone(), terminal);
            // Existing history is already read on startup; active work is still noteworthy.
            if (previous.is_none() && (self.initialized || !terminal))
                || previous == Some(false) && terminal
            {
                if let Some(shared) = &self.shared {
                    let _ = shared.announce_activity(&record.id);
                }
                self.unread.insert(record.id.clone());
            }
            if self.shared.as_ref().is_some_and(|shared| {
                shared.activity_was_announced(&record.id)
                    && !shared.activity_is_read(&record.id, terminal)
            }) {
                self.unread.insert(record.id.clone());
            }
            if self
                .shared
                .as_ref()
                .is_some_and(|shared| shared.activity_is_read(&record.id, terminal))
            {
                self.unread.remove(&record.id);
            }
        }
        self.initialized = true;
    }
    pub(super) fn count(&self) -> usize {
        self.unread.len()
    }
    fn mark_all_read(&mut self) {
        for id in self.unread.clone() {
            self.mark_read(&id);
        }
    }
    fn dismiss(&mut self, ctx: &egui::Context) {
        self.mark_all_read();
        for id in self.highlight_ids.drain() {
            ctx.animate_bool_with_time(id, false, 0.0);
        }
    }
    fn is_unread(&self, id: &str) -> bool {
        self.unread.contains(id)
    }
    fn mark_read(&mut self, id: &str) {
        if self.unread.remove(id) {
            if let (Some(shared), Some(terminal)) = (&self.shared, self.seen.get(id)) {
                let _ = shared.acknowledge_activity(id, *terminal);
            }
        }
    }
}

impl App {
    pub(super) fn activity_panel(&mut self, ctx: &egui::Context) {
        let visible_id = egui::Id::new("activity-panel-was-visible");
        if !self.activity_open {
            if ctx.data(|data| data.get_temp::<bool>(visible_id).unwrap_or(false)) {
                self.activity_notifications.dismiss(ctx);
                ctx.request_repaint();
            }
            for id in self.activity_notifications.highlight_ids.drain() {
                ctx.animate_bool_with_time(id, false, 0.0);
            }
            ctx.data_mut(|data| data.remove::<bool>(visible_id));
            return;
        }
        let was_visible = ctx.data_mut(|data| {
            let previous = data.get_temp::<bool>(visible_id).unwrap_or(false);
            data.insert_temp(visible_id, true);
            previous
        });
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.activity_open = false;
            self.activity_notifications.dismiss(ctx);
            ctx.request_repaint();
            return;
        }
        let p = theme::palette();
        let light = !ctx.style().visuals.dark_mode;
        let width = 440.0_f32.min(ctx.screen_rect().width() - 48.0);
        let anchor = self
            .activity_anchor
            .unwrap_or(egui::pos2(ctx.screen_rect().right() - 16.0, 70.0));
        let position = egui::pos2((anchor.x - width - 48.0).max(8.0), anchor.y);
        let bounds = egui::Rect::from_min_max(egui::pos2(0.0, anchor.y), ctx.screen_rect().max);
        let viewport_height = (ctx.screen_rect().bottom() - anchor.y - 180.0).clamp(80.0, 420.0);
        let panel = egui::Window::new("manager-activity-panel-v3")
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .movable(false)
            .fixed_pos(position)
            .constrain_to(bounds)
            .default_height(0.0)
            .default_width(width)
            .frame(
                egui::Frame::new()
                    .fill(if light {
                        egui::Color32::from_gray(250)
                    } else {
                        p.panel
                    })
                    .stroke(egui::Stroke::new(
                        1.0_f32,
                        if light {
                            egui::Color32::from_gray(210)
                        } else {
                            p.border_strong
                        },
                    ))
                    .corner_radius(12)
                    .inner_margin(24)
                    .shadow(egui::Shadow {
                        offset: [0, 8],
                        blur: 24,
                        spread: 0,
                        color: egui::Color32::from_black_alpha(if light { 28 } else { 90 }),
                    }),
            )
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 4.0);
                ui.horizontal(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
                    theme::paint_icon(ui.painter(), rect, theme::Icon::Activity, p.accent_text);
                    ui.label(egui::RichText::new("Activity").font(theme::bold(18.0)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let count = self.activity_notifications.count();
                        theme::text(
                            ui,
                            if count == 0 {
                                "All caught up".into()
                            } else {
                                format!("{count} unread")
                            },
                            12.0,
                            p.muted,
                        );
                        if theme::icon_button(ui, theme::Icon::Close, "Close Activity", p.muted)
                            .clicked()
                        {
                            self.activity_open = false;
                        }
                    });
                });
                theme::text(ui, "Downloads, installations and builds", 12.0, p.muted);
                ui.add_space(18.0);
                let records = self.operation_records.clone();
                // Match the sidebar's expanding scrollbar, reserving a gutter so it never covers a row.
                ui.spacing_mut().scroll = egui::style::ScrollStyle::floating();
                ui.spacing_mut().scroll.floating_allocated_width = 14.0;
                ui.spacing_mut().scroll.dormant_handle_opacity = 0.08;
                ui.spacing_mut().scroll.active_handle_opacity = 0.35;
                ui.spacing_mut().scroll.interact_handle_opacity = 0.65;
                egui::ScrollArea::vertical()
                    .id_salt("activity-viewport")
                    .max_height(viewport_height)
                    .min_scrolled_height(viewport_height)
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(
                        egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded,
                    )
                    .show(ui, |ui| {
                        for (heading, active) in [("In progress", true), ("Recent activity", false)]
                        {
                            let parents: Vec<_> = records
                                .iter()
                                .filter(|op| {
                                    let root = op.parent.as_ref().is_none_or(|id| {
                                        !records.iter().any(|parent| &parent.id == id)
                                    });
                                    root && !op.lifecycle.terminal() == active
                                        && (active
                                            || self.activity_history
                                            || !matches!(
                                                op.kind.as_str(),
                                                "Review install" | "Review releases"
                                            ))
                                })
                                .collect();
                            theme::section_label(ui, heading);
                            ui.add_space(8.0);
                            if parents.is_empty() {
                                theme::text(
                                    ui,
                                    if active {
                                        "No operations are running."
                                    } else {
                                        "Completed operations will appear here."
                                    },
                                    13.0,
                                    p.muted,
                                );
                                ui.add_space(14.0);
                            }
                            let mut remaining =
                                parents.len().min(if active || self.activity_history {
                                    100
                                } else {
                                    5
                                });
                            for parent in
                                parents
                                    .into_iter()
                                    .take(if active || self.activity_history {
                                        100
                                    } else {
                                        5
                                    })
                            {
                                self.activity_row(ui, parent, &records);
                                remaining -= 1;
                                if remaining > 0 {
                                    if active {
                                        ui.add_space(8.0);
                                        ui.separator();
                                        ui.add_space(8.0);
                                    } else {
                                        ui.add_space(6.0);
                                    }
                                }
                            }
                            if active {
                                ui.separator();
                                ui.add_space(16.0);
                            }
                        }
                    });
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(12.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::btn(if self.activity_history {
                        "Show recent activity"
                    } else {
                        "View all activity"
                    })
                    .show(ui)
                    .clicked()
                    {
                        self.activity_history = !self.activity_history;
                    }
                });
            });
        // Ignore the toolbar click that first opens the panel.
        if was_visible && panel.is_some_and(|panel| panel.response.clicked_elsewhere()) {
            self.activity_open = false;
        }
        if !self.activity_open {
            self.activity_notifications.dismiss(ctx);
            ctx.request_repaint();
            ctx.data_mut(|data| data.remove::<bool>(visible_id));
        }
    }
    fn activity_row(&mut self, ui: &mut egui::Ui, op: &Operation, records: &[Operation]) {
        let highlight = ui.painter().add(egui::Shape::Noop);
        let p = theme::palette();
        let app_children: Vec<_> = records
            .iter()
            .filter(|child| child.parent.as_deref() == Some(&op.id))
            .collect();
        let mut children = app_children.clone();
        if op.kind == "Downgrade" {
            let step_parent_ids: Vec<_> = children.iter().map(|child| child.id.as_str()).collect();
            let steps: Vec<_> = records
                .iter()
                .filter(|step| {
                    step.parent
                        .as_deref()
                        .is_some_and(|parent| step_parent_ids.contains(&parent))
                })
                .collect();
            if !steps.is_empty() {
                children = steps;
            }
        }
        let facts = if children.len() == 1 { children[0] } else { op };
        let name = if op.resource.is_empty() {
            "Manager".into()
        } else {
            craft_apps_manager::model::title(&op.resource)
        };
        let title = match (op.kind.as_str(), op.lifecycle) {
            ("Install" | "Install / update", Lifecycle::Completed) => format!("{name} installed"),
            ("Uninstall", Lifecycle::Completed) => format!("{name} uninstalled"),
            ("Build", Lifecycle::Completed) => format!("{name} built"),
            (kind, state) if state.terminal() => {
                format!("{name} · {kind} · {}", activity_status(state))
            }
            ("Build", _) => format!("Building {name}"),
            ("Install", _) => format!("Installing {name}"),
            ("Uninstall", _) => format!("Uninstalling {name}"),
            (kind, _) => format!("{name} · {kind}"),
        };
        if !op.lifecycle.terminal() {
            let (rect, _) = ui
                .allocate_exact_size(egui::vec2(ui.available_width(), 94.0), egui::Sense::hover());
            let icon_rect =
                egui::Rect::from_min_size(rect.min + egui::vec2(0.0, 6.0), egui::vec2(32.0, 32.0));
            if let Some(texture) = self.icons.get(&op.resource) {
                egui::Image::new((texture.id(), icon_rect.size()))
                    .corner_radius(6)
                    .paint_at(ui, icon_rect);
            } else {
                theme::paint_icon(
                    ui.painter(),
                    icon_rect.shrink(5.0),
                    theme::Icon::Activity,
                    p.accent_text,
                );
            }
            let text_x = rect.left() + 44.0;
            let requested = op.lifecycle == Lifecycle::CancellationRequested;
            let cancel_button = theme::btn(if requested { "Cancelling…" } else { "Cancel" })
                .enabled(!requested && op.cancellation_available && op.parent.is_none());
            let button_size = cancel_button.desired_size(ui);
            let button_rect = egui::Rect::from_min_size(
                egui::pos2(rect.right() - button_size.x, rect.top() + 6.0),
                button_size,
            );
            let mut controls = ui.new_child(egui::UiBuilder::new().max_rect(button_rect));
            if cancel_button.show(&mut controls).clicked() {
                if let Err(error) = self.job.operations.request_cancel(&op.id) {
                    self.error = Some(format!("Could not request cancellation: {error:#}"));
                }
            }
            let text_rect = egui::Rect::from_min_max(
                egui::pos2(text_x, rect.top()),
                egui::pos2((button_rect.left() - 10.0).max(text_x), rect.bottom()),
            );
            self.activity_unread_highlight(ui, op, rect, (text_rect, false), highlight);
            let painter = ui.painter().with_clip_rect(text_rect);
            painter.text(
                egui::pos2(text_x, rect.top() + 6.0),
                egui::Align2::LEFT_TOP,
                &title,
                theme::medium(14.0),
                p.text,
            );
            let stage = if op.detail.is_empty() {
                op.stage.clone()
            } else {
                format!("{} · {}", op.stage, op.detail)
            };
            painter.text(
                egui::pos2(text_x, rect.top() + 29.0),
                egui::Align2::LEFT_TOP,
                stage,
                egui::FontId::proportional(12.0),
                p.muted,
            );
            let track = egui::Rect::from_min_size(
                egui::pos2(text_x, rect.top() + 54.0),
                egui::vec2((rect.right() - text_x).max(0.0), 5.0),
            );
            ui.painter().rect_filled(track, 3, p.progress_track);
            if let Some(progress) = facts.progress {
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(
                        track.min,
                        egui::vec2(track.width() * progress.clamp(0.0, 1.0), track.height()),
                    ),
                    3,
                    p.accent,
                );
            } else {
                let phase = (ui.input(|input| input.time) as f32 * 0.7).fract();
                let length = track.width() * 0.28;
                let x = track.left() - length + phase * (track.width() + length);
                ui.painter().with_clip_rect(track).rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(x, track.top()),
                        egui::vec2(length, track.height()),
                    ),
                    3,
                    p.accent,
                );
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(33));
            }
            if let (Some(format), Some(arch)) = (&facts.format, &facts.architecture) {
                ui.painter().text(
                    egui::pos2(text_x, rect.top() + 69.0),
                    egui::Align2::LEFT_TOP,
                    format!(
                        "{} · {arch}",
                        if format == "portable" {
                            "Portable"
                        } else if format == "installer" {
                            "Installer"
                        } else {
                            format.as_str()
                        }
                    ),
                    egui::FontId::proportional(12.0),
                    p.muted,
                );
            }
        } else {
            let (icon, color) = match op.lifecycle {
                Lifecycle::Completed => (theme::Icon::Check, p.green),
                Lifecycle::Failed => (theme::Icon::Alert, p.red),
                _ => (theme::Icon::Alert, p.muted),
            };
            let id = ui.make_persistent_id(("activity-details", &op.id));
            let mut expanded = egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                id,
                std::env::args().any(|arg| arg == "activity-details"),
            );
            let subtitle = if children.len() > 1 {
                format!(
                    "{} of {} completed",
                    children
                        .iter()
                        .filter(|child| child.lifecycle == Lifecycle::Completed)
                        .count(),
                    children.len()
                )
            } else if op.lifecycle == Lifecycle::Failed {
                if op.detail.is_empty() {
                    "View error details".into()
                } else {
                    op.detail.lines().next().unwrap_or_default().into()
                }
            } else if let (Some(format), Some(arch)) = (&facts.format, &facts.architecture) {
                format!(
                    "{} · {arch}",
                    if format == "portable" {
                        "Portable"
                    } else if format == "installer" {
                        "Installer"
                    } else {
                        format.as_str()
                    }
                )
            } else {
                activity_status(op.lifecycle).into()
            };
            let (rect, response) = ui
                .allocate_exact_size(egui::vec2(ui.available_width(), 64.0), egui::Sense::click());
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    true,
                    format!("{title}. Show details"),
                )
            });
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            theme::paint_icon(
                ui.painter(),
                egui::Rect::from_center_size(
                    egui::pos2(rect.left() + 22.0, rect.center().y),
                    egui::vec2(16.0, 16.0),
                ),
                icon,
                color,
            );
            let text_x = rect.left() + 44.0;
            let title_rect = egui::Rect::from_min_max(
                egui::pos2(text_x, rect.top()),
                egui::pos2((rect.right() - 108.0).max(text_x), rect.bottom()),
            );
            let unread_opacity = self.activity_unread_highlight(
                ui,
                op,
                rect,
                (title_rect, response.clicked()),
                highlight,
            );
            let painter = ui.painter().with_clip_rect(title_rect);
            painter.text(
                egui::pos2(text_x, rect.top() + 13.0),
                egui::Align2::LEFT_TOP,
                &title,
                theme::medium(14.0),
                p.text,
            );
            if unread_opacity > 0.0 {
                let title_width = painter
                    .layout_no_wrap(title.clone(), theme::medium(14.0), p.text)
                    .size()
                    .x;
                painter.circle_filled(
                    egui::pos2(text_x + title_width + 9.0, rect.top() + 21.0),
                    2.5,
                    p.accent_text.gamma_multiply(unread_opacity),
                );
            }
            painter.text(
                egui::pos2(text_x, rect.top() + 35.0),
                egui::Align2::LEFT_TOP,
                subtitle,
                egui::FontId::proportional(12.0),
                p.muted,
            );
            if let Some(time) = op
                .completed
                .or(op.started)
                .and_then(chrono::DateTime::from_timestamp_millis)
            {
                ui.painter().text(
                    egui::pos2(rect.right() - 30.0, rect.center().y),
                    egui::Align2::RIGHT_CENTER,
                    time.with_timezone(&chrono::Local)
                        .format("%H:%M")
                        .to_string(),
                    egui::FontId::proportional(12.0),
                    p.muted,
                );
            }
            let chevron = egui::Rect::from_center_size(
                egui::pos2(rect.right() - 10.0, rect.center().y),
                egui::vec2(14.0, 14.0),
            );
            if !expanded.is_open() {
                theme::paint_icon(ui.painter(), chevron, theme::Icon::ChevronDown, p.muted);
            } else {
                let center = chevron.center();
                ui.painter().add(egui::Shape::line(
                    vec![
                        center + egui::vec2(-4.0, 2.0),
                        center + egui::vec2(0.0, -2.0),
                        center + egui::vec2(4.0, 2.0),
                    ],
                    egui::Stroke::new(1.5_f32, p.muted),
                ));
            }
            if response.clicked() {
                expanded.toggle(ui);
            }
            expanded.show_body_unindented(ui, |ui| {
                let light = !ui.visuals().dark_mode;
                let surface = ui.painter().add(egui::Shape::Noop);
                let details = egui::Frame::new()
                    .corner_radius(6)
                    .stroke(if light {
                        egui::Stroke::new(1.0_f32, egui::Color32::from_gray(218))
                    } else {
                        egui::Stroke::new(1.0_f32, p.border)
                    })
                    .inner_margin(egui::Margin {
                        left: 32,
                        right: 12,
                        top: 12,
                        bottom: 12,
                    })
                    .show(ui, |ui| {
                        let primary = if children.len() == 1 && op.lifecycle == Lifecycle::Completed
                        {
                            children[0]
                        } else {
                            op
                        };
                        operation_details(ui, primary);
                        if op.kind == "Downgrade"
                            && children
                                .iter()
                                .any(|child| !app_children.iter().any(|app| app.id == child.id))
                        {
                            for app in &app_children {
                                operation_details(ui, app);
                            }
                        }
                        if children.len() > 1
                            || (op.lifecycle != Lifecycle::Completed
                                && children.iter().any(|child| {
                                    matches!(
                                        child.lifecycle,
                                        Lifecycle::Failed | Lifecycle::Cancelled
                                    )
                                }))
                        {
                            for child in &children {
                                ui.add_space(8.0);
                                ui.separator();
                                ui.strong(format!(
                                    "{} · {}",
                                    craft_apps_manager::model::title(&child.resource),
                                    activity_status(child.lifecycle)
                                ));
                                operation_details(ui, child);
                            }
                        }
                    });
                ui.painter()
                    .set(surface, gray_details_surface(details.response.rect, light));
            });
        }
    }
    fn activity_unread_highlight(
        &mut self,
        ui: &egui::Ui,
        op: &Operation,
        rect: egui::Rect,
        read_target: (egui::Rect, bool),
        shape: egui::layers::ShapeIdx,
    ) -> f32 {
        // Opening the panel alone does not acknowledge any operation. Only the
        // visible text hit area or an explicit details click marks this item read.
        let (text_rect, clicked) = read_target;
        if clicked || ui.rect_contains_pointer(text_rect.intersect(ui.clip_rect())) {
            self.activity_notifications.mark_read(&op.id);
        }
        let highlight_id = ui.make_persistent_id(("activity-unread", &op.id));
        self.activity_notifications
            .highlight_ids
            .insert(highlight_id);
        let opacity = ui.ctx().animate_bool_with_time(
            highlight_id,
            self.activity_notifications.is_unread(&op.id),
            0.18,
        );
        if opacity > 0.0 {
            let color = if ui.visuals().dark_mode {
                egui::Color32::from_rgb(36, 48, 68)
            } else {
                egui::Color32::from_rgb(234, 241, 252)
            };
            ui.painter().set(
                shape,
                egui::Shape::rect_filled(rect, 8, color.gamma_multiply(opacity)),
            );
        }
        opacity
    }
}
/// A restrained neutral gradient with rounded corners behind expanded details.
fn gray_details_surface(rect: egui::Rect, light: bool) -> egui::Shape {
    let mut mesh = egui::epaint::Mesh::default();
    let shade = |point: egui::Pos2| {
        let t = ((point.y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0);
        let gray = if light {
            247.0 - 9.0 * t
        } else {
            38.0 - 6.0 * t
        };
        egui::Color32::from_gray(gray.round() as u8)
    };
    mesh.colored_vertex(rect.center(), shade(rect.center()));
    let radius = 6.0_f32.min(rect.width() / 2.0).min(rect.height() / 2.0);
    for (center, start) in [
        (
            egui::pos2(rect.right() - radius, rect.top() + radius),
            -std::f32::consts::FRAC_PI_2,
        ),
        (
            egui::pos2(rect.right() - radius, rect.bottom() - radius),
            0.0,
        ),
        (
            egui::pos2(rect.left() + radius, rect.bottom() - radius),
            std::f32::consts::FRAC_PI_2,
        ),
        (
            egui::pos2(rect.left() + radius, rect.top() + radius),
            std::f32::consts::PI,
        ),
    ] {
        for step in 0..=8 {
            let angle = start + step as f32 / 8.0 * std::f32::consts::FRAC_PI_2;
            let point = center + radius * egui::vec2(angle.cos(), angle.sin());
            mesh.colored_vertex(point, shade(point));
        }
    }
    let last = mesh.vertices.len() as u32 - 1;
    for index in 1..=last {
        mesh.add_triangle(0, index, if index == last { 1 } else { index + 1 });
    }
    egui::Shape::mesh(mesh)
}
fn activity_status(state: Lifecycle) -> &'static str {
    match state {
        Lifecycle::Queued => "Pending",
        Lifecycle::Running => "Running",
        Lifecycle::CancellationRequested => "Cancelling",
        Lifecycle::Completed => "Completed",
        Lifecycle::Failed => "Failed",
        Lifecycle::Cancelled => "Cancelled",
        Lifecycle::Interrupted => "Interrupted",
    }
}

fn operation_message(op: &Operation) -> String {
    if op.lifecycle == Lifecycle::Completed {
        if let Some(result) = op.facts.get("Result") {
            return result.clone();
        }
        let text = match op.kind.as_str() {
            "Install" | "Update" | "Install / update" => "Installation completed successfully.",
            "Uninstall" => "The application was uninstalled successfully.",
            "Downgrade" => "The older version was installed successfully. Individual steps are listed below.",
            "Restore installation" => "The previous installation was restored successfully.",
            "Manager update download" => "The manager update was downloaded and verified. Installation has not been performed yet.",
            "Build" => "Compilation completed successfully. The compiled build is ready.",
            "Source download" => "Source operation completed successfully.",
            "Move" => "The installation was moved successfully.",
            "Remove source" => "The downloaded source was removed successfully.",
            "Delete cache" | "clean" => "Cache cleanup completed successfully.",
            "Restore backup" => "The selected backup was restored successfully.",
            "Delete backups" => "The selected backups were deleted successfully.",
            "Set up tools" => "Build tool setup completed successfully.",
            "Bulk update" => {
                "The batch completed successfully. Individual results are listed below."
            }
            "Review install" | "Review releases" | "Check" | "Background update check" => {
                "The check completed successfully. This check did not install applications."
            }
            "clear" | "clear-app" => "Backup cleanup completed successfully.",
            _ => "The operation completed successfully.",
        };
        text.into()
    } else {
        match op.lifecycle {
            Lifecycle::Failed => format!("The operation failed. {}",op.result.as_deref().unwrap_or("No additional error information was recorded.")),
            Lifecycle::Cancelled => "Cancellation was confirmed. Any changes made before cancellation may still need review.".into(),
            Lifecycle::Interrupted => "The operation was interrupted or stopped reporting. Its final outcome is unknown; check the app before retrying.".into(),
            Lifecycle::CancellationRequested => "Cancellation has been requested but is not yet confirmed.".into(),
            Lifecycle::Queued => "This operation has not started yet.".into(),
            _ => "The operation is still running.".into(),
        }
    }
}
fn operation_details(ui: &mut egui::Ui, op: &Operation) {
    ui.label(operation_message(op));
    if op.lifecycle == Lifecycle::Cancelled {
        if let Some(result) = &op.result {
            ui.label(result);
        }
    }
    for (label, value) in &op.facts {
        if label == "Result" {
            continue;
        }
        ui.add_space(4.0);
        theme::text(ui, label, 12.0, theme::palette().muted);
        ui.add(egui::Label::new(value).wrap());
    }
    if let Some(time) = op
        .completed
        .and_then(chrono::DateTime::from_timestamp_millis)
    {
        ui.add_space(4.0);
        theme::text(
            ui,
            time.with_timezone(&chrono::Local)
                .format("Finished %b %d, %Y at %H:%M:%S")
                .to_string(),
            12.0,
            theme::palette().muted,
        );
    }
}

#[cfg(test)]
mod activity_tests {
    use super::*;
    fn record(kind: &str, state: Lifecycle) -> Operation {
        let mut op: Operation = serde_json::from_value(serde_json::json!({
            "id":uuid::Uuid::new_v4().to_string(),"parent":null,"resource":"photocraft","kind":kind,"lifecycle":"Completed", "stage":"Completed","detail":"photocraft","progress":1.0,"started":null,"completed":null,"heartbeat":0,"cancellation_available":false,"result":"Success","owner":0,"format":null,"architecture":null
        })).unwrap();
        op.lifecycle = state;
        op
    }
    #[test]
    fn notification_acknowledgements_sync_both_windows_and_preserve_completion() {
        let root =
            std::env::temp_dir().join(format!("craft-activity-sync-{}", uuid::Uuid::new_v4()));
        let mut manager = ActivityNotifications::new(&root);
        let mut builder = ActivityNotifications::new(&root);
        manager.observe(&[]);
        builder.observe(&[]);
        let mut op = record("Build", Lifecycle::Running);
        manager.observe(&[op.clone()]);
        builder.observe(&[op.clone()]);
        assert_eq!(manager.count(), 1);
        assert_eq!(builder.count(), 1);
        let mut reopened = ActivityNotifications::new(&root);
        let mut completed = op.clone();
        completed.lifecycle = Lifecycle::Completed;
        manager.observe(&[completed.clone()]);
        reopened.observe(&[completed]);
        assert_eq!(reopened.count(), manager.count());
        builder.mark_read(&op.id);
        manager.observe(&[op.clone()]);
        assert_eq!(manager.count(), 0);
        op.lifecycle = Lifecycle::Completed;
        manager.observe(&[op.clone()]);
        builder.observe(&[op.clone()]);
        assert_eq!(manager.count(), 1);
        assert_eq!(builder.count(), 1);
        manager.mark_all_read();
        builder.observe(&[op.clone()]);
        assert_eq!(builder.count(), 0);
        assert!(!builder.is_unread(&op.id));
        let next = record("Build", Lifecycle::Running);
        manager.observe(&[op.clone(), next.clone()]);
        builder.observe(&[op, next]);
        assert_eq!(manager.count(), 1);
        assert_eq!(builder.count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn activity_badge_retains_unread_completion_until_viewed() {
        let mut badge = ActivityNotifications::default();
        badge.observe(&[]);
        let mut first = record("Install", Lifecycle::Running);
        first.id = "first".into();
        badge.observe(&[first.clone()]);
        assert_eq!(badge.count(), 1);
        first.lifecycle = Lifecycle::Completed;
        badge.observe(&[first.clone()]);
        assert_eq!(badge.count(), 1);
        let mut second = record("Install", Lifecycle::Failed);
        second.id = "second".into();
        badge.observe(&[first.clone(), second.clone()]);
        assert_eq!(badge.count(), 2);
        badge.mark_read("first");
        assert_eq!(badge.count(), 1);
        badge.mark_read("second");
        badge.observe(&[first, second]);
        assert_eq!(badge.count(), 0);
    }
    #[test]
    fn activity_badge_reports_completion_after_active_work_was_viewed() {
        let mut badge = ActivityNotifications::default();
        let mut op = record("Install", Lifecycle::Running);
        badge.observe(std::slice::from_ref(&op));
        badge.mark_read(&op.id);
        op.lifecycle = Lifecycle::Completed;
        badge.observe(std::slice::from_ref(&op));
        assert_eq!(badge.count(), 1);
        badge.mark_read(&op.id);
        badge.observe(&[op]);
        assert_eq!(badge.count(), 0);
    }
    #[test]
    fn dismissing_panel_finishes_highlight_animation_before_reopening() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("unread-row-test");
        let mut badge = ActivityNotifications::default();
        badge.highlight_ids.insert(id);
        assert_eq!(ctx.animate_bool_with_time(id, true, 0.18), 1.0);
        badge.dismiss(&ctx);
        assert_eq!(ctx.animate_bool_with_time(id, false, 0.18), 0.0);
    }
    #[test]
    fn closing_panel_reads_remaining_rows_but_new_activity_notifies_again() {
        let mut badge = ActivityNotifications::default();
        badge.observe(&[]);
        let op = record("Install", Lifecycle::Completed);
        badge.observe(std::slice::from_ref(&op));
        assert_eq!(badge.count(), 1);
        badge.mark_all_read();
        badge.observe(std::slice::from_ref(&op));
        assert_eq!(badge.count(), 0);
        assert!(!badge.is_unread(&op.id));
        let next = record("Build", Lifecycle::Completed);
        badge.observe(&[op.clone(), next]);
        assert_eq!(badge.count(), 1);
        assert!(!badge.is_unread(&op.id));
    }
    #[test]
    fn reading_one_activity_preserves_other_unread_items() {
        let mut badge = ActivityNotifications::default();
        badge.observe(&[]);
        let first = record("Install", Lifecycle::Completed);
        let second = record("Build", Lifecycle::Failed);
        badge.observe(&[first.clone(), second.clone()]);
        badge.mark_read(&first.id);
        badge.mark_read(&first.id);
        badge.mark_read("missing");
        badge.observe(&[first.clone(), second.clone()]);
        assert!(!badge.is_unread(&first.id));
        assert!(badge.is_unread(&second.id));
        assert_eq!(badge.count(), 1);
    }
    #[test]
    fn existing_history_is_read_but_new_completions_are_unread() {
        let mut badge = ActivityNotifications::default();
        let old = record("Install", Lifecycle::Completed);
        badge.observe(std::slice::from_ref(&old));
        assert_eq!(badge.count(), 0);
        let new = record("Build", Lifecycle::Completed);
        badge.observe(&[old, new.clone()]);
        assert!(badge.is_unread(&new.id));
        assert_eq!(badge.count(), 1);
    }
    #[test]
    fn activity_details_do_not_invent_install_paths_or_shortcut_success() {
        let op = record("Install", Lifecycle::Completed);
        assert_eq!(
            operation_message(&op),
            "Installation completed successfully."
        );
        assert!(!operation_message(&op).contains("shortcut"));
        assert!(op.facts.is_empty());
    }
    #[test]
    fn activity_details_preserve_skips_errors_and_uncertain_outcomes() {
        let mut op = record("Install", Lifecycle::Completed);
        op.facts.insert(
            "Result".into(),
            "Skipped because the application was open.".into(),
        );
        assert!(operation_message(&op).starts_with("Skipped"));
        op.lifecycle = Lifecycle::Failed;
        op.result = Some("Access denied".into());
        assert!(operation_message(&op).contains("Access denied"));
        op.lifecycle = Lifecycle::Interrupted;
        assert!(operation_message(&op).contains("unknown"));
        op.lifecycle = Lifecycle::CancellationRequested;
        assert!(operation_message(&op).contains("not yet confirmed"));
    }
}
