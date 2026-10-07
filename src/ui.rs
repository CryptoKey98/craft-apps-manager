use anyhow::Result;
use craft_apps_updater::{
    apps, backups, builder, files,
    jobs::{Job, State},
    model::{self, BuilderPreferences, Paths, Preferences, APPS, SOURCES},
    platform, scheduler, tools, updates,
};
use eframe::egui::{self, Color32, RichText};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, process::Command, sync::atomic::Ordering, time::Duration};
const BLUE: Color32 = Color32::from_rgb(70, 150, 245);
type ReleaseCheck = Result<Option<String>, String>;
type CheckMessage = (String, String, ReleaseCheck);
#[derive(Default, Serialize, Deserialize)]
pub struct Locations {
    pub root: Option<PathBuf>,
    pub tools: Option<PathBuf>,
}
pub struct App {
    paths: Paths,
    home: PathBuf,
    builder: bool,
    app: String,
    latest: bool,
    job: Job,
    preferences: Preferences,
    build_preferences: BuilderPreferences,
    settings: bool,
    selection: bool,
    source_selection: bool,
    selection_draft: Vec<String>,
    settings_draft: Preferences,
    build_draft: BuilderPreferences,
    auto: bool,
    auto_source: bool,
    error: Option<String>,
    root_text: String,
    tools_text: String,
    confirm_clear: bool,
    confirm_clean: bool,
    closing: bool,
    capture_frame: usize,
    launch_settings_open: bool,
    launch_draft: apps::LaunchSettings,
    launch_arguments: String,
    confirm_uninstall: bool,
    app_selected: bool,
    confirm_install: Option<String>,
    release_checks: std::collections::BTreeMap<String, (String, ReleaseCheck)>,
    check_receiver: Option<std::sync::mpsc::Receiver<CheckMessage>>,
}
impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        paths: Paths,
        home: PathBuf,
        builder: bool,
    ) -> Result<Self> {
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        style.animation_time = 0.18;
        style.visuals.panel_fill = Color32::from_rgb(35, 35, 35);
        style.visuals.window_fill = Color32::from_rgb(43, 43, 43);
        style.visuals.extreme_bg_color = Color32::from_rgb(25, 25, 25);
        style.visuals.faint_bg_color = Color32::from_rgb(48, 48, 48);
        style.visuals.selection.bg_fill = BLUE;
        style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(58, 58, 58);
        style.visuals.widgets.inactive.fg_stroke.color = Color32::from_gray(210);
        style.visuals.widgets.noninteractive.fg_stroke.color = Color32::from_gray(210);
        style.spacing.item_spacing = egui::vec2(10.0, 10.0);
        style.spacing.button_padding = egui::vec2(14.0, 7.0);
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
        cc.egui_ctx.set_style(style);
        let mut fonts = egui::FontDefinitions::default();
        if let Ok(bytes) = std::fs::read("C:/Windows/Fonts/segoeui.ttf") {
            fonts.font_data.insert(
                "Segoe UI".into(),
                std::sync::Arc::new(egui::FontData::from_owned(bytes)),
            );
            fonts
                .families
                .get_mut(&egui::FontFamily::Proportional)
                .unwrap()
                .insert(0, "Segoe UI".into());
        }
        cc.egui_ctx.set_fonts(fonts);
        let preferences = paths.preferences()?;
        let build_preferences = paths.builder_preferences()?;
        let args: Vec<_> = std::env::args().collect();
        let app = args
            .windows(2)
            .find(|a| a[0] == "--app")
            .map(|a| a[1].clone())
            .unwrap_or_else(|| "filmcraft".into());
        model::valid_app(&app)?;
        let job = Job::new(
            paths.at(if builder {
                format!("logs/{app}.log")
            } else {
                "logs/updates.log".into()
            }),
            &build_preferences,
        );
        job.history(&job.log_path);
        if builder {
            job.state.lock().unwrap().output = builder::history(&paths, &app)
        }
        let auto = scheduler::enabled(false);
        let auto_source = scheduler::enabled(true);
        Ok(Self {
            root_text: paths.root.display().to_string(),
            tools_text: paths.tools.display().to_string(),
            paths,
            home,
            builder,
            app,
            latest: true,
            job,
            settings: std::env::args().any(|a| a == "--preview-settings" || a == "--preview-apps"),
            selection: std::env::args().any(|a| a == "--preview-apps"),
            source_selection: false,
            selection_draft: preferences.selected_apps.clone(),
            settings_draft: preferences.clone(),
            build_draft: build_preferences.clone(),
            preferences,
            build_preferences,
            auto,
            auto_source,
            error: None,
            confirm_clear: false,
            confirm_clean: false,
            closing: false,
            capture_frame: 0,
            launch_settings_open: false,
            launch_draft: Default::default(),
            launch_arguments: String::new(),
            confirm_uninstall: false,
            app_selected: std::env::args().any(|a| a == "--preview-details"),
            confirm_install: None,
            release_checks: Default::default(),
            check_receiver: None,
        })
    }
    fn result(&mut self, result: Result<()>) {
        if let Err(e) = result {
            self.error = Some(format!("{e:#}"));
        }
    }
    fn check_selected_app(&mut self, ctx: &egui::Context, installed_version: String) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.check_receiver = Some(rx);
        self.release_checks.remove(&self.app);
        let app = self.app.clone();
        let paths = self.paths.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let result = updates::check_app(&paths, &app).map_err(|e| format!("{e:#}"));
            // Keep fast cached checks visible long enough to acknowledge the click.
            std::thread::sleep(Duration::from_millis(750).saturating_sub(started.elapsed()));
            let _ = tx.send((app, installed_version, result));
            ctx.request_repaint();
        });
    }
    fn start(&mut self, action: &str) {
        let paths = self.paths.clone();
        let app = self.app.clone();
        let latest = self.latest;
        let action = action.to_string();
        let log = if self.builder {
            format!("logs/{app}.log")
        } else {
            "logs/updates.log".into()
        };
        let previous = self.job.state.lock().unwrap().output.clone();
        self.job = Job::new(paths.at(log), &self.build_preferences);
        self.job.history(&self.job.log_path);
        self.job.state.lock().unwrap().output = previous;
        self.job.spawn(move |job| match action.as_str() {
            "releases" => updates::releases(&paths, &job, false),
            "install-app" => updates::install_app(&paths, &app, &job),
            "uninstall-app" => apps::uninstall(&paths, &app),
            "sources" => updates::sources(&paths, &paths.preferences()?.selected_sources, &job),
            "build" => builder::build(&paths, &app, latest, &job),
            "setup" => tools::setup(&paths, &app, &job),
            "clear" => backups::clear(&paths),
            "clean" => builder::clean(&paths),
            _ => unreachable!(),
        });
    }
    fn open_settings(&mut self) {
        self.settings_draft = self.preferences.clone();
        self.build_draft = self.build_preferences.clone();
        self.root_text = self.paths.root.display().to_string();
        self.tools_text = self.paths.tools.display().to_string();
        self.settings = true;
    }
    fn open_builder(&mut self) {
        let result = (|| -> Result<()> {
            Command::new(std::env::current_exe()?)
                .arg("--builder")
                .arg("--app")
                .arg(&self.app)
                .arg("--root")
                .arg(&self.paths.root)
                .arg("--tools")
                .arg(&self.paths.tools)
                .spawn()?;
            Ok(())
        })();
        self.result(result)
    }
    fn settings_ui(&mut self, ctx: &egui::Context) {
        let mut show = self.settings;
        if !show {
            return;
        }
        egui::Window::new(if self.builder{"Builder settings"}else{"Updater settings"}).open(&mut show).collapsible(false).resizable(false).default_width(570.0).anchor(egui::Align2::CENTER_CENTER,[0.0,0.0]).vscroll(false).show(ctx,|ui|{
            ui.heading(if self.builder{"Build maintenance"}else{"Release preferences"});ui.separator();
egui::ScrollArea::vertical().max_height((ctx.screen_rect().height()-200.0).max(240.0)).show(ui,|ui|{
            if self.builder{
                ui.checkbox(&mut self.build_draft.delete_cache_after_success,"Delete compilation cache after a successful build");ui.checkbox(&mut self.build_draft.delete_workspace_after_success,"Delete extracted source and node_modules after success");
                ui.horizontal(|ui|{ui.label("Rotate each app log at (MB)");ui.add(egui::DragValue::new(&mut self.build_draft.log_size_mb).range(1..=100));});ui.horizontal(|ui|{ui.label("Older log files to keep");ui.add(egui::DragValue::new(&mut self.build_draft.log_archives).range(0..=5));});
                ui.small("Cancelled and failed builds keep their cache. Completed builds and tools are retained.");if ui.button("Clean temporary files now...").clicked(){self.confirm_clean=true;}
            }else{
                ui.horizontal(|ui|{ui.label("Release format");egui::ComboBox::from_id_salt("format").selected_text(if self.settings_draft.release_format=="portable"{"Portable ZIP"}else{"Installer"}).show_ui(ui,|ui|{ui.selectable_value(&mut self.settings_draft.release_format,"portable".into(),"Portable ZIP");ui.selectable_value(&mut self.settings_draft.release_format,"installer".into(),"Installer");});});
                ui.horizontal(|ui|{ui.label("Architecture");egui::ComboBox::from_id_salt("arch").selected_text(&self.settings_draft.architecture).show_ui(ui,|ui|{for (value,label) in [("x64","64-bit (x64)"),("x86","32-bit (x86)"),("arm64","ARM64")]{ui.selectable_value(&mut self.settings_draft.architecture,value.into(),label);}});});
                ui.horizontal(|ui|{if ui.button("Choose release apps...").clicked(){self.source_selection=false;self.selection_draft=self.settings_draft.selected_apps.clone();self.selection=true;}ui.label(format!("{} of 7 apps selected",self.settings_draft.selected_apps.len()));});ui.horizontal(|ui|{if ui.button("Choose source apps...").clicked(){self.source_selection=true;self.selection_draft=self.settings_draft.selected_sources.clone();self.selection=true;}ui.label(format!("{} of 8 sources selected",self.settings_draft.selected_sources.len()));});
                ui.separator();ui.checkbox(&mut self.settings_draft.keep_app_backups,"Create app backups (portable ZIP releases only)");ui.checkbox(&mut self.settings_draft.keep_source_backups,"Create source backups");ui.checkbox(&mut self.settings_draft.compress_backups,"Compress portable app backups (7-Zip Ultra / LZMA2)");ui.checkbox(&mut self.settings_draft.compress_source_backups,"Recompress source backups (7-Zip Ultra / LZMA2)");ui.checkbox(&mut self.settings_draft.notify_updates,"Notify me when automatic updates install new versions");
                ui.horizontal(|ui|{ui.label("Previous versions to keep per app / source");ui.add(egui::DragValue::new(&mut self.settings_draft.backup_versions).range(1..=10));});
                ui.small("A temporary rollback copy is kept until the update succeeds. Installer mode downloads and opens the Windows installer wizard.");if ui.button("Clear backups...").clicked(){self.confirm_clear=true;}
            }
            ui.separator();ui.collapsing("Folders",|ui|{ui.label("Data folder (releases, sources, builds, logs, backups)");ui.add(egui::TextEdit::singleline(&mut self.root_text).desired_width(520.0));ui.label("Build tools folder");ui.add(egui::TextEdit::singleline(&mut self.tools_text).desired_width(520.0));ui.small("Use your existing PowerShell data folder to access its sources and builds. Folder changes apply after reopening this window.");});
            });
            ui.separator();ui.horizontal(|ui|{
                if ui.button("Save").clicked(){let result=(||->Result<()>{self.settings_draft.validate()?;if self.builder{files::write_json(&self.paths.at("builder-settings.json"),&self.build_draft)?;self.build_preferences=self.build_draft.clone();}else{files::write_json(&self.paths.at("updater-settings.json"),&self.settings_draft)?;self.preferences=self.settings_draft.clone();}let root=PathBuf::from(self.root_text.trim());let tools=PathBuf::from(self.tools_text.trim());if !root.is_absolute()||!tools.is_absolute(){anyhow::bail!("Folder paths must be absolute");}files::write_json(&self.home.join("data-root.json"),&Locations{root:Some(root),tools:Some(tools)})?;self.settings=false;Ok(())})();self.result(result);}
                if ui.button("Cancel").clicked(){self.settings=false;}
            });
        });
        if !show {
            self.settings = false;
        }
    }
    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(app) = self.confirm_install.clone() {
            let installer = self.preferences.release_format == "installer";
            egui::Window::new("Confirm installation")
                .collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0,0.0])
                .show(ctx, |ui| {
                    ui.label(if installer {
                        format!("Download and install the latest {} release using its Windows installer?", model::title(&app))
                    } else {
                        format!("Are you sure you want to download and install {}?", model::title(&app))
                    });
                    ui.small(format!("Source: https://github.com/storytold/{}/releases/latest", model::repository(&app)));
                    ui.small(format!("Latest stable release · {} · {}", self.preferences.architecture, self.preferences.release_format));
                    if installer { ui.small("The Windows installer will open. Follow its wizard; Windows may ask for administrator permission."); }
                    ui.horizontal(|ui| {
                        if ui.button("Yes").clicked() {
                            self.app = app.clone();
                            self.confirm_install = None;
                            self.start("install-app");
                        }
                        if ui.button("No").clicked() { self.confirm_install = None; }
                    });
                });
        }
        if self.launch_settings_open {
            egui::Window::new(format!("{} launch settings", model::title(&self.app)))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("Executable");
                    egui::ComboBox::from_id_salt("launch-executable")
                        .selected_text(if self.launch_draft.executable.is_empty() {
                            "Default executable"
                        } else {
                            &self.launch_draft.executable
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.launch_draft.executable,
                                String::new(),
                                "Default executable",
                            );
                            for exe in apps::executables(&self.paths, &self.app).unwrap_or_default()
                            {
                                ui.selectable_value(
                                    &mut self.launch_draft.executable,
                                    exe.clone(),
                                    &exe,
                                );
                            }
                        });
                    ui.label("Launch arguments — one argument per line");
                    ui.add(
                        egui::TextEdit::multiline(&mut self.launch_arguments)
                            .desired_width(400.0)
                            .desired_rows(5),
                    );
                    ui.small("Arguments are passed directly to the app. Do not add shell quotes.");
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() {
                            self.launch_draft.arguments = self
                                .launch_arguments
                                .lines()
                                .filter(|s| !s.is_empty())
                                .map(String::from)
                                .collect();
                            let result = apps::save(&self.paths, &self.app, &self.launch_draft);
                            if result.is_ok() {
                                self.launch_settings_open = false;
                            }
                            self.result(result);
                        }
                        if ui.button("Cancel").clicked() {
                            self.launch_settings_open = false;
                        }
                    });
                });
        }
        if self.confirm_uninstall {
            egui::Window::new(format!("Uninstall {}?", model::title(&self.app)))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("Are you sure you want to uninstall this app?");
                    if let Ok(app) = apps::installed(&self.paths, &self.app) {
                        ui.label(app.path);
                    }
                    ui.small("Source ZIPs, builds, backups and launch settings will be kept.");
                    ui.horizontal(|ui| {
                        if ui.button("Uninstall").clicked() {
                            self.start("uninstall-app");
                            self.confirm_uninstall = false;
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm_uninstall = false;
                        }
                    });
                });
        }
        if self.selection {
            let choices: &[&str] = if self.source_selection {
                &SOURCES
            } else {
                &APPS
            };
            egui::Window::new(if self.source_selection {
                "Choose source apps"
            } else {
                "Choose release apps"
            })
            .collapsible(false)
            .resizable(false)
            .default_width(340.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(if self.source_selection {
                    "Choose which source ZIPs to update. ArtCraft X is source only."
                } else {
                    "Choose which apps receive release updates."
                });
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_min_width(320.0);
                    egui::ScrollArea::vertical()
                        .max_height(250.0)
                        .show(ui, |ui| {
                            for &name in choices {
                                let mut checked = self.selection_draft.iter().any(|s| s == name);
                                if ui.checkbox(&mut checked, model::title(name)).changed() {
                                    if checked {
                                        self.selection_draft.push(name.into())
                                    } else {
                                        self.selection_draft.retain(|s| s != name)
                                    }
                                }
                            }
                        });
                });
                ui.horizontal(|ui| {
                    if ui.button("Select all").clicked() {
                        self.selection_draft = choices.iter().map(|s| s.to_string()).collect();
                    }
                    if ui.button("Deselect all").clicked() {
                        self.selection_draft.clear();
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("OK").clicked() {
                        if self.source_selection {
                            self.settings_draft.selected_sources = self.selection_draft.clone();
                        } else {
                            self.settings_draft.selected_apps = self.selection_draft.clone();
                        }
                        self.selection = false;
                    }
                    if ui.button("Cancel").clicked() {
                        self.selection = false;
                    }
                });
            });
        }
        if self.confirm_clear || self.confirm_clean {
            egui::Window::new("Confirm cleanup")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(if self.confirm_clear {
                        "Delete managed release and source backups?"
                    } else {
                        "Delete compilation caches and extracted source workspaces?"
                    });
                    ui.small("Installed apps, finished builds, source ZIPs and tools will remain.");
                    ui.horizontal(|ui| {
                        if ui.button("Delete").clicked() {
                            self.start(if self.confirm_clear { "clear" } else { "clean" });
                            self.confirm_clear = false;
                            self.confirm_clean = false;
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm_clear = false;
                            self.confirm_clean = false;
                        }
                    });
                });
        }
        if let Some(error) = self.error.clone() {
            egui::Window::new("Craft Apps Updater")
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.colored_label(Color32::from_rgb(240, 130, 120), error);
                    if ui.button("OK").clicked() {
                        self.error = None;
                    }
                });
        }
    }
    fn log_panel(&self, ui: &mut egui::Ui, state: &State) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(if self.builder {
                    "BUILD LOG"
                } else {
                    "ACTIVITY LOG"
                })
                .small()
                .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Open log").clicked() {
                    let _ = platform::open(&self.job.log_path);
                }
            });
        });
        egui::Frame::new()
            .fill(Color32::from_rgb(22, 22, 22))
            .inner_margin(12.0)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .max_height(ui.available_height() - 20.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(if state.log.is_empty() {
                                    "Ready. No update check runs when you open this window."
                                } else {
                                    &state.log
                                })
                                .monospace()
                                .size(12.5)
                                .color(Color32::from_rgb(205, 205, 205)),
                            )
                            .selectable(true)
                            .wrap_mode(egui::TextWrapMode::Extend),
                        );
                    });
            });
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if let Some(receiver) = &self.check_receiver {
            if let Ok((app, version, result)) = receiver.try_recv() {
                self.release_checks.insert(app, (version, result));
                self.check_receiver = None;
            }
        }
        if let Ok(path) = std::env::var("CRAFT_SCREENSHOT_TO") {
            self.capture_frame += 1;
            if self.capture_frame == 10 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            ctx.input(|i| {
                for event in &i.events {
                    if let egui::Event::Screenshot { image, .. } = event {
                        let rgba: Vec<u8> =
                            image.pixels.iter().flat_map(|p| p.to_array()).collect();
                        if let Err(e) = image::save_buffer(
                            &path,
                            &rgba,
                            image.size[0] as u32,
                            image.size[1] as u32,
                            image::ColorType::Rgba8,
                        ) {
                            self.error = Some(e.to_string());
                        } else {
                            // This mode is only for unattended visual verification.
                            std::process::exit(0);
                        }
                    }
                }
            });
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        let state = self.job.state.lock().unwrap().clone();
        if ctx.input(|i| i.viewport().close_requested()) && state.busy {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.closing = true;
            self.job.cancel.store(true, Ordering::Relaxed);
        }
        if self.closing && !state.busy {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        egui::TopBottomPanel::top("header")
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(28, 28, 28))
                    .inner_margin(16.0),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(if self.builder {
                        "Craft Apps Builder"
                    } else {
                        "Craft Apps Updater"
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(format!("RUST  ·  {}", env!("CARGO_PKG_VERSION"))).small().color(Color32::GRAY));
                    });
                });
                ui.label(
                    RichText::new(if self.builder {
                        "Build original source. Keep control of your tools and output."
                    } else {
                        "Your creative apps, releases and sources in one place."
                    })
                    .color(Color32::from_gray(160)),
                );
            });
        egui::TopBottomPanel::bottom("footer").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.small(format!("Data: {}", self.paths.root.display()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.small(if state.busy { "Working" } else { "Ready" });
                });
            });
        });
        egui::SidePanel::left("sidebar")
            .resizable(false)
            .exact_width(220.0)
            .show(ctx, |ui| {
                ui.add_space(16.0);
                ui.label(
                    RichText::new(if self.builder {
                        "BUILD CONTROLS"
                    } else {
                        "CREATIVE APPS"
                    })
                    .small()
                    .strong()
                    .color(Color32::GRAY),
                );
                ui.add_space(6.0);
                if self.builder {
                    let previous = self.app.clone();
                    egui::ComboBox::from_id_salt("app")
                        .width(185.0)
                        .selected_text(model::title(&self.app))
                        .show_ui(ui, |ui| {
                            for name in SOURCES {
                                ui.add_enabled_ui(!state.busy, |ui| {
                                    ui.selectable_value(
                                        &mut self.app,
                                        name.into(),
                                        model::title(name),
                                    );
                                });
                            }
                        });
                    if previous != self.app {
                        self.job = Job::new(
                            self.paths.at(format!("logs/{}.log", self.app)),
                            &self.build_preferences,
                        );
                        self.job.history(&self.job.log_path);
                        self.job.state.lock().unwrap().output =
                            builder::history(&self.paths, &self.app);
                    }
                    ui.add_enabled(
                        !state.busy,
                        egui::Checkbox::new(&mut self.latest, "Use latest source"),
                    );
                    ui.small(
                        "Unchecked builds from your local source ZIP without contacting GitHub.",
                    );
                    ui.add_space(10.0);
                    if ui
                        .add_enabled(
                            !state.busy,
                            egui::Button::new("Build executable")
                                .fill(BLUE)
                                .min_size(egui::vec2(190.0, 34.0)),
                        )
                        .clicked()
                    {
                        self.start("build")
                    }
                    if ui
                        .add_enabled(
                            !state.busy,
                            egui::Button::new("Set up build tools")
                                .min_size(egui::vec2(190.0, 32.0)),
                        )
                        .clicked()
                    {
                        self.start("setup")
                    }
                    if ui
                        .add_enabled(
                            state.busy,
                            egui::Button::new("Cancel build").min_size(egui::vec2(190.0, 32.0)),
                        )
                        .clicked()
                    {
                        self.job.cancel.store(true, Ordering::Relaxed);
                    }
                    if ui
                        .add_enabled(
                            state.output.is_some(),
                            egui::Button::new("Open build folder")
                                .min_size(egui::vec2(190.0, 32.0)),
                        )
                        .clicked()
                    {
                        if let Some(out) = &state.output {
                            self.result(platform::open(out));
                        }
                    }
                } else {
                    let config = self.paths.config();
                    for name in APPS {
                        let version = config
                            .as_ref()
                            .ok()
                            .and_then(|c| c.apps.iter().find(|a| a.name == name))
                            .filter(|a| !a.version.is_empty())
                            .map(|a| a.version.as_str())
                            .unwrap_or("Not installed");
                        let selected = self.app_selected && self.app == name;
                        let (rect, response) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), 56.0),
                            egui::Sense::click(),
                        );
                        if selected || response.hovered() {
                            ui.painter().rect_filled(
                                rect,
                                3.0,
                                if selected {
                                    Color32::from_rgb(36, 72, 110)
                                } else {
                                    Color32::from_gray(48)
                                },
                            );
                        }
                        if selected {
                            ui.painter().rect_filled(
                                egui::Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())),
                                0.0,
                                BLUE,
                            );
                        }
                        let enabled = self.preferences.selected_apps.iter().any(|s| s == name);
                        ui.painter().circle_filled(
                            rect.min + egui::vec2(12.0, 19.0),
                            3.0,
                            if enabled { BLUE } else { Color32::GRAY },
                        );
                        ui.painter().text(
                            rect.min + egui::vec2(25.0, 10.0),
                            egui::Align2::LEFT_TOP,
                            model::title(name),
                            egui::FontId::proportional(14.0),
                            Color32::from_gray(220),
                        );
                        ui.painter().text(
                            rect.min + egui::vec2(25.0, 34.0),
                            egui::Align2::LEFT_TOP,
                            version,
                            egui::FontId::proportional(11.0),
                            Color32::from_gray(165),
                        );
                        if response.clicked() {
                            self.app_selected = !selected;
                            self.app = name.into();
                            self.launch_settings_open = false;
                            self.confirm_uninstall = false;
                        }
                    }
                    ui.separator();
                    ui.label(
                        RichText::new(format!(
                            "{} of 7 apps selected",
                            self.preferences.selected_apps.len()
                        ))
                        .size(13.0)
                        .color(Color32::from_gray(190)),
                    );
                }
                ui.allocate_ui_with_layout(
                    ui.available_size(),
                    egui::Layout::bottom_up(egui::Align::Center),
                    |ui| {
                        ui.add_space(12.0);
                        if ui
                            .add_enabled(
                                !state.busy,
                                egui::Button::new("Settings").min_size(egui::vec2(190.0, 32.0)),
                            )
                            .clicked()
                        {
                            self.open_settings()
                        }
                        ui.add_space(4.0);
                        ui.separator();
                    },
                );
            });
        if !self.builder {
            egui::SidePanel::right("app-details")
                .default_width(250.0)
                .min_width(220.0)
                .show_animated(ctx, self.app_selected, |ui| {
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        ui.heading(model::title(&self.app));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .small_button("×")
                                .on_hover_text("Close app details")
                                .clicked()
                            {
                                self.app_selected = false;
                            }
                        });
                    });
                    let installed = apps::installed(&self.paths, &self.app)
                        .ok()
                        .filter(|a| !a.version.is_empty());
                    if let Some(app) = &installed {
                        ui.label(format!("Version {} · {}", app.version, app.architecture));
                        ui.small(if app.install_kind == "installer" {
                            "Installed with Windows installer"
                        } else {
                            "Installed portable release"
                        });
                    } else {
                        ui.label("Not installed");
                    }
                    ui.add_space(12.0);
                    if ui
                        .add_enabled(
                            installed.is_some(),
                            egui::Button::new("Launch")
                                .fill(BLUE)
                                .min_size(egui::vec2(210.0, 36.0)),
                        )
                        .clicked()
                    {
                        self.result(apps::launch(&self.paths, &self.app));
                    }
                    ui.separator();
                    if ui
                        .add_enabled(
                            installed.is_some(),
                            egui::Button::new("Launch settings…").min_size(egui::vec2(210.0, 30.0)),
                        )
                        .clicked()
                    {
                        match apps::settings(&self.paths, &self.app) {
                            Ok(settings) => {
                                self.launch_arguments = settings.arguments.join("\n");
                                self.launch_draft = settings;
                                self.launch_settings_open = true;
                            }
                            Err(error) => self.result(Err(error)),
                        }
                    }
                    if ui
                        .add_enabled(
                            installed.is_some(),
                            egui::Button::new("Open app folder").min_size(egui::vec2(210.0, 30.0)),
                        )
                        .clicked()
                    {
                        if let Some(app) = &installed {
                            let path = PathBuf::from(&app.path);
                            let folder = if path.is_file() {
                                path.parent().unwrap_or(&path)
                            } else {
                                &path
                            };
                            self.result(platform::open(folder));
                        }
                    }
                    if ui.button("Build from source").clicked() {
                        self.open_builder();
                    }
                    ui.hyperlink_to(
                        "View repository",
                        format!(
                            "https://github.com/storytold/{}",
                            model::repository(&self.app)
                        ),
                    );
                    if let Some(app) = &installed {
                        ui.add_space(8.0);
                        ui.small(&app.path);
                    }
                    ui.add_space(22.0);
                    ui.separator();
                    let matching_format = installed.as_ref().is_some_and(|app| {
                        (app.install_kind == "installer")
                            == (self.preferences.release_format == "installer")
                    });
                    let checked = self.release_checks.get(&self.app).filter(|(v, _)| {
                        matching_format && installed.as_ref().is_some_and(|a| a.version == *v)
                    });
                    let available =
                        checked.is_some_and(|(_, result)| matches!(result, Ok(Some(_))));
                    let install_label = if self.check_receiver.is_some() {
                        "Checking…"
                    } else if matching_format && !available {
                        "Check for updates"
                    } else if matching_format {
                        "Update (latest release)"
                    } else {
                        "Install (latest release)"
                    };
                    let mut button =
                        egui::Button::new(install_label).min_size(egui::vec2(210.0, 36.0));
                    if available {
                        let pulse = (ctx.input(|i| i.time) * 3.0).sin() as f32 * 0.5 + 0.5;
                        button = button.stroke(egui::Stroke::new(
                            1.5 + pulse,
                            Color32::from_rgb(70, (130.0 + 60.0 * pulse) as u8, 245),
                        ));
                        ctx.request_repaint_after(Duration::from_millis(33));
                    }
                    let clicked = ui
                        .add_enabled(!state.busy && self.check_receiver.is_none(), button)
                        .clicked();
                    if self.check_receiver.is_some() {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.small("Checking for updates…");
                        });
                        ctx.request_repaint_after(Duration::from_millis(33));
                    }
                    if let Some((_, result)) = checked {
                        match result {
                            Ok(None) => {
                                ui.small("You’re running the latest version.");
                            }
                            Ok(Some(version)) => {
                                ui.small(format!("Version {version} is available."));
                            }
                            Err(error) => {
                                ui.small(
                                    RichText::new(error).color(Color32::from_rgb(230, 130, 130)),
                                );
                            }
                        }
                    }
                    if clicked {
                        if let Some(app) = &installed {
                            if matching_format && !available {
                                self.check_selected_app(ctx, app.version.clone());
                            } else {
                                self.confirm_install = Some(self.app.clone());
                            }
                        } else {
                            self.confirm_install = Some(self.app.clone());
                        }
                    }
                    if ui
                        .add_enabled(
                            installed.is_some() && !state.busy,
                            egui::Button::new(
                                RichText::new("Uninstall…").color(Color32::from_rgb(230, 130, 130)),
                            )
                            .min_size(egui::vec2(210.0, 30.0)),
                        )
                        .clicked()
                    {
                        self.confirm_uninstall = true;
                    }
                    if installed.as_ref().is_some_and(|app| {
                        app.install_kind == "installer" && app.product_code.is_empty()
                    }) {
                        ui.small("Use Windows Installed apps to remove this installer version.");
                        if ui.button("Windows Installed apps").clicked() {
                            self.result(
                                Command::new("explorer.exe")
                                    .arg("ms-settings:appsfeatures")
                                    .spawn()
                                    .map(|_| ())
                                    .map_err(Into::into),
                            );
                        }
                    }
                });
        }
        egui::CentralPanel::default().show(ctx,|ui|{ui.add_space(12.0);
            if !self.builder{ui.horizontal_top(|ui|{
ui.vertical(|ui|{ui.set_width(225.0);if ui.add_enabled(!state.busy,egui::Button::new("Update selected releases").fill(BLUE).min_size(egui::vec2(225.0,36.0))).clicked(){self.start("releases")}});
ui.vertical(|ui|{ui.set_width(225.0);if ui.add_enabled(!state.busy,egui::Button::new("Update selected sources").min_size(egui::vec2(225.0,36.0))).clicked(){self.start("sources")}});
if ui.button("Build from source").clicked(){self.open_builder()}});ui.horizontal(|ui|{ui.spacing_mut().item_spacing.x=3.0;ui.small("Choose apps for release and source updates in");if ui.link(RichText::new("Settings").small().color(BLUE)).clicked(){self.open_settings();}});
                ui.horizontal(|ui|{if ui.add_enabled(!state.busy,egui::Checkbox::new(&mut self.auto,"Automatic app updates")).changed(){let result=scheduler::set(&self.paths,false,self.auto);if result.is_err(){self.auto= !self.auto;}self.result(result)}
if ui.add_enabled(!state.busy,egui::Checkbox::new(&mut self.auto_source,"Automatic source updates")).changed(){let result=scheduler::set(&self.paths,true,self.auto_source);if result.is_err(){self.auto_source= !self.auto_source;}self.result(result)}});ui.small("Automatic updates run hourly and after sign-in. Opening this window never checks GitHub.");ui.horizontal(|ui|{if ui.button("Open releases").clicked(){self.result(platform::open(&self.paths.at("releases")));}
if ui.button("Open sources").clicked(){self.result(platform::open(&self.paths.at("sources")));}
if state.busy&&ui.button("Cancel update").clicked(){self.job.cancel.store(true,Ordering::Relaxed);}});ui.separator();}
            ui.horizontal(|ui|{if state.busy{ui.spinner();}ui.strong(if state.stage.is_empty(){if state.output.is_some(){"Previous build available"}else{"Ready"}}else{&state.stage});});
            if !state.detail.is_empty(){ui.label(&state.detail);}
if state.busy{ui.add(egui::ProgressBar::new(state.progress.unwrap_or(0.0)).animate(state.progress.is_none()).text(state.progress.map(|p|format!("{:.0}%",p*100.0)).unwrap_or_else(||"Working…".into())));}ui.add_space(10.0);self.log_panel(ui,&state);
        });
        self.settings_ui(ctx);
        self.dialogs(ctx);
        if state.busy {
            ctx.request_repaint_after(Duration::from_millis(100));
        } else {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }
}
