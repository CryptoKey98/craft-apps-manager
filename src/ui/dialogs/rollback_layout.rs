//! Geometry of the approved Rollback mockup, using the manager's shared palette.
use super::super::*;

pub(super) use super::super::shared::animated_dialog_body as animated_view;

pub(super) fn header(ui: &mut egui::Ui, subtitle: &str, close_enabled: bool) -> bool {
    let mut close = false;
    band(ui, egui::Margin::symmetric(22, 20), 0.0, |ui| {
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 43.5), Sense::hover());
        theme::paint_icon(
            ui.painter(),
            egui::Rect::from_center_size(
                egui::pos2(rect.left() + 10.0, rect.center().y),
                egui::Vec2::splat(20.0),
            ),
            Icon::Rollback,
            theme::palette().muted,
        );
        ui.painter().galley(
            rect.min + egui::vec2(32.0, 0.0),
            ui.painter().layout_no_wrap(
                "Rollback".into(),
                theme::bold(17.0),
                theme::palette().text,
            ),
            theme::palette().text,
        );
        let mut job = egui::text::LayoutJob::simple_singleline(
            subtitle.to_owned(),
            egui::FontId::proportional(12.0),
            theme::palette().muted,
        );
        job.wrap.max_width = (rect.width() - 76.0).max(1.0);
        job.wrap.max_rows = 1;
        ui.painter().galley(
            rect.min + egui::vec2(32.0, 25.5),
            ui.painter().layout_job(job),
            theme::palette().muted,
        );
        let cross = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 12.0, rect.center().y),
            egui::Vec2::splat(24.0),
        );
        let response = ui
            .add_enabled_ui(close_enabled, |ui| {
                ui.interact(cross, ui.id().with("rollback-close"), Sense::click())
            })
            .inner;
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, close_enabled, "Close rollback")
        });
        if response.hovered() || response.has_focus() {
            ui.painter()
                .rect_filled(cross, CornerRadius::same(6), theme::palette().button_hover);
        }
        theme::paint_icon(
            ui.painter(),
            egui::Rect::from_center_size(cross.center(), egui::Vec2::splat(18.0)),
            Icon::Close,
            theme::palette().muted,
        );
        close = response.clicked();
    });
    rule(ui);
    close
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CardStatus {
    Checking,
    Ready,
    Failed,
}

fn ready_check_progress(ui: &egui::Ui, id: egui::Id, status: CardStatus) -> f32 {
    let id = id.with("ready-check-start");
    if status != CardStatus::Ready {
        ui.ctx().data_mut(|data| data.remove::<f64>(id));
        return 0.0;
    }
    if ui.style().animation_time <= 0.0 {
        return 1.0;
    }
    let now = ui.input(|input| input.time);
    let started = ui.ctx().data_mut(|data| {
        let started = data.get_temp::<f64>(id).unwrap_or(now);
        data.insert_temp(id, started);
        started
    });
    let progress = ((now - started - 0.18) / 0.24).clamp(0.0, 1.0) as f32;
    if progress < 1.0 {
        ui.ctx().request_repaint();
    }
    progress
}

pub(super) fn previous_card(
    ui: &mut egui::Ui,
    version: &str,
    bytes: u64,
    enabled: bool,
    status: CardStatus,
    details_opacity: f32,
) -> egui::Response {
    let enabled = enabled && status == CardStatus::Ready;
    let palette = theme::palette();
    let (status_text, status_color) = match status {
        CardStatus::Checking => ("Checking files…", palette.amber),
        CardStatus::Ready => ("Ready to restore", palette.green),
        CardStatus::Failed => ("Verification failed", palette.red_text),
    };
    let response = ui
        .scope(|ui| {
            let widgets = &mut ui.visuals_mut().widgets;
            widgets.inactive.weak_bg_fill = palette.card;
            widgets.inactive.bg_stroke = Stroke::new(1.0_f32, palette.border);
            widgets.hovered.weak_bg_fill = palette.selected;
            widgets.hovered.bg_stroke = Stroke::new(1.0_f32, palette.link);
            widgets.active.weak_bg_fill = palette.selected;
            widgets.active.bg_stroke = Stroke::new(1.0_f32, palette.link);
            ui.add_enabled(
                enabled,
                egui::Button::new("")
                    .corner_radius(9)
                    .min_size(egui::vec2(ui.available_width(), 100.0)),
            )
        })
        .inner;
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            enabled,
            format!("Roll back to {version}"),
        )
    });
    // A fast verification can finish before the first Checking frame is drawn.
    // Restart the icon when fresh details begin, including on reopening Review.
    if status == CardStatus::Ready && details_opacity <= 0.0 {
        ui.ctx()
            .data_mut(|data| data.remove::<f64>(response.id.with("ready-check-start")));
    }
    let check_progress = ready_check_progress(ui, response.id, status);
    let rect = response.rect;
    let ink = |color: Color32| color.gamma_multiply(if enabled { 1.0 } else { 0.45 });
    let painter = ui.painter();
    theme::painter_text(
        painter,
        rect.min + egui::vec2(16.0, 16.0),
        egui::Align2::LEFT_TOP,
        "Previous version",
        12.0,
        ink(palette.muted),
    );
    let mut title = egui::text::LayoutJob::simple_singleline(
        version.to_owned(),
        theme::bold(14.0),
        ink(palette.text).gamma_multiply(details_opacity),
    );
    title.wrap.max_rows = 1;
    title.wrap.max_width = (rect.width() - 132.0).max(1.0);
    painter.galley(
        rect.min + egui::vec2(16.0, 34.0),
        painter.layout_job(title),
        ink(palette.text).gamma_multiply(details_opacity),
    );
    if status == CardStatus::Ready {
        theme::painter_text(
            painter,
            egui::pos2(rect.right() - 44.0, rect.top() + 34.0),
            egui::Align2::RIGHT_TOP,
            format!("{:.0} MB", bytes as f64 / 1_048_576.0),
            12.0,
            ink(palette.muted).gamma_multiply(details_opacity),
        );
    }
    theme::paint_icon(
        painter,
        egui::Rect::from_center_size(
            egui::pos2(rect.right() - 24.0, rect.top() + 43.0),
            egui::Vec2::splat(16.0),
        ),
        Icon::ChevronRight,
        ink(palette.muted),
    );
    let check =
        egui::Rect::from_min_size(rect.min + egui::vec2(16.0, 67.0), egui::Vec2::splat(14.0));
    if status != CardStatus::Checking {
        painter.circle_stroke(check.center(), 6.0, Stroke::new(1.2_f32, status_color));
        if status == CardStatus::Ready {
            let start = check.min + egui::vec2(3.0, 7.0);
            let middle = check.min + egui::vec2(6.0, 10.0);
            let end = check.min + egui::vec2(11.0, 4.0);
            let progress = check_progress;
            let stroke = Stroke::new(1.2_f32, status_color);
            if progress > 0.0 {
                painter.line_segment(
                    [start, start.lerp(middle, (progress * 3.0).min(1.0))],
                    stroke,
                );
            }
            if progress > 1.0 / 3.0 {
                painter.line_segment(
                    [
                        middle,
                        middle.lerp(end, ((progress - 1.0 / 3.0) * 1.5).min(1.0)),
                    ],
                    stroke,
                );
            }
        } else {
            theme::paint_icon(painter, check.shrink(3.0), Icon::Close, status_color);
        }
    }
    theme::painter_text(
        painter,
        rect.min + egui::vec2(36.0, 65.0),
        egui::Align2::LEFT_TOP,
        status_text,
        12.0,
        status_color,
    );
    if status == CardStatus::Checking {
        egui::Spinner::new()
            .size(14.0)
            .color(status_color)
            .paint_at(ui, check);
    }
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

pub(super) fn footer(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    rule(ui);
    band(ui, egui::Margin::symmetric(20, 14), 0.0, |ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 39.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                content(ui);
            },
        );
    });
}

pub(super) fn retained_row(ui: &mut egui::Ui, enabled: bool, opacity: f32) -> bool {
    let mut remove = false;
    ui.scope(|ui| {
        ui.multiply_opacity(opacity);
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 21.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                theme::text(
                    ui,
                    "One previous version retained",
                    12.0,
                    theme::palette().muted,
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_enabled_ui(enabled, |ui| {
                        remove = theme::link(ui, "Remove previous version…").clicked();
                    });
                });
            },
        );
    });
    remove
}

pub(super) fn action(ui: &mut egui::Ui, text: &str, kind: Kind, enabled: bool) -> egui::Response {
    btn(text)
        .kind(kind)
        .medium()
        .height(39.0)
        .enabled(enabled)
        .show(ui)
}

pub(super) fn note(ui: &mut egui::Ui, text: &str) {
    ui.add(
        egui::Label::new(
            RichText::new(text)
                .size(12.0)
                .line_height(Some(18.0))
                .color(theme::palette().muted),
        )
        .wrap(),
    );
}

#[cfg(test)]
mod animation_tests {
    use super::*;

    #[test]
    fn confirmation_resizes_gradually_in_both_directions() {
        let ctx = egui::Context::default();
        let frame = |time, confirmation| {
            let mut height = 0.0;
            let _ = ctx.run(
                egui::RawInput {
                    time: Some(time),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let top = ui.cursor().top();
                        animated_view(ui, egui::Id::new("view"), confirmation, |ui| {
                            ui.allocate_exact_size(
                                egui::vec2(300.0, if confirmation { 40.0 } else { 100.0 }),
                                Sense::hover(),
                            );
                        });
                        height = ui.cursor().top() - top;
                    });
                },
            );
            height
        };
        assert!((frame(0.0, false) - 100.0).abs() < 1.0);
        frame(0.1, false);
        assert!((frame(0.2, true) - 100.0).abs() < 1.0);
        frame(0.21, true);
        let middle = frame(0.32, true);
        assert!(middle > 40.0 && middle < 100.0, "{middle}");
        assert!((frame(0.6, true) - 40.0).abs() < 1.0);
        frame(0.7, false);
        let middle = frame(0.8, false);
        assert!(middle > 40.0 && middle < 100.0, "{middle}");
        assert!((frame(1.1, false) - 100.0).abs() < 1.0);
    }

    #[test]
    fn ready_check_waits_then_draws_and_resets_when_rechecking() {
        let ctx = egui::Context::default();
        let frame = |time, status| {
            let mut progress = 0.0;
            let _ = ctx.run(
                egui::RawInput {
                    time: Some(time),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        progress = ready_check_progress(ui, egui::Id::new("check"), status);
                    });
                },
            );
            progress
        };
        assert_eq!(frame(0.0, CardStatus::Ready), 0.0);
        assert_eq!(frame(0.1, CardStatus::Ready), 0.0);
        assert!((frame(0.3, CardStatus::Ready) - 0.5).abs() < 0.001);
        assert_eq!(frame(0.5, CardStatus::Ready), 1.0);
        assert_eq!(frame(0.6, CardStatus::Checking), 0.0);
        assert_eq!(frame(0.7, CardStatus::Ready), 0.0);
    }
}
