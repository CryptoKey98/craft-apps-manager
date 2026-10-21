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
            checkbox(ui, &mut on, 16.0, DARK.accent, "App backups", true);
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
