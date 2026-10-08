//! Capture the real UI using a fictional library, without desktop or cursor pixels.
#![windows_subsystem = "windows"]
#[path = "../src/ui.rs"]
mod ui;
use eframe::egui;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
struct Capture {
    app: ui::App,
    output: PathBuf,
    started: Instant,
    requested: bool,
}
impl eframe::App for Capture {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.update(ctx, frame);
        let screenshot = ctx.input(|input| {
            input.events.iter().find_map(|event| {
                if let egui::Event::Screenshot { image, .. } = event {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(image) = screenshot {
            let pixels: Vec<u8> = image
                .pixels
                .iter()
                .flat_map(|pixel| pixel.to_array())
                .collect();
            image::RgbaImage::from_raw(image.width() as u32, image.height() as u32, pixels)
                .expect("Invalid screenshot size")
                .save(&self.output)
                .expect("Could not save screenshot");
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if !self.requested && self.started.elapsed() >= Duration::from_secs(4) {
            self.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        ctx.request_repaint_after(Duration::from_millis(50));
    }
}
fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let value = |key: &str| {
        args.windows(2)
            .find(|pair| pair[0] == key)
            .map(|pair| PathBuf::from(&pair[1]))
    };
    let root = value("--root").expect("Supply a fictional demo library with --root");
    let output = value("--output").expect("Supply --output screenshot.png");
    let prefs = craft_apps_manager::model::Paths::new(root.clone(), None).preferences()?;
    anyhow::ensure!(
        !prefs.check_installed_apps_on_startup && !prefs.check_manager_on_startup,
        "Disable startup network checks in the demo library"
    );
    let builder = args.iter().any(|arg| arg == "--builder");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1160.0, 720.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Craft Apps Manager — Demo",
        options,
        Box::new(move |cc| {
            Ok(Box::new(Capture {
                app: ui::App::new(
                    cc,
                    craft_apps_manager::model::Paths::new(root.clone(), None),
                    root,
                    builder,
                )?,
                output,
                started: Instant::now(),
                requested: false,
            }))
        }),
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))
}
