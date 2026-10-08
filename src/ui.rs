use anyhow::Result;
use craft_apps_manager::{
    apps, backups, builder,
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
        "wordcraft" => include_bytes!("../assets/app-icons/wordcraft.png"),
        "gridcraft" => include_bytes!("../assets/app-icons/gridcraft.png"),
        "deckcraft" => include_bytes!("../assets/app-icons/deckcraft.png"),
        "cadcraft" => include_bytes!("../assets/app-icons/cadcraft.png"),
        "soundcraft" => include_bytes!("../assets/app-icons/soundcraft.png"),
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
    Vec<model::Installed>,
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
    manager_plan: Option<std::sync::mpsc::Receiver<Result<PathBuf, (bool, String)>>>,
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
    alternates: Vec<model::Installed>,
    release_plan: Option<updates::ReleasePlan>,
    plan_receiver: Option<std::sync::mpsc::Receiver<Result<updates::ReleasePlan, (bool, String)>>>,
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
            alternates: Vec::new(),
            release_plan: None,
            plan_receiver: None,
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
    fn release_pending(&self) -> bool {
        self.plan_receiver.is_some() || self.release_plan.is_some()
    }
    fn start(&mut self, action: &str) {
        if self.release_pending() {
            return;
        }
        if action == "releases" && self.preferences.selected_apps.is_empty() {
            self.selection_notice = Some("No release apps are selected. Choose apps in Settings → Choose release apps, then try again.".into());
            return;
        }
        if action == "sources" && self.preferences.selected_sources.is_empty() {
            self.selection_notice = Some("No source apps are selected. Choose apps in Settings → Choose source apps, then try again.".into());
            return;
        }
        if action == "releases" {
            let paths = self.paths.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            self.plan_receiver = Some(rx);
            self.job = Job::new(paths.at("logs/updates.log"), &self.build_preferences);
            self.job.spawn(move |job| {
                let result = updates::plan_releases(&paths, &job);
                let _ = tx.send(result_for_display(&result));
                result.map(|_| ())
            });
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
        if self.release_pending() || self.job.state.lock().unwrap().busy {
            return;
        }
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
        let show = self.settings;
        if !show {
            return;
        }
        let modal = modal(
            ctx,
            if self.builder {
                "Builder settings"
            } else {
                "Manager settings"
            },
            |ui| {
                ui.heading(if self.builder {
                    "Build maintenance"
                } else {
                    "Release preferences"
                });
                ui.separator();
                egui::ScrollArea::vertical().max_height((ctx.screen_rect().height()-180.0).max(240.0)).show(ui,|ui|{
            if self.builder{
                ui.checkbox(&mut self.build_draft.delete_cache_after_success,"Delete compilation cache after a successful build");ui.checkbox(&mut self.build_draft.delete_workspace_after_success,"Delete extracted source and node_modules after success");
                ui.horizontal(|ui|{ui.label("Rotate each app log at (MB)");ui.add(egui::DragValue::new(&mut self.build_draft.log_size_mb).range(1..=100));});ui.horizontal(|ui|{ui.label("Older log files to keep");ui.add(egui::DragValue::new(&mut self.build_draft.log_archives).range(0..=5));});
                ui.small("Cancelled and failed builds keep their cache. Completed builds and tools are retained.");if ui.button("Clean temporary files now...").clicked(){self.confirm_clean=true;}
            }else{
                ui.horizontal(|ui|{ui.label("Release format");egui::ComboBox::from_id_salt("format").selected_text(if self.settings_draft.release_format=="portable"{if cfg!(target_os = "linux") {"AppImage"} else if cfg!(target_os = "macos") {"Portable app"} else {"Portable ZIP"}}else{"Installer"}).show_ui(ui,|ui|{ui.selectable_value(&mut self.settings_draft.release_format,"portable".into(),if cfg!(target_os = "linux") {"AppImage"} else if cfg!(target_os = "macos") {"Portable app"} else {"Portable ZIP"});ui.selectable_value(&mut self.settings_draft.release_format,"installer".into(),"Installer");});});
                if cfg!(target_os = "macos") { ui.label("Architecture: Universal (Intel + Apple silicon)"); } else {
                    ui.horizontal(|ui|{ui.label("Architecture");egui::ComboBox::from_id_salt("arch").selected_text(&self.settings_draft.architecture).show_ui(ui,|ui|{for (value,label) in [("x64","64-bit (x64)"),("x86","32-bit (x86)"),("arm64","ARM64")]{ui.selectable_value(&mut self.settings_draft.architecture,value.into(),label);}});});
                }
                ui.horizontal(|ui|{if ui.button("Choose release apps...").clicked(){self.source_selection=false;self.selection_draft=self.settings_draft.selected_apps.clone();self.selection=true;}ui.label(format!("{} of {} apps selected",self.settings_draft.selected_apps.len(),APPS.len()));});ui.horizontal(|ui|{if ui.button("Choose source apps...").clicked(){self.source_selection=true;self.selection_draft=self.settings_draft.selected_sources.clone();self.selection=true;}ui.label(format!("{} of {} sources selected",self.settings_draft.selected_sources.len(),SOURCES.len()));});
                ui.separator();ui.checkbox(&mut self.settings_draft.keep_app_backups,"Create app backups (portable releases only)");ui.checkbox(&mut self.settings_draft.keep_source_backups,"Create source backups");ui.checkbox(&mut self.settings_draft.compress_backups,"Compress portable app backups (7-Zip Ultra / LZMA2)");ui.checkbox(&mut self.settings_draft.compress_source_backups,"Recompress source backups (7-Zip Ultra / LZMA2)");ui.checkbox(&mut self.settings_draft.notify_updates,"Notify me when app or source updates are available");ui.checkbox(&mut self.settings_draft.check_installed_apps_on_startup,"Check installed apps for updates on startup").on_hover_text("Checks installed apps in the selected release format. Reports availability only; downloads and installation require confirmation.");ui.checkbox(&mut self.settings_draft.check_manager_on_startup,"Check for a new version of this program on startup").on_hover_text("Checks for a new Craft Apps Manager release. Downloads require your confirmation.");
                ui.horizontal(|ui|{ui.label("Previous versions to keep per app / source");ui.add(egui::DragValue::new(&mut self.settings_draft.backup_versions).range(1..=10));});
                ui.small(if cfg!(target_os = "linux") {"AppImage updates retain a rollback copy until successful. System packages require administrator authorization."} else if cfg!(target_os = "macos") {"A temporary rollback copy is kept until the update succeeds. Installer mode copies the signed app into Applications; portable mode keeps it in the library."} else {"A temporary rollback copy is kept until the update succeeds. Installer mode downloads and opens the Windows installer wizard."});if ui.button("Clear backups...").clicked(){self.confirm_clear=true;}
                if ui.add_enabled(self.manager_receiver.is_none() && self.manager_plan.is_none(), egui::Button::new("Check for updates...")).on_hover_text("Check for a newer Craft Apps Manager release").clicked() { self.check_manager(ctx); }
                let built=env!("CRAFT_BUILD_TIMESTAMP").parse::<i64>().ok().and_then(|t|chrono::DateTime::from_timestamp(t,0)).map(|t|t.format("%Y-%m-%d %H:%M UTC").to_string()).unwrap_or_default();ui.small(format!("Version {} · Build {} · {} · {} {}",env!("CARGO_PKG_VERSION"),built,env!("CRAFT_BUILD_PROFILE"),model::release_os(),model::MANAGER_ARCH));
                if self.manager_receiver.is_some() || self.manager_plan.is_some() {ui.spinner();ctx.request_repaint_after(Duration::from_millis(100));}
                if !self.manager_message.is_empty(){ui.small(&self.manager_message);}
                if self.manager_available.is_some() && ui.add_enabled(!self.job.state.lock().unwrap().busy && self.manager_plan.is_none(),egui::Button::new(if self_update::installed_with_msi() {"Download and install…"} else {"Download and restart…"})).clicked(){self.confirm_self_update=true;}
            }
            ui.separator();ui.collapsing("Folders",|ui|{ui.label("Data folder (releases, sources, builds, logs, backups)");ui.add(egui::TextEdit::singleline(&mut self.root_text).desired_width(520.0));ui.label("Build tools folder");ui.add(egui::TextEdit::singleline(&mut self.tools_text).desired_width(520.0));ui.small("Folder changes apply after reopening this window.");});
            });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !self.release_pending() && !self.job.state.lock().unwrap().busy,
                            egui::Button::new("Save"),
                        )
                        .clicked()
                    {
                        let root = PathBuf::from(self.root_text.trim());
                        let tools = PathBuf::from(self.tools_text.trim());
                        let locations = Locations {
                            root: Some(root.clone()),
                            tools: Some(tools.clone()),
                        };
                        let result = craft_apps_manager::settings::save(
                            &self.paths,
                            &self.home.join("data-root.json"),
                            &locations,
                            &root,
                            &tools,
                            if self.builder {
                                None
                            } else {
                                Some(&self.settings_draft)
                            },
                            if self.builder {
                                Some(&self.build_draft)
                            } else {
                                None
                            },
                        );
                        if result.is_ok() {
                            if self.builder {
                                self.build_preferences = self.build_draft.clone();
                            } else {
                                self.settings_draft.validate().expect("validated settings");
                                let changed = self.preferences.release_format
                                    != self.settings_draft.release_format
                                    || self.preferences.architecture
                                        != self.settings_draft.architecture;
                                self.preferences = self.settings_draft.clone();
                                if changed {
                                    self.check_generation += 1;
                                    self.checking_apps.clear();
                                    self.release_checks.clear();
                                }
                                self.display_config = None;
                                self.config_receiver = None;
                                self.config_refresh_at = std::time::Instant::now();
                            }
                            self.settings = false;
                        }
                        self.result(result);
                    }
                    if ui.button("Cancel").clicked() {
                        self.settings = false;
                    }
                });
            },
        );
        if !self.selection
            && !self.confirm_clear
            && !self.confirm_clean
            && !self.confirm_self_update
            && self.error.is_none()
            && self.selection_notice.is_none()
            && modal.should_close()
        {
            self.settings = false;
        }

        if !show {
            self.settings = false;
        }
    }
    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(plan) = self.release_plan.clone() {
            let response = modal(ctx, "Review release operations", |ui| {
                ui.set_width(570.0);
                ui.label(format!(
                    "{} · {}",
                    plan.preferences.release_format,
                    model::architecture_label(&plan.preferences.architecture)
                ));
                ui.small("Only the listed Install and Update entries will run.");
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    for entry in &plan.entries {
                        ui.separator(); ui.strong(format!("{}: {} {}", model::title(&entry.app), entry.action, entry.version));
                        if let Some(error) = &entry.error {ui.colored_label(Color32::LIGHT_RED, error);}
                        else {
                            ui.small(entry.destination.as_ref().map(|p| format!("Destination: {}", p.display())).unwrap_or_else(|| "Destination: Applications / system installation (installer controlled)".into()));
                            if let Some(asset) = &entry.asset { ui.hyperlink_to(RichText::new(&asset.name).small(), &asset.browser_download_url); }
                        }
                    }
                });
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            plan.executable_count() > 0,
                            egui::Button::new(format!(
                                "Confirm {} operations",
                                plan.executable_count()
                            )),
                        )
                        .clicked()
                    {
                        self.release_plan = None;
                        let paths = self.paths.clone();
                        self.job = Job::new(paths.at("logs/updates.log"), &self.build_preferences);
                        self.operation_was_busy = true;
                        self.job
                            .spawn(move |job| updates::execute_plan(&paths, &plan, &job));
                    }
                    if ui.button("Cancel").clicked() {
                        self.release_plan = None;
                    }
                });
            });
            if response.should_close() {
                self.release_plan = None;
            }
        }

        if let Some(app) = self.confirm_install.clone() {
            let installer = self.preferences.release_format == "installer";
            let modal = modal(ctx, "Confirm installation", |ui| {
                ui.label(if installer {
                    format!(
                        "Download and install the latest {} {} package?",
                        model::title(&app),
                        craft_apps_manager::installers::installer_label()
                    )
                } else {
                    format!(
                        "Are you sure you want to download and install {}?",
                        model::title(&app)
                    )
                });
                ui.small(format!(
                    "Source: https://github.com/storytold/{}/releases/latest",
                    model::repository(&app)
                ));
                ui.small(format!(
                    "Latest stable release · {} · {}",
                    model::architecture_label(&self.preferences.architecture),
                    self.preferences.release_format
                ));
                if installer {
                    ui.small(if cfg!(target_os = "linux") {"You will be asked to authorize package installation."} else if cfg!(target_os = "macos") {"The app will be copied into your Applications folder."} else {"The Windows installer will open. Follow its wizard; Windows may ask for administrator permission."});
                }
                ui.horizontal(|ui| {
                    if ui.button("Yes").clicked() {
                        self.app = app.clone();
                        self.confirm_install = None;
                        self.start("install-app");
                    }
                    if ui.button("No").clicked() {
                        self.confirm_install = None;
                    }
                });
            });
            if modal.should_close() {
                self.confirm_install = None;
            }
        }
        if self.launch_settings_open {
            let modal = modal(
                ctx,
                format!("{} launch settings", model::title(&self.app)),
                |ui| {
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
                },
            );
            if modal.should_close() {
                self.launch_settings_open = false;
            }
        }
        if self.confirm_uninstall {
            let modal = modal(
                ctx,
                format!("Uninstall {}?", model::title(&self.app)),
                |ui| {
                    ui.label("Are you sure you want to uninstall this app?");

                    if let Some(app) = self
                        .display_config
                        .as_ref()
                        .and_then(|c| c.apps.iter().find(|a| a.name == self.app))
                    {
                        ui.label(&app.path);
                    }
                    ui.small("Source ZIPs, builds, backups and launch settings will be kept.");
                    let targets = profiles::targets(&self.paths, &self.app);
                    ui.add_space(8.0);
                    ui.add_enabled(
                        targets.is_ok(),
                        egui::Checkbox::new(&mut self.delete_profile, "Delete app profile data"),
                    );
                    ui.small("Removes settings, caches, plug-ins and recovery/autosave copies from the folders below. Other installed copies may share this data.");
                    match &targets {
                        Ok(targets) => {
                            let existing: Vec<_> =
                                targets.iter().filter(|t| t.path.exists()).collect();
                            if existing.is_empty() {
                                ui.small("No existing profile folders found.");
                            }
                            egui::ScrollArea::vertical()
                                .max_height(100.0)
                                .show(ui, |ui| {
                                    for target in existing {
                                        ui.small(target.path.display().to_string());
                                    }
                                });
                            ui.small("Custom profile locations outside these folders are kept.");
                        }
                        Err(error) => {
                            ui.small(format!("Could not identify profile folders: {error}"));
                        }
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
                },
            );
            if modal.should_close() {
                self.confirm_uninstall = false;
            }
        }
        if self.selection {
            let choices: &[&str] = if self.source_selection {
                &SOURCES
            } else {
                &APPS
            };
            let modal = modal(
                ctx,
                if self.source_selection {
                    "Choose source apps"
                } else {
                    "Choose release apps"
                },
                |ui| {
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
                                    let mut checked =
                                        self.selection_draft.iter().any(|s| s == name);
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
                },
            );
            if modal.should_close() {
                self.selection = false;
            }
        }
        if let Some(app) = self.restore_app.clone() {
            let show = true;
            let modal = modal(ctx, format!("{} backups", model::title(&app)), |ui| {
                ui.heading("Available backups");

                ui.horizontal(|ui| {
                    let before = self.backup_delete_mode;
                    ui.selectable_value(&mut self.backup_delete_mode, false, "Restore");
                    ui.selectable_value(&mut self.backup_delete_mode, true, "Delete");
                    if before != self.backup_delete_mode {
                        self.confirm_restore = false;
                        self.restore_selected = None;
                        self.backup_delete_selected.clear();
                    }
                });
                ui.small(if self.backup_delete_mode {"Select the backups you want to delete. Installed apps and source files are kept."}else{"Choose one backup to restore. The selected backup will be kept."});
                if self.backup_delete_mode {
                    ui.horizontal(|ui| {
                        if ui.button("Select all").clicked() {
                            self.backup_delete_selected = (0..self.restore_backups.len()).collect();
                            self.confirm_restore = false;
                        }
                        if ui.button("Deselect all").clicked() {
                            self.backup_delete_selected.clear();
                            self.confirm_restore = false;
                        }
                        ui.small(format!("{} selected", self.backup_delete_selected.len()));
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
                                .strip_prefix(&if backup.source {
                                    format!("{app}-source-")
                                } else {
                                    format!("{app}-")
                                })
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
                                    egui::Button::new(label).selected(if self.backup_delete_mode {
                                        self.backup_delete_selected.contains(&i)
                                    } else {
                                        self.restore_selected == Some(i)
                                    }),
                                )
                                .clicked()
                            {
                                if self.backup_delete_mode {
                                    if !self.backup_delete_selected.insert(i) {
                                        self.backup_delete_selected.remove(&i);
                                    }
                                } else {
                                    self.restore_selected = Some(i);
                                }
                                self.confirm_restore = false;
                            }
                        }
                    });
                ui.separator();
                if self.confirm_restore {
                    ui.colored_label(
                        Color32::from_rgb(230, 180, 100),
                        if self.backup_delete_mode {
                            "Permanently delete the selected backups?"
                        } else {
                            "This replaces the current managed copy. Continue?"
                        },
                    );
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            (if self.backup_delete_mode {
                                !self.backup_delete_selected.is_empty()
                            } else {
                                self.restore_selected.is_some()
                            }) && !self.job.state.lock().unwrap().busy,
                            egui::Button::new(if self.backup_delete_mode && self.confirm_restore {
                                "Confirm deletion"
                            } else if self.backup_delete_mode {
                                "Delete selected backups…"
                            } else if self.confirm_restore {
                                "Confirm restore"
                            } else {
                                "Restore selected backup…"
                            })
                            .fill(BLUE),
                        )
                        .clicked()
                    {
                        if self.confirm_restore {
                            let selected: Vec<_> = if self.backup_delete_mode {
                                self.backup_delete_selected
                                    .iter()
                                    .map(|i| self.restore_backups[*i].clone())
                                    .collect()
                            } else {
                                vec![self.restore_backups[self.restore_selected.unwrap()].clone()]
                            };
                            let deleting = self.backup_delete_mode;
                            let paths = self.paths.clone();
                            let app = app.clone();
                            self.release_checks.remove(&app);
                            self.job =
                                Job::new(paths.at("logs/updates.log"), &self.build_preferences);
                            self.job.spawn(move |job| {
                                if deleting {
                                    backups::delete_selected(&paths, &app, &selected)?;
                                    job.log(&format!(
                                        "Deleted {} backup(s) for {}",
                                        selected.len(),
                                        model::title(&app)
                                    ));
                                    Ok(())
                                } else {
                                    backups::restore(&paths, &app, &selected[0], &job)
                                }
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
            if modal.should_close() {
                self.restore_app = None;
            }

            if !show {
                self.restore_app = None;
            }
        }
        if self.confirm_self_update {
            let modal = modal(ctx, "Update Craft Apps Manager", |ui| {
                ui.label(if self_update::installed_with_msi() {
                    "Download and install the new manager version?"
                } else {
                    "Download the new manager and restart this window?"
                });
                ui.small("Close other manager and builder windows first. Your library and settings will be kept.");
                ui.hyperlink_to(
                    format!("Release source: {}", self_update::REPOSITORY_NAME),
                    self_update::REPOSITORY,
                );
                ui.horizontal(|ui| {
                    if ui
                        .button(if self_update::installed_with_msi() {
                            "Download and install"
                        } else {
                            "Download and restart"
                        })
                        .clicked()
                    {
                        if let Some(available) = self.manager_available.clone() {
                            let paths = self.paths.clone();
                            let ctx = ctx.clone();
                            let (tx, rx) = std::sync::mpsc::channel();
                            self.manager_plan = Some(rx);
                            self.job = Job::new(
                                self.paths.at("logs/updates.log"),
                                &self.build_preferences,
                            );
                            self.job.spawn(move |job| {
                                let result = self_update::prepare(&paths, &available, &job);
                                let _ = tx.send(result_for_display(&result));
                                ctx.request_repaint();
                                result.map(|_| ())
                            });
                            self.manager_message = "Downloading and verifying manager…".into();
                        }
                        self.confirm_self_update = false;
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm_self_update = false;
                    }
                });
            });
            if modal.should_close() {
                self.confirm_self_update = false;
            }
        }
        if self.confirm_clear || self.confirm_clean {
            let modal = modal(ctx, "Confirm cleanup", |ui| {
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
            if modal.should_close() {
                self.confirm_clear = false;
                self.confirm_clean = false;
            }
        }
        if let Some(message) = self.selection_notice.clone() {
            let modal = modal(ctx, "No apps selected", |ui| {
                ui.label(message);
                ui.add_space(8.0);
                if ui.button("OK").clicked() {
                    self.selection_notice = None;
                }
            });
            if modal.should_close() {
                self.selection_notice = None;
            }
        }
        if let Some(error) = self.error.clone() {
            let modal = modal(ctx, "Craft Apps Manager", |ui| {
                ui.colored_label(Color32::from_rgb(240, 130, 120), error);
                if ui.button("OK").clicked() {
                    self.error = None;
                }
            });
            if modal.should_close() {
                self.error = None;
            }
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
        ctx.data_mut(|data| {
            data.insert_temp(egui::Id::new("active-dialogs"), Vec::<egui::Id>::new())
        });
        if let Some(receiver) = &self.plan_receiver {
            match receiver.try_recv() {
                Ok(result) => {
                    self.plan_receiver = None;
                    match result {
                        Ok(plan) => {
                            if !self.closing {
                                self.release_plan = Some(plan);
                            }
                        }
                        Err((cancelled, error)) => {
                            if !cancelled {
                                self.error = Some(error);
                            }
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.plan_receiver = None;
                    self.error =
                        Some("Release planner stopped unexpectedly. See the activity log.".into());
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        let busy = self.job.state.lock().unwrap().busy;
        if self.operation_was_busy && !busy {
            self.config_refresh_at = std::time::Instant::now();
        }
        self.operation_was_busy = busy;
        if let Some(receiver) = &self.config_receiver {
            if let Ok(result) = receiver.try_recv() {
                match result {
                    Ok((config, alternates, backups, checks)) => {
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
                        self.alternates = alternates;
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
                    let alternates = paths.alternate_installations(&config)?;
                    let backups = APPS
                        .into_iter()
                        .map(|name| {
                            backups::list(&paths, name).map(|items| (name.to_owned(), items))
                        })
                        .collect::<Result<_>>()?;
                    let checks = craft_apps_manager::hourly::read(&paths)?;
                    Ok((config, alternates, backups, checks))
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
                    Err((cancelled, error)) => {
                        if cancelled {
                            self.manager_message = "Manager download cancelled.".into();
                        } else {
                            self.manager_message = "Manager download failed.".into();
                            self.error = Some(error);
                        }
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
        let mut state = self.job.state.lock().unwrap().clone();
        state.busy |= self.release_pending();
        if ctx.input(|i| i.viewport().close_requested()) && state.busy {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.closing = true;
            self.release_plan = None;
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
                        "CRAFT APPS"
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
                                let alternate = self.alternates.iter().find(|a| a.name == name);
                                let other_label = alternate.map(|a| {
                                    if a.install_kind == "installer" {
                                        if cfg!(target_os = "macos") {
                                            "Applications"
                                        } else {
                                            "System install"
                                        }
                                    } else {
                                        "Portable"
                                    }
                                });
                                let status_text = if is_installed {
                                    format!(
                                        "Installed · {version} · {}{}",
                                        self.preferences.release_format,
                                        alternate
                                            .map(|a| format!(
                                                " · also {} {}",
                                                other_label.unwrap_or("Alternate"),
                                                a.version
                                            ))
                                            .unwrap_or_default()
                                    )
                                } else if let Some(alternate) = alternate {
                                    format!(
                                        "{} {} · no {} copy",
                                        other_label.unwrap_or("Alternate"),
                                        alternate.version,
                                        self.preferences.release_format
                                    )
                                } else {
                                    version.to_owned()
                                };
                                let selected = self.app_selected && self.app == name;
                                let (rect, response) = ui.allocate_exact_size(
                                    egui::vec2(ui.available_width(), 56.0),
                                    if state.busy {
                                        egui::Sense::hover()
                                    } else {
                                        egui::Sense::click_and_drag()
                                    },
                                );
                                let response = response.on_hover_text(&status_text);
                                response.widget_info(|| {
                                    egui::WidgetInfo::selected(
                                        egui::WidgetType::SelectableLabel,
                                        !state.busy,
                                        selected,
                                        format!("{} · {}", model::title(name), status_text),
                                    )
                                });
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
                                let mut status_job = egui::text::LayoutJob::simple_singleline(
                                    status_text.clone(),
                                    egui::FontId::proportional(11.0),
                                    Color32::from_gray(160),
                                );
                                status_job.wrap.max_width = (rect.width() - 88.0).max(1.0);
                                status_job.wrap.max_rows = 1;
                                let status_galley = ui.painter().layout_job(status_job);
                                ui.painter().galley(
                                    rect.min + egui::vec2(52.0, 32.0),
                                    status_galley,
                                    if is_installed {
                                        Color32::from_rgb(130, 195, 155)
                                    } else {
                                        Color32::from_gray(125)
                                    },
                                );
                                let mut install_clicked = false;
                                if (!is_installed && config.is_some()) || update_available {
                                    let button_rect = egui::Rect::from_min_size(
                                        egui::pos2(rect.right() - 28.0, rect.top() + 29.0),
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
                                    response.request_focus();
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
                                match craft_apps_manager::settings::reorder(
                                    &self.paths,
                                    &dragged,
                                    &target,
                                    before,
                                ) {
                                    Ok(preferences) => {
                                        if self.preferences.release_format
                                            != preferences.release_format
                                            || self.preferences.architecture
                                                != preferences.architecture
                                        {
                                            self.check_generation += 1;
                                            self.checking_apps.clear();
                                            self.release_checks.clear();
                                            self.display_config = None;
                                            self.config_receiver = None;
                                            self.config_refresh_at = std::time::Instant::now();
                                        }
                                        self.settings_draft = preferences.clone();
                                        self.preferences = preferences;
                                    }
                                    Err(error) => {
                                        self.error =
                                            Some(format!("Could not save app order: {error:#}"))
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
                            "{} of {} apps selected",
                            self.preferences.selected_apps.len(),
                            APPS.len()
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
                        ui.label(format!(
                            "Version {} · {}",
                            app.version,
                            model::architecture_label(&app.architecture)
                        ));
                        ui.small(if app.install_kind == "installer" {
                            if cfg!(target_os = "linux") {
                                "Installed Linux package"
                            } else if cfg!(target_os = "macos") {
                                "Installed in Applications"
                            } else {
                                "Installed with Windows installer"
                            }
                        } else {
                            "Installed portable release"
                        });
                    } else {
                        ui.label(
                            RichText::new(
                                if self.alternates.iter().any(|app| app.name == self.app) {
                                    format!("No {} copy", self.preferences.release_format)
                                } else {
                                    "Not installed".into()
                                },
                            )
                            .size(12.0)
                            .color(Color32::from_gray(165)),
                        );
                    }
                    if let Some(alternate) = self.alternates.iter().find(|app| app.name == self.app)
                    {
                        let location = if alternate.install_kind == "installer" {
                            if cfg!(target_os = "macos") {
                                "Applications"
                            } else {
                                "system installation"
                            }
                        } else {
                            "the portable library"
                        };
                        ui.label(format!(
                            "Installed in {location} ({}){}",
                            alternate.version,
                            if installed.is_none() {
                                format!("; no {} copy", self.preferences.release_format)
                            } else {
                                String::new()
                            }
                        ));
                        ui.small(&alternate.path);
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
            if !self.builder{ui.label(format!("Active release format: {} · {}", self.preferences.release_format, model::architecture_label(&self.preferences.architecture)));ui.horizontal_top(|ui|{
ui.vertical(|ui|{ui.set_width(225.0);if ui.add_enabled(!state.busy,egui::Button::new("Install / update selected").fill(BLUE).min_size(egui::vec2(225.0,36.0))).clicked(){self.start("releases")}});
ui.vertical(|ui|{ui.set_width(225.0);if ui.add_enabled(!state.busy,egui::Button::new("Update selected sources").min_size(egui::vec2(225.0,36.0))).clicked(){self.start("sources")}});
if ui.button("Build from source").clicked(){self.open_builder()}});ui.horizontal(|ui|{ui.spacing_mut().item_spacing.x=3.0;ui.small("Choose apps for release and source updates in");if ui.link(RichText::new("Settings").small().color(BLUE)).clicked(){self.open_settings();}});
                ui.horizontal(|ui|{if ui.add_enabled(!state.busy,egui::Checkbox::new(&mut self.auto,"Check for app updates hourly")).on_hover_text("Checks selected installed apps and notifies you when a new version is available. Supports installers and portable ZIPs; nothing downloads automatically.").changed(){let result=scheduler::set(&self.paths,false,self.auto);if result.is_err(){self.auto= !self.auto;}self.result(result)}
if ui.add_enabled(!state.busy,egui::Checkbox::new(&mut self.auto_source,"Check for source updates hourly")).changed(){let result=scheduler::set(&self.paths,true,self.auto_source);if result.is_err(){self.auto_source= !self.auto_source;}self.result(result)}});ui.small("Scheduled checks run hourly and after sign-in. Downloads require confirmation. Choose apps and sources in Settings.");ui.horizontal(|ui|{if ui.button("Open releases").clicked(){self.result(platform::open(&self.paths.at("releases")));}
if ui.button("Open sources").clicked(){self.result(platform::open(&self.paths.at("sources")));}
if state.busy&&ui.add_enabled(!self.job.cancel.load(Ordering::Relaxed) && !authorizing_package(&state.stage),egui::Button::new(if self.job.cancel.load(Ordering::Relaxed){"Cancel requested…"}else{"Cancel update"})).on_hover_text("Downloads can be cancelled. An authorized Linux package transaction must finish to keep the package database consistent.").clicked(){self.job.cancel.store(true,Ordering::Relaxed);}});ui.separator();}
            ui.horizontal(|ui|{if state.busy{ui.spinner();}ui.strong(if state.stage.is_empty(){if state.output.is_some(){"Previous build available"}else{"Ready"}}else{&state.stage});});
            if !state.detail.is_empty(){ui.label(&state.detail);}
if state.busy{progress_bar(ui,state.progress);}ui.add_space(10.0);self.log_panel(ui,&state);
        });
        self.settings_ui(ctx);
        self.dialogs(ctx);
        restore_dialog_focus(ctx);
        if state.busy {
            ctx.request_repaint_after(Duration::from_millis(100));
        } else {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }
}
/// Linux package installs wait on a PolicyKit prompt and cannot be cancelled.
fn authorizing_package(stage: &str) -> bool {
    cfg!(target_os = "linux") && stage == "Installing package"
}

#[derive(Clone, Default)]
struct DialogFocus(Vec<(egui::Id, Option<egui::Id>)>);
fn modal(
    ctx: &egui::Context,
    title: impl Into<String>,
    content: impl FnOnce(&mut egui::Ui),
) -> egui::ModalResponse<()> {
    let title = title.into();
    let id = egui::Id::new(&title);
    let focused = ctx.memory(|memory| memory.focused());
    ctx.data_mut(|data| {
        let stack = data.get_temp_mut_or_default::<DialogFocus>(egui::Id::new("dialog-focus"));
        if !stack.0.iter().any(|(open, _)| *open == id) {
            stack.0.push((id, focused));
        }
        data.get_temp_mut_or_default::<Vec<egui::Id>>(egui::Id::new("active-dialogs"))
            .push(id);
    });
    egui::Modal::new(id).show(ctx, |ui| {
        ui.set_max_width(570.0);
        ui.heading(&title);
        content(ui);
    })
}
fn restore_dialog_focus(ctx: &egui::Context) {
    let focus = ctx.data_mut(|data| {
        let active = data
            .get_temp::<Vec<egui::Id>>(egui::Id::new("active-dialogs"))
            .unwrap_or_default();
        let stack = data.get_temp_mut_or_default::<DialogFocus>(egui::Id::new("dialog-focus"));
        let mut focus = None;
        // When several nested dialogs close together, restore the outer opener.
        for (id, opener) in stack.0.iter().rev() {
            if !active.contains(id) {
                focus = *opener;
            }
        }
        stack.0.retain(|(id, _)| active.contains(id));
        focus
    });
    if let Some(id) = focus {
        ctx.memory_mut(|memory| memory.request_focus(id));
    }
}

fn result_for_display<T: Clone>(result: &Result<T>) -> Result<T, (bool, String)> {
    match result {
        Ok(value) => Ok(value.clone()),
        Err(error) => Err((
            craft_apps_manager::jobs::is_cancelled(error),
            format!("{error:#}"),
        )),
    }
}
#[cfg(test)]
mod outcome_tests {
    use super::*;
    #[test]
    fn display_copy_retains_cancellation_and_real_failure_distinction() {
        let cancelled: Result<PathBuf> =
            Err(anyhow::Error::new(craft_apps_manager::jobs::Cancelled)
                .context("Manager download interrupted"));
        let copy = result_for_display(&cancelled).unwrap_err();
        assert!(copy.0);
        assert!(copy.1.contains("Manager download interrupted"));
        if let Err(error) = &cancelled {
            assert!(craft_apps_manager::jobs::is_cancelled(error));
        }
        let failure: Result<PathBuf> = Err(anyhow::anyhow!("Digest mismatch"));
        assert!(!result_for_display(&failure).unwrap_err().0);
        let success = Ok(PathBuf::from("prepared-update"));
        assert_eq!(
            result_for_display(&success).unwrap(),
            PathBuf::from("prepared-update")
        );
    }
}
