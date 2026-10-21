use craft_apps_manager::model::Theme;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Response, Sense, Stroke, StrokeKind,
    Ui, Vec2,
};
use std::time::Duration;

mod controls;
mod icons;
pub use controls::*;
pub use icons::*;

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

pub fn resolved(ctx: &egui::Context, mode: Theme) -> Theme {
    if mode == Theme::System {
        if ctx.system_theme() == Some(egui::Theme::Light) {
            Theme::Light
        } else {
            Theme::Dark
        }
    } else {
        mode
    }
}

pub fn apply(ctx: &egui::Context, mode: Theme) {
    let requested = mode;
    let mode = resolved(ctx, mode);
    let preference_id = egui::Id::new("manager-theme-preference");
    let changed = ctx.data_mut(|data| {
        let changed = data.get_temp::<Theme>(preference_id) != Some(requested);
        data.insert_temp(preference_id, requested);
        changed
    });
    if changed {
        ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(
            if requested == Theme::System {
                egui::SystemTheme::SystemDefault
            } else if mode == Theme::Light {
                egui::SystemTheme::Light
            } else {
                egui::SystemTheme::Dark
            },
        ));
    }

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
    v.menu_corner_radius = CornerRadius::same(8);
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

#[cfg(test)]
mod tests;
