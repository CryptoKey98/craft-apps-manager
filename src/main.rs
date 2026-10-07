#![windows_subsystem = "windows"]
mod ui;
use anyhow::{Context, Result};
use craft_apps_updater::{builder, files, jobs::Job, model::Paths, platform, tools, updates};
use std::{fs, path::PathBuf};
fn main() {
    if let Err(e) = run() {
        let message = format!("{e:#}");
        let path = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join("logs/startup.log")));
        if let Some(path) = path {
            let _ = fs::create_dir_all(path.parent().unwrap());
            let _ = fs::write(path, &message);
        }
        if std::env::args().any(|a| a == "--update" || a == "--update-source") {
            std::process::exit(1);
        }
        unsafe {
            let text = platform::wide(&message);
            let title = platform::wide("Craft Apps Updater");
            windows::Win32::UI::WindowsAndMessaging::MessageBoxW(
                None,
                windows::core::PCWSTR(text.as_ptr()),
                windows::core::PCWSTR(title.as_ptr()),
                windows::Win32::UI::WindowsAndMessaging::MB_OK
                    | windows::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
            );
        }
    }
}
fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let arg = |key: &str| {
        args.iter()
            .position(|s| s == key)
            .and_then(|i| args.get(i + 1))
            .map(PathBuf::from)
    };
    let home = std::env::current_exe()?
        .parent()
        .context("Executable has no parent")?
        .to_path_buf();
    let saved: ui::Locations = files::read_or_default(&home.join("data-root.json"))?;
    let root = arg("--root").or(saved.root).unwrap_or_else(|| home.clone());
    let paths = Paths::new(root, arg("--tools").or(saved.tools));
    if args.iter().any(|s| s == "--install-shortcuts") {
        let profile = std::env::var("USERPROFILE")?;
        let desktop = PathBuf::from(profile).join("Desktop");
        let exe = std::env::current_exe()?;
        platform::shortcut(
            &desktop.join("Craft Apps Updater Rust.lnk"),
            &exe,
            "",
            &home,
        )?;
        platform::shortcut(
            &desktop.join("Craft Apps Builder Rust.lnk"),
            &exe,
            "--builder",
            &home,
        )?;
        return Ok(());
    }
    if args
        .iter()
        .any(|s| s == "--update" || s == "--update-source")
    {
        let job = Job::new(paths.at("logs/updates.log"), &paths.builder_preferences()?);
        if args.iter().any(|s| s == "--update-source") {
            updates::sources(&paths, &paths.preferences()?.selected_sources, &job)?;
        } else {
            updates::releases(&paths, &job, args.iter().any(|s| s == "--background"))?;
        }
        return Ok(());
    }
    if args.iter().any(|s| s == "--diagnostics") {
        let config = paths.config()?;
        files::write_json(
            &home.join("diagnostics.json"),
            &serde_json::json!({"root":paths.root,"tools":paths.tools,"cargo":tools::cargo(&paths),"cpp":tools::visual_cpp()?,"apps":config.apps,"lastFilmCraftBuild":builder::history(&paths,"filmcraft"),"lastArtCraftXBuild":builder::history(&paths,"artcraftx")}),
        )?;
        return Ok(());
    }
    let builder = args.iter().any(|s| s == "--builder");
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size(if builder {
                [1050.0, 740.0]
            } else {
                [1160.0, 720.0]
            })
            .with_min_inner_size(if builder {
                [780.0, 580.0]
            } else {
                [1100.0, 660.0]
            }),
        ..Default::default()
    };
    eframe::run_native(
        if builder {
            "Craft Apps Builder"
        } else {
            "Craft Apps Updater"
        },
        options,
        Box::new(move |cc| Ok(Box::new(ui::App::new(cc, paths, home, builder)?))),
    )
    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok(())
}
