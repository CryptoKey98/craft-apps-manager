use anyhow::Result;
use craft_apps_manager::{
    apps, backups, builder, files,
    jobs::{Job, State},
    model::{self, BuilderPreferences, Paths, Preferences, APPS, SOURCES},
    platform, profiles, scheduler, self_update, tools, updates,
};
use eframe::egui::{self, Color32, RichText};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, process::Command, sync::atomic::Ordering, time::Duration};
const BLUE: Color32 = Color32::from_rgb(70, 150, 245);
fn app_icon(name: &str) -> &'static [u8] {
    match name {
        "designcraft" => include_bytes!("../assets/app-icons/designcraft.png"),
        "effectcraft" => include_bytes!("../assets/app-icons/effectcraft.png"),
        "filmcraft" => include_bytes!("../assets/app-icons/filmcraft.png"),
        "lightcraft" => include_bytes!("../assets/app-icons/lightcraft.png"),
        "photocraft" => include_bytes!("../assets/app-icons/photocraft.png"),
        "vectorcraft" => include_bytes!("../assets/app-icons/vectorcraft.png"),
        _ => include_bytes!("../assets/app-icons/pdfcraft.png"),
    }
}

fn progress_bar(ui: &mut egui::Ui, progress: Option<f32>) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, 4.0, Color32::from_rgb(48, 48, 48));
    let text = if let Some(progress) = progress {
        let progress = progress.clamp(0.0, 1.0);
        if progress > 0.0 {
            let fill = egui::Rect::from_min_size(
                rect.min,
                egui::vec2(rect.width() * progress, rect.height()),
            );
            ui.painter().rect_filled(fill, 4.0, BLUE);
        }
        format!("{:.0}%", progress * 100.0)
    } else {
        let time = ui.ctx().input(|i| i.time);
        let position = ((1.0 - (time * std::f64::consts::TAU / 2.8).cos()) * 0.5) as f32;
        let width = rect.width() * 0.18;
        let segment = egui::Rect::from_min_size(
            rect.min + egui::vec2((rect.width() - width) * position, 0.0),
            egui::vec2(width, rect.height()),
        );
        ui.painter().rect_filled(segment, 4.0, BLUE);
        ui.ctx().request_repaint_after(Duration::from_millis(16));
        "Working…".into()
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(13.0),
        Color32::WHITE,
    );
}
type ReleaseCheck = Result<Option<String>, String>;
type CheckMessage = (u64, String, String, ReleaseCheck);
type DisplaySnapshot = (
    model::Config,
    std::collections::BTreeMap<String, Vec<backups::Backup>>,
    craft_apps_manager::hourly::Checks,
);
#[derive(Default, Serialize, Deserialize)]
pub struct Locations {
    pub root: Option<PathBuf>,
    pub tools: Option<PathBuf>,
}
pub struct App {
    icons: std::collections::BTreeMap<String, egui::TextureHandle>,
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
    selection_notice: Option<String>,
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
    delete_profile: bool,
    app_selected: bool,
    confirm_install: Option<String>,
    release_checks: std::collections::BTreeMap<String, (String, ReleaseCheck)>,
    check_receiver: std::sync::mpsc::Receiver<CheckMessage>,
    check_sender: std::sync::mpsc::Sender<CheckMessage>,
    checking_apps: std::collections::BTreeSet<String>,
    check_generation: u64,
    apps_startup_pending: bool,
    manager_receiver:
        Option<std::sync::mpsc::Receiver<Result<Option<self_update::Available>, String>>>,
    manager_available: Option<self_update::Available>,
    manager_message: String,
    manager_startup_pending: bool,
    manager_plan: Option<std::sync::mpsc::Receiver<Result<PathBuf, String>>>,
    confirm_self_update: bool,
    restore_app: Option<String>,
    restore_backups: Vec<backups::Backup>,
    restore_selected: Option<usize>,
    confirm_restore: bool,
    backup_delete_mode: bool,
    backup_delete_selected: std::collections::BTreeSet<usize>,
    display_config: Option<model::Config>,
    config_receiver: Option<std::sync::mpsc::Receiver<Result<DisplaySnapshot, String>>>,
    display_backups: std::collections::BTreeMap<String, Vec<backups::Backup>>,
    config_refresh_at: std::time::Instant,
    operation_was_busy: bool,
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
        style.spacing.item_spacing = egui::vec2(10.0, 8.0);
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
        if std::env::args().any(|a| a == "--preview-progress") {
            let mut state = job.state.lock().unwrap();
            state.busy = true;
            state.stage = "Working".into();
        }
        let auto = scheduler::enabled(false);
        let auto_source = scheduler::enabled(true);
        let manager_startup_pending = !builder && preferences.check_manager_on_startup;
        let apps_startup_pending = !builder && preferences.check_installed_apps_on_startup;
        let (check_sender, check_receiver) = std::sync::mpsc::channel();
        let preview_backups = std::env::args().any(|a| a == "--preview-backups");
        let restore_backups = if preview_backups {
            backups::list(&paths, &app)?
        } else {
            Vec::new()
        };
        let restore_app = preview_backups.then(|| app.clone());
        let icons = APPS
            .into_iter()
            .map(|name| -> Result<_> {
                let image = image::load_from_memory(app_icon(name))?.into_rgba8();
                let size = [image.width() as usize, image.height() as usize];
                let pixels = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
                Ok((
                    name.to_owned(),
                    cc.egui_ctx
                        .load_texture(name, pixels, egui::TextureOptions::LINEAR),
                ))
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            icons,
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
            selection_notice: None,
            confirm_clear: false,
            confirm_clean: false,
            closing: false,
            capture_frame: 0,
            launch_settings_open: false,
            launch_draft: Default::default(),
            launch_arguments: String::new(),
            confirm_uninstall: false,
            delete_profile: false,
            app_selected: std::env::args().any(|a| a == "--preview-details"),
            confirm_install: None,
            release_checks: Default::default(),
            check_receiver,
            check_sender,
            checking_apps: Default::default(),
            check_generation: 0,
            apps_startup_pending,
            manager_receiver: None,
            manager_available: None,
            manager_message: String::new(),
            manager_startup_pending,
            manager_plan: None,
            confirm_self_update: false,
            restore_app,
            restore_backups,
            restore_selected: None,
            confirm_restore: false,
            backup_delete_mode: false,
            backup_delete_selected: Default::default(),
            display_config: None,
            config_receiver: None,
            display_backups: Default::default(),
            config_refresh_at: std::time::Instant::now(),
            operation_was_busy: false,
        })
    }
    fn result(&mut self, result: Result<()>) {
        if let Err(e) = result {
            self.error = Some(format!("{e:#}"));
        }
    }
    fn check_manager(&mut self, ctx: &egui::Context) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.manager_receiver = Some(rx);
        self.manager_available = None;
        self.manager_message = "Checking for manager updates…".into();
        let paths = self.paths.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = self_update::check(&paths).map_err(|e| format!("{e:#}"));
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }
    fn check_selected_app(&mut self, ctx: &egui::Context, installed_version: String) {
        self.check_apps(ctx, vec![(self.app.clone(), installed_version)]);
    }
    fn check_apps(&mut self, ctx: &egui::Context, apps: Vec<(String, String)>) {
        let apps: Vec<_> = apps
            .into_iter()
            .filter(|(app, _)| self.checking_apps.insert(app.clone()))
            .collect();
        for (app, _) in &apps {
            self.release_checks.remove(app);
        }
        let tx = self.check_sender.clone();
        let generation = self.check_generation;
        let paths = self.paths.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            for (app, installed_version) in apps {
                let started = std::time::Instant::now();
                let result = updates::check_app(&paths, &app).map_err(|e| format!("{e:#}"));
                // Keep fast cached checks visible long enough to acknowledge the click.
                std::thread::sleep(Duration::from_millis(750).saturating_sub(started.elapsed()));
                if tx
                    .send((generation, app, installed_version, result))
                    .is_err()
                {
                    break;
                }
                ctx.request_repaint();
            }
        });
    }
    fn start(&mut self, action: &str) {
        if action == "releases" && self.preferences.selected_apps.is_empty() {
            self.selection_notice = Some("No release apps are selected. Choose apps in Settings → Choose release apps, then try again.".into());
            return;
        }
        if action == "sources" && self.preferences.selected_sources.is_empty() {
            self.selection_notice = Some("No source apps are selected. Choose apps in Settings → Choose source apps, then try again.".into());
            return;
        }
        let paths = self.paths.clone();
        let app = self.app.clone();
        let latest = self.latest;
        let delete_profile = self.delete_profile;
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
        self.operation_was_busy = true;
        self.job.spawn(move |job| match action.as_str() {
            "releases" => updates::releases(&paths, &job, false),
            "install-app" => updates::install_app(&paths, &app, &job),
            "uninstall-app" => apps::uninstall_with_profile(&paths, &app, delete_profile),
            "sources" => updates::sources(&paths, &paths.preferences()?.selected_sources, &job),
            "build" => builder::build(&paths, &app, latest, &job),
            "setup" => tools::setup(&paths, &app, &job),
            "clear" => backups::clear(&paths),
            "clear-app" => backups::clear_app(&paths, &app),
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
            Command::new(platform::relaunch_executable()?)
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
        egui::Window::new(if self.builder{"Builder settings"}else{"Manager settings"}).open(&mut show).collapsible(false).resizable(false).default_width(570.0).anchor(egui::Align2::CENTER_CENTER,[0.0,0.0]).vscroll(false).show(ctx,|ui|{
            ui.heading(if self.builder{"Build maintenance"}else{"Release preferences"});ui.separator();
egui::ScrollArea::vertical().max_height((ctx.screen_rect().height()-180.0).max(240.0)).show(ui,|ui|{
            if self.builder{
                ui.checkbox(&mut self.build_draft.delete_cache_after_success,"Delete compilation cache after a successful build");ui.checkbox(&mut self.build_draft.delete_workspace_after_success,"Delete extracted source and node_modules after success");
                ui.horizontal(|ui|{ui.label("Rotate each app log at (MB)");ui.add(egui::DragValue::new(&mut self.build_draft.log_size_mb).range(1..=100));});ui.horizontal(|ui|{ui.label("Older log files to keep");ui.add(egui::DragValue::new(&mut self.build_draft.log_archives).range(0..=5));});
                ui.small("Cancelled and failed builds keep their cache. Completed builds and tools are retained.");if ui.button("Clean temporary files now...").clicked(){self.confirm_clean=true;}
            }else{
                ui.horizontal(|ui|{ui.label("Release format");egui::ComboBox::from_id_salt("format").selected_text(if self.settings_draft.release_format=="portable"{if cfg!(target_os = "linux") {"AppImage"} else {"Portable ZIP"}}else{"Installer"}).show_ui(ui,|ui|{ui.selectable_value(&mut self.settings_draft.release_format,"portable".into(),if cfg!(target_os = "linux") {"AppImage"} else {"Portable ZIP"});ui.selectable_value(&mut self.settings_draft.release_format,"installer".into(),"Installer");});});
                ui.horizontal(|ui|{ui.label("Architecture");egui::ComboBox::from_id_salt("arch").selected_text(&self.settings_draft.architecture).show_ui(ui,|ui|{for (value,label) in [("x64","64-bit (x64)"),("x86","32-bit (x86)"),("arm64","ARM64")]{ui.selectable_value(&mut self.settings_draft.architecture,value.into(),label);}});});
                ui.horizontal(|ui|{if ui.button("Choose release apps...").clicked(){self.source_selection=false;self.selection_draft=self.settings_draft.selected_apps.clone();self.selection=true;}ui.label(format!("{} of 7 apps selected",self.settings_draft.selected_apps.len()));});ui.horizontal(|ui|{if ui.button("Choose source apps...").clicked(){self.source_selection=true;self.selection_draft=self.settings_draft.selected_sources.clone();self.selection=true;}ui.label(format!("{} of 8 sources selected",self.settings_draft.selected_sources.len()));});
                ui.separator();ui.checkbox(&mut self.settings_draft.keep_app_backups,"Create app backups (portable releases only)");ui.checkbox(&mut self.settings_draft.keep_source_backups,"Create source backups");ui.checkbox(&mut self.settings_draft.compress_backups,"Compress portable app backups (7-Zip Ultra / LZMA2)");ui.checkbox(&mut self.settings_draft.compress_source_backups,"Recompress source backups (7-Zip Ultra / LZMA2)");ui.checkbox(&mut self.settings_draft.notify_updates,"Notify me when app or source updates are available");ui.checkbox(&mut self.settings_draft.check_installed_apps_on_startup,"Check installed apps for updates on startup").on_hover_text("Checks installed apps in the selected release format. Reports availability only; downloads and installation require confirmation.");ui.checkbox(&mut self.settings_draft.check_manager_on_startup,"Check for a new version of this program on startup").on_hover_text("Checks for a new Craft Apps Manager release. Downloads require your confirmation.");
                ui.horizontal(|ui|{ui.label("Previous versions to keep per app / source");ui.add(egui::DragValue::new(&mut self.settings_draft.backup_versions).range(1..=10));});
                ui.small(if cfg!(target_os = "linux") {"AppImage updates retain a rollback copy until successful. System packages require administrator authorization."} else {"A temporary rollback copy is kept until the update succeeds. Installer mode downloads and opens the Windows installer wizard."});if ui.button("Clear backups...").clicked(){self.confirm_clear=true;}
                if ui.add_enabled(self.manager_receiver.is_none() && self.manager_plan.is_none(), egui::Button::new("Check for updates...")).on_hover_text("Check for a newer Craft Apps Manager release").clicked() { self.check_manager(ctx); }
                let built=env!("CRAFT_BUILD_TIMESTAMP").parse::<i64>().ok().and_then(|t|chrono::DateTime::from_timestamp(t,0)).map(|t|t.format("%Y-%m-%d %H:%M UTC").to_string()).unwrap_or_default();ui.small(format!("Version {} · Build {} · {} · {} {}",env!("CARGO_PKG_VERSION"),built,env!("CRAFT_BUILD_PROFILE"),model::release_os(),model::MANAGER_ARCH));
                if self.manager_receiver.is_some() || self.manager_plan.is_some() {ui.spinner();ctx.request_repaint_after(Duration::from_millis(100));}
                if !self.manager_message.is_empty(){ui.small(&self.manager_message);}
                if self.manager_available.is_some() && ui.add_enabled(!self.job.state.lock().unwrap().busy && self.manager_plan.is_none(),egui::Button::new(if self_update::installed_with_msi() {"Download and install…"} else {"Download and restart…"})).clicked(){self.confirm_self_update=true;}
            }
            ui.separator();ui.collapsing("Folders",|ui|{ui.label("Data folder (releases, sources, builds, logs, backups)");ui.add(egui::TextEdit::singleline(&mut self.root_text).desired_width(520.0));ui.label("Build tools folder");ui.add(egui::TextEdit::singleline(&mut self.tools_text).desired_width(520.0));ui.small("Folder changes apply after reopening this window.");});
            });
            ui.separator();ui.horizontal(|ui|{
                if ui.button("Save").clicked(){let result=(||->Result<()>{self.settings_draft.validate()?;if self.builder{files::write_json(&self.paths.at("builder-settings.json"),&self.build_draft)?;self.build_preferences=self.build_draft.clone();}else{let config=self.paths.config()?;self.paths.save_config(&config)?;files::write_json(&self.paths.at("manager-settings.json"),&self.settings_draft)?;let check_mode_changed=self.preferences.release_format!=self.settings_draft.release_format||self.preferences.architecture!=self.settings_draft.architecture;self.preferences=self.settings_draft.clone();if check_mode_changed{self.check_generation+=1;self.checking_apps.clear();self.release_checks.clear();}self.display_config=None;self.config_receiver=None;self.config_refresh_at=std::time::Instant::now();}let root=PathBuf::from(self.root_text.trim());let tools=PathBuf::from(self.tools_text.trim());if !root.is_absolute()||!tools.is_absolute(){anyhow::bail!("Folder paths must be absolute");}files::write_json(&self.home.join("data-root.json"),&Locations{root:Some(root),tools:Some(tools)})?;self.settings=false;Ok(())})();self.result(result);}
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
                        format!("Download and install the latest {} {} package?", model::title(&app), craft_apps_manager::installers::installer_label())
                    } else {
                        format!("Are you sure you want to download and install {}?", model::title(&app))
                    });
                    ui.small(format!("Source: https://github.com/storytold/{}/releases/latest", model::repository(&app)));
                    ui.small(format!("Latest stable release · {} · {}", self.preferences.architecture, self.preferences.release_format));
                    if installer { ui.small(if cfg!(target_os = "linux") {"You will be asked to authorize package installation."} else {"The Windows installer will open. Follow its wizard; Windows may ask for administrator permission."}); }
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
                    if let Some(app) = self.display_config.as_ref().and_then(|c| c.apps.iter().find(|a| a.name == self.app)) {
                        ui.label(&app.path);
                    }
                    ui.small("Source ZIPs, builds, backups and launch settings will be kept.");
                    let targets=profiles::targets(&self.paths,&self.app);
                    ui.add_space(8.0);
                    ui.add_enabled(targets.is_ok(),egui::Checkbox::new(&mut self.delete_profile,"Delete app profile data"));
                    ui.small("Removes settings, caches, plug-ins and recovery/autosave copies from the folders below. Other installed copies may share this data.");
                    match &targets {
                        Ok(targets)=>{
                            let existing:Vec<_>=targets.iter().filter(|t|t.path.exists()).collect();
                            if existing.is_empty(){ui.small("No existing profile folders found.");}
                            egui::ScrollArea::vertical().max_height(100.0).show(ui,|ui|{for target in existing {ui.small(target.path.display().to_string());}});
                            ui.small("Custom profile locations outside these folders are kept.");
                        }
                        Err(error)=>{ui.small(format!("Could not identify profile folders: {error}"));}
                    }
                    ui.add_space(8.0);
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
        if let Some(app) = self.restore_app.clone() {
            let mut show = true;
            egui::Window::new(format!("{} backups", model::title(&app)))
                .open(&mut show)
                .collapsible(false)
                .resizable(false)
                .default_width(560.0)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.heading("Available backups");
                    ui.horizontal(|ui| {
                        let before=self.backup_delete_mode;
                        ui.selectable_value(&mut self.backup_delete_mode,false,"Restore");
                        ui.selectable_value(&mut self.backup_delete_mode,true,"Delete");
                        if before!=self.backup_delete_mode {self.confirm_restore=false;self.restore_selected=None;self.backup_delete_selected.clear();}
                    });
                    ui.small(if self.backup_delete_mode {"Select the backups you want to delete. Installed apps and source files are kept."}else{"Choose one backup to restore. The selected backup will be kept."});
                    if self.backup_delete_mode {
                        ui.horizontal(|ui| {
                            if ui.button("Select all").clicked(){self.backup_delete_selected=(0..self.restore_backups.len()).collect();self.confirm_restore=false;}
                            if ui.button("Deselect all").clicked(){self.backup_delete_selected.clear();self.confirm_restore=false;}
                            ui.small(format!("{} selected",self.backup_delete_selected.len()));
                        });
                    }
                    ui.separator();
                    if self.restore_backups.is_empty() {
                        ui.label("No backups are available for this app.");
                    }
                    egui::ScrollArea::vertical()
                        .max_height(280.0)
                        .show(ui, |ui| {
                            for (i, backup) in self.restore_backups.iter().enumerate() {
                                let filename = backup.path.file_name().unwrap().to_string_lossy();
                                let version = filename
                                    .strip_prefix(&if backup.source {format!("{app}-source-")}else{format!("{app}-")})
                                    .unwrap_or(&filename)
                                    .split('-')
                                    .take(1)
                                    .collect::<Vec<_>>()
                                    .join(" ");
                                let date = std::fs::metadata(&backup.path)
                                    .and_then(|m| m.modified())
                                    .ok()
                                    .map(|t| {
                                        chrono::DateTime::<chrono::Local>::from(t)
                                            .format("%b %d, %Y · %I:%M %p")
                                            .to_string()
                                    })
                                    .unwrap_or_default();
                                let label = format!(
                                    "{} · {}\n{} · {}",
                                    if backup.source {
                                        "Source"
                                    } else {
                                        "Portable release"
                                    },
                                    version,
                                    date,
                                    if backup.path.is_dir() {
                                        "Folder"
                                    } else if backup.path.extension().is_some_and(|e| e == "7z") {
                                        "7-Zip archive"
                                    } else {
                                        "ZIP archive"
                                    }
                                );
                                if ui
                                    .add_sized(
                                        [ui.available_width(), 56.0],
                                        egui::Button::new(label)
                                            .selected(if self.backup_delete_mode {self.backup_delete_selected.contains(&i)}else{self.restore_selected == Some(i)}),
                                    )
                                    .clicked()
                                {
                                    if self.backup_delete_mode {
                                        if !self.backup_delete_selected.insert(i){self.backup_delete_selected.remove(&i);}
                                    }else{self.restore_selected = Some(i);}
                                    self.confirm_restore = false;
                                }
                            }
                        });
                    ui.separator();
                    if self.confirm_restore {
                        ui.colored_label(
                            Color32::from_rgb(230, 180, 100),
                            if self.backup_delete_mode {"Permanently delete the selected backups?"}else{"This replaces the current managed copy. Continue?"},
                        );
                    }
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                (if self.backup_delete_mode {!self.backup_delete_selected.is_empty()}else{self.restore_selected.is_some()})
                                    && !self.job.state.lock().unwrap().busy,
                                egui::Button::new(if self.backup_delete_mode && self.confirm_restore {"Confirm deletion"}else if self.backup_delete_mode {"Delete selected backups…"}else if self.confirm_restore {
                                    "Confirm restore"
                                } else {
                                    "Restore selected backup…"
                                })
                                .fill(BLUE),
                            )
                            .clicked()
                        {
                            if self.confirm_restore {
                                let selected:Vec<_>= if self.backup_delete_mode {self.backup_delete_selected.iter().map(|i|self.restore_backups[*i].clone()).collect()}else{vec![self.restore_backups[self.restore_selected.unwrap()].clone()]};
                                let deleting=self.backup_delete_mode;
                                let paths = self.paths.clone();
                                let app = app.clone();
                                self.release_checks.remove(&app);
                                self.job =
                                    Job::new(paths.at("logs/updates.log"), &self.build_preferences);
                                self.job.spawn(move |job| {
                                    if deleting {backups::delete_selected(&paths,&app,&selected)?;job.log(&format!("Deleted {} backup(s) for {}",selected.len(),model::title(&app)));Ok(())}else{backups::restore(&paths, &app, &selected[0], &job)}
                                });
                                self.restore_app = None;
                            } else {
                                self.confirm_restore = true;
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            self.restore_app = None;
                        }
                    });
                });
            if !show {
                self.restore_app = None;
            }
        }
        if self.confirm_self_update {
            egui::Window::new("Update Craft Apps Manager").collapsible(false).resizable(false).show(ctx, |ui| {
                ui.label(if self_update::installed_with_msi() {"Download and install the new manager version?"} else {"Download the new manager and restart this window?"});
                ui.small("Close other manager and builder windows first. Your library and settings will be kept.");
                ui.hyperlink_to("Release source: CryptoKey98/craft-apps-manager", self_update::REPOSITORY);
                ui.horizontal(|ui| {
                    if ui.button(if self_update::installed_with_msi() {"Download and install"} else {"Download and restart"}).clicked() {
                        if let Some(available)=self.manager_available.clone() {
                            let paths=self.paths.clone(); let ctx=ctx.clone(); let (tx,rx)=std::sync::mpsc::channel(); self.manager_plan=Some(rx);
                            self.job=Job::new(self.paths.at("logs/updates.log"), &self.build_preferences);
                            self.job.spawn(move |job| {
                                let result=self_update::prepare(&paths,&available,&job);
                                let message=result.as_ref().err().map(|e|format!("{e:#}"));
                                let _=tx.send(result.map_err(|e|format!("{e:#}")));ctx.request_repaint();
                                if let Some(message)=message {anyhow::bail!(message);} Ok(())
                            });
                            self.manager_message="Downloading and verifying manager…".into();
                        }
                        self.confirm_self_update=false;
                    }
                    if ui.button("Cancel").clicked(){self.confirm_self_update=false;}
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
        if let Some(message) = self.selection_notice.clone() {
            egui::Window::new("No apps selected")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .default_width(410.0)
                .show(ctx, |ui| {
                    ui.label(message);
                    ui.add_space(8.0);
                    if ui.button("OK").clicked() {
                        self.selection_notice = None;
                    }
                });
        }
        if let Some(error) = self.error.clone() {
            egui::Window::new("Craft Apps Manager")
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.colored_label(Color32::from_rgb(240, 130, 120), error);
                    if ui.button("OK").clicked() {
                        self.error = None;
                    }
                });
        }
    }
    fn log_panel(&mut self, ui: &mut egui::Ui, state: &State) {
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
                if ui
                    .add_enabled(
                        self.job.log_path.is_file(),
                        egui::Button::new("Open log").small(),
                    )
                    .on_hover_text(if self.job.log_path.is_file() {
                        self.job.log_path.display().to_string()
                    } else {
                        "No log file yet. Run an operation to create one.".into()
                    })
                    .clicked()
                {
                    let result = platform::open(&self.job.log_path);
                    self.result(result);
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
                                    "Ready. Craft app checks run only when requested or scheduled."
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
        let busy = self.job.state.lock().unwrap().busy;
        if self.operation_was_busy && !busy {
            self.config_refresh_at = std::time::Instant::now();
        }
        self.operation_was_busy = busy;
        if let Some(receiver) = &self.config_receiver {
            if let Ok(result) = receiver.try_recv() {
                match result {
                    Ok((config, backups, checks)) => {
                        if self.display_config.is_none() {
                            for app in &config.apps {
                                if let Some(check) = checks.get(&craft_apps_manager::hourly::key(
                                    &self.preferences,
                                    &app.name,
                                )) {
                                    if check.installed == app.version && !app.path.is_empty() {
                                        self.release_checks.entry(app.name.clone()).or_insert((
                                            check.installed.clone(),
                                            Ok(check.latest.clone()),
                                        ));
                                    }
                                }
                            }
                        }
                        self.display_config = Some(config);
                        self.display_backups = backups;
                    }
                    Err(error) => self.error = Some(error),
                }
                self.config_receiver = None;
            }
        }
        if !self.builder
            && !busy
            && self.config_receiver.is_none()
            && std::time::Instant::now() >= self.config_refresh_at
        {
            let (tx, rx) = std::sync::mpsc::channel();
            self.config_receiver = Some(rx);
            self.config_refresh_at = std::time::Instant::now() + Duration::from_secs(10);
            let paths = self.paths.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let result = (|| -> Result<DisplaySnapshot> {
                    let config = paths.config()?;
                    let backups = APPS
                        .into_iter()
                        .map(|name| {
                            backups::list(&paths, name).map(|items| (name.to_owned(), items))
                        })
                        .collect::<Result<_>>()?;
                    let checks = craft_apps_manager::hourly::read(&paths)?;
                    Ok((config, backups, checks))
                })();
                let _ = tx.send(result.map_err(|e| format!("{e:#}")));
                ctx.request_repaint();
            });
        }
        if self.apps_startup_pending && !busy {
            if let Some(config) = &self.display_config {
                let apps = updates::installed_check_targets(config)
                    .into_iter()
                    .map(|app| (app.name, app.version))
                    .collect();
                self.apps_startup_pending = false;
                self.check_apps(ctx, apps);
            }
        }
        if self.manager_startup_pending {
            self.manager_startup_pending = false;
            self.check_manager(ctx);
        }
        if let Some(receiver) = &self.manager_receiver {
            if let Ok(result) = receiver.try_recv() {
                match result {
                    Ok(Some(available)) => {
                        self.manager_message =
                            format!("Manager {} is available.", available.version);
                        self.manager_available = Some(available);
                        self.settings = true;
                    }
                    Ok(None) => {
                        self.manager_message = "You’re running the latest manager version.".into()
                    }
                    Err(error) => {
                        self.manager_message = format!("Could not check for updates: {error}")
                    }
                }
                self.manager_receiver = None;
            }
        }
        if let Some(receiver) = &self.manager_plan {
            if let Ok(result) = receiver.try_recv() {
                self.manager_plan = None;
                match result {
                    Ok(plan) => match self_update::launch(&plan) {
                        Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                        Err(error) => self.result(Err(error)),
                    },
                    Err(error) => {
                        self.manager_message = "Manager download failed.".into();
                        self.error = Some(error);
                    }
                }
            }
        }
        while let Ok((generation, app, version, result)) = self.check_receiver.try_recv() {
            if generation == self.check_generation {
                self.checking_apps.remove(&app);
                self.release_checks.insert(app, (version, result));
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
                        "Craft Apps Manager"
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("RUST  ·  {}", env!("CARGO_PKG_VERSION")))
                                .small()
                                .color(Color32::GRAY),
                        );
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
            .frame(egui::Frame::side_top_panel(&ctx.style()).fill(Color32::from_rgb(29, 29, 29)))
            .show(ctx, |sidebar_ui| {
                let viewport = sidebar_ui.max_rect();
                let mut content = sidebar_ui.new_child(egui::UiBuilder::new().max_rect(viewport));
                content.set_clip_rect(sidebar_ui.clip_rect().intersect(viewport));
                let ui = &mut content;
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
                    egui::ScrollArea::vertical()
                        .id_salt("creative-app-list")
                        .auto_shrink([false, false])
                        .max_height((ui.available_height() - 100.0).max(0.0))
                        .show(ui, |ui| {
                            let config = &self.display_config;
                            let app_order = self.preferences.app_order.clone();
                            let mut reorder = None;
                            for app_name in &app_order {
                                let name = app_name.as_str();
                                let version = config
                                    .as_ref()
                                    .and_then(|c| c.apps.iter().find(|a| a.name == name))
                                    .filter(|a| !a.version.is_empty())
                                    .map(|a| a.version.as_str())
                                    .unwrap_or(if config.is_some() {
                                        "Not installed"
                                    } else {
                                        "Checking…"
                                    });
                                let is_installed =
                                    version != "Not installed" && version != "Checking…";
                                let update_available = is_installed
                                    && self.release_checks.get(name).is_some_and(
                                        |(checked_version, result)| {
                                            checked_version == version
                                                && matches!(result, Ok(Some(_)))
                                        },
                                    );
                                let status_text = if is_installed {
                                    format!("Installed · {version}")
                                } else {
                                    version.to_owned()
                                };
                                let selected = self.app_selected && self.app == name;
                                let (rect, response) = ui.allocate_exact_size(
                                    egui::vec2(ui.available_width(), 56.0),
                                    egui::Sense::click_and_drag(),
                                );
                                response.dnd_set_drag_payload(app_name.clone());
                                if response.dragged() {
                                    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
                                    ui.painter().rect_stroke(
                                        rect.shrink(1.0),
                                        3.0,
                                        egui::Stroke::new(1.0_f32, BLUE),
                                        egui::StrokeKind::Inside,
                                    );
                                }
                                let before = ctx
                                    .input(|i| i.pointer.interact_pos())
                                    .is_none_or(|pos| pos.y < rect.center().y);
                                if let Some(dragged) = response.dnd_hover_payload::<String>() {
                                    if dragged.as_str() != name {
                                        let y = if before { rect.top() } else { rect.bottom() };
                                        ui.painter().line_segment(
                                            [
                                                egui::pos2(rect.left() + 4.0, y),
                                                egui::pos2(rect.right() - 4.0, y),
                                            ],
                                            egui::Stroke::new(2.0_f32, BLUE),
                                        );
                                    }
                                }
                                if let Some(dragged) = response.dnd_release_payload::<String>() {
                                    if dragged.as_str() != name {
                                        reorder = Some((
                                            dragged.as_ref().clone(),
                                            app_name.clone(),
                                            before,
                                        ));
                                    }
                                }
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
                                        egui::Rect::from_min_size(
                                            rect.min,
                                            egui::vec2(3.0, rect.height()),
                                        ),
                                        0.0,
                                        BLUE,
                                    );
                                }
                                let icon = egui::Rect::from_min_size(
                                    rect.min + egui::vec2(10.0, 12.0),
                                    egui::vec2(32.0, 32.0),
                                );
                                if let Some(texture) = self.icons.get(name) {
                                    ui.painter().image(
                                        texture.id(),
                                        icon,
                                        egui::Rect::from_min_max(
                                            egui::Pos2::ZERO,
                                            egui::pos2(1.0, 1.0),
                                        ),
                                        Color32::WHITE,
                                    );
                                }
                                ui.painter().text(
                                    rect.min + egui::vec2(52.0, 9.0),
                                    egui::Align2::LEFT_TOP,
                                    model::title(name),
                                    egui::FontId::proportional(14.0),
                                    Color32::from_gray(220),
                                );
                                ui.painter().text(
                                    rect.min + egui::vec2(52.0, 32.0),
                                    egui::Align2::LEFT_TOP,
                                    &status_text,
                                    egui::FontId::proportional(11.0),
                                    if is_installed {
                                        Color32::from_rgb(130, 195, 155)
                                    } else {
                                        Color32::from_gray(125)
                                    },
                                );
                                let mut install_clicked = false;
                                if (!is_installed && config.is_some()) || update_available {
                                    let text_width = ui
                                        .painter()
                                        .layout_no_wrap(
                                            status_text.clone(),
                                            egui::FontId::proportional(11.0),
                                            Color32::from_gray(125),
                                        )
                                        .size()
                                        .x;
                                    let button_rect = egui::Rect::from_min_size(
                                        rect.min + egui::vec2(52.0 + text_width + 8.0, 29.0),
                                        egui::vec2(20.0, 20.0),
                                    );
                                    let mut button_ui =
                                        ui.new_child(egui::UiBuilder::new().max_rect(button_rect));
                                    if state.busy {
                                        button_ui.disable();
                                    }
                                    let install_response = button_ui
                                        .interact(
                                            button_rect,
                                            ui.id().with(("install-app", name)),
                                            egui::Sense::click(),
                                        )
                                        .on_hover_text(format!(
                                            "{} {} (latest release)",
                                            if update_available {
                                                "Update available for"
                                            } else {
                                                "Install"
                                            },
                                            model::title(name),
                                        ));
                                    install_response.widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::WidgetType::Button,
                                            !state.busy,
                                            format!(
                                                "{} {} (latest release)",
                                                if update_available {
                                                    "Update"
                                                } else {
                                                    "Install"
                                                },
                                                model::title(name)
                                            ),
                                        )
                                    });
                                    let center = button_rect.center();
                                    let face = if state.busy {
                                        44
                                    } else if install_response.is_pointer_button_down_on() {
                                        52
                                    } else if install_response.hovered() {
                                        76
                                    } else {
                                        62
                                    };
                                    ui.painter().circle_filled(
                                        center + egui::vec2(0.0, 1.0),
                                        9.0,
                                        Color32::from_gray(20),
                                    );
                                    ui.painter().circle_filled(
                                        center,
                                        9.0,
                                        Color32::from_gray(face),
                                    );
                                    let paint_arrow =
                                        |origin: egui::Pos2, fill: Color32, outline: bool| {
                                            ui.painter().rect_filled(
                                                egui::Rect::from_min_max(
                                                    origin
                                                        + egui::vec2(
                                                            if update_available {
                                                                -0.9
                                                            } else {
                                                                -1.5
                                                            },
                                                            if update_available {
                                                                0.0
                                                            } else {
                                                                -4.5
                                                            },
                                                        ),
                                                    origin
                                                        + egui::vec2(
                                                            if update_available {
                                                                0.9
                                                            } else {
                                                                1.5
                                                            },
                                                            if update_available {
                                                                4.5
                                                            } else {
                                                                0.0
                                                            },
                                                        ),
                                                ),
                                                1.0,
                                                fill,
                                            );
                                            let direction =
                                                if update_available { -1.0 } else { 1.0 };
                                            let corners = [
                                                egui::vec2(
                                                    if update_available { -3.0 } else { -4.0 },
                                                    -0.5 * direction,
                                                ),
                                                egui::vec2(
                                                    if update_available { 3.0 } else { 4.0 },
                                                    -0.5 * direction,
                                                ),
                                                egui::vec2(0.0, 4.5 * direction),
                                            ];
                                            let mut rounded = Vec::with_capacity(15);
                                            for index in 0..3 {
                                                let corner = corners[index];
                                                let entry = corner
                                                    + (corners[(index + 2) % 3] - corner)
                                                        .normalized()
                                                        * 0.9;
                                                let exit = corner
                                                    + (corners[(index + 1) % 3] - corner)
                                                        .normalized()
                                                        * 0.9;
                                                for step in 0..=4 {
                                                    let t = step as f32 / 4.0;
                                                    rounded.push(
                                                        origin
                                                            + entry * (1.0 - t).powi(2)
                                                            + corner * (2.0 * t * (1.0 - t))
                                                            + exit * t.powi(2),
                                                    );
                                                }
                                            }
                                            if update_available {
                                                rounded.reverse();
                                            }
                                            ui.painter().add(egui::Shape::convex_polygon(
                                                rounded,
                                                fill,
                                                egui::Stroke::NONE,
                                            ));
                                            if outline {
                                                let corners = [
                                                    egui::vec2(-0.9, 4.5),
                                                    egui::vec2(0.9, 4.5),
                                                    egui::vec2(0.9, 0.5),
                                                    egui::vec2(3.0, 0.5),
                                                    egui::vec2(0.0, -4.5),
                                                    egui::vec2(-3.0, 0.5),
                                                    egui::vec2(-0.9, 0.5),
                                                ];
                                                let mut contour = Vec::new();
                                                for index in 0..corners.len() {
                                                    let corner = corners[index];
                                                    let entry = corner
                                                        + (corners[(index + corners.len() - 1)
                                                            % corners.len()]
                                                            - corner)
                                                            .normalized()
                                                            * 0.45;
                                                    let exit = corner
                                                        + (corners[(index + 1) % corners.len()]
                                                            - corner)
                                                            .normalized()
                                                            * 0.45;
                                                    for step in 0..=4 {
                                                        let t = step as f32 / 4.0;
                                                        contour.push(
                                                            origin
                                                                + entry * (1.0 - t).powi(2)
                                                                + corner * (2.0 * t * (1.0 - t))
                                                                + exit * t.powi(2),
                                                        );
                                                    }
                                                }
                                                contour.push(contour[0]);
                                                ui.painter().add(egui::Shape::line(
                                                    contour,
                                                    egui::Stroke::new(
                                                        0.55_f32,
                                                        Color32::from_rgb(195, 235, 255),
                                                    ),
                                                ));
                                            }
                                        };
                                    if update_available {
                                        let pulse =
                                            (ctx.input(|i| i.time) * 3.0).sin() as f32 * 0.5 + 0.5;
                                        let color = Color32::from_rgb(
                                            70,
                                            (130.0 + 60.0 * pulse) as u8,
                                            245,
                                        );
                                        ui.painter().circle_filled(
                                            center,
                                            9.0,
                                            Color32::from_rgb(30, 48, (65.0 + 20.0 * pulse) as u8),
                                        );
                                        ui.painter().circle_stroke(
                                            center,
                                            9.0,
                                            egui::Stroke::new(1.0_f32 + pulse * 0.5, color),
                                        );
                                        let hovered = install_response.hovered() && !state.busy;
                                        if hovered {
                                            for (radius, alpha) in
                                                [(12.0, 18), (11.0, 32), (10.0, 65)]
                                            {
                                                ui.painter().circle_stroke(
                                                    center,
                                                    radius,
                                                    egui::Stroke::new(
                                                        1.5_f32,
                                                        Color32::from_rgba_unmultiplied(
                                                            85, 175, 255, alpha,
                                                        ),
                                                    ),
                                                );
                                            }
                                            ui.painter().circle_stroke(
                                                center,
                                                9.0,
                                                egui::Stroke::new(
                                                    1.8_f32,
                                                    Color32::from_rgb(125, 205, 255),
                                                ),
                                            );
                                        }
                                        ctx.request_repaint_after(Duration::from_millis(33));
                                    }
                                    // Both directions share the same rounded, filled recessed arrow.
                                    if update_available {
                                        for offset in [
                                            egui::vec2(-0.35, 0.0),
                                            egui::vec2(0.35, 0.0),
                                            egui::vec2(0.0, -0.35),
                                            egui::vec2(0.0, 0.35),
                                        ] {
                                            paint_arrow(
                                                center + offset,
                                                Color32::from_rgba_unmultiplied(65, 165, 255, 22),
                                                false,
                                            );
                                        }
                                    }
                                    if !update_available {
                                        paint_arrow(
                                            center + egui::vec2(0.0, 0.75),
                                            Color32::from_gray(if state.busy { 54 } else { 80 }),
                                            false,
                                        );
                                    }
                                    paint_arrow(
                                        center,
                                        if update_available {
                                            if install_response.hovered() {
                                                Color32::from_rgb(40, 135, 220)
                                            } else {
                                                Color32::from_rgb(25, 100, 180)
                                            }
                                        } else {
                                            Color32::from_gray(32)
                                        },
                                        update_available,
                                    );
                                    install_clicked = install_response.clicked();
                                }
                                if install_clicked {
                                    self.confirm_install = Some(name.into());
                                } else if response.clicked() {
                                    self.app_selected = !selected;
                                    self.app = name.into();
                                    self.launch_settings_open = false;
                                    self.confirm_uninstall = false;
                                }
                                if self.checking_apps.contains(name) {
                                    let spinner_rect = egui::Rect::from_min_size(
                                        egui::pos2(rect.right() - 22.0, rect.top() + 30.0),
                                        egui::vec2(16.0, 16.0),
                                    );
                                    let mut spinner_ui =
                                        ui.new_child(egui::UiBuilder::new().max_rect(spinner_rect));
                                    spinner_ui
                                        .add(egui::Spinner::new().size(12.0))
                                        .on_hover_text("Checking for updates…");
                                }
                            }
                            if let Some((dragged, target, before)) = reorder {
                                let mut preferences = self.preferences.clone();
                                preferences.app_order.retain(|name| name != &dragged);
                                if let Some(index) = preferences
                                    .app_order
                                    .iter()
                                    .position(|name| name == &target)
                                {
                                    preferences
                                        .app_order
                                        .insert(index + usize::from(!before), dragged);
                                    match files::write_json(
                                        &self.paths.at("manager-settings.json"),
                                        &preferences,
                                    ) {
                                        Ok(()) => {
                                            self.settings_draft.app_order =
                                                preferences.app_order.clone();
                                            self.preferences = preferences;
                                        }
                                        Err(error) => {
                                            self.error =
                                                Some(format!("Could not save app order: {error:#}"))
                                        }
                                    }
                                }
                            }
                        });
                    let footer = egui::Rect::from_min_max(
                        egui::pos2(viewport.left(), viewport.bottom() - 96.0),
                        viewport.right_bottom(),
                    );
                    let separator = egui::Stroke::new(1.0_f32, Color32::from_gray(58));
                    for offset in [2.0, 38.0] {
                        ui.painter().line_segment(
                            [
                                egui::pos2(footer.left(), footer.top() + offset),
                                egui::pos2(footer.right(), footer.top() + offset),
                            ],
                            separator,
                        );
                    }
                    ui.painter().text(
                        egui::pos2(footer.left() + 4.0, footer.top() + 20.0),
                        egui::Align2::LEFT_CENTER,
                        format!(
                            "{} of 7 apps selected",
                            self.preferences.selected_apps.len()
                        ),
                        egui::FontId::proportional(13.0),
                        Color32::from_gray(190),
                    );
                    let button_rect = egui::Rect::from_center_size(
                        egui::pos2(footer.center().x, footer.top() + 68.0),
                        egui::vec2(190.0_f32.min(footer.width()), 32.0),
                    );
                    if ui
                        .add_enabled_ui(!state.busy, |ui| {
                            ui.put(button_rect, egui::Button::new("Settings"))
                        })
                        .inner
                        .clicked()
                    {
                        self.open_settings();
                    }
                }
                if self.builder {
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
                }
            });
        let expansion =
            ctx.animate_bool_with_time(egui::Id::new("app-details-slide"), self.app_selected, 0.18);
        if !self.builder && expansion > 0.001 {
            egui::SidePanel::left("app-details")
                .resizable(false)
                .exact_width(260.0 * expansion)
                .frame(egui::Frame::NONE.fill(Color32::from_rgb(40, 40, 40)))
                .show(ctx, |panel_ui| {
                    // Fixed-size controls are clipped to the animated viewport;
                    // padding cannot impose a minimum width at the closing edge.
                    let viewport = panel_ui.max_rect();
                    let mut content = panel_ui.new_child(egui::UiBuilder::new().max_rect(
                        egui::Rect::from_min_size(
                            viewport.min + egui::vec2(12.0, 0.0),
                            egui::vec2(236.0, viewport.height()),
                        ),
                    ));
                    content.set_clip_rect(panel_ui.clip_rect().intersect(viewport));
                    let ui = &mut content;
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
                    let installed = self
                        .display_config
                        .as_ref()
                        .and_then(|config| {
                            config.apps.iter().find(|a| {
                                a.name == self.app && !a.path.is_empty() && !a.version.is_empty()
                            })
                        })
                        .cloned();
                    if let Some(app) = &installed {
                        let update_available =
                            self.release_checks
                                .get(&self.app)
                                .is_some_and(|(version, result)| {
                                    version == &app.version && matches!(result, Ok(Some(_)))
                                });
                        ui.label(
                            RichText::new(if update_available {
                                "Update available"
                            } else {
                                "Installed"
                            })
                            .size(12.0)
                            .color(if update_available {
                                BLUE
                            } else {
                                Color32::from_rgb(130, 195, 155)
                            }),
                        );
                        ui.label(format!("Version {} · {}", app.version, app.architecture));
                        ui.small(if app.install_kind == "installer" {
                            if cfg!(target_os = "linux") {
                                "Installed Linux package"
                            } else {
                                "Installed with Windows installer"
                            }
                        } else {
                            "Installed portable release"
                        });
                    } else {
                        ui.label(
                            RichText::new("Not installed")
                                .size(12.0)
                                .color(Color32::from_gray(165)),
                        );
                    }
                    ui.add_space(12.0);
                    if ui
                        .add_enabled(
                            installed.is_some(),
                            egui::Button::new("Launch")
                                .fill(BLUE)
                                .min_size(egui::vec2(ui.available_width(), 36.0)),
                        )
                        .clicked()
                    {
                        self.result(apps::launch(&self.paths, &self.app));
                    }
                    ui.separator();
                    if ui
                        .add_enabled(
                            installed.is_some(),
                            egui::Button::new("Launch settings…")
                                .min_size(egui::vec2(ui.available_width(), 30.0)),
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
                            egui::Button::new("Open app folder")
                                .min_size(egui::vec2(ui.available_width(), 30.0)),
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
                    if ui
                        .add(
                            egui::Button::new("Build from source")
                                .min_size(egui::vec2(ui.available_width(), 30.0)),
                        )
                        .clicked()
                    {
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
                    let app_backups: Result<Vec<backups::Backup>> = Ok(self
                        .display_backups
                        .get(&self.app)
                        .cloned()
                        .unwrap_or_default());
                    if ui
                        .add_enabled(
                            !state.busy
                                && app_backups.as_ref().is_ok_and(|items| !items.is_empty()),
                            egui::Button::new("Backups…")
                                .min_size(egui::vec2(ui.available_width(), 30.0)),
                        )
                        .on_disabled_hover_text(if state.busy {
                            "Wait for the current operation to finish.".into()
                        } else if let Err(error) = &app_backups {
                            format!("Could not read backups: {error}")
                        } else {
                            "No backups are available for this app.".into()
                        })
                        .clicked()
                    {
                        match app_backups {
                            Ok(backups) => {
                                self.restore_backups = backups;
                                self.restore_selected = None;
                                self.confirm_restore = false;
                                self.backup_delete_mode = false;
                                self.backup_delete_selected.clear();
                                self.restore_app = Some(self.app.clone());
                            }
                            Err(error) => self.result(Err(error)),
                        }
                    }
                    let matching_format = installed.as_ref().is_some_and(|app| {
                        (app.install_kind == "installer")
                            == (self.preferences.release_format == "installer")
                    });
                    let checked = self.release_checks.get(&self.app).filter(|(v, _)| {
                        matching_format && installed.as_ref().is_some_and(|a| a.version == *v)
                    });
                    let available =
                        checked.is_some_and(|(_, result)| matches!(result, Ok(Some(_))));
                    let install_label = if self.checking_apps.contains(&self.app) {
                        "Checking…"
                    } else if matching_format && !available {
                        "Check for updates"
                    } else if matching_format {
                        "Update (latest release)"
                    } else {
                        "Install (latest release)"
                    };
                    let mut button = egui::Button::new(install_label)
                        .min_size(egui::vec2(ui.available_width(), 36.0));
                    if available {
                        let pulse = (ctx.input(|i| i.time) * 3.0).sin() as f32 * 0.5 + 0.5;
                        button = button.stroke(egui::Stroke::new(
                            1.5 + pulse,
                            Color32::from_rgb(70, (130.0 + 60.0 * pulse) as u8, 245),
                        ));
                        ctx.request_repaint_after(Duration::from_millis(33));
                    }
                    let clicked = ui
                        .add_enabled(
                            !state.busy && !self.checking_apps.contains(&self.app),
                            button,
                        )
                        .clicked();
                    if self.checking_apps.contains(&self.app) {
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
                            .min_size(egui::vec2(ui.available_width(), 30.0)),
                        )
                        .clicked()
                    {
                        self.confirm_uninstall = true;
                        self.delete_profile = false;
                    }
                    if cfg!(target_os = "windows")
                        && installed.as_ref().is_some_and(|app| {
                            app.install_kind == "installer" && app.product_code.is_empty()
                        })
                    {
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
                ui.horizontal(|ui|{if ui.add_enabled(!state.busy,egui::Checkbox::new(&mut self.auto,"Check for app updates hourly")).on_hover_text("Checks selected installed apps and notifies you when a new version is available. Supports installers and portable ZIPs; nothing downloads automatically.").changed(){let result=scheduler::set(&self.paths,false,self.auto);if result.is_err(){self.auto= !self.auto;}self.result(result)}
if ui.add_enabled(!state.busy,egui::Checkbox::new(&mut self.auto_source,"Check for source updates hourly")).changed(){let result=scheduler::set(&self.paths,true,self.auto_source);if result.is_err(){self.auto_source= !self.auto_source;}self.result(result)}});ui.small("Scheduled checks run hourly and after sign-in. Downloads require confirmation. Choose apps and sources in Settings.");ui.horizontal(|ui|{if ui.button("Open releases").clicked(){self.result(platform::open(&self.paths.at("releases")));}
if ui.button("Open sources").clicked(){self.result(platform::open(&self.paths.at("sources")));}
if state.busy&&ui.add_enabled(!self.job.cancel.load(Ordering::Relaxed) && !(cfg!(target_os = "linux") && state.stage == "Installing package"),egui::Button::new(if self.job.cancel.load(Ordering::Relaxed){"Cancel requested…"}else{"Cancel update"})).on_hover_text("Downloads can be cancelled. An authorized Linux package transaction must finish to keep the package database consistent.").clicked(){self.job.cancel.store(true,Ordering::Relaxed);}});ui.separator();}
            ui.horizontal(|ui|{if state.busy{ui.spinner();}ui.strong(if state.stage.is_empty(){if state.output.is_some(){"Previous build available"}else{"Ready"}}else{&state.stage});});
            if !state.detail.is_empty(){ui.label(&state.detail);}
if state.busy{progress_bar(ui,state.progress);}ui.add_space(10.0);self.log_panel(ui,&state);
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
