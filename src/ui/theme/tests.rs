use super::*;

#[test]
fn disabled_buttons_follow_the_window_theme_transition_in_both_directions() {
    fn draw(ctx: &egui::Context, time: f64, mode: Theme) -> Vec<Color32> {
        let mut rects = Vec::new();
        let output = ctx.run(
            egui::RawInput {
                time: Some(time),
                ..Default::default()
            },
            |ctx| {
                apply(ctx, mode);
                egui::CentralPanel::default().show(ctx, |ui| {
                    for (label, kind) in [
                        ("Update All", Kind::Primary),
                        ("Open build folder", Kind::Secondary),
                        ("Build", Kind::Tinted),
                    ] {
                        rects.push(btn(label).kind(kind).enabled(false).show(ui).rect);
                    }
                });
            },
        );
        rects
            .into_iter()
            .map(|rect| {
                output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Rect(shape) if shape.rect == rect => Some(shape.fill),
                        _ => None,
                    })
                    .expect("button background")
            })
            .collect()
    }
    let ctx = egui::Context::default();
    crate::ui::fonts(&ctx);
    let dark = draw(&ctx, 0.0, Theme::Dark);
    assert_eq!(
        draw(&ctx, 1.0, Theme::Light),
        dark,
        "Disabled controls must not jump to the target theme on its first frame"
    );
    let halfway = draw(&ctx, 1.125, Theme::Light);
    let light = draw(&ctx, 1.5, Theme::Light);
    for ((from, middle), to) in dark.iter().zip(&halfway).zip(&light) {
        assert_ne!(from, middle);
        assert_ne!(middle, to);
    }
    assert_eq!(draw(&ctx, 2.0, Theme::Dark), light);
    let halfway_back = draw(&ctx, 2.125, Theme::Dark);
    assert_ne!(halfway_back, light);
    assert_ne!(halfway_back, dark);
    assert_eq!(draw(&ctx, 2.5, Theme::Dark), dark);
}

#[test]
fn explicit_theme_survives_late_system_appearance_events() {
    // Manager and Builder each own a context; neither may follow the OS when
    // the user has chosen an explicit theme.
    for requested in [Theme::Dark, Theme::Light] {
        let ctx = egui::Context::default();
        apply(&ctx, requested);
        for (time, system) in [
            (1.0, egui::Theme::Light),
            (2.0, egui::Theme::Dark),
            (3.0, egui::Theme::Light),
        ] {
            let _ = ctx.run(
                egui::RawInput {
                    time: Some(time),
                    system_theme: Some(system),
                    ..Default::default()
                },
                |ctx| {
                    apply(ctx, requested);
                    let expected = if requested == Theme::Dark {
                        DARK
                    } else {
                        LIGHT
                    };
                    assert_eq!(ctx.style().visuals.extreme_bg_color, expected.field);
                    assert_eq!(
                        ctx.style().visuals.widgets.inactive.bg_fill,
                        expected.button
                    );
                    assert_eq!(
                        ctx.style().visuals.widgets.noninteractive.bg_stroke.color,
                        expected.border
                    );
                    assert_eq!(ctx.style().visuals.dark_mode, requested == Theme::Dark);
                    assert_eq!(palette().bg, expected.bg);
                },
            );
        }
    }
}

#[test]
fn system_theme_changes_keep_standard_controls_and_palette_together() {
    let ctx = egui::Context::default();
    apply(&ctx, Theme::System);
    for (time, system, expected) in [
        (1.0, egui::Theme::Light, LIGHT),
        (1.5, egui::Theme::Light, LIGHT),
        (2.0, egui::Theme::Dark, DARK),
        (2.5, egui::Theme::Dark, DARK),
    ] {
        let _ = ctx.run(
            egui::RawInput {
                time: Some(time),
                system_theme: Some(system),
                ..Default::default()
            },
            |ctx| {
                apply(ctx, Theme::System);
                assert_eq!(ctx.style().visuals.panel_fill, palette().bg);
                assert_eq!(ctx.style().visuals.extreme_bg_color, palette().field);
                if time.fract() != 0.0 {
                    assert_eq!(palette().bg, expected.bg);
                }
            },
        );
    }
}
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
fn split_button_clicks_are_independent_and_disabled_halves_do_nothing() {
    fn draw(ctx: &egui::Context, input: egui::RawInput, enabled: bool) -> (Response, Response) {
        let mut responses = None;
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                responses = Some(
                    btn("Update PhotoCraft")
                        .primary()
                        .size(Size::Card)
                        .enabled(enabled)
                        .split(ui, "Choose version"),
                );
            });
        });
        responses.unwrap()
    }
    for enabled in [true, false] {
        for arrow in [false, true] {
            let ctx = egui::Context::default();
            crate::ui::fonts(&ctx);
            let (main, menu) = draw(&ctx, Default::default(), enabled);
            assert_eq!(main.rect.right(), menu.rect.left());
            assert_eq!(main.rect.height(), menu.rect.height());
            assert_ne!(main.id, menu.id);
            let point = if arrow {
                menu.rect.center()
            } else {
                main.rect.center()
            };
            for pressed in [true, false] {
                let input = egui::RawInput {
                    events: vec![
                        egui::Event::PointerMoved(point),
                        egui::Event::PointerButton {
                            pos: point,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                    ..Default::default()
                };
                let (main, menu) = draw(&ctx, input, enabled);
                if !pressed {
                    assert_eq!(main.clicked(), enabled && !arrow);
                    assert_eq!(menu.clicked(), enabled && arrow);
                }
            }
        }
    }
}
#[test]
fn long_links_wrap_inside_narrow_containers() {
    let ctx = egui::Context::default();
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(176.0, 300.0),
        )),
        ..Default::default()
    };
    let _ = ctx.run(input, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.set_width(160.0);
            let left = ui.cursor().left();
            let width = ui.available_width();
            let response = hyperlink(
                ui,
                "filmcraft-0.6.0-windows-x86_64-portable-with-a-very-long-filename.zip",
                "https://example.com/package.zip",
                12.0,
            );
            assert!(response.rect.right() <= left + width + 1.0);
            assert!(response.rect.height() > 20.0);
        });
    });
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
fn unchecked_dark_checkboxes_never_paint_white_surfaces() {
    let ctx = egui::Context::default();
    apply(&ctx, Theme::Dark);
    let output = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let mut on = false;
            checkbox(
                ui,
                &mut on,
                16.0,
                DARK.accent,
                "Keep a rollback version",
                true,
            );
        });
    });
    let fills: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| {
            if let egui::Shape::Rect(rect) = &shape.shape {
                Some(rect.fill)
            } else {
                None
            }
        })
        .collect();
    assert!(fills.iter().filter(|fill| **fill == DARK.button).count() >= 1);
    assert!(!fills.contains(&Color32::WHITE));
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
