#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod ui;
use anyhow::{Context, Result};
use craft_apps_manager::{builder, files, jobs::Job, model::Paths, platform, tools, updates};
use std::{fs, path::PathBuf};
fn main() {
    if let Err(e) = run() {
        let message = format!("{e:#}");
        #[cfg(target_os = "linux")]
        if std::env::args().any(|arg| arg == "--approved-admin-batch") {
            // The root helper reports through its captured stderr, never user settings/log paths.
            eprintln!("{message}");
            std::process::exit(1);
        }
        let path = craft_apps_manager::self_update::startup_log_path();
        if let Some(path) = path {
            let _ = fs::create_dir_all(path.parent().unwrap());
            let _ = fs::write(path, &message);
        }
        if std::env::args().any(|a| {
            a == "--update"
                || a == "--approved-admin-batch"
                || a == "--update-source"
                || a == "--background"
                || a == "--check-app-updates"
                || a == "--check-source-updates"
                || a == "--apply-self-update"
                || a == "--list-apps"
                || a == "--install-app"
                || a == "--uninstall-app"
                || a == "--restore-relocation-files"
        }) {
            eprintln!("{message}");
            std::process::exit(1);
        }
        #[cfg(unix)]
        eprintln!("{message}");
        #[cfg(target_os = "windows")]
        unsafe {
            let text = platform::wide(&message);
            let title = platform::wide("Craft Apps Manager");
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
    if args.iter().any(|arg| arg == "--build-identity") {
        println!(
            "{}",
            serde_json::json!({"version": env!("CARGO_PKG_VERSION"), "revision": craft_apps_manager::update_revisions::BUILD_REVISION, "target": env!("CRAFT_BUILD_TARGET")})
        );
        return Ok(());
    }
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    if let Some(index) = args.iter().position(|arg| arg == "--approved-admin-batch") {
        return craft_apps_manager::batch_admin::worker(
            std::path::Path::new(args.get(index + 1).context("Missing batch manifest")?),
            args.get(index + 2).context("Missing batch checksum")?,
        );
    }
    if let Some(i) = args.iter().position(|s| s == "--apply-self-update") {
        return craft_apps_manager::self_update::apply(std::path::Path::new(
            args.get(i + 1).context("Missing update plan")?,
        ));
    }
    let arg = |key: &str| {
        args.iter()
            .position(|s| s == key)
            .and_then(|i| args.get(i + 1))
            .map(PathBuf::from)
    };
    #[cfg(target_os = "windows")]
    let home = if craft_apps_manager::self_update::installed_with_msi() {
        PathBuf::from(std::env::var_os("LOCALAPPDATA").context("No Windows user data directory")?)
            .join("Craft Apps Manager")
    } else {
        std::env::current_exe()?
            .parent()
            .context("Executable has no parent")?
            .to_path_buf()
    };
    #[cfg(target_os = "linux")]
    let home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .context("No Linux user data directory")?
        .join("craft-apps-manager");
    #[cfg(target_os = "macos")]
    let home = PathBuf::from(std::env::var_os("HOME").context("No macOS home directory")?)
        .join("Library/Application Support/Craft Apps Manager");
    let executable = std::env::current_exe()?;
    #[cfg(target_os = "linux")]
    let executable = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .unwrap_or(executable);
    let portable_folder = craft_apps_manager::portable_mode::executable_folder(&executable)?;
    // A pre-0.8.1 helper restarts the replacement with its old library arguments,
    // but cannot copy the newly introduced portable marker itself.
    if !craft_apps_manager::self_update::installed_with_msi()
        && !craft_apps_manager::self_update::installed_with_linux_package()
        && !craft_apps_manager::portable_mode::local_storage(&portable_folder)
    {
        let legacy_root = arg("--root").or_else(|| {
            (home == portable_folder && home.join("settings.json").is_file()).then(|| home.clone())
        });
        if let Some(root) = legacy_root.filter(|root| root.is_dir()) {
            craft_apps_manager::portable_mode::adopt_updated_portable(
                &portable_folder,
                &root,
                arg("--tools").as_deref(),
            )?;
        }
    }
    let portable = craft_apps_manager::portable_mode::local_storage(&portable_folder)
        && !craft_apps_manager::self_update::installed_with_msi()
        && !craft_apps_manager::self_update::installed_with_linux_package();
    let previous_home = home.clone();
    let home = if portable {
        craft_apps_manager::portable_mode::initialize(&portable_folder)?;
        portable_folder
    } else {
        home
    };
    let mut saved: ui::Locations = files::read_or_default(&home.join("data-root.json"))?;
    // Existing Linux/macOS ZIP users may already have a system data library.
    // Retain it rather than silently starting an empty library or copying gigabytes.
    if portable
        && home != previous_home
        && !home.join("data-root.json").exists()
        && !home.join("data/settings.json").exists()
        && !home.join("settings.json").exists()
    {
        let legacy: ui::Locations = files::read_or_default(&previous_home.join("data-root.json"))?;
        let legacy_root = legacy.root.unwrap_or(previous_home);
        if legacy_root.join("settings.json").is_file() {
            saved.root = Some(legacy_root);
            saved.tools = legacy.tools;
            files::write_json(&home.join("data-root.json"), &saved)?;
        }
    }
    let root = arg("--root").or(saved.root).unwrap_or_else(|| {
        if portable {
            craft_apps_manager::portable_mode::default_library(&home)
        } else {
            home.clone()
        }
    });
    let resolve = |path: PathBuf| -> Result<PathBuf> {
        if portable {
            craft_apps_manager::portable_mode::resolve_folder(&home, &path)
        } else {
            Ok(std::path::absolute(path)?)
        }
    };
    let root = resolve(root)?;
    if portable {
        craft_apps_manager::portable_mode::register_library(&home, &root);
    }
    let tools = arg("--tools")
        .or(saved.tools)
        .or_else(|| {
            (portable && home.join("workspace/tools").is_dir())
                .then(|| home.join("workspace/tools"))
        })
        .map(resolve)
        .transpose()?;
    let paths = Paths::new(root, tools);
    #[cfg(target_os = "windows")]
    if let Some(plan) = arg("--restore-relocation-files") {
        return craft_apps_manager::relocation::restore_elevated(&paths, &plan);
    }
    // The app list saved by the last discovery; the window refreshes it in the background.
    craft_apps_manager::catalog::load(&paths.root);
    #[cfg(target_os = "linux")]
    craft_apps_manager::apps::repair_linux_shortcuts(&paths)?;
    // Old scheduled tasks keep their command line after an executable update.
    // Route both legacy background commands and new commands to checks only.
    let background = args.iter().any(|s| s == "--background");
    if args
        .iter()
        .any(|s| s == "--check-app-updates" || s == "--check-source-updates")
        || (background
            && args
                .iter()
                .any(|s| s == "--update" || s == "--update-source"))
    {
        let job = Job::new(paths.at("logs/updates.log"), &paths.builder_preferences()?);
        if args
            .iter()
            .any(|s| s == "--check-source-updates" || s == "--update-source")
        {
            craft_apps_manager::hourly::sources(&paths, &job)?;
        } else {
            craft_apps_manager::hourly::run(&paths, &job)?;
        }
        return Ok(());
    }
    if args.iter().any(|s| s == "--install-shortcuts") {
        let profile = std::env::var(if cfg!(target_os = "windows") {
            "USERPROFILE"
        } else {
            "HOME"
        })?;
        let desktop = PathBuf::from(profile).join("Desktop");
        let exe = std::env::current_exe()?;
        platform::shortcut(
            &desktop.join(if cfg!(target_os = "windows") {
                "Craft Apps Manager.lnk"
            } else {
                "craft-apps-manager.desktop"
            }),
            &exe,
            "",
            &home,
        )?;
        platform::shortcut(
            &desktop.join(if cfg!(target_os = "windows") {
                "Craft Apps Builder.lnk"
            } else {
                "craft-apps-builder.desktop"
            }),
            &exe,
            "--builder",
            &home,
        )?;
        return Ok(());
    }
    // Commands for scripts and end-to-end checks.
    let text = |key: &str| {
        args.iter()
            .position(|s| s == key)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    if args.iter().any(|s| s == "--list-apps") {
        let entries = craft_apps_manager::catalog::refresh(&paths.root)?;
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }
    if text("--release-format").is_some() || text("--architecture").is_some() {
        let app = text("--install-app").context(
            "Package options require --install-app; there are no global installation settings",
        )?;
        craft_apps_manager::model::valid_app(&app)?;
        let mut preferences = paths.read_preferences()?;
        let current = craft_apps_manager::install_options::preferred(&preferences, &app);
        let options = craft_apps_manager::install_options::InstallOptions {
            release_format: text("--release-format").unwrap_or(current.release_format),
            architecture: text("--architecture").unwrap_or(current.architecture),
        };
        options.validate()?;
        preferences
            .app_install_options
            .insert(craft_apps_manager::install_options::identity(&app), options);
        files::write_json(&paths.at("manager-settings.json"), &preferences)?;
    }
    if let Some(app) = text("--install-app") {
        let job = Job::new(paths.at("logs/updates.log"), &paths.builder_preferences()?);
        updates::install_app(&paths, &app, &job)
            .with_context(|| format!("See {}", job.log_path.display()))?;
        let installed = craft_apps_manager::apps::installed(&paths, &app)?;
        println!(
            "{} {} {} {}",
            installed.name, installed.version, installed.install_kind, installed.path
        );
        return Ok(());
    }
    if let Some(app) = text("--uninstall-app") {
        let job = Job::new(paths.at("logs/updates.log"), &paths.builder_preferences()?);
        job.tracked("Uninstall", &app, |_| {
            craft_apps_manager::apps::uninstall(&paths, &app)
        })?;
        println!("{app} uninstalled");
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
    let icon = image::load_from_memory(include_bytes!("../assets/icon.png"))?.into_rgba8();
    let icon = eframe::egui::IconData {
        width: icon.width(),
        height: icon.height(),
        rgba: icon.into_raw(),
    };
    // Screenshot runs can pick a window size, e.g. CRAFT_SCREENSHOT_SIZE=1280x800.
    let screenshot_size = std::env::var("CRAFT_SCREENSHOT_SIZE")
        .ok()
        .and_then(|size| {
            let (width, height) = size.split_once('x')?;
            Some([width.parse::<f32>().ok()?, height.parse::<f32>().ok()?])
        });
    let options = eframe::NativeOptions {
        #[cfg(target_os = "linux")]
        event_loop_builder: Some(Box::new(move |event_loop| {
            // Wayland cannot hide a winit window. Prefer XWayland when present
            // so enabling close-to-tray later also works without a restart.
            if platform::prefer_x11_for_manager(builder) {
                use winit::platform::x11::EventLoopBuilderExtX11;
                event_loop.with_x11();
            }
        })),
        viewport: eframe::egui::ViewportBuilder::default()
            .with_icon(icon)
            .with_inner_size(screenshot_size.unwrap_or(if builder {
                [1050.0, 740.0]
            } else {
                [1320.0, 820.0]
            }))
            .with_min_inner_size(if builder {
                [780.0, 580.0]
            } else {
                [1100.0, 480.0]
            }),
        ..Default::default()
    };
    if !builder {
        let migration_paths = paths.clone();
        std::thread::spawn(move || {
            if let Err(error) =
                craft_apps_manager::portable::migrate_installation_labels(&migration_paths)
            {
                eprintln!("Installation label migration: {error:#}");
            }
        });
    }
    eframe::run_native(
        if builder {
            "Craft Apps Builder"
        } else {
            "Craft Apps Manager"
        },
        options,
        Box::new(move |cc| Ok(Box::new(ui::App::new(cc, paths, home, builder)?))),
    )
    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok(())
}
