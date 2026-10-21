use super::*;

/// Accent of the destructive checkboxes (`accent-color: #d9534f` in the mockups).
pub(in crate::ui) const CHECK_RED: Color32 = Color32::from_rgb(0xd9, 0x53, 0x4f);

/// Footer, title bar and the dialog's own height, from the mockups.
pub(in crate::ui) const BAR_HEIGHT: f32 = 61.0;

/// A padded band of a dialog that is at least `height` tall, borders included.
pub(in crate::ui) fn band<R>(
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

/// A full-width 1pt hairline.
pub(in crate::ui) fn rule(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::ZERO, theme::palette().border);
}

/// Title bar: 16pt title on the left, `right` (a close button or a control) on the
/// right, padding 14/16/14/20 and a hairline below.
pub(in crate::ui) fn title_bar(ui: &mut egui::Ui, title: &str, right: impl FnOnce(&mut egui::Ui)) {
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
pub(in crate::ui) fn close_button(ui: &mut egui::Ui) -> egui::Response {
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
pub(in crate::ui) fn footer(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
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
pub(in crate::ui) fn action(
    ui: &mut egui::Ui,
    text: &str,
    kind: Kind,
    enabled: bool,
) -> egui::Response {
    btn(text).kind(kind).medium().enabled(enabled).show(ui)
}

/// Body text of a dialog: 14pt with a 1.5 line height.
pub(super) fn paragraph(ui: &mut egui::Ui, text: &str) {
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
pub(super) fn header(
    ui: &mut egui::Ui,
    art: Art,
    title: &str,
    text: &str,
    more: impl FnOnce(&mut egui::Ui),
) {
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

pub(super) enum Art {
    Texture(egui::TextureId),
    /// Icon, tile fill, icon colour and tile size (40 or 48).
    Icon(Icon, Color32, Color32, f32),
    None,
}

/// Padding of a dialog body without a title bar.
pub(super) const BODY: egui::Margin = egui::Margin::same(24);

pub(super) fn note(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.add(egui::Label::new(RichText::new(text).size(12.0).color(theme::palette().muted)).wrap());
}

/// Crossfade changing metadata without collapsing its row or delaying interaction.
pub(super) fn install_detail(
    ui: &mut egui::Ui,
    id: egui::Id,
    text: &str,
    size: f32,
    color: Color32,
) {
    let now = ui.input(|input| input.time);
    let (previous, current, started) = ui.ctx().data_mut(|data| {
        let state = data.get_temp::<(String, String, f64)>(id);
        let next = match state {
            Some((_, current, _)) if current != text => (current, text.to_owned(), now),
            Some(state) => state,
            None => (text.to_owned(), text.to_owned(), now - 1.0),
        };
        data.insert_temp(id, next.clone());
        next
    });
    let progress = ((now - started) / 0.14).clamp(0.0, 1.0) as f32;
    let alpha = progress * progress * (3.0 - 2.0 * progress);
    let font = FontId::proportional(size);
    let old = ui
        .painter()
        .layout(previous, font.clone(), color, ui.available_width());
    let new = ui
        .painter()
        .layout(current, font, color, ui.available_width());
    let height = egui::lerp(old.size().y..=new.size().y, alpha);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    if progress < 1.0 {
        let mut painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        painter.multiply_opacity(1.0 - alpha);
        painter.galley(rect.min, old, color);
        ui.ctx().request_repaint();
    }
    let mut painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    painter.multiply_opacity(alpha);
    painter.galley(rect.min, new, color);
}

pub(super) fn danger() -> (Icon, Color32, Color32) {
    (Icon::Trash, theme::palette().red_bg, theme::palette().red)
}

/// Field look of the mockups' inputs and selects: dark fill, 1pt #33363d outline,
/// 8pt corners.
pub(in crate::ui) fn field_style(ui: &mut egui::Ui) {
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
