use super::*;

fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
    fn visit(shape: &egui::Shape, result: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => result.push(text.galley.job.text.clone()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| visit(shape, result)),
            _ => {}
        }
    }
    let mut result = Vec::new();
    for shape in shapes {
        visit(&shape.shape, &mut result);
    }
    result
}

fn frame(
    ctx: &egui::Context,
    observation: Option<ApiObservation>,
    time: f64,
    open: bool,
    events: Vec<egui::Event>,
) -> (egui::FullOutput, egui::Rect, egui::Id, bool) {
    let mut footer = Rect::NOTHING;
    let mut popup = egui::Id::NULL;
    let mut manage = false;
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 700.0),
            )),
            time: Some(time),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::TopBottomPanel::bottom("test-footer").show(ctx, |ui| {
                popup = ui.make_persistent_id("github-activity-popup");
                if open {
                    ui.memory_mut(|memory| memory.open_popup(popup));
                }
                let (clicked, response) =
                    draw(ui, observation.clone(), TokenStatus::NotConfigured, 1000);
                footer = response.rect;
                manage = clicked;
            });
        },
    );
    (output, footer, popup, manage)
}

#[test]
fn unknown_footer_stays_compact_without_a_status_label() {
    let ctx = egui::Context::default();
    let (output, footer, _, _) = frame(&ctx, None, 0.0, false, Vec::new());
    assert!(footer.width() <= 32.0);
    assert!(!texts(&output.shapes).iter().any(|text| text == "Unknown"));
}

#[test]
fn opening_activity_does_not_shift_the_following_data_label() {
    let ctx = egui::Context::default();
    let mut positions = Vec::new();
    for (time, open) in [(0.0, false), (0.2, true), (0.4, true), (0.6, false)] {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 700.0),
                )),
                time: Some(time),
                ..Default::default()
            },
            |ctx| {
                egui::TopBottomPanel::bottom("footer-position").show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        let id = ui.make_persistent_id("github-activity-popup");
                        ui.memory_mut(|memory| {
                            if open {
                                memory.open_popup(id)
                            } else {
                                memory.close_popup()
                            }
                        });
                        draw(ui, None, TokenStatus::NotConfigured, 1000);
                        positions.push(ui.label("Data: test").rect.left());
                    });
                });
            },
        );
    }
    assert!(
        positions.iter().all(|x| (x - positions[0]).abs() < 0.1),
        "{positions:?}"
    );
}

#[test]
fn healthy_footer_is_compact_and_problem_text_expands_then_collapses() {
    let ctx = egui::Context::default();
    ctx.style_mut(|style| style.animation_time = 0.18);
    let available = Some(ApiObservation {
        status: ApiStatus::Available,
        observed_at: 1000,
    });
    let limited = Some(ApiObservation {
        status: ApiStatus::RateLimited,
        observed_at: 1000,
    });
    let (output, compact, _, _) = frame(&ctx, available.clone(), 0.0, false, Vec::new());
    assert!(compact.width() <= 32.0);
    assert!(!texts(&output.shapes).iter().any(|text| text == "Available"));
    frame(&ctx, limited.clone(), 1.0, false, Vec::new());
    let (_, growing, _, _) = frame(&ctx, limited.clone(), 1.09, false, Vec::new());
    let (_, expanded, _, _) = frame(&ctx, limited.clone(), 1.3, false, Vec::new());
    assert!(growing.width() > compact.width() && growing.width() < expanded.width());
    frame(&ctx, available.clone(), 1.4, false, Vec::new());
    let (_, shrinking, _, _) = frame(&ctx, available.clone(), 1.49, false, Vec::new());
    let (_, settled, _, _) = frame(&ctx, available, 1.7, false, Vec::new());
    assert!(shrinking.width() > settled.width() && shrinking.width() < expanded.width());
    assert!((settled.width() - compact.width()).abs() < 0.1);
    frame(&ctx, limited.clone(), 1.8, false, Vec::new());
    frame(&ctx, limited, 2.1, false, Vec::new());
    frame(&ctx, None, 2.2, false, Vec::new());
    let (output, unknown, _, _) = frame(&ctx, None, 2.5, false, Vec::new());
    assert!((unknown.width() - compact.width()).abs() < 0.1);
    assert!(!texts(&output.shapes).iter().any(|text| text == "Unknown"));
}

#[test]
fn popup_opens_above_footer_and_escape_discards_hidden_animation() {
    let ctx = egui::Context::default();
    let (_, _, id, _) = frame(&ctx, None, 0.0, true, Vec::new());
    let (output, footer, _, _) = frame(&ctx, None, 0.1, false, Vec::new());
    let text = texts(&output.shapes);
    for expected in [
        "GitHub activity",
        "Recent API request",
        "Public access · no token",
        "Manage token…",
    ] {
        assert!(
            text.iter().any(|text| text == expected),
            "Missing {expected}: {text:?}"
        );
    }
    let popup = ctx.memory(|memory| memory.area_rect(id)).unwrap();
    assert!(popup.bottom() < footer.top());
    assert!((popup.width() - 310.0).abs() <= 3.0, "{popup:?}");
    assert!(ctx.screen_rect().contains_rect(popup));
    let event = egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    frame(&ctx, None, 0.2, false, vec![event]);
    assert!(!ctx.memory(|memory| memory.is_popup_open(id)));
    frame(&ctx, None, 0.3, false, Vec::new());
    assert!(ctx
        .data(|data| data.get_temp::<Motion>(id.with("request-transition")))
        .is_none());
}

#[test]
fn manage_token_action_closes_popup() {
    let ctx = egui::Context::default();
    frame(&ctx, None, 0.0, true, Vec::new());
    let (output, _, id, _) = frame(&ctx, None, 0.1, false, Vec::new());
    let pos = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == "Manage token…" => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            _ => None,
        })
        .expect("Manage token link");
    let click = |pressed| {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    };
    frame(&ctx, None, 0.2, false, click(true));
    let (_, _, _, manage) = frame(&ctx, None, 0.3, false, click(false));
    assert!(manage);
    assert!(!ctx.memory(|memory| memory.is_popup_open(id)));
}

#[test]
fn token_summary_and_manage_link_match_the_mockup_padding() {
    let ctx = egui::Context::default();
    frame(&ctx, None, 0.0, true, Vec::new());
    let (output, _, id, _) = frame(&ctx, None, 0.1, false, Vec::new());
    let popup = ctx.memory(|memory| memory.area_rect(id)).unwrap();
    let text_rect = |label| {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == label => {
                    Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                }
                _ => None,
            })
            .expect("access row text")
    };
    let summary = text_rect("Public access · no token");
    let action = text_rect("Manage token…");
    assert!(
        (summary.left() - popup.left() - 16.0).abs() <= 2.0,
        "{summary:?} {popup:?}"
    );
    assert!(
        (popup.right() - action.right() - 16.0).abs() <= 2.0,
        "{action:?} {popup:?}"
    );
    assert!((summary.center().y - action.center().y).abs() <= 1.0);
    assert!(action.left() - summary.right() >= 10.0);
}

#[test]
fn state_transition_fades_out_then_in_and_honors_disabled_animations() {
    let mut motion = Motion::default();
    assert_eq!(
        motion.frame(ApiStatus::Unknown, 0.0, true),
        (ApiStatus::Unknown, 1.0, 0.0, false)
    );
    motion.frame(ApiStatus::Available, 1.0, true);
    let (shown, alpha, shift, active) = motion.frame(ApiStatus::Available, 1.045, true);
    assert_eq!(shown, ApiStatus::Unknown);
    assert!((alpha - 0.5).abs() < 0.001 && shift > 0.0 && active);
    let (shown, alpha, _, active) = motion.frame(ApiStatus::Available, 1.135, true);
    assert_eq!(shown, ApiStatus::Available);
    assert!((alpha - 0.5).abs() < 0.001 && active);
    assert_eq!(
        motion.frame(ApiStatus::Available, 1.181, true),
        (ApiStatus::Available, 1.0, 0.0, false)
    );
    assert_eq!(
        motion.frame(ApiStatus::RateLimited, 1.2, false),
        (ApiStatus::RateLimited, 1.0, 0.0, false)
    );
    assert!(motion.change.is_none());
}

#[test]
fn interrupted_transition_finishes_with_the_latest_result() {
    let mut motion = Motion::default();
    motion.frame(ApiStatus::Unknown, 0.0, true);
    motion.frame(ApiStatus::Available, 1.0, true);
    motion.frame(ApiStatus::Available, 1.12, true);
    motion.frame(ApiStatus::RateLimited, 1.13, true);
    assert_eq!(
        motion.frame(ApiStatus::RateLimited, 1.32, true),
        (ApiStatus::RateLimited, 1.0, 0.0, false)
    );
}

#[test]
fn classic_token_guide_uses_no_scopes_and_existing_save_flow() {
    assert_eq!(CLASSIC_TOKEN_URL, "https://github.com/settings/tokens/new");
    assert!(TOKEN_STEPS[1].contains("all scopes unchecked"));
    assert!(TOKEN_STEPS[2].contains("Add or Replace, and Save"));
}

#[test]
fn bundled_logo_has_transparent_edges_and_antialiasing() {
    let image = image::load_from_memory(include_bytes!("../../../assets/github-footer.png"))
        .unwrap()
        .into_rgba8();
    assert!(image.pixels().any(|p| p[3] == 0));
    assert!(image.pixels().any(|p| p[3] > 0 && p[3] < 255));
    assert!(image.pixels().any(|p| p[3] == 255));
}

#[test]
fn rate_limit_uses_a_centered_plain_exclamation_inside_the_bubble() {
    let ctx = egui::Context::default();
    let center = egui::pos2(100.0, 100.0);
    let output = ctx.run(egui::RawInput::default(), |ctx| {
        status_icon(
            &ctx.debug_painter(),
            center,
            ApiStatus::RateLimited,
            IconMotion::default(),
        );
    });
    let shapes: Vec<_> = output.shapes.iter().map(|shape| &shape.shape).collect();
    assert_eq!(
        shapes.len(),
        3,
        "Bubble, straight stem and dot; no triangle"
    );
    assert!(
        matches!(shapes[0], egui::Shape::Circle(circle) if circle.center == center && circle.radius == 9.0)
    );
    assert!(
        matches!(shapes[1], egui::Shape::LineSegment { points, .. } if points.iter().all(|point| point.x == center.x))
    );
    assert!(
        matches!(shapes[2], egui::Shape::Circle(circle) if circle.center.x == center.x && circle.radius < 1.0)
    );
}

#[test]
fn animated_result_exposes_its_explanation_to_screen_readers() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, None, 0.0, true, Vec::new());
    let (output, _, _, _) = frame(&ctx, None, 0.1, false, Vec::new());
    let tree = output
        .platform_output
        .accesskit_update
        .expect("accessibility tree");
    // egui maps Label text to AccessKit's value property.
    assert!(tree.nodes.iter().any(|(_, node)| node
        .value()
        .is_some_and(|label| label.contains("No recent response or a request failed."))));
}

// A static icon misses the approved one-shot entrance, or replays it every frame.
#[test]
fn popup_icon_settles_once_and_replays_only_after_reopening() {
    let ctx = egui::Context::default();
    frame(&ctx, None, 0.0, true, Vec::new());
    let (entering, _, id, _) = frame(&ctx, None, 0.01, false, Vec::new());
    let bubble = |output: &egui::FullOutput| {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Circle(circle) if circle.radius > 7.0 && circle.radius < 10.0 => {
                    Some(*circle)
                }
                _ => None,
            })
            .expect("status bubble")
    };
    assert!(
        bubble(&entering).radius < 9.0,
        "entrance starts below full size"
    );
    let (settled, _, _, _) = frame(&ctx, None, 0.4, false, Vec::new());
    assert_eq!(bubble(&settled).radius, 9.0);
    let (later, _, _, _) = frame(&ctx, None, 3.0, false, Vec::new());
    assert_eq!(bubble(&later).radius, 9.0);
    assert_eq!(bubble(&later).center, bubble(&settled).center);
    ctx.memory_mut(|memory| memory.close_popup());
    frame(&ctx, None, 3.1, false, Vec::new());
    assert!(ctx
        .data(|data| data.get_temp::<Motion>(id.with("request-transition")))
        .is_none());
    frame(&ctx, None, 4.0, true, Vec::new());
    let (reopened, _, _, _) = frame(&ctx, None, 4.01, false, Vec::new());
    assert!(bubble(&reopened).radius < 9.0);
}

// Cover scaling clips the top of this 496x512 source in each square atlas cell.
#[test]
fn bundled_logo_keeps_the_source_top_margin_at_every_native_size() {
    let atlas = image::load_from_memory(include_bytes!("../../../assets/github-footer.png"))
        .unwrap()
        .into_rgba8();
    assert_eq!(atlas.dimensions(), (120, 840));
    for (size, y) in [
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
    ] {
        assert!(
            (0..size).all(|x| atlas.get_pixel(x, y)[3] < 255),
            "{size}px cell must retain its uncropped top margin"
        );
    }
}

#[test]
fn entrance_obeys_delay_duration_and_disabled_animations() {
    let mut motion = Motion::default();
    let first = motion.entrance(1.0, true);
    assert_eq!(first.scale, 0.87);
    assert_eq!(first.draw, 0.0);
    assert_eq!(first.reveal, 0.0);
    assert!(first.active);
    let delayed = motion.entrance(1.034, true);
    assert_eq!(delayed.draw, 0.0);
    assert_eq!(delayed.reveal, 0.0);
    let peak = motion.entrance(1.2448, true);
    assert!((peak.scale - 1.035).abs() < 0.001);
    let settled = motion.entrance(1.361, true);
    assert_eq!(
        (settled.scale, settled.draw, settled.reveal),
        (1.0, 1.0, 1.0)
    );
    assert!(!settled.active);
    let mut disabled = Motion::default();
    let still = disabled.entrance(1.0, false);
    assert_eq!((still.scale, still.draw, still.reveal), (1.0, 1.0, 1.0));
    assert!(!still.active);
}

#[test]
fn settled_glyphs_are_centered_without_changing_the_bubble() {
    for status in [
        ApiStatus::Available,
        ApiStatus::Unknown,
        ApiStatus::RateLimited,
    ] {
        let ctx = egui::Context::default();
        let center = egui::pos2(100.0, 100.0);
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            status_icon(&ctx.debug_painter(), center, status, IconMotion::default());
        });
        let glyph = output
            .shapes
            .iter()
            .skip(1)
            .fold(Rect::NOTHING, |bounds, shape| {
                bounds.union(shape.shape.visual_bounding_rect())
            });
        assert!(
            (glyph.center() - center).length() < 0.01,
            "{status:?}: {glyph:?}"
        );
    }
}
