use super::*;

pub fn theme_toggle(ui: &mut Ui, mode: Theme) -> Response {
    let mode = resolved(ui.ctx(), mode);
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

/// A restrained glow around a small badge; no continuous idle repaint loop.
pub fn new_app_badge(ui: &mut Ui, rect: Rect, opacity: f32) {
    if opacity < 1.0 && opacity > 0.0 {
        ui.ctx().request_repaint();
    }
    let p = palette();
    let light = !ui.visuals().dark_mode;
    for (spread, alpha) in [(6.0, 0.025), (4.0, 0.04), (2.0, 0.07)] {
        ui.painter().rect_filled(
            rect.expand(spread),
            CornerRadius::same(8),
            p.link
                .gamma_multiply(alpha * opacity * if light { 0.35 } else { 1.0 }),
        );
    }
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(5),
        if light {
            Color32::from_rgb(237, 242, 249)
        } else {
            p.accent_soft
        }
        .gamma_multiply(opacity),
    );
    if light {
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            CornerRadius::same(5),
            Stroke::new(
                1.0_f32,
                Color32::from_rgb(190, 205, 224).gamma_multiply(opacity),
            ),
            egui::StrokeKind::Inside,
        );
        ui.painter().line_segment(
            [
                egui::pos2(rect.left() + 5.0, rect.top() + 1.5),
                egui::pos2(rect.right() - 5.0, rect.top() + 1.5),
            ],
            Stroke::new(1.0_f32, Color32::WHITE.gamma_multiply(0.8 * opacity)),
        );
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        "New",
        medium(11.0),
        p.accent_text.gamma_multiply(opacity),
    );
    let response = ui.interact(
        rect,
        ui.id()
            .with(("new-app-badge", rect.min.x.to_bits(), rect.min.y.to_bits())),
        Sense::hover(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Label, true, "Newly detected app")
    });
    response.on_hover_text("Newly detected app. Open its page to clear this badge.");
}

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
    text_color: Option<Color32>,
    enabled: bool,
    min_width: f32,
    corners: Option<CornerRadius>,
}

pub fn btn(text: &str) -> Btn<'_> {
    Btn {
        text,
        kind: Kind::Secondary,
        size: Size::Small,
        height: None,
        icon: None,
        icon_weight: 2.0,
        text_color: None,
        enabled: true,
        min_width: 0.0,
        corners: None,
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
    pub fn text_color(mut self, color: Color32) -> Self {
        self.text_color = Some(color);
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

    /// Reserve both halves as one widget so wrapping never separates the arrow.
    pub fn split(mut self, ui: &mut Ui, menu_label: &str) -> (Response, Response) {
        let (_, _, _, _, _, radius) = self.metrics();
        let size = self.desired_size(ui);
        let arrow_width = 36.0;
        let (rect, placeholder) =
            ui.allocate_exact_size(egui::vec2(size.x + arrow_width, size.y), Sense::hover());
        let main_rect = Rect::from_min_size(rect.min, size);
        let arrow_rect = Rect::from_min_max(egui::pos2(main_rect.right(), rect.top()), rect.max);
        let mut main_ui = ui.new_child(egui::UiBuilder::new().max_rect(main_rect));
        let mut arrow_ui = ui.new_child(egui::UiBuilder::new().max_rect(arrow_rect));
        let kind = self.kind;
        let mut arrow = btn("")
            .kind(kind)
            .size(self.size)
            .height(size.y)
            .icon(Icon::ChevronDown)
            .enabled(self.enabled);
        arrow.corners = Some(CornerRadius {
            nw: 0,
            sw: 0,
            ne: radius,
            se: radius,
        });
        self.corners = Some(CornerRadius {
            nw: radius,
            sw: radius,
            ne: 0,
            se: 0,
        });
        // Fit the icon-only half to its 36-point hit area.
        let main = self.show(&mut main_ui);
        let arrow = arrow.show_in_rect(&mut arrow_ui, arrow_rect, placeholder.id.with("arrow"));
        arrow.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, arrow.enabled(), menu_label)
        });
        if matches!(kind, Kind::Primary) {
            ui.painter().line_segment(
                [
                    egui::pos2(main_rect.right(), rect.top() + 9.0),
                    egui::pos2(main_rect.right(), rect.bottom() - 9.0),
                ],
                Stroke::new(1.0_f32, Color32::WHITE.gamma_multiply(0.3)),
            );
        }
        (main, arrow)
    }
    pub fn show(self, ui: &mut Ui) -> Response {
        let size = self.desired_size(ui);
        let (rect, placeholder) = ui.allocate_exact_size(size, Sense::hover());
        self.show_in_rect(ui, rect, placeholder.id)
    }
    fn show_in_rect(self, ui: &mut Ui, rect: Rect, id: egui::Id) -> Response {
        let (_, _, _, icon_size, _, radius) = self.metrics();
        let (fill, hover_fill, stroke, text_color) = self.colors();
        let text_color = self.text_color.unwrap_or(text_color);
        let (galley, icon_space, _) = self.layout(ui);
        // Reserve the space in this layout first, so wrapping rows can move the
        // button to their next line. The click area then goes in a disabled child
        // when needed, so egui knows the state: disabled-hover tooltips show and
        // plain tooltips do not.
        let mut area = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        if !self.enabled {
            area.disable();
        }
        let response = area.interact(rect, id.with("button"), Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, self.enabled, self.text)
        });
        if ui.is_rect_visible(rect) {
            let hovered = self.enabled && response.hovered();
            let depth = button_press_depth(ui, &response);
            let paint_rect = pressed_button_rect(rect, depth);
            let fill = if hovered { hover_fill } else { fill };
            let light_disabled = !self.enabled && !ui.visuals().dark_mode;
            let fill = if light_disabled {
                Color32::from_rgb(233, 233, 233)
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
            let fill = fill.gamma_multiply(1.0 - 0.12 * depth);
            let stroke = if light_disabled {
                Stroke::new(stroke.width, Color32::from_rgb(218, 218, 218))
            } else {
                stroke
            };
            let painter = ui.painter();
            painter.rect(
                paint_rect,
                self.corners.unwrap_or(CornerRadius::same(radius)),
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
            let mut x = paint_rect.center().x - (icon_space + galley.size().x) / 2.0;
            if let Some((icon, color)) = self.icon {
                let icon_rect = Rect::from_center_size(
                    egui::pos2(x + icon_size / 2.0, paint_rect.center().y),
                    Vec2::splat(icon_size),
                );
                paint_icon_weight(
                    painter,
                    icon_rect,
                    icon,
                    if light_disabled {
                        Color32::from_rgb(153, 153, 153)
                    } else {
                        color.unwrap_or(text_color).gamma_multiply(alpha)
                    },
                    self.icon_weight,
                );
                x += icon_space;
            }
            painter.galley(
                egui::pos2(x, paint_rect.center().y - galley.size().y / 2.0),
                galley,
                if light_disabled {
                    Color32::from_rgb(153, 153, 153)
                } else {
                    text_color.gamma_multiply(alpha)
                },
            );
        }
        if self.enabled && response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        response
    }
}

// Paint-only feedback: layout and hit areas never move while a button is pressed.
fn button_press_depth(ui: &Ui, response: &Response) -> f32 {
    let now = ui.input(|input| input.time);
    let pulse_id = response.id.with("press-pulse");
    let clicked = response.enabled() && response.clicked();
    let pulse = ui.ctx().data_mut(|data| {
        if clicked {
            data.insert_temp(pulse_id, now);
        }
        data.get_temp::<f64>(pulse_id)
            .is_some_and(|clicked| now - clicked < 0.06)
    });
    let pressed = response.enabled() && (response.is_pointer_button_down_on() || pulse);
    if pulse {
        ui.ctx().request_repaint();
    }
    ui.ctx()
        .animate_bool_with_time(response.id.with("press-depth"), pressed, 0.10)
}

fn pressed_button_rect(rect: Rect, depth: f32) -> Rect {
    rect.shrink2(egui::vec2(0.7, 0.7) * depth)
        .translate(egui::vec2(0.0, 0.6 * depth))
}

pub fn icon_button(ui: &mut Ui, icon: Icon, label: &str, color: Color32) -> Response {
    sized_icon_button(ui, icon, label, color, 32.0, 16.0)
}

pub fn sized_icon_button(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    color: Color32,
    size: f32,
    glyph: f32,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let depth = button_press_depth(ui, &response);
    let paint_rect = pressed_button_rect(rect, depth);
    if response.hovered() || depth > 0.0 {
        ui.painter().rect_filled(
            paint_rect,
            CornerRadius::same(8),
            palette().button_hover.gamma_multiply(1.0 - 0.12 * depth),
        );
    }
    paint_icon(
        ui.painter(),
        Rect::from_center_size(paint_rect.center(), Vec2::splat(glyph)),
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
    // Asset filenames and repository URLs must stay within their containing
    // card. RichText underlining follows every wrapped line, unlike a manually
    // painted underline across the response's bounding rectangle.
    let style = ui.style().clone();
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
    let states = &mut ui.visuals_mut().widgets;
    for state in [
        &mut states.inactive,
        &mut states.hovered,
        &mut states.active,
    ] {
        state.fg_stroke.width = 0.0;
    }
    let response = ui.hyperlink_to(
        egui::RichText::new(text)
            .size(size)
            .color(palette().link)
            .underline(),
        url,
    );
    ui.set_style(style);
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
            let position = (time as f32 * 0.7).fract();
            let width = rect.width() * 0.22;
            ui.ctx().request_repaint_after(Duration::from_millis(16));
            Rect::from_min_size(
                rect.min + egui::vec2((rect.width() + width) * position - width, 0.0),
                egui::vec2(width, height),
            )
        }
    };
    if fill.width() > 0.0 {
        ui.painter()
            .with_clip_rect(rect.intersect(ui.clip_rect()))
            .rect_filled(fill, radius, palette().accent);
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

/// Neutral Spectrum surface and border, or accent fill with a white tick.
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
            palette().button.gamma_multiply(alpha),
            Stroke::new(1.0_f32, palette().border_strong.gamma_multiply(alpha)),
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
