//! Colours, icons and the small set of widgets the manager window is built from.
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Response, Sense, Stroke, StrokeKind,
    Ui, Vec2,
};
use std::time::Duration;

use craft_apps_manager::model::Theme;

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub panel: Color32,
    pub card: Color32,
    pub field: Color32,
    pub log: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub button: Color32,
    pub button_hover: Color32,
    pub selected: Color32,
    pub hover: Color32,
    pub text: Color32,
    pub text_2: Color32,
    pub text_3: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_soft: Color32,
    pub accent_border: Color32,
    pub accent_text: Color32,
    pub progress_bg: Color32,
    pub progress_track: Color32,
    pub link: Color32,
    pub green: Color32,
    pub red: Color32,
    pub red_text: Color32,
    pub red_bg: Color32,
    pub red_border: Color32,
    pub danger: Color32,
    pub amber: Color32,
    pub control_border: Color32,
}
// Neutral colors use exact Adobe Spectrum gray tokens:
// https://opensource.adobe.com/spectrum-design-data/tokens/color-palette/
const DARK: Palette = Palette {
    bg: Color32::from_rgb(17, 17, 17),
    panel: Color32::from_rgb(27, 27, 27),
    card: Color32::from_rgb(34, 34, 34),
    field: Color32::from_rgb(17, 17, 17),
    log: Color32::from_rgb(17, 17, 17),
    border: Color32::from_rgb(50, 50, 50),
    border_strong: Color32::from_rgb(68, 68, 68),
    button: Color32::from_rgb(34, 34, 34),
    button_hover: Color32::from_rgb(50, 50, 50),
    selected: Color32::from_rgb(0x23, 0x2a, 0x37),
    hover: Color32::from_rgb(44, 44, 44),
    text: Color32::from_rgb(242, 242, 242),
    text_2: Color32::from_rgb(219, 219, 219),
    text_3: Color32::from_rgb(175, 175, 175),
    muted: Color32::from_rgb(175, 175, 175),
    accent: Color32::from_rgb(0x25, 0x63, 0xeb),
    accent_hover: Color32::from_rgb(0x3a, 0x74, 0xf0),
    accent_soft: Color32::from_rgb(0x1d, 0x2d, 0x4f),
    accent_border: Color32::from_rgb(0x2f, 0x4a, 0x7d),
    accent_text: Color32::from_rgb(0xa9, 0xc4, 0xff),
    progress_bg: Color32::from_rgb(0x17, 0x22, 0x38),
    progress_track: Color32::from_rgb(0x2a, 0x35, 0x50),
    link: Color32::from_rgb(0x8f, 0xb4, 0xff),
    green: Color32::from_rgb(0x6f, 0xcf, 0x97),
    red: Color32::from_rgb(0xf2, 0x77, 0x7d),
    red_text: Color32::from_rgb(0xf0, 0xb9, 0xbc),
    red_bg: Color32::from_rgb(0x2a, 0x17, 0x19),
    red_border: Color32::from_rgb(0x6a, 0x32, 0x36),
    danger: Color32::from_rgb(0xb3, 0x36, 0x3b),
    amber: Color32::from_rgb(0xe6, 0xb4, 0x64),
    control_border: Color32::from_rgb(138, 138, 138),
};
const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(248, 248, 248),
    panel: Color32::from_rgb(255, 255, 255),
    card: Color32::from_rgb(243, 243, 243),
    field: Color32::from_rgb(243, 243, 243),
    log: Color32::from_rgb(248, 248, 248),
    border: Color32::from_rgb(225, 225, 225),
    border_strong: Color32::from_rgb(198, 198, 198),
    button: Color32::from_rgb(243, 243, 243),
    button_hover: Color32::from_rgb(225, 225, 225),
    selected: Color32::from_rgb(0xe3, 0xed, 0xfc),
    hover: Color32::from_rgb(233, 233, 233),
    text: Color32::from_rgb(19, 19, 19),
    text_2: Color32::from_rgb(41, 41, 41),
    text_3: Color32::from_rgb(80, 80, 80),
    muted: Color32::from_rgb(80, 80, 80),
    accent: Color32::from_rgb(0x25, 0x63, 0xeb),
    accent_hover: Color32::from_rgb(0x1d, 0x4e, 0xd8),
    accent_soft: Color32::from_rgb(0xe5, 0xed, 0xfc),
    accent_border: Color32::from_rgb(0xa6, 0xc2, 0xf2),
    accent_text: Color32::from_rgb(0x19, 0x4f, 0xb4),
    progress_bg: Color32::from_rgb(0xee, 0xf3, 0xfc),
    progress_track: Color32::from_rgb(0xd4, 0xe1, 0xf7),
    link: Color32::from_rgb(0x1d, 0x56, 0xbf),
    green: Color32::from_rgb(0x20, 0x78, 0x44),
    red: Color32::from_rgb(0xb8, 0x2f, 0x3b),
    red_text: Color32::from_rgb(0x9c, 0x25, 0x30),
    red_bg: Color32::from_rgb(0xff, 0xf0, 0xf1),
    red_border: Color32::from_rgb(0xe8, 0xaa, 0xb0),
    danger: Color32::from_rgb(0xb3, 0x36, 0x3b),
    amber: Color32::from_rgb(0x8a, 0x5a, 0x0d),
    control_border: Color32::from_rgb(113, 113, 113),
};
// Paint helpers run on the UI thread. Re-select the palette for the active
// window each frame, so custom widgets and egui use the same theme.
thread_local! {
    static CURRENT: std::cell::Cell<Palette> = const { std::cell::Cell::new(DARK) };
}
pub fn palette() -> Palette {
    CURRENT.with(std::cell::Cell::get)
}

impl Palette {
    fn blend(self, target: Self, amount: f32) -> Self {
        let blend = |a: Color32, b: Color32| {
            let a = a.to_array();
            let b = b.to_array();
            let channel =
                |i: usize| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * amount).round() as u8;
            Color32::from_rgba_premultiplied(channel(0), channel(1), channel(2), channel(3))
        };
        Self {
            bg: blend(self.bg, target.bg),
            panel: blend(self.panel, target.panel),
            card: blend(self.card, target.card),
            field: blend(self.field, target.field),
            log: blend(self.log, target.log),
            border: blend(self.border, target.border),
            border_strong: blend(self.border_strong, target.border_strong),
            button: blend(self.button, target.button),
            button_hover: blend(self.button_hover, target.button_hover),
            selected: blend(self.selected, target.selected),
            hover: blend(self.hover, target.hover),
            text: blend(self.text, target.text),
            text_2: blend(self.text_2, target.text_2),
            text_3: blend(self.text_3, target.text_3),
            muted: blend(self.muted, target.muted),
            accent: blend(self.accent, target.accent),
            accent_hover: blend(self.accent_hover, target.accent_hover),
            accent_soft: blend(self.accent_soft, target.accent_soft),
            accent_border: blend(self.accent_border, target.accent_border),
            accent_text: blend(self.accent_text, target.accent_text),
            progress_bg: blend(self.progress_bg, target.progress_bg),
            progress_track: blend(self.progress_track, target.progress_track),
            link: blend(self.link, target.link),
            green: blend(self.green, target.green),
            red: blend(self.red, target.red),
            red_text: blend(self.red_text, target.red_text),
            red_bg: blend(self.red_bg, target.red_bg),
            red_border: blend(self.red_border, target.red_border),
            danger: blend(self.danger, target.danger),
            amber: blend(self.amber, target.amber),
            control_border: blend(self.control_border, target.control_border),
        }
    }
}

#[derive(Clone, Copy)]
struct ThemeTransition {
    mode: Theme,
    from: Palette,
    started: f64,
}
pub fn apply(ctx: &egui::Context, mode: Theme) {
    let id = egui::Id::new("manager-visual-theme");
    let now = ctx.input(|input| input.time);
    let target = if mode == Theme::Light { LIGHT } else { DARK };
    let previous = ctx.data(|data| data.get_temp::<ThemeTransition>(id));
    let transition = match previous {
        Some(state) if state.mode == mode => state,
        Some(state) => {
            let elapsed = ((now - state.started) / 0.25).clamp(0.0, 1.0) as f32;
            let eased = elapsed * elapsed * (3.0 - 2.0 * elapsed);
            let old_target = if state.mode == Theme::Light {
                LIGHT
            } else {
                DARK
            };
            ThemeTransition {
                mode,
                from: state.from.blend(old_target, eased),
                started: now,
            }
        }
        None => ThemeTransition {
            mode,
            from: target,
            started: now - 0.25,
        },
    };
    let amount = ((now - transition.started) / 0.25).clamp(0.0, 1.0) as f32;
    let eased = amount * amount * (3.0 - 2.0 * amount);
    CURRENT.with(|palette| palette.set(transition.from.blend(target, eased)));
    let settled_id = id.with("settled");
    let settled = ctx
        .data(|data| data.get_temp::<bool>(settled_id))
        .unwrap_or(false);
    ctx.data_mut(|data| {
        data.insert_temp(id, transition);
        data.insert_temp(settled_id, amount >= 1.0);
    });
    if settled && amount >= 1.0 && previous.is_some_and(|state| state.mode == mode) {
        return;
    }
    if amount < 1.0 {
        ctx.request_repaint();
    }
    if previous.is_none_or(|state| state.mode != mode) {
        ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(if mode == Theme::Light {
            egui::SystemTheme::Light
        } else {
            egui::SystemTheme::Dark
        }));
    }
    let mut style = (*ctx.style()).clone();
    style.visuals = if mode == Theme::Light {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    style.animation_time = 0.18;
    let v = &mut style.visuals;
    v.panel_fill = palette().bg;
    v.window_fill = palette().panel;
    v.window_stroke = Stroke::new(1.0_f32, palette().border_strong);
    v.window_corner_radius = CornerRadius::same(12);
    v.extreme_bg_color = palette().field;
    v.faint_bg_color = palette().card;
    v.code_bg_color = palette().log;
    v.hyperlink_color = palette().link;
    v.selection.bg_fill = palette().accent;
    v.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
    v.override_text_color = None;
    for (w, fill, stroke) in [
        (
            &mut v.widgets.noninteractive,
            palette().panel,
            palette().border,
        ),
        (
            &mut v.widgets.inactive,
            palette().button,
            palette().border_strong,
        ),
        (
            &mut v.widgets.hovered,
            palette().button_hover,
            palette().border_strong,
        ),
        (
            &mut v.widgets.active,
            palette().selected,
            palette().border_strong,
        ),
        (
            &mut v.widgets.open,
            palette().button_hover,
            palette().border_strong,
        ),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.bg_stroke = Stroke::new(1.0_f32, stroke);
        w.corner_radius = CornerRadius::same(5);
    }
    v.widgets.noninteractive.fg_stroke.color = palette().text_2;
    v.widgets.inactive.fg_stroke.color = palette().text;
    v.widgets.hovered.fg_stroke.color = palette().text;
    v.widgets.active.fg_stroke.color = palette().text;
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
    Sun,
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

pub fn theme_toggle(ui: &mut Ui, mode: Theme) -> Response {
    let label = if mode == Theme::Dark {
        "Switch to light theme"
    } else {
        "Switch to dark theme"
    };
    let (hit_rect, response) = ui.allocate_exact_size(egui::vec2(56.0, 36.0), Sense::click());
    let rect = Rect::from_center_size(hit_rect.center(), egui::vec2(48.0, 20.0));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let light = mode == Theme::Light;
    let position = ui
        .ctx()
        .animate_bool_with_time(response.id.with("theme-thumb"), light, 0.18);
    let track = if light {
        Color32::from_gray(if response.hovered() { 248 } else { 255 })
    } else {
        Color32::from_gray(if response.hovered() { 57 } else { 50 })
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(10),
        track,
        Stroke::new(1.0_f32, palette().border_strong),
        StrokeKind::Inside,
    );
    let center = egui::pos2(
        egui::lerp((rect.left() + 10.0)..=(rect.right() - 10.0), position),
        rect.center().y,
    );
    let thumb = if light {
        Color32::from_gray(41)
    } else {
        Color32::from_gray(242)
    };
    let ink = if light {
        Color32::WHITE
    } else {
        Color32::from_gray(27)
    };
    ui.painter().circle_filled(
        center + egui::vec2(0.0, 1.0),
        8.5,
        Color32::from_black_alpha(28),
    );
    ui.painter().circle_filled(center, 8.0, thumb);
    if light {
        ui.painter().circle_filled(center, 5.0, ink);
        ui.painter()
            .circle_filled(center + egui::vec2(2.5, -2.0), 4.5, thumb);
    } else {
        paint_icon(
            ui.painter(),
            Rect::from_center_size(center, Vec2::splat(14.0)),
            Icon::Sun,
            ink,
        );
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(2.0),
            CornerRadius::same(12),
            Stroke::new(1.0_f32, palette().accent),
            StrokeKind::Inside,
        );
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.on_hover_text(label)
}

/// Strokes an icon drawn on the 24-unit grid the mockups use, scaled into `rect`.
pub fn paint_icon(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    paint_icon_weight(painter, rect, icon, color, 2.0);
}

/// Like `paint_icon` with a stroke of `weight` grid units (the mockups use 2, and
/// 2.5 for the small arrow in the update pill).
pub fn paint_icon_weight(
    painter: &egui::Painter,
    rect: Rect,
    icon: Icon,
    color: Color32,
    weight: f32,
) {
    let scale = rect.width().min(rect.height()) / 24.0;
    let origin = rect.center() - Vec2::splat(12.0 * scale);
    let p = |x: f32, y: f32| origin + egui::vec2(x, y) * scale;
    let stroke = Stroke::new((weight * scale).max(1.2), color);
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
        Icon::Sun => {
            painter.circle_stroke(p(12.0, 12.0), 4.0 * scale, stroke);
            for angle in (0..360).step_by(45) {
                let angle = (angle as f32).to_radians();
                line(vec![
                    p(12.0 + 7.0 * angle.cos(), 12.0 + 7.0 * angle.sin()),
                    p(12.0 + 10.0 * angle.cos(), 12.0 + 10.0 * angle.sin()),
                ]);
            }
        }
        Icon::Gear => {
            painter.circle_stroke(p(12.0, 12.0), 3.0 * scale, stroke);
            let outline = GEAR
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&[x, y]| p(x, y))
                .collect();
            painter.add(egui::Shape::closed_line(outline, stroke));
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

/// The outline of Feather's "settings" path on its 24-unit grid, sampled as x, y pairs.
#[rustfmt::skip]
const GEAR: [f32; 288] = [
    19.40, 15.00, 19.27, 15.47, 19.29, 15.96, 19.44, 16.42, 19.73, 16.82, 19.79, 16.88,
    20.19, 17.45, 20.37, 18.12, 20.31, 18.81, 20.01, 19.44, 19.52, 19.93, 18.89, 20.23,
    18.20, 20.29, 17.53, 20.11, 16.96, 19.71, 16.90, 19.65, 16.50, 19.36, 16.04, 19.21,
    15.55, 19.19, 15.08, 19.32, 14.56, 19.68, 14.20, 20.21, 14.08, 20.83, 14.08, 21.00,
    13.96, 21.68, 13.61, 22.29, 13.08, 22.73, 12.43, 22.97, 11.73, 22.97, 11.08, 22.73,
    10.55, 22.29, 10.20, 21.68, 10.08, 21.00, 10.08, 20.91, 10.00, 20.43, 9.77, 19.99,
    9.43, 19.64, 9.00, 19.40, 8.53, 19.27, 8.04, 19.29, 7.58, 19.44, 7.18, 19.73,
    7.12, 19.79, 6.47, 20.22, 5.70, 20.38, 4.94, 20.22, 4.29, 19.79, 3.86, 19.14,
    3.70, 18.37, 3.86, 17.61, 4.29, 16.96, 4.35, 16.90, 4.66, 16.49, 4.82, 16.00,
    4.83, 15.49, 4.68, 15.00, 4.32, 14.48, 3.79, 14.12, 3.17, 14.00, 3.00, 14.00,
    2.32, 13.88, 1.71, 13.53, 1.27, 13.00, 1.03, 12.35, 1.03, 11.65, 1.27, 11.00,
    1.71, 10.47, 2.32, 10.12, 3.00, 10.00, 3.09, 10.00, 3.71, 9.88, 4.24, 9.52,
    4.60, 9.00, 4.73, 8.53, 4.71, 8.04, 4.56, 7.58, 4.27, 7.18, 4.21, 7.12,
    3.81, 6.55, 3.63, 5.88, 3.69, 5.19, 3.99, 4.56, 4.48, 4.07, 5.11, 3.77,
    5.80, 3.71, 6.47, 3.89, 7.04, 4.29, 7.10, 4.35, 7.51, 4.66, 8.00, 4.82,
    8.51, 4.83, 9.00, 4.68, 9.52, 4.32, 9.88, 3.79, 10.00, 3.17, 10.00, 3.00,
    10.12, 2.32, 10.47, 1.71, 11.00, 1.27, 11.65, 1.03, 12.35, 1.03, 13.00, 1.27,
    13.53, 1.71, 13.88, 2.32, 14.00, 3.00, 14.00, 3.09, 14.12, 3.71, 14.48, 4.24,
    15.00, 4.60, 15.47, 4.73, 15.96, 4.71, 16.42, 4.56, 16.82, 4.27, 16.88, 4.21,
    17.45, 3.81, 18.12, 3.63, 18.81, 3.69, 19.44, 3.99, 19.93, 4.48, 20.23, 5.11,
    20.29, 5.80, 20.11, 6.47, 19.71, 7.04, 19.65, 7.10, 19.36, 7.52, 19.22, 8.01,
    19.23, 8.52, 19.40, 9.00, 19.76, 9.52, 20.29, 9.88, 20.91, 10.00, 21.00, 10.00,
    21.68, 10.12, 22.29, 10.47, 22.73, 11.00, 22.97, 11.65, 22.97, 12.35, 22.73, 13.00,
    22.29, 13.53, 21.68, 13.88, 21.00, 14.00, 20.91, 14.00, 20.29, 14.12, 19.76, 14.48,
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Accent fill, semibold white text.
    Primary,
    /// Dark fill with a hairline border.
    Secondary,
    /// Red fill, semibold white text.
    Danger,
    /// Red outline and text on no fill, for destructive actions that open a confirmation.
    DangerOutline,
    /// Blue-tinted, medium weight: the manager "Update available" pill.
    Soft,
    /// Dark-blue tinted, medium weight: the Build button.
    Tinted,
    /// No fill until hovered.
    Ghost,
}

/// Button sizes from the mockups.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Size {
    /// 40pt actions on the Overview cards and the app page, 14pt text.
    Card,
    /// 36pt dialog and window actions, 14pt text.
    Medium,
    /// 36pt top-bar button, 13pt text.
    Toolbar,
    /// 32pt row buttons, 13pt text.
    Small,
    /// 24pt rounded pill, 12pt text.
    Pill,
    /// 24pt rounded chip, 11pt text.
    Chip,
}

pub struct Btn<'a> {
    text: &'a str,
    kind: Kind,
    size: Size,
    height: Option<f32>,
    icon: Option<(Icon, Option<Color32>)>,
    icon_weight: f32,
    enabled: bool,
    min_width: f32,
}

pub fn btn(text: &str) -> Btn<'_> {
    Btn {
        text,
        kind: Kind::Secondary,
        size: Size::Small,
        height: None,
        icon: None,
        icon_weight: 2.0,
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
    pub fn medium(self) -> Self {
        self.size(Size::Medium)
    }
    /// Overrides the size's height, keeping its padding and text.
    pub fn height(mut self, height: f32) -> Self {
        self.height = Some(height);
        self
    }
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some((icon, None));
        self
    }
    pub fn icon_colored(mut self, icon: Icon, color: Color32) -> Self {
        self.icon = Some((icon, Some(color)));
        self
    }
    pub fn icon_weight(mut self, weight: f32) -> Self {
        self.icon_weight = weight;
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

    /// Height, side padding, text size, icon size, icon gap and corner radius.
    fn metrics(&self) -> (f32, f32, f32, f32, f32, u8) {
        let filled = matches!(self.kind, Kind::Primary | Kind::Danger);
        let (height, pad, text, icon, gap, radius) = match self.size {
            Size::Card => (40.0, if filled { 18.0 } else { 16.0 }, 14.0, 16.0, 8.0, 8),
            Size::Medium => (36.0, if filled { 18.0 } else { 16.0 }, 14.0, 16.0, 8.0, 8),
            Size::Toolbar => (36.0, 14.0, 13.0, 16.0, 8.0, 8),
            Size::Small => (32.0, 12.0, 13.0, 14.0, 8.0, 8),
            Size::Pill => (24.0, 10.0, 12.0, 12.0, 5.0, 12),
            Size::Chip => (24.0, 10.0, 11.0, 11.0, 5.0, 12),
        };
        (self.height.unwrap_or(height), pad, text, icon, gap, radius)
    }

    /// Fill, hovered fill, border and text colour.
    fn colors(&self) -> (Color32, Color32, Stroke, Color32) {
        let hairline = |color| Stroke::new(1.0_f32, color);
        let outline = palette().border_strong;
        match self.kind {
            Kind::Primary => (
                palette().accent,
                palette().accent_hover,
                Stroke::NONE,
                Color32::WHITE,
            ),
            Kind::Danger => (
                palette().danger,
                Color32::from_rgb(0xc4, 0x44, 0x49),
                Stroke::NONE,
                Color32::WHITE,
            ),
            Kind::Soft => (
                palette().accent_soft,
                palette().selected,
                hairline(palette().accent_border),
                palette().accent_text,
            ),
            Kind::Tinted => (
                palette().accent_soft,
                palette().selected,
                hairline(palette().accent_border),
                palette().text,
            ),
            Kind::Secondary => (
                palette().button,
                palette().button_hover,
                hairline(outline),
                palette().text,
            ),
            Kind::DangerOutline => (
                Color32::TRANSPARENT,
                palette().red_bg,
                hairline(palette().red_border),
                palette().red,
            ),
            Kind::Ghost => (
                Color32::TRANSPARENT,
                palette().button_hover,
                if self.size == Size::Chip {
                    hairline(palette().border_strong)
                } else {
                    Stroke::NONE
                },
                palette().text,
            ),
        }
    }

    /// The text laid out, the icon's width with its gap, and the button's size.
    fn layout(&self, ui: &Ui) -> (std::sync::Arc<egui::Galley>, f32, Vec2) {
        let (height, pad, text_size, icon_size, gap, _) = self.metrics();
        // Weights follow the designs: filled actions and chips are semibold,
        // tinted buttons medium, outlined buttons regular.
        let font = match (self.kind, self.size) {
            (Kind::Primary | Kind::Danger, _) | (_, Size::Chip) => bold(text_size),
            (Kind::Soft | Kind::Tinted, _) => medium(text_size),
            _ => FontId::proportional(text_size),
        };
        // Placeholder colour lets the paint call dim disabled text.
        let galley = ui
            .painter()
            .layout_no_wrap(self.text.to_owned(), font, Color32::PLACEHOLDER);
        let icon_space = match self.icon {
            Some(_) if self.text.is_empty() => icon_size,
            Some(_) => icon_size + gap,
            None => 0.0,
        };
        // As in the browser, a border adds to the padded width.
        let border = if self.colors().2.is_empty() { 0.0 } else { 2.0 };
        let width = (galley.size().x + icon_space + pad * 2.0 + border).max(self.min_width);
        (galley, icon_space, egui::vec2(width, height))
    }

    /// The size the button will take, for layouts that place it themselves.
    pub fn desired_size(&self, ui: &Ui) -> Vec2 {
        self.layout(ui).2
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let (_, _, _, icon_size, _, radius) = self.metrics();
        let (fill, hover_fill, stroke, text_color) = self.colors();
        let (galley, icon_space, size) = self.layout(ui);
        let (width, height) = (size.x, size.y);
        // Reserve the space in this layout first, so wrapping rows can move the
        // button to their next line. The click area then goes in a disabled child
        // when needed, so egui knows the state: disabled-hover tooltips show and
        // plain tooltips do not.
        let (rect, placeholder) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
        let mut area = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        if !self.enabled {
            area.disable();
        }
        let response = area.interact(rect, placeholder.id.with("button"), Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, self.enabled, self.text)
        });
        if ui.is_rect_visible(rect) {
            let hovered = self.enabled && response.hovered();
            let pressed = self.enabled && response.is_pointer_button_down_on();
            let fill = if hovered { hover_fill } else { fill };
            let fill = if pressed {
                fill.gamma_multiply(0.85)
            } else if !self.enabled
                && matches!(
                    self.kind,
                    Kind::Primary | Kind::Danger | Kind::Soft | Kind::Tinted
                )
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
                    Stroke::new(2.0_f32, palette().link),
                    StrokeKind::Outside,
                );
            }
            let alpha = if self.enabled { 1.0 } else { 0.4 };
            let mut x = rect.center().x - (icon_space + galley.size().x) / 2.0;
            if let Some((icon, color)) = self.icon {
                let icon_rect = Rect::from_center_size(
                    egui::pos2(x + icon_size / 2.0, rect.center().y),
                    Vec2::splat(icon_size),
                );
                paint_icon_weight(
                    painter,
                    icon_rect,
                    icon,
                    color.unwrap_or(text_color).gamma_multiply(alpha),
                    self.icon_weight,
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

pub fn icon_button(ui: &mut Ui, icon: Icon, label: &str, color: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(8), palette().button_hover);
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
    ui.label(
        egui::RichText::new(text)
            .font(bold(size))
            .color(palette().text),
    )
}

/// Uppercase, letter-spaced group label such as "INSTALLED" or "ACTIVITY".
pub fn section_label(ui: &mut Ui, text: &str) -> Response {
    ui.label(
        egui::RichText::new(text.to_uppercase())
            .font(medium(12.0))
            .color(palette().muted)
            .extra_letter_spacing(0.7),
    )
}

/// Underlines link text at `rect` the way the browser does: just below the
/// baseline rather than at the bottom of the line.
fn underline(ui: &Ui, rect: Rect, size: f32, color: Color32) {
    ui.painter().hline(
        rect.x_range(),
        rect.top() + size + 2.0,
        Stroke::new(1.0_f32, color),
    );
}

fn focus_ring(ui: &Ui, response: &Response) {
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.expand(2.0),
            CornerRadius::same(3),
            Stroke::new(1.5_f32, palette().link),
            StrokeKind::Outside,
        );
    }
}

pub fn link(ui: &mut Ui, text: impl Into<String>) -> Response {
    link_enabled(ui, text, true)
}

/// An underlined 13pt text link that acts like a button: focusable, announced as a
/// link, and dimmed with a disabled-hover reason when it cannot be used.
pub fn link_enabled(ui: &mut Ui, text: impl Into<String>, enabled: bool) -> Response {
    let text = text.into();
    let response = ui
        .add_enabled_ui(enabled, |ui| {
            let response = ui.add(
                egui::Label::new(egui::RichText::new(&text).size(13.0).color(palette().link))
                    // One line, so the underline matches the text; a wrapping layout
                    // moves the whole link to the next line instead.
                    .extend()
                    .sense(Sense::click()),
            );
            // Painted in the disabled scope so it dims with the text.
            underline(ui, response.rect, 13.0, palette().link);
            response
        })
        .inner;
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Link, enabled, &text));
    focus_ring(ui, &response);
    if enabled && response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// An underlined link of `size` points that opens `url`, like `Ui::hyperlink_to`
/// with the design's permanent underline in place of egui's hover underline.
pub fn hyperlink(ui: &mut Ui, text: impl Into<String>, url: impl ToString, size: f32) -> Response {
    // Changed in place rather than in a child scope, so that a wrapping row can
    // still move the whole link to its next line: one line keeps the underline
    // under the text, and egui's own hover underline is turned off.
    let style = ui.style().clone();
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
    let states = &mut ui.visuals_mut().widgets;
    for state in [
        &mut states.inactive,
        &mut states.hovered,
        &mut states.active,
    ] {
        state.fg_stroke.width = 0.0;
    }
    let response = ui.hyperlink_to(
        egui::RichText::new(text).size(size).color(palette().link),
        url,
    );
    ui.set_style(style);
    underline(ui, response.rect, size, palette().link);
    focus_ring(ui, &response);
    response
}

pub fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(palette().panel)
        .stroke(Stroke::new(1.0_f32, palette().border))
        .corner_radius(CornerRadius::same(12))
}

pub fn group() -> egui::Frame {
    egui::Frame::new()
        .fill(palette().card)
        .stroke(Stroke::new(1.0_f32, palette().border))
        .corner_radius(CornerRadius::same(10))
}

/// Paints a full-width hairline at the current cursor, used between rows of a group.
pub fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::ZERO, palette().border);
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
    ui.painter()
        .rect_filled(rect, radius, palette().progress_track);
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
        ui.painter().rect_filled(fill, radius, palette().accent);
    }
}

/// A rounded badge with a number or short word, e.g. the update count next to Overview.
pub fn badge(ui: &mut Ui, text: &str, fill: Color32, color: Color32) -> Response {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), bold(11.0), color);
    let size = egui::vec2((galley.size().x + 12.0).max(20.0), 20.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(10), fill);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, color);
    response
}

/// Two-option segmented control: `height` segments with `pad` side padding in a
/// 3pt-padded field. Keeps the option order in right-to-left layouts too.
/// Returns true when the value changed.
pub fn segmented<T: PartialEq + Clone>(
    ui: &mut egui::Ui,
    height: f32,
    pad: f32,
    value: &mut T,
    options: &[(T, &str)],
) -> bool {
    let galleys: Vec<_> = options
        .iter()
        .map(|(_, label)| {
            ui.painter().layout_no_wrap(
                (*label).to_owned(),
                FontId::proportional(13.0),
                Color32::PLACEHOLDER,
            )
        })
        .collect();
    let width: f32 = galleys.iter().map(|g| g.size().x + pad * 2.0).sum::<f32>() + 8.0;
    let mut changed = false;
    ui.allocate_ui_with_layout(
        egui::vec2(width, height + 8.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            egui::Frame::new()
                .fill(palette().field)
                .stroke(Stroke::new(1.0_f32, palette().border_strong))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(egui::Margin::same(3))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for ((option, label), galley) in options.iter().zip(galleys) {
                        let selected = value == option;
                        let (rect, response) = ui.allocate_exact_size(
                            egui::vec2(galley.size().x + pad * 2.0, height),
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
                                    palette().button_hover
                                } else {
                                    palette().hover
                                },
                            );
                        }
                        if response.has_focus() {
                            ui.painter().rect_stroke(
                                rect,
                                CornerRadius::same(6),
                                Stroke::new(1.5_f32, palette().link),
                                StrokeKind::Inside,
                            );
                        }
                        ui.painter().galley(
                            rect.center() - galley.size() / 2.0,
                            galley,
                            palette().text,
                        );
                        if response.clicked() && !selected {
                            *value = option.clone();
                            changed = true;
                        }
                    }
                });
        },
    );
    changed
}

/// Paints a checkbox the way the mockups render a native one: a white box with a
/// grey outline, or the accent colour with a white tick.
pub fn paint_check(ui: &egui::Ui, rect: egui::Rect, on: bool, accent: Color32, enabled: bool) {
    let alpha = if enabled { 1.0 } else { 0.5 };
    let painter = ui.painter();
    if on {
        painter.rect_filled(rect, CornerRadius::same(2), accent.gamma_multiply(alpha));
        paint_icon(
            painter,
            rect.shrink(rect.width() * 0.12),
            Icon::Check,
            Color32::WHITE.gamma_multiply(alpha),
        );
    } else {
        painter.rect(
            rect,
            CornerRadius::same(2),
            Color32::WHITE.gamma_multiply(alpha),
            Stroke::new(1.0_f32, palette().control_border),
            StrokeKind::Inside,
        );
    }
}

/// Paints a radio button the way the mockups render a native one.
pub fn paint_radio(ui: &egui::Ui, center: egui::Pos2, on: bool) {
    let painter = ui.painter();
    if on {
        painter.circle_filled(center, 8.0, palette().accent);
        painter.circle_filled(center, 6.5, Color32::WHITE);
        painter.circle_filled(center, 4.5, palette().accent);
    } else {
        painter.circle(
            center,
            7.5,
            Color32::WHITE,
            Stroke::new(1.0_f32, palette().control_border),
        );
    }
}

/// A square checkbox of `size` points. Toggles `on` when clicked and announces
/// itself as a checkbox named `label`.
pub fn checkbox(
    ui: &mut egui::Ui,
    on: &mut bool,
    size: f32,
    accent: Color32,
    label: &str,
    enabled: bool,
) -> egui::Response {
    let (rect, mut response) = ui
        .add_enabled_ui(enabled, |ui| {
            ui.allocate_exact_size(egui::Vec2::splat(size), Sense::click())
        })
        .inner;
    if enabled && response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let checked = *on;
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, checked, label)
    });
    paint_check(ui, rect, checked, accent, enabled);
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(2.0),
            CornerRadius::same(4),
            Stroke::new(2.0_f32, palette().link),
            StrokeKind::Outside,
        );
    }
    if enabled && response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// A 16pt checkbox with its label, drawn like the mockup's native checkbox: accent fill and
/// a white tick when on, white with a grey border when off. The whole row toggles it.
pub fn check_label(ui: &mut Ui, on: &mut bool, label: &str, enabled: bool) -> Response {
    let font = FontId::proportional(14.0);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, Color32::PLACEHOLDER);
    // 4pt and 3pt margins around the box, then a 10pt gap to the label.
    let size = egui::vec2(4.0 + 16.0 + 3.0 + 10.0 + galley.size().x, 22.0);
    let (rect, mut response) = ui
        .add_enabled_ui(enabled, |ui| ui.allocate_exact_size(size, Sense::click()))
        .inner;
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let checked = *on;
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, checked, label)
    });
    if ui.is_rect_visible(rect) {
        let alpha = if enabled { 1.0 } else { 0.45 };
        let painter = ui.painter();
        let tick_box = Rect::from_min_size(
            egui::pos2(rect.left() + 4.0, rect.center().y - 8.0),
            Vec2::splat(16.0),
        );
        if checked {
            let fill = if enabled && response.hovered() {
                palette().accent_hover
            } else {
                palette().accent
            };
            painter.rect_filled(tick_box, CornerRadius::same(2), fill.gamma_multiply(alpha));
            let p = |x: f32, y: f32| tick_box.min + egui::vec2(x, y) * 16.0;
            painter.add(egui::Shape::line(
                vec![p(0.24, 0.52), p(0.42, 0.70), p(0.77, 0.32)],
                Stroke::new(2.2_f32, Color32::WHITE.gamma_multiply(alpha)),
            ));
        } else {
            painter.rect(
                tick_box,
                CornerRadius::same(2),
                Color32::WHITE.gamma_multiply(alpha),
                Stroke::new(1.0_f32, palette().control_border.gamma_multiply(alpha)),
                StrokeKind::Inside,
            );
        }
        if response.has_focus() {
            painter.rect_stroke(
                tick_box.expand(2.0),
                CornerRadius::same(4),
                Stroke::new(2.0_f32, palette().link),
                StrokeKind::Outside,
            );
        }
        painter.galley(
            egui::pos2(
                tick_box.right() + 3.0 + 10.0,
                rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            palette()
                .text_2
                .gamma_multiply(if enabled { 1.0 } else { 0.5 }),
        );
    }
    if enabled && response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// A log on the dark log background: monospace lines 1.7 apart, as tall as its text
/// up to the space left, then scrolling and following new lines. `wrap` breaks long
/// lines; otherwise they scroll sideways.
pub fn log_view(ui: &mut Ui, text: &str, wrap: bool) {
    let line_height = 12.0 * 1.7;
    let available = ui.available_height();
    egui::Frame::new()
        .fill(palette().log)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let scroll = if wrap {
                egui::ScrollArea::vertical()
            } else {
                egui::ScrollArea::both()
            };
            scroll
                .auto_shrink([false, true])
                .stick_to_bottom(true)
                .max_height((available - 28.0).max(80.0))
                .show(ui, |ui| {
                    // As in CSS, half the extra line height goes above each line.
                    let row = ui.fonts(|f| f.row_height(&FontId::monospace(12.0)));
                    ui.add_space((line_height - row) / 2.0);
                    let label = egui::Label::new(
                        egui::RichText::new(text.trim_end_matches('\n'))
                            .monospace()
                            .size(12.0)
                            .line_height(Some(line_height))
                            .color(palette().text_3),
                    )
                    .selectable(true);
                    ui.add(if wrap {
                        label.wrap()
                    } else {
                        label.wrap_mode(egui::TextWrapMode::Extend)
                    });
                });
        });
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

pub fn arrow() -> &'static str {
    "→"
}
pub fn bold_family() -> egui::FontFamily {
    egui::FontFamily::Name("semibold".into())
}
pub fn medium_family() -> egui::FontFamily {
    egui::FontFamily::Name("medium".into())
}
/// IBM Plex Sans SemiBold (600), used for headings and primary buttons.
pub fn bold(size: f32) -> FontId {
    FontId::new(size, bold_family())
}
/// IBM Plex Sans Medium (500), used for labels such as app names and group titles.
pub fn medium(size: f32) -> FontId {
    FontId::new(size, medium_family())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn luminance(color: Color32) -> f32 {
        let linear = |component: u8| {
            let component = f32::from(component) / 255.0;
            if component <= 0.04045 {
                component / 12.92
            } else {
                ((component + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
    }
    #[test]
    fn both_themes_keep_body_text_and_statuses_readable() {
        for palette in [DARK, LIGHT] {
            for background in [palette.bg, palette.panel, palette.card, palette.log] {
                for text in [
                    palette.text,
                    palette.text_2,
                    palette.text_3,
                    palette.muted,
                    palette.green,
                    palette.red,
                ] {
                    let a = luminance(text);
                    let b = luminance(background);
                    let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                    assert!(
                        contrast >= 4.5,
                        "Insufficient contrast: {text:?} on {background:?}: {contrast}"
                    );
                }
            }
        }
    }
    #[test]
    fn switching_theme_updates_native_and_custom_widget_colors() {
        let ctx = egui::Context::default();
        apply(&ctx, Theme::Light);
        assert!(!ctx.style().visuals.dark_mode);
        assert_eq!(ctx.style().visuals.panel_fill, LIGHT.bg);
        assert_eq!(palette().text, LIGHT.text);
        apply(&ctx, Theme::Dark);
        assert!(ctx.style().visuals.dark_mode);
        assert_eq!(palette().text, LIGHT.text);
        // Advance the transition halfway, then verify that it reaches the exact endpoint.
        let id = egui::Id::new("manager-visual-theme");
        ctx.data_mut(|data| {
            let mut state = data.get_temp::<ThemeTransition>(id).unwrap();
            state.started -= 0.125;
            data.insert_temp(id, state);
        });
        apply(&ctx, Theme::Dark);
        assert_ne!(palette().bg, LIGHT.bg);
        assert_ne!(palette().bg, DARK.bg);
        assert_eq!(ctx.style().visuals.panel_fill, palette().bg);
        ctx.data_mut(|data| {
            let mut state = data.get_temp::<ThemeTransition>(id).unwrap();
            state.started -= 0.125;
            data.insert_temp(id, state);
        });
        apply(&ctx, Theme::Dark);
        assert_eq!(palette().text, DARK.text);
        assert_eq!(ctx.style().visuals.panel_fill, DARK.bg);
    }
}
