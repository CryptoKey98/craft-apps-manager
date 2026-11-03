use crate::ui::theme;
use craft_apps_manager::network::{ApiObservation, ApiStatus, TokenStatus};
use eframe::egui::{self, Rect};

fn github(ui: &mut egui::Ui, rect: Rect) {
    let size = (rect.width() * ui.ctx().pixels_per_point())
        .round()
        .clamp(1.0, 256.0) as u32;
    let id = egui::Id::new(("github-footer-mark", size));
    let texture = ui
        .ctx()
        .data(|data| data.get_temp::<egui::TextureHandle>(id))
        .unwrap_or_else(|| {
            // Vector-rendered native sizes for 100/125/150/200/250/300/400% DPI.
            let atlas =
                image::load_from_memory(include_bytes!("../../../assets/github-footer.png"))
                    .expect("bundled GitHub footer icon")
                    .into_rgba8();
            let sizes = [
                (16, 0),
                (20, 16),
                (24, 36),
                (30, 60),
                (32, 90),
                (40, 122),
                (45, 162),
                (48, 207),
                (60, 255),
                (64, 315),
                (75, 379),
                (80, 454),
                (90, 534),
                (96, 624),
                (120, 720),
            ];
            let (source, y) = sizes
                .into_iter()
                .find(|(source, _)| *source >= size)
                .unwrap_or((120, 720));
            let native = image::imageops::crop_imm(&atlas, 0, y, source, source).to_image();
            let image = if source == size {
                native
            } else {
                image::imageops::resize(&native, size, size, image::imageops::FilterType::Lanczos3)
            };
            let mask = egui::ColorImage {
                size: [size as usize, size as usize],
                pixels: image
                    .pixels()
                    .map(|pixel| egui::Color32::from_white_alpha(pixel[3]))
                    .collect(),
            };
            let texture = ui.ctx().load_texture(
                format!("github-footer-{size}"),
                mask,
                egui::TextureOptions::LINEAR,
            );
            ui.ctx()
                .data_mut(|data| data.insert_temp(id, texture.clone()));
            texture
        });
    ui.painter().image(
        texture.id(),
        rect,
        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        theme::palette().muted,
    );
}

pub(in crate::ui) const CLASSIC_TOKEN_URL: &str = "https://github.com/settings/tokens/new";
pub(in crate::ui) const TOKEN_STEPS: [&str; 3] = [
    "Name it “Craft Apps Manager” and choose an expiration.",
    "Leave all scopes unchecked. Public app information needs no permissions.",
    "Generate and copy the token, then return here, paste it, choose Add or Replace, and Save.",
];

#[derive(Clone, Copy)]
struct Change {
    from: ApiStatus,
    to: ApiStatus,
    started: f64,
}
#[derive(Clone, Default)]
struct Motion {
    current: Option<ApiStatus>,
    change: Option<Change>,
    opened: Option<f64>,
}
impl Motion {
    fn entrance(&mut self, now: f64, animate: bool) -> IconMotion {
        let elapsed = (now - *self.opened.get_or_insert(now)).max(0.0) as f32;
        if !animate || elapsed >= 0.36 {
            return IconMotion::default();
        }
        let progress = elapsed / 0.36;
        let (scale, alpha) = if progress < 0.68 {
            let t = bezier(progress / 0.68, 0.22, 0.7, 0.26, 1.0);
            (egui::lerp(0.87..=1.035, t), egui::lerp(0.35..=1.0, t))
        } else {
            let t = bezier((progress - 0.68) / 0.32, 0.22, 0.7, 0.26, 1.0);
            (egui::lerp(1.035..=1.0, t), 1.0)
        };
        IconMotion {
            scale,
            alpha,
            draw: bezier(
                ((elapsed - 0.035) / 0.3).clamp(0.0, 1.0),
                0.0,
                0.0,
                0.58,
                1.0,
            ),
            reveal: bezier(
                ((elapsed - 0.035) / 0.28).clamp(0.0, 1.0),
                0.0,
                0.0,
                0.58,
                1.0,
            ),
            active: true,
        }
    }
    fn frame(&mut self, target: ApiStatus, now: f64, animate: bool) -> (ApiStatus, f32, f32, bool) {
        if !animate || self.current.is_none() {
            self.current = Some(target);
            self.change = None;
            return (target, 1.0, 0.0, false);
        }
        let current = self.current.unwrap();
        if self
            .change
            .map_or(current != target, |change| change.to != target)
        {
            self.change = Some(Change {
                from: current,
                to: target,
                started: now,
            });
        }
        let Some(change) = self.change else {
            return (current, 1.0, 0.0, false);
        };
        let progress = ((now - change.started) / 0.18).clamp(0.0, 1.0) as f32;
        if progress >= 1.0 {
            self.current = Some(target);
            self.change = None;
            return (target, 1.0, 0.0, false);
        }
        let (shown, opacity) = if progress < 0.5 {
            (change.from, 1.0 - progress * 2.0)
        } else {
            (change.to, progress * 2.0 - 1.0)
        };
        self.current = Some(shown);
        (shown, opacity, 3.0 * (1.0 - opacity), true)
    }
}

#[derive(Clone, Copy)]
struct IconMotion {
    scale: f32,
    alpha: f32,
    draw: f32,
    reveal: f32,
    active: bool,
}
impl Default for IconMotion {
    fn default() -> Self {
        Self {
            scale: 1.0,
            alpha: 1.0,
            draw: 1.0,
            reveal: 1.0,
            active: false,
        }
    }
}

// CSS cubic-bezier timing from the approved Quiet settle preview.
fn bezier(progress: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    if progress <= 0.0 || progress >= 1.0 {
        return progress;
    }
    let coordinate =
        |t: f32, a, b| 3.0 * (1.0 - t).powi(2) * t * a + 3.0 * (1.0 - t) * t * t * b + t.powi(3);
    let (mut low, mut high) = (0.0, 1.0);
    for _ in 0..14 {
        let t = (low + high) * 0.5;
        if coordinate(t, x1, x2) < progress {
            low = t;
        } else {
            high = t;
        }
    }
    coordinate((low + high) * 0.5, y1, y2)
}

fn title(status: ApiStatus) -> &'static str {
    match status {
        ApiStatus::Available => "Available",
        ApiStatus::RateLimited => "Rate limited",
        ApiStatus::Unknown => "Unknown",
    }
}
fn access(status: TokenStatus) -> &'static str {
    match status {
        TokenStatus::Connected => "Token configured",
        TokenStatus::NotConfigured => "Public access · no token",
        TokenStatus::Invalid => "Invalid token",
        TokenStatus::NotVerified => "Token verification pending",
        TokenStatus::UnableToVerify => "Unable to verify token",
    }
}

/// Passive information only: opening this menu never contacts GitHub.
pub(in crate::ui) fn show(ui: &mut egui::Ui, token: TokenStatus) -> bool {
    draw(
        ui,
        craft_apps_manager::network::api_status(),
        token,
        chrono::Utc::now().timestamp(),
    )
    .0
}
fn draw(
    ui: &mut egui::Ui,
    observation: Option<ApiObservation>,
    token: TokenStatus,
    now: i64,
) -> (bool, egui::Response) {
    let p = theme::palette();
    let status = observation
        .as_ref()
        .map_or(ApiStatus::Unknown, |value| value.status_at(now));
    if let Some(value) = observation
        .as_ref()
        .filter(|value| value.status_at(now) != ApiStatus::Unknown)
    {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs(
                (value.observed_at.saturating_add(300) - now).max(1) as u64,
            ));
    }
    let popup_id = ui.make_persistent_id("github-activity-popup");
    let motion_id = popup_id.with("request-transition");
    let text = title(status);
    let galley = ui.painter().layout_no_wrap(
        title(ApiStatus::RateLimited).into(),
        egui::FontId::proportional(12.0),
        p.text_2,
    );
    let detail_target = f32::from(status == ApiStatus::RateLimited);
    let duration = ui.style().animation_time;
    let animate = |id, target| {
        if duration > 0.0 {
            ui.ctx().animate_value_with_time(id, target, duration)
        } else {
            target
        }
    };
    let opacity = animate(popup_id.with("footer-detail-opacity"), detail_target);
    let detail_width = animate(
        popup_id.with("footer-detail-width"),
        if detail_target == 0.0 {
            0.0
        } else {
            galley.size().x + 12.0
        },
    );
    let width = (30.0 + detail_width).min((ui.available_width() - 24.0).max(30.0));
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 24.0), egui::Sense::click());
    let open = ui.memory(|memory| memory.is_popup_open(popup_id));
    if response.hovered() || open {
        ui.painter()
            .rect_filled(rect.expand2(egui::vec2(4.0, 0.0)), 4.0, p.hover);
    }
    github(
        ui,
        Rect::from_center_size(
            egui::pos2(rect.left() + 8.0, rect.center().y),
            egui::vec2(16.0, 16.0),
        ),
    );
    let clip = Rect::from_min_max(
        egui::pos2(rect.left() + 24.0, rect.top()),
        egui::pos2(rect.right() - 14.0, rect.bottom()),
    );
    if opacity > 0.0 && clip.is_positive() {
        let mut painter = ui.painter().with_clip_rect(clip.intersect(ui.clip_rect()));
        painter.set_opacity(opacity);
        painter.galley_with_override_text_color(
            egui::pos2(rect.left() + 24.0, rect.center().y - galley.size().y * 0.5),
            galley,
            p.amber,
        );
    }
    let center = egui::pos2(rect.right() - 5.0, rect.center().y);
    ui.painter().add(egui::Shape::line(
        vec![
            center + egui::vec2(-3.0, 1.5),
            center + egui::vec2(0.0, -1.5),
            center + egui::vec2(3.0, 1.5),
        ],
        egui::Stroke::new(1.4_f32, p.text_3),
    ));
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("GitHub activity: {text}"),
        )
    });
    response.clone().on_hover_text("GitHub activity");
    if response.clicked() {
        ui.memory_mut(|memory| memory.toggle_popup(popup_id));
    }
    let mut manage = false;
    if ui.memory(|memory| memory.is_popup_open(popup_id)) {
        let mut anchor = response.clone();
        anchor.rect = anchor.rect.translate(egui::vec2(0.0, -9.0));
        {
            // Styling a floating popup must not allocate a scoped item in the
            // footer row: even an empty scope advances it by item_spacing.x.
            let mut popup_style =
                ui.new_child(egui::UiBuilder::new().id_salt("github-popup-style"));
            let ui = &mut popup_style;
            ui.spacing_mut().menu_margin = egui::Margin::ZERO;
            ui.visuals_mut().menu_corner_radius = egui::CornerRadius::same(9);
            ui.visuals_mut().window_fill = p.panel;
            ui.visuals_mut().window_stroke = egui::Stroke::new(1.0_f32, p.border);
            egui::popup::popup_above_or_below_widget(
                ui,
                popup_id,
                &anchor,
                egui::AboveOrBelow::Above,
                egui::popup::PopupCloseBehavior::CloseOnClickOutside,
                |ui| {
                    ui.set_width(310.0_f32.min((ui.ctx().screen_rect().width() - 24.0).max(160.0)));
                    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                    egui::Frame::new().inner_margin(16).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 11.0;
                            let (logo, _) = ui
                                .allocate_exact_size(egui::vec2(30.0, 30.0), egui::Sense::hover());
                            github(ui, logo);
                            ui.vertical(|ui| {
                                ui.label(
                                    egui::RichText::new("GitHub activity")
                                        .size(14.0)
                                        .strong()
                                        .color(p.text),
                                );
                                ui.add_space(3.0);
                                ui.label(
                                    egui::RichText::new("Recent API request")
                                        .size(12.0)
                                        .color(p.muted),
                                );
                            });
                        });
                    });
                    rule(ui);
                    let (body, result_response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 85.0),
                        egui::Sense::hover(),
                    );
                    let mut motion = ui
                        .ctx()
                        .data(|data| data.get_temp::<Motion>(motion_id))
                        .unwrap_or_default();
                    let (shown, opacity, shift, animating) = motion.frame(
                        status,
                        ui.input(|i| i.time),
                        ui.style().animation_time > 0.0,
                    );
                    let icon_motion =
                        motion.entrance(ui.input(|i| i.time), ui.style().animation_time > 0.0);
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(motion_id, motion));
                    if animating || icon_motion.active {
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(16));
                    }
                    let mut painter = ui.painter().with_clip_rect(body.intersect(ui.clip_rect()));
                    painter.set_opacity(opacity);
                    let origin = body.left_top() + egui::vec2(16.0, 14.0 + shift);
                    status_icon(&painter, origin + egui::vec2(9.0, 9.0), shown, icon_motion);
                    painter.text(
                        origin + egui::vec2(27.0, 0.0),
                        egui::Align2::LEFT_TOP,
                        title(shown),
                        egui::FontId::proportional(13.0),
                        p.text,
                    );
                    let detail = match shown {
                        ApiStatus::Available => {
                            let age = observation
                                .as_ref()
                                .map_or(0, |value| now.saturating_sub(value.observed_at));
                            if age < 60 {
                                "Last response just now".into()
                            } else {
                                format!("Last response {} min ago", age / 60)
                            }
                        }
                        ApiStatus::RateLimited => {
                            "GitHub limited the last request. Try again later.".into()
                        }
                        ApiStatus::Unknown => "No recent response or a request failed.".into(),
                    };
                    result_response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Label,
                            true,
                            format!("{}. {detail}", title(shown)),
                        )
                    });
                    let detail = painter.layout(
                        detail,
                        egui::FontId::proportional(12.0),
                        p.muted,
                        (body.width() - 59.0).max(1.0),
                    );
                    painter.galley(origin + egui::vec2(27.0, 20.0), detail, p.muted);
                    egui::Frame::new()
                        .fill(p.card)
                        .corner_radius(egui::CornerRadius {
                            nw: 0,
                            ne: 0,
                            sw: 8,
                            se: 8,
                        })
                        .inner_margin(egui::Margin::symmetric(16, 11))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 10.0;
                                let action_width = ui
                                    .fonts(|fonts| {
                                        fonts.layout_no_wrap(
                                            "Manage token…".into(),
                                            egui::FontId::proportional(12.0),
                                            p.link,
                                        )
                                    })
                                    .size()
                                    .x;
                                ui.allocate_ui_with_layout(
                                    egui::vec2(
                                        (ui.available_width() - action_width - 10.0).max(1.0),
                                        18.0,
                                    ),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.set_min_width(ui.available_width());
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(access(token))
                                                    .size(12.0)
                                                    .color(p.text_2),
                                            )
                                            .truncate(),
                                        );
                                    },
                                );
                                if ui
                                    .link(egui::RichText::new("Manage token…").size(12.0))
                                    .clicked()
                                {
                                    manage = true;
                                }
                            });
                        });
                },
            );
        }
        if manage {
            ui.memory_mut(|memory| memory.close_popup());
        }
    } else {
        // Hidden menus neither animate nor retain a stale outgoing result.
        ui.ctx().data_mut(|data| data.remove::<Motion>(motion_id));
    }
    (manage, response)
}
fn rule(ui: &mut egui::Ui) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::palette().border);
}
fn status_icon(painter: &egui::Painter, center: egui::Pos2, status: ApiStatus, motion: IconMotion) {
    let p = theme::palette();
    let color = match status {
        ApiStatus::Available => p.green,
        ApiStatus::RateLimited => p.amber,
        ApiStatus::Unknown => p.muted,
    };
    painter.circle_filled(
        center,
        9.0 * motion.scale,
        color.gamma_multiply(0.16 * motion.alpha),
    );
    match status {
        ApiStatus::Available | ApiStatus::Unknown => {
            let offsets = if status == ApiStatus::Available {
                vec![
                    egui::vec2(-4.0, 0.25),
                    egui::vec2(-1.5, 2.75),
                    egui::vec2(4.0, -2.75),
                ]
            } else {
                vec![egui::vec2(-3.0, 0.0), egui::vec2(3.0, 0.0)]
            };
            let length: f32 = offsets
                .windows(2)
                .map(|pair| (pair[1] - pair[0]).length())
                .sum();
            let mut remaining = length * motion.draw;
            for pair in offsets.windows(2) {
                let segment = (pair[1] - pair[0]).length();
                if remaining <= 0.0 {
                    break;
                }
                painter.line_segment(
                    [
                        center + pair[0],
                        center + (pair[0] + (pair[1] - pair[0]) * (remaining / segment).min(1.0)),
                    ],
                    egui::Stroke::new(
                        1.4_f32,
                        color.gamma_multiply(egui::lerp(0.25..=1.0, motion.draw)),
                    ),
                );
                remaining -= segment;
            }
        }
        ApiStatus::RateLimited => {
            // Reveal the centered plain exclamation downwards without moving it.
            let clip = Rect::from_min_max(
                center - egui::vec2(5.0, 5.0),
                center + egui::vec2(5.0, -5.0 + 10.0 * motion.reveal),
            );
            let painter = painter.with_clip_rect(clip.intersect(painter.clip_rect()));
            let color = color.gamma_multiply(egui::lerp(0.3..=1.0, motion.reveal));
            painter.line_segment(
                [center - egui::vec2(0.0, 3.5), center + egui::vec2(0.0, 0.5)],
                egui::Stroke::new(1.6_f32, color),
            );
            painter.circle_filled(center + egui::vec2(0.0, 3.5), 0.8, color);
        }
    }
}

#[cfg(test)]
#[path = "github_activity_tests.rs"]
mod tests;
