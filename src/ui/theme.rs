//! Colours, icons and the small set of widgets the manager window is built from.
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Response, Sense, Stroke, StrokeKind,
    Ui, Vec2,
};
use std::time::Duration;

pub const BG: Color32 = Color32::from_rgb(0x13, 0x14, 0x17);
pub const PANEL: Color32 = Color32::from_rgb(0x17, 0x18, 0x1c);
pub const CARD: Color32 = Color32::from_rgb(0x1b, 0x1d, 0x22);
pub const FIELD: Color32 = Color32::from_rgb(0x13, 0x14, 0x17);
pub const LOG: Color32 = Color32::from_rgb(0x10, 0x11, 0x14);
pub const BORDER: Color32 = Color32::from_rgb(0x2a, 0x2d, 0x33);
pub const BORDER_STRONG: Color32 = Color32::from_rgb(0x33, 0x36, 0x3d);
pub const BUTTON: Color32 = Color32::from_rgb(0x1f, 0x21, 0x26);
pub const BUTTON_HOVER: Color32 = Color32::from_rgb(0x28, 0x2b, 0x31);
pub const SELECTED: Color32 = Color32::from_rgb(0x23, 0x2a, 0x37);
pub const HOVER: Color32 = Color32::from_rgb(0x1e, 0x21, 0x27);
pub const TEXT: Color32 = Color32::from_rgb(0xec, 0xec, 0xee);
pub const TEXT_2: Color32 = Color32::from_rgb(0xc9, 0xcc, 0xd2);
pub const TEXT_3: Color32 = Color32::from_rgb(0xb3, 0xb7, 0xbe);
pub const MUTED: Color32 = Color32::from_rgb(0x8f, 0x94, 0x9c);
pub const ACCENT: Color32 = Color32::from_rgb(0x25, 0x63, 0xeb);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(0x3a, 0x74, 0xf0);
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(0x1d, 0x2d, 0x4f);
pub const ACCENT_BORDER: Color32 = Color32::from_rgb(0x2f, 0x4a, 0x7d);
pub const ACCENT_TEXT: Color32 = Color32::from_rgb(0xa9, 0xc4, 0xff);
pub const PROGRESS_BG: Color32 = Color32::from_rgb(0x17, 0x22, 0x38);
pub const PROGRESS_TRACK: Color32 = Color32::from_rgb(0x2a, 0x35, 0x50);
pub const LINK: Color32 = Color32::from_rgb(0x8f, 0xb4, 0xff);
pub const GREEN: Color32 = Color32::from_rgb(0x6f, 0xcf, 0x97);
pub const RED: Color32 = Color32::from_rgb(0xf2, 0x77, 0x7d);
pub const RED_TEXT: Color32 = Color32::from_rgb(0xf0, 0xb9, 0xbc);
pub const RED_BG: Color32 = Color32::from_rgb(0x2a, 0x17, 0x19);
pub const RED_BORDER: Color32 = Color32::from_rgb(0x6a, 0x32, 0x36);
pub const DANGER: Color32 = Color32::from_rgb(0xb3, 0x36, 0x3b);
pub const AMBER: Color32 = Color32::from_rgb(0xe6, 0xb4, 0x64);

pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.animation_time = 0.18;
    let v = &mut style.visuals;
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.window_stroke = Stroke::new(1.0, Color32::from_rgb(0x2f, 0x32, 0x38));
    v.window_corner_radius = CornerRadius::same(12);
    v.extreme_bg_color = FIELD;
    v.faint_bg_color = CARD;
    v.code_bg_color = LOG;
    v.hyperlink_color = LINK;
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.override_text_color = None;
    for (w, fill, stroke) in [
        (&mut v.widgets.noninteractive, PANEL, BORDER),
        (&mut v.widgets.inactive, BUTTON, BORDER_STRONG),
        (
            &mut v.widgets.hovered,
            BUTTON_HOVER,
            Color32::from_rgb(0x45, 0x49, 0x52),
        ),
        (
            &mut v.widgets.active,
            Color32::from_rgb(0x30, 0x34, 0x3c),
            Color32::from_rgb(0x50, 0x55, 0x5f),
        ),
        (&mut v.widgets.open, BUTTON_HOVER, BORDER_STRONG),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.bg_stroke = Stroke::new(1.0, stroke);
        w.corner_radius = CornerRadius::same(5);
    }
    v.widgets.noninteractive.fg_stroke.color = TEXT_2;
    v.widgets.inactive.fg_stroke.color = TEXT;
    v.widgets.hovered.fg_stroke.color = Color32::WHITE;
    v.widgets.active.fg_stroke.color = Color32::WHITE;
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.spacing.interact_size.y = 28.0;
    style.spacing.icon_width = 16.0;
    style.spacing.icon_width_inner = 9.0;
    style.spacing.combo_height = 300.0;
    style.spacing.scroll.bar_width = 8.0;
    style.spacing.scroll.floating = true;
    style
        .text_styles
        .insert(egui::TextStyle::Body, FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Small, FontId::proportional(12.0));
    style
        .text_styles
        .insert(egui::TextStyle::Heading, FontId::proportional(17.0));
    style
        .text_styles
        .insert(egui::TextStyle::Monospace, FontId::monospace(12.0));
    ctx.set_style(style);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Gear,
    Search,
    Grid,
    Download,
    Refresh,
    Play,
    Trash,
    ArrowUp,
    Close,
    Sliders,
    Archive,
    Code,
    Wrench,
    Alert,
    Folder,
    Check,
}

/// Strokes an icon drawn on the 24-unit grid the mockups use, scaled into `rect`.
pub fn paint_icon(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let scale = rect.width().min(rect.height()) / 24.0;
    let origin = rect.center() - Vec2::splat(12.0 * scale);
    let p = |x: f32, y: f32| origin + egui::vec2(x, y) * scale;
    let stroke = Stroke::new((2.0 * scale).max(1.2), color);
    let line = |points: Vec<Pos2>| {
        painter.add(egui::Shape::line(points, stroke));
    };
    let arc = |cx: f32, cy: f32, r: f32, from: f32, to: f32| -> Vec<Pos2> {
        let steps = ((to - from).abs() / 12.0).ceil().max(2.0) as usize;
        (0..=steps)
            .map(|i| {
                let a = (from + (to - from) * i as f32 / steps as f32).to_radians();
                p(cx + r * a.cos(), cy + r * a.sin())
            })
            .collect()
    };
    match icon {
        Icon::Gear => {
            painter.circle_stroke(p(12.0, 12.0), 3.0 * scale, stroke);
            painter.circle_stroke(p(12.0, 12.0), 7.0 * scale, stroke);
            for i in 0..8 {
                let a = (i as f32 * 45.0).to_radians();
                line(vec![
                    p(12.0 + 7.0 * a.cos(), 12.0 + 7.0 * a.sin()),
                    p(12.0 + 10.0 * a.cos(), 12.0 + 10.0 * a.sin()),
                ]);
            }
        }
        Icon::Search => {
            painter.circle_stroke(p(11.0, 11.0), 7.5 * scale, stroke);
            line(vec![p(16.5, 16.5), p(21.0, 21.0)]);
        }
        Icon::Grid => {
            for (x, y) in [(3.0, 3.0), (14.0, 3.0), (3.0, 14.0), (14.0, 14.0)] {
                painter.rect_stroke(
                    Rect::from_min_max(p(x, y), p(x + 7.0, y + 7.0)),
                    CornerRadius::same((1.5 * scale) as u8),
                    stroke,
                    StrokeKind::Middle,
                );
            }
        }
        Icon::Download => {
            line(vec![
                p(3.0, 15.0),
                p(3.0, 19.0),
                p(5.0, 21.0),
                p(19.0, 21.0),
                p(21.0, 19.0),
                p(21.0, 15.0),
            ]);
            line(vec![p(7.0, 10.0), p(12.0, 15.0), p(17.0, 10.0)]);
            line(vec![p(12.0, 15.0), p(12.0, 3.0)]);
        }
        Icon::Refresh => {
            line(arc(12.0, 12.0, 9.0, 0.0, 312.0));
            line(vec![p(18.0, 5.3), p(21.0, 8.0)]);
            line(vec![p(21.0, 3.0), p(21.0, 8.0), p(16.0, 8.0)]);
        }
        Icon::Play => {
            painter.add(egui::Shape::closed_line(
                vec![p(6.0, 3.0), p(20.0, 12.0), p(6.0, 21.0)],
                stroke,
            ));
        }
        Icon::Trash => {
            line(vec![p(3.0, 6.0), p(21.0, 6.0)]);
            line(vec![
                p(19.0, 6.0),
                p(19.0, 20.0),
                p(17.0, 22.0),
                p(7.0, 22.0),
                p(5.0, 20.0),
                p(5.0, 6.0),
            ]);
            line(vec![
                p(8.0, 6.0),
                p(8.0, 4.0),
                p(10.0, 2.0),
                p(14.0, 2.0),
                p(16.0, 4.0),
                p(16.0, 6.0),
            ]);
        }
        Icon::ArrowUp => {
            line(vec![p(12.0, 19.0), p(12.0, 5.0)]);
            line(vec![p(5.0, 12.0), p(12.0, 5.0), p(19.0, 12.0)]);
        }
        Icon::Close => {
            line(vec![p(18.0, 6.0), p(6.0, 18.0)]);
            line(vec![p(6.0, 6.0), p(18.0, 18.0)]);
        }
        Icon::Sliders => {
            for (x, a, b, c) in [
                (4.0, 21.0, 14.0, 10.0),
                (12.0, 21.0, 12.0, 8.0),
                (20.0, 21.0, 16.0, 12.0),
            ] {
                line(vec![p(x, a), p(x, b)]);
                line(vec![p(x, c), p(x, 3.0)]);
            }
            line(vec![p(1.0, 14.0), p(7.0, 14.0)]);
            line(vec![p(9.0, 8.0), p(15.0, 8.0)]);
            line(vec![p(17.0, 16.0), p(23.0, 16.0)]);
        }
        Icon::Archive => {
            painter.rect_stroke(
                Rect::from_min_max(p(2.0, 3.0), p(22.0, 8.0)),
                CornerRadius::same(scale as u8),
                stroke,
                StrokeKind::Middle,
            );
            line(vec![
                p(4.0, 8.0),
                p(4.0, 19.0),
                p(6.0, 21.0),
                p(18.0, 21.0),
                p(20.0, 19.0),
                p(20.0, 8.0),
            ]);
            line(vec![p(10.0, 12.0), p(14.0, 12.0)]);
        }
        Icon::Code => {
            line(vec![p(16.0, 18.0), p(22.0, 12.0), p(16.0, 6.0)]);
            line(vec![p(8.0, 6.0), p(2.0, 12.0), p(8.0, 18.0)]);
        }
        Icon::Wrench => {
            line(arc(15.5, 8.5, 5.0, 100.0, 350.0));
            line(vec![p(12.0, 12.0), p(4.5, 19.5)]);
            line(vec![p(6.0, 21.0), p(3.0, 18.0)]);
        }
        Icon::Alert => {
            painter.circle_stroke(p(12.0, 12.0), 10.0 * scale, stroke);
            line(vec![p(12.0, 8.0), p(12.0, 12.5)]);
            painter.circle_filled(p(12.0, 16.0), 1.2 * scale, color);
        }
        Icon::Folder => {
            painter.add(egui::Shape::closed_line(
                vec![
                    p(2.0, 5.0),
                    p(9.0, 5.0),
                    p(11.0, 7.5),
                    p(22.0, 7.5),
                    p(22.0, 20.0),
                    p(2.0, 20.0),
                ],
                stroke,
            ));
        }
        Icon::Check => line(vec![p(20.0, 6.0), p(9.0, 17.0), p(4.0, 12.0)]),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Primary,
    Secondary,
    Danger,
    Soft,
    Ghost,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Large,
    Medium,
    Small,
    Pill,
    Chip,
}

pub struct Btn<'a> {
    text: &'a str,
    kind: Kind,
    size: Size,
    icon: Option<(Icon, Option<Color32>)>,
    enabled: bool,
    min_width: f32,
}

pub fn btn(text: &str) -> Btn<'_> {
    Btn {
        text,
        kind: Kind::Secondary,
        size: Size::Small,
        icon: None,
        enabled: true,
        min_width: 0.0,
    }
}

impl<'a> Btn<'a> {
    pub fn kind(mut self, kind: Kind) -> Self {
        self.kind = kind;
        self
    }
    pub fn primary(self) -> Self {
        self.kind(Kind::Primary)
    }
    pub fn size(mut self, size: Size) -> Self {
        self.size = size;
        self
    }
    pub fn large(self) -> Self {
        self.size(Size::Large)
    }
    pub fn medium(self) -> Self {
        self.size(Size::Medium)
    }
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some((icon, None));
        self
    }
    pub fn icon_colored(mut self, icon: Icon, color: Color32) -> Self {
        self.icon = Some((icon, Some(color)));
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = width;
        self
    }
    pub fn show(self, ui: &mut Ui) -> Response {
        let (height, font, pad, radius, icon_size) = match self.size {
            Size::Large => (44.0, 15.0, 20.0, 8, 18.0),
            Size::Medium => (38.0, 14.0, 16.0, 8, 16.0),
            Size::Small => (32.0, 13.0, 12.0, 8, 14.0),
            Size::Pill => (24.0, 12.0, 10.0, 12, 12.0),
            Size::Chip => (24.0, 11.0, 10.0, 12, 11.0),
        };
        let font = FontId::proportional(font);
        let text_color = match self.kind {
            Kind::Primary | Kind::Danger => Color32::WHITE,
            Kind::Soft => Color32::from_rgb(0xcf, 0xe0, 0xff),
            _ => TEXT,
        };
        let galley = ui
            .painter()
            .layout_no_wrap(self.text.to_owned(), font, text_color);
        let icon_space = if self.icon.is_some() {
            icon_size + if self.text.is_empty() { 0.0 } else { 7.0 }
        } else {
            0.0
        };
        let width = (galley.size().x + icon_space + pad * 2.0).max(self.min_width);
        let sense = if self.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), sense);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, self.enabled, self.text)
        });
        if ui.is_rect_visible(rect) {
            let hovered = self.enabled && response.hovered();
            let pressed = self.enabled && response.is_pointer_button_down_on();
            let (fill, stroke) = match self.kind {
                Kind::Primary => (if hovered { ACCENT_HOVER } else { ACCENT }, Stroke::NONE),
                Kind::Danger => (
                    if hovered {
                        Color32::from_rgb(0xc4, 0x44, 0x49)
                    } else {
                        DANGER
                    },
                    Stroke::NONE,
                ),
                Kind::Soft => (
                    if hovered {
                        Color32::from_rgb(0x24, 0x38, 0x61)
                    } else {
                        ACCENT_SOFT
                    },
                    Stroke::new(1.0, ACCENT_BORDER),
                ),
                Kind::Secondary => (
                    if hovered { BUTTON_HOVER } else { BUTTON },
                    Stroke::new(
                        1.0,
                        if self.size == Size::Large || self.size == Size::Medium {
                            Color32::from_rgb(0x3a, 0x3d, 0x44)
                        } else {
                            BORDER_STRONG
                        },
                    ),
                ),
                Kind::Ghost => (
                    if hovered {
                        BUTTON_HOVER
                    } else {
                        Color32::TRANSPARENT
                    },
                    if self.size == Size::Chip {
                        Stroke::new(1.0, Color32::from_rgb(0x3a, 0x3d, 0x44))
                    } else {
                        Stroke::NONE
                    },
                ),
            };
            let fill = if pressed {
                fill.gamma_multiply(0.85)
            } else if !self.enabled
                && matches!(self.kind, Kind::Primary | Kind::Danger | Kind::Soft)
            {
                fill.gamma_multiply(0.45)
            } else {
                fill
            };
            let painter = ui.painter();
            painter.rect(
                rect,
                CornerRadius::same(radius),
                fill,
                stroke,
                StrokeKind::Inside,
            );
            if response.has_focus() {
                painter.rect_stroke(
                    rect.expand(2.0),
                    CornerRadius::same(radius + 2),
                    Stroke::new(2.0, LINK),
                    StrokeKind::Outside,
                );
            }
            let content = icon_space + galley.size().x;
            let mut x = rect.center().x - content / 2.0;
            let alpha = if self.enabled { 1.0 } else { 0.4 };
            if let Some((icon, color)) = self.icon {
                let icon_rect = Rect::from_center_size(
                    egui::pos2(x + icon_size / 2.0, rect.center().y),
                    Vec2::splat(icon_size),
                );
                paint_icon(
                    painter,
                    icon_rect,
                    icon,
                    color.unwrap_or(text_color).gamma_multiply(alpha),
                );
                x += icon_space;
            }
            painter.galley(
                egui::pos2(x, rect.center().y - galley.size().y / 2.0),
                galley,
                text_color.gamma_multiply(alpha),
            );
        }
        if self.enabled && response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        response
    }
}

/// A square icon-only button, used for close and dismiss controls.
pub fn icon_button(ui: &mut Ui, icon: Icon, label: &str, color: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(8), BUTTON_HOVER);
    }
    paint_icon(
        ui.painter(),
        Rect::from_center_size(rect.center(), Vec2::splat(16.0)),
        icon,
        color,
    );
    response.on_hover_text(label)
}

pub fn text(ui: &mut Ui, text: impl Into<String>, size: f32, color: Color32) -> Response {
    ui.label(egui::RichText::new(text).size(size).color(color))
}

pub fn heading(ui: &mut Ui, text: impl Into<String>, size: f32) -> Response {
    ui.label(egui::RichText::new(text).font(bold(size)).color(TEXT))
}

/// Uppercase, letter-spaced group label such as "INSTALLED" or "ACTIVITY".
pub fn section_label(ui: &mut Ui, text: &str) -> Response {
    ui.label(
        egui::RichText::new(text.to_uppercase())
            .size(11.5)
            .color(MUTED)
            .extra_letter_spacing(0.7),
    )
}

pub fn link(ui: &mut Ui, text: impl Into<String>) -> Response {
    let response = ui.add(
        egui::Label::new(egui::RichText::new(text).size(13.0).color(LINK)).sense(Sense::click()),
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

pub fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(12))
}

pub fn group() -> egui::Frame {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(10))
}

/// Paints a full-width hairline at the current cursor, used between rows of a group.
pub fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::ZERO, BORDER);
}

pub fn dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

/// A thin progress bar; `None` animates an indeterminate segment.
pub fn progress(ui: &mut Ui, progress: Option<f32>, height: f32) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    let radius = CornerRadius::same((height / 2.0) as u8);
    ui.painter().rect_filled(rect, radius, PROGRESS_TRACK);
    let fill = match progress {
        Some(value) => Rect::from_min_size(
            rect.min,
            egui::vec2(rect.width() * value.clamp(0.0, 1.0), height),
        ),
        None => {
            let time = ui.ctx().input(|i| i.time);
            let position = ((1.0 - (time * std::f64::consts::TAU / 2.8).cos()) * 0.5) as f32;
            let width = rect.width() * 0.22;
            ui.ctx().request_repaint_after(Duration::from_millis(16));
            Rect::from_min_size(
                rect.min + egui::vec2((rect.width() - width) * position, 0.0),
                egui::vec2(width, height),
            )
        }
    };
    if fill.width() > 0.0 {
        ui.painter().rect_filled(fill, radius, ACCENT);
    }
}

/// A rounded badge with a number or short word, e.g. the update count next to Overview.
pub fn badge(ui: &mut Ui, text: &str, fill: Color32, color: Color32) -> Response {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), FontId::proportional(11.0), color);
    let size = egui::vec2((galley.size().x + 12.0).max(20.0), 20.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(10), fill);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, color);
    response
}

/// Two-option segmented control. Returns true when the value changed.
pub fn segmented<T: PartialEq + Clone>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let mut changed = false;
    egui::Frame::new()
        .fill(FIELD)
        .stroke(Stroke::new(1.0, Color32::from_rgb(0x2f, 0x32, 0x38)))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::same(3))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.horizontal(|ui| {
                for (option, label) in options {
                    let selected = value == option;
                    let galley = ui.painter().layout_no_wrap(
                        (*label).to_owned(),
                        FontId::proportional(13.0),
                        TEXT,
                    );
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(galley.size().x + 24.0, 28.0),
                        Sense::click(),
                    );
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::SelectableLabel,
                            true,
                            selected,
                            *label,
                        )
                    });
                    if selected || response.hovered() {
                        ui.painter().rect_filled(
                            rect,
                            CornerRadius::same(6),
                            if selected {
                                Color32::from_rgb(0x2b, 0x2f, 0x37)
                            } else {
                                HOVER
                            },
                        );
                    }
                    ui.painter().galley(
                        rect.center() - galley.size() / 2.0,
                        galley,
                        if selected { TEXT } else { TEXT_3 },
                    );
                    if response.clicked() && !selected {
                        *value = option.clone();
                        changed = true;
                    }
                }
            });
        });
    changed
}

/// One row of a settings or tools group: title and optional detail on the left,
/// the control on the right.
pub fn row<R>(
    ui: &mut Ui,
    title: &str,
    detail: Option<&str>,
    control: impl FnOnce(&mut Ui) -> R,
) -> R {
    let mut out = None;
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.set_min_height(32.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    out = Some(control(ui));
                    ui.add_space(12.0);
                    // A lone title is one widget, so the row centres it beside the control.
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        match detail {
                            Some(detail) => {
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 2.0;
                                    text(ui, title, 14.0, TEXT);
                                    text(ui, detail, 12.0, MUTED);
                                });
                            }
                            None => {
                                text(ui, title, 14.0, TEXT);
                            }
                        }
                    });
                });
            });
        });
    out.expect("row control rendered")
}

/// A checkbox that lives on the right side of a settings row.
pub fn switch(ui: &mut Ui, on: &mut bool, label: &str, enabled: bool) -> Response {
    let checked = *on;
    let response = ui.add_enabled(enabled, egui::Checkbox::without_text(on));
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, checked, label)
    });
    response
}

pub fn painter_text(
    painter: &egui::Painter,
    pos: Pos2,
    anchor: Align2,
    text: impl ToString,
    size: f32,
    color: Color32,
) -> Rect {
    painter.text(pos, anchor, text, FontId::proportional(size), color)
}

static SYSTEM_FONT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn set_system_font(loaded: bool) {
    SYSTEM_FONT.store(loaded, std::sync::atomic::Ordering::Relaxed);
}
/// "→" when the system font is loaded; the bundled font has no arrow glyph.
pub fn arrow() -> &'static str {
    if SYSTEM_FONT.load(std::sync::atomic::Ordering::Relaxed) {
        "→"
    } else {
        "›"
    }
}
/// Semibold text for headings, falling back to the regular face.
pub fn bold(size: f32) -> FontId {
    FontId::new(size, egui::FontFamily::Name("bold".into()))
}
