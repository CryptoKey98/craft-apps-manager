//! Bundled icons/fonts and existing resource-loading helpers.
use super::{catalog, model, theme, Paths};
use eframe::egui;

pub(super) fn app_icon(name: &str) -> Option<&'static [u8]> {
    Some(match name {
        "designcraft" => include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/app-icons/designcraft.png"
        )),
        "effectcraft" => include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/app-icons/effectcraft.png"
        )),
        "filmcraft" => include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/app-icons/filmcraft.png"
        )),
        "lightcraft" => include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/app-icons/lightcraft.png"
        )),
        "photocraft" => include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/app-icons/photocraft.png"
        )),
        "vectorcraft" => include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/app-icons/vectorcraft.png"
        )),
        "wordcraft" => include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/app-icons/wordcraft.png"
        )),
        "gridcraft" => include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/app-icons/gridcraft.png"
        )),
        "deckcraft" => include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/app-icons/deckcraft.png"
        )),
        "cadcraft" => include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/app-icons/cadcraft.png"
        )),
        "soundcraft" => include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/app-icons/soundcraft.png"
        )),
        "printcraft" => include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/app-icons/pdfcraft.png"
        )),
        _ => return None,
    })
}

/// Icon for an app without a bundled one, downloaded once from its repository.
pub(super) fn icon_bytes(paths: &Paths, name: &str) -> Option<Vec<u8>> {
    if let Some(bytes) = app_icon(name) {
        return Some(bytes.to_vec());
    }
    let cached = paths.at(format!("runtime/icons/{name}.png"));
    if let Ok(bytes) = std::fs::read(&cached) {
        return Some(bytes);
    }
    catalog::get(name)?;
    let url = format!(
        "https://raw.githubusercontent.com/{}/HEAD/assets/app-icon/hicolor/64x64/apps/ai.storyteller.{}.png",
        model::repository(name),
        model::repository_name(name)
    );
    let bytes = reqwest::blocking::get(url)
        .ok()?
        .error_for_status()
        .ok()?
        .bytes()
        .ok()?
        .to_vec();
    image::load_from_memory(&bytes).ok()?;
    craft_apps_manager::files::write_bytes(&cached, &bytes).ok()?;
    Some(bytes)
}

pub(super) fn texture(
    ctx: &egui::Context,
    name: &str,
    bytes: &[u8],
) -> Option<egui::TextureHandle> {
    let image = image::load_from_memory(bytes).ok()?.into_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let pixels = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
    Some(ctx.load_texture(name, pixels, egui::TextureOptions::LINEAR))
}

/// A neutral rounded tile for apps whose repository has no icon.
pub(super) fn placeholder(ctx: &egui::Context, name: &str) -> egui::TextureHandle {
    const SIZE: usize = 64;
    let hue = name
        .bytes()
        .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b.into()))
        % 360;
    let color = egui::ecolor::Hsva::new(hue as f32 / 360.0, 0.35, 0.62, 1.0);
    let [r, g, b, _] = egui::Color32::from(color).to_array();
    let mut pixels = vec![egui::Color32::TRANSPARENT; SIZE * SIZE];
    let radius = 14.0f32;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = (x as f32 + 0.5 - SIZE as f32 / 2.0).abs() - (SIZE as f32 / 2.0 - radius);
            let dy = (y as f32 + 0.5 - SIZE as f32 / 2.0).abs() - (SIZE as f32 / 2.0 - radius);
            let outside = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt() - radius;
            let alpha = (0.5 - outside).clamp(0.0, 1.0);
            pixels[y * SIZE + x] =
                egui::Color32::from_rgba_unmultiplied(r, g, b, (alpha * 255.0) as u8);
        }
    }
    ctx.load_texture(
        format!("placeholder-{name}"),
        egui::ColorImage {
            size: [SIZE, SIZE],
            pixels,
        },
        egui::TextureOptions::LINEAR,
    )
}

/// IBM Plex, as in the approved designs, bundled so every platform renders the
/// same. egui has no font weights, so each weight is its own family.
pub(super) fn fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let faces: [(&str, &'static [u8], egui::FontFamily); 4] = [
        (
            "IBM Plex Sans",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/fonts/IBMPlexSans-Regular.ttf"
            )),
            egui::FontFamily::Proportional,
        ),
        (
            "IBM Plex Sans Medium",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/fonts/IBMPlexSans-Medium.ttf"
            )),
            theme::medium_family(),
        ),
        (
            "IBM Plex Sans SemiBold",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/fonts/IBMPlexSans-SemiBold.ttf"
            )),
            theme::bold_family(),
        ),
        (
            "IBM Plex Mono",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/fonts/IBMPlexMono-Regular.ttf"
            )),
            egui::FontFamily::Monospace,
        ),
    ];
    // The bundled egui fonts stay as fallbacks for symbols Plex lacks.
    let fallback = fonts.families[&egui::FontFamily::Proportional].clone();
    for (name, bytes, family) in faces {
        fonts.font_data.insert(
            name.into(),
            std::sync::Arc::new(egui::FontData::from_static(bytes)),
        );
        let list = fonts.families.entry(family).or_default();
        list.insert(0, name.into());
        for font in &fallback {
            if !list.contains(font) {
                list.push(font.clone());
            }
        }
    }
    ctx.set_fonts(fonts);
}
