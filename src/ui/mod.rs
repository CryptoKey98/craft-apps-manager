use anyhow::Result;
use craft_apps_manager::{
    apps, backups, builder, catalog, hourly,
    jobs::Job,
    model::{self, BuilderPreferences, Paths, Preferences},
    platform, scheduler, self_update, tools, updates,
};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    process::Command,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

mod app_page;
mod builder_view;
mod dialogs;
mod overview;
mod settings;
mod sidebar;
mod theme;

fn app_icon(name: &str) -> Option<&'static [u8]> {
    Some(match name {
        "designcraft" => include_bytes!("../../assets/app-icons/designcraft.png"),
        "effectcraft" => include_bytes!("../../assets/app-icons/effectcraft.png"),
        "filmcraft" => include_bytes!("../../assets/app-icons/filmcraft.png"),
        "lightcraft" => include_bytes!("../../assets/app-icons/lightcraft.png"),
        "photocraft" => include_bytes!("../../assets/app-icons/photocraft.png"),
        "vectorcraft" => include_bytes!("../../assets/app-icons/vectorcraft.png"),
        "wordcraft" => include_bytes!("../../assets/app-icons/wordcraft.png"),
        "gridcraft" => include_bytes!("../../assets/app-icons/gridcraft.png"),
        "deckcraft" => include_bytes!("../../assets/app-icons/deckcraft.png"),
        "cadcraft" => include_bytes!("../../assets/app-icons/cadcraft.png"),
        "soundcraft" => include_bytes!("../../assets/app-icons/soundcraft.png"),
        "printcraft" => include_bytes!("../../assets/app-icons/pdfcraft.png"),
        _ => return None,
    })
}
/// Icon for an app without a bundled one, downloaded once from its repository.
fn icon_bytes(paths: &Paths, name: &str) -> Option<Vec<u8>> {
    let cached = paths.at(format!("runtime/icons/{name}.png"));
    if let Ok(bytes) = std::fs::read(&cached) {
        return Some(bytes);
    }
    let entry = catalog::get(name)?;
    let url = format!(
        "https://raw.githubusercontent.com/{}/{}/HEAD/assets/app-icon/hicolor/64x64/apps/ai.storyteller.{}.png",
        catalog::ORG,
        entry.repository,
        entry.repository
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
fn texture(ctx: &egui::Context, name: &str, bytes: &[u8]) -> Option<egui::TextureHandle> {
    let image = image::load_from_memory(bytes).ok()?.into_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let pixels = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
    Some(ctx.load_texture(name, pixels, egui::TextureOptions::LINEAR))
}
/// A neutral rounded tile for apps whose repository has no icon.
fn placeholder(ctx: &egui::Context, name: &str) -> egui::TextureHandle {
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

type ReleaseCheck = Result<Option<String>, String>;
type CheckMessage = (u64, String, String, ReleaseCheck);

/// Everything the window shows that is read from disk, refreshed off the UI thread.
struct Snapshot {
    config: model::Config,
    alternates: Vec<model::Installed>,
    backups: BTreeMap<String, Vec<backups::Backup>>,
    checks: craft_apps_manager::hourly::Checks,
    sources: BTreeMap<String, model::Source>,
    builds: BTreeMap<String, (PathBuf, Option<model::BuildInfo>)>,
    launch: BTreeMap<String, apps::LaunchSettings>,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Locations {
    pub root: Option<PathBuf>,
    pub tools: Option<PathBuf>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Overview,
    App,
}

/// What the window knows about one app in the active release format.
#[derive(Default)]
struct AppStatus {
    installed: Option<model::Installed>,
    alternate: Option<model::Installed>,
    /// The newer release, when a check found one for the installed version.
    update: Option<String>,
    /// The last check for the installed version: Ok(None) means up to date.
    check: Option<ReleaseCheck>,
    checking: bool,
    loaded: bool,
}

pub struct App {
    icons: BTreeMap<String, egui::TextureHandle>,
    icon_receiver: std::sync::mpsc::Receiver<(String, Option<Vec<u8>>)>,
    icon_sender: std::sync::mpsc::Sender<(String, Option<Vec<u8>>)>,
    icons_requested: std::collections::BTreeSet<String>,
    catalog_receiver: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
    catalog_message: String,
    paths: Paths,
    home: PathBuf,
    builder: bool,
    app: String,
    latest: bool,
    job: Job,
    /// The app and action the current release job was started for, so its
    /// progress and failure show on that app's page.
    job_target: Option<(String, String)>,
    failure_dismissed: bool,
    /// Builds and tool setup started from an app page run beside release jobs,
    /// as they did in the separate builder window.
    build_job: Job,
    build_app: String,
    /// "build" or "setup", so the tools column labels and retries the right one.
    build_action: String,
    build_was_busy: bool,
    /// The action of the current release job, e.g. "clean".
    job_action: String,
    page: Page,
    search: String,
    preferences: Preferences,
    build_preferences: BuilderPreferences,
    settings: bool,
    settings_jump: Option<usize>,
    selection: bool,
    source_selection: bool,
    selection_draft: Vec<String>,
    selection_from_review: bool,
    settings_draft: Preferences,
    build_draft: BuilderPreferences,
    auto: bool,
    auto_draft: bool,
    error: Option<String>,
    selection_notice: Option<String>,
    root_text: String,
    tools_text: String,
    confirm_clear: bool,
    confirm_clean: bool,
    closing: bool,
    capture_frame: usize,
    launch_settings_open: bool,
    launch_build_options: bool,
    build_options_open: bool,
    delete_build_confirm: bool,
    launch_draft: apps::LaunchSettings,
    launch_arguments: String,
    confirm_uninstall: bool,
    delete_profile: bool,
    confirm_install: Option<String>,
    versions_app: Option<String>,
    versions_receiver: Option<std::sync::mpsc::Receiver<Result<Vec<model::Release>, String>>>,
    versions_result: Option<Result<Vec<model::Release>, String>>,
    release_checks: BTreeMap<String, (String, ReleaseCheck)>,
    check_receiver: std::sync::mpsc::Receiver<CheckMessage>,
    check_sender: std::sync::mpsc::Sender<CheckMessage>,
    checking_apps: std::collections::BTreeSet<String>,
    check_generation: u64,
    apps_startup_pending: bool,
    app_check_at: Instant,
    periodic_receiver: Option<std::sync::mpsc::Receiver<Result<hourly::Checks, String>>>,
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
    config_receiver: Option<std::sync::mpsc::Receiver<Result<Snapshot, String>>>,
    display_backups: BTreeMap<String, Vec<backups::Backup>>,
    display_sources: BTreeMap<String, model::Source>,
    display_builds: BTreeMap<String, (PathBuf, Option<model::BuildInfo>)>,
    display_launch: BTreeMap<String, apps::LaunchSettings>,
    config_refresh_at: std::time::Instant,
    operation_was_busy: bool,
    alternates: Vec<model::Installed>,
    release_plan: Option<updates::ReleasePlan>,
    plan_receiver: Option<std::sync::mpsc::Receiver<Result<updates::ReleasePlan, (bool, String)>>>,
}

/// IBM Plex, as in the approved designs, bundled so every platform renders the
/// same. egui has no font weights, so each weight is its own family.
fn fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let faces: [(&str, &'static [u8], egui::FontFamily); 4] = [
        (
            "IBM Plex Sans",
            include_bytes!("../../assets/fonts/IBMPlexSans-Regular.ttf"),
            egui::FontFamily::Proportional,
        ),
        (
            "IBM Plex Sans Medium",
            include_bytes!("../../assets/fonts/IBMPlexSans-Medium.ttf"),
            theme::medium_family(),
        ),
        (
            "IBM Plex Sans SemiBold",
            include_bytes!("../../assets/fonts/IBMPlexSans-SemiBold.ttf"),
            theme::bold_family(),
        ),
        (
            "IBM Plex Mono",
            include_bytes!("../../assets/fonts/IBMPlexMono-Regular.ttf"),
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

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        paths: Paths,
        home: PathBuf,
        builder: bool,
    ) -> Result<Self> {
        fonts(&cc.egui_ctx);
        let preferences = paths.preferences()?;
        theme::apply(&cc.egui_ctx, preferences.theme);
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
        let build_job = Job::new(paths.at(format!("logs/{app}.log")), &build_preferences);
        let auto = scheduler::enabled(false);
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
        let icons = model::apps()
            .into_iter()
            .chain(model::sources())
            .filter_map(|name| {
                let handle = texture(&cc.egui_ctx, &name, app_icon(&name)?)?;
                Some((name, handle))
            })
            .collect();
        let (icon_sender, icon_receiver) = std::sync::mpsc::channel();
        // The app list is refreshed in the background; newer apps appear when it finishes.
        let catalog_receiver = (preferences.check_catalog_on_startup
            && std::env::args().all(|a| !a.starts_with("--preview")))
        .then(|| {
            let (tx, rx) = std::sync::mpsc::channel();
            let root = paths.root.clone();
            let ctx = cc.egui_ctx.clone();
            std::thread::spawn(move || {
                let result = catalog::refresh_if_older(&root, catalog::REFRESH_INTERVAL)
                    .map(|_| ())
                    .map_err(|e| format!("{e:#}"));
                let _ = tx.send(result);
                ctx.request_repaint();
            });
            rx
        });
        let mut window = Self {
            icons,
            icon_receiver,
            icon_sender,
            icons_requested: Default::default(),
            catalog_receiver,
            catalog_message: String::new(),
            root_text: paths.root.display().to_string(),
            tools_text: paths.tools.display().to_string(),
            paths,
            home,
            builder,
            build_app: app.clone(),
            build_action: "build".into(),
            build_was_busy: false,
            job_action: String::new(),
            app,
            latest: true,
            job,
            job_target: None,
            failure_dismissed: false,
            build_job,
            page: if std::env::args().any(|a| a == "--preview-details") {
                Page::App
            } else {
                Page::Overview
            },
            search: String::new(),
            settings: std::env::args().any(|a| a == "--preview-settings" || a == "--preview-apps"),
            settings_jump: None,
            selection: std::env::args().any(|a| a == "--preview-apps"),
            source_selection: false,
            selection_draft: preferences.selected_apps.clone(),
            selection_from_review: false,
            settings_draft: preferences.clone(),
            build_draft: build_preferences.clone(),
            preferences,
            build_preferences,
            auto,
            auto_draft: auto,
            error: self_update::startup_message(),
            selection_notice: None,
            confirm_clear: false,
            confirm_clean: false,
            closing: false,
            capture_frame: 0,
            launch_settings_open: false,
            launch_build_options: false,
            build_options_open: false,
            delete_build_confirm: false,
            launch_draft: Default::default(),
            launch_arguments: String::new(),
            confirm_uninstall: false,
            delete_profile: false,
            confirm_install: None,
            versions_app: None,
            versions_receiver: None,
            versions_result: None,
            release_checks: Default::default(),
            check_receiver,
            check_sender,
            checking_apps: Default::default(),
            check_generation: 0,
            apps_startup_pending,
            app_check_at: Instant::now(),
            periodic_receiver: None,
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
            display_sources: Default::default(),
            display_builds: Default::default(),
            display_launch: Default::default(),
            config_refresh_at: std::time::Instant::now(),
            operation_was_busy: false,
            alternates: Vec::new(),
            release_plan: None,
            plan_receiver: None,
        };
        window.previews();
        Ok(window)
    }
    /// Opens a dialog or state directly, for screenshots of each part of the window:
    /// `--preview-dialog install|uninstall|launch|cleanup|manager|error|notice` and
    /// `--preview-build running|done|failed`. `--preview-progress` with
    /// `--preview-details` shows the progress on the app page.
    fn previews(&mut self) {
        let args: Vec<_> = std::env::args().collect();
        let value = |flag: &str| args.windows(2).find(|a| a[0] == flag).map(|a| a[1].clone());
        if args.iter().any(|a| a == "--preview-progress") {
            self.job_target = Some((self.app.clone(), "install-app".into()));
            let mut state = self.job.state.lock().unwrap();
            state.stage = "Downloading".into();
            state.detail = "32 MB of 81 MB".into();
            state.progress = Some(0.4);
        }
        match value("--preview-dialog").as_deref() {
            Some("build-options") => {
                self.launch_draft = builder::launch_options(&self.paths, &self.app).unwrap_or_default();
                self.launch_arguments = self.launch_draft.arguments.join("\n");
                self.build_options_open = true;
            }
            Some("install") => self.confirm_install = Some(self.app.clone()),
            Some("uninstall") => self.confirm_uninstall = true,
            Some("launch") => {
                self.launch_draft = apps::settings(&self.paths, &self.app).unwrap_or_default();
                self.launch_arguments = self.launch_draft.arguments.join("\n");
                self.launch_settings_open = true;
            }
            Some("cleanup") => self.confirm_clear = true,
            Some("manager") => self.confirm_self_update = true,
            Some("error") => {
                self.error = Some("Close VectorCraft before updating, then try again.".into())
            }
            Some("notice") => {
                self.selection_notice = Some("No apps are chosen for Update all. Choose apps in Settings › Updates, then try again.".into())
            }
            _ => {}
        }
        if let Some(kind) = value("--preview-build") {
            self.build_app = self.app.clone();
            let mut state = self.build_job.state.lock().unwrap();
            state.log = "Source commit: 8d0cede\nCompiling kurbo\nCompiling peniko\n".into();
            match kind.as_str() {
                "running" => {
                    state.busy = true;
                    state.stage = "Compiling".into();
                    state.detail = "212 steps".into();
                }
                "failed" => {
                    state.stage = "Failed".into();
                    state.outcome = "Command failed (exit status: 101). See the log.".into();
                }
                _ => state.stage = "Complete".into(),
            }
        }
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
        // Starting a job replaces self.job, which would orphan a running one.
        if self.release_pending() || self.job.state.lock().unwrap().busy {
            return;
        }
        // Cleaning and building share the builder lock.
        if action == "clean" && self.build_job.state.lock().unwrap().busy {
            return;
        }
        if action == "releases" && self.preferences.selected_apps.is_empty() {
            self.selection_notice = Some("No apps are chosen for Update all. Choose apps in Settings › Updates, then try again.".into());
            return;
        }
        if action == "sources" && self.preferences.selected_sources.is_empty() {
            self.selection_notice = Some("No sources are chosen for Update all sources. Choose sources in Settings › Updates, then try again.".into());
            return;
        }
        self.failure_dismissed = false;
        self.job_action = action.to_string();
        if action == "releases" {
            self.begin_release_review();
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
        self.job_target = matches!(
            action.as_str(),
            "install-app" | "uninstall-app" | "source-app"
        )
        .then(|| (app.clone(), action.clone()));
        self.operation_was_busy = true;
        self.job.spawn(move |job| match action.as_str() {
            "releases" => updates::releases(&paths, &job, false),
            "install-app" => updates::install_app(&paths, &app, &job),
            "uninstall-app" => apps::uninstall_with_profile(&paths, &app, delete_profile),
            "sources" => updates::sources(&paths, &paths.preferences()?.selected_sources, &job),
            "source-app" => updates::sources(&paths, &[app], &job),
            "build" => builder::build(&paths, &app, latest, &job),
            "setup" => tools::setup(&paths, &app, &job),
            "clear" => backups::clear(&paths),
            "clear-app" => backups::clear_app(&paths, &app),
            "clean" => builder::clean(&paths),
            _ => unreachable!(),
        });
    }
    /// Builds or sets up tools for the selected app, beside any release job.
    fn start_build(&mut self, action: &str) {
        if self.build_job.state.lock().unwrap().busy || self.cleaning() {
            return;
        }
        let paths = self.paths.clone();
        let app = self.app.clone();
        let latest = self.latest;
        let action = action.to_string();
        self.build_job = Job::new(paths.at(format!("logs/{app}.log")), &self.build_preferences);
        self.build_job.history(&self.build_job.log_path);
        self.build_job.state.lock().unwrap().output = builder::history(&paths, &app);
        self.build_app = app.clone();
        self.build_action = action.clone();
        self.build_was_busy = true;
        self.build_job.spawn(move |job| match action.as_str() {
            "build" => builder::build(&paths, &app, latest, &job),
            "setup" => tools::setup(&paths, &app, &job),
            _ => unreachable!(),
        });
    }
    fn open_versions(&mut self, ctx: &egui::Context) {
        if self.job.state.lock().unwrap().busy || self.release_pending() {
            return;
        }
        let app = self.app.clone();
        self.versions_app = Some(app.clone());
        self.versions_result = None;
        let paths = self.paths.clone();
        let ctx = ctx.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.versions_receiver = Some(rx);
        std::thread::spawn(move || {
            let result = updates::available_versions(&paths, &app).map_err(|e| format!("{e:#}"));
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }
    fn review_version(&mut self, app: String, tag: String) {
        if self.job.state.lock().unwrap().busy || self.release_pending() {
            return;
        }
        let paths = self.paths.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.plan_receiver = Some(rx);
        self.job = Job::new(paths.at("logs/updates.log"), &self.build_preferences);
        self.job_target = Some((app.clone(), "install-app".into()));
        self.failure_dismissed = false;
        self.job_action = "releases".into();
        self.versions_app = None;
        self.job.spawn(move |job| {
            let result = updates::plan_version(&paths, &job, &app, &tag);
            let _ = tx.send(result_for_display(&result));
            result.map(|_| ())
        });
    }
    fn begin_release_review(&mut self) {
        let paths = self.paths.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.plan_receiver = Some(rx);
        self.job = Job::new(paths.at("logs/updates.log"), &self.build_preferences);
        self.job_target = None;
        self.failure_dismissed = false;
        self.job_action = "releases".into();
        self.job.spawn(move |job| {
            let result = updates::plan_releases(&paths, &job);
            let _ = tx.send(result_for_display(&result));
            result.map(|_| ())
        });
    }
    /// True while a release job cleans build files, which blocks builds.
    fn cleaning(&self) -> bool {
        self.job_action == "clean" && self.job.state.lock().unwrap().busy
    }
    /// Settings opens while jobs run so folders and the builder window stay
    /// reachable; Save and the cleanup actions are disabled until they finish.
    fn open_settings(&mut self) {
        // The builder window may have saved its preferences since startup.
        if let Ok(preferences) = self.paths.builder_preferences() {
            self.build_preferences = preferences;
        }
        self.auto = scheduler::enabled(false);
        self.auto_draft = self.auto;
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
    fn select(&mut self, app: &str) {
        if self.app != app {
            self.launch_settings_open = false;
            self.confirm_uninstall = false;
        }
        self.app = app.into();
        self.page = Page::App;
    }
    fn status(&self, app: &str) -> AppStatus {
        let config = self.display_config.as_ref();
        let installed = config
            .and_then(|c| c.apps.iter().find(|a| a.name == app))
            .filter(|a| !a.version.is_empty() && !a.path.is_empty())
            .cloned();
        // Only a copy in the active release format can be updated in place.
        let matching_format = installed.as_ref().is_some_and(|i| {
            (i.install_kind == "installer") == (self.preferences.release_format == "installer")
        });
        let check = installed
            .as_ref()
            .filter(|_| matching_format)
            .and_then(|installed| {
                self.release_checks
                    .get(app)
                    .filter(|(version, _)| *version == installed.version)
                    .map(|(_, result)| result.clone())
            });
        AppStatus {
            update: check.as_ref().and_then(|c| c.clone().ok().flatten()),
            check,
            alternate: self.alternates.iter().find(|a| a.name == app).cloned(),
            installed,
            checking: self.checking_apps.contains(app),
            loaded: config.is_some(),
        }
    }
    /// Re-reads settings after the app list changed, adopting newly found apps.
    fn catalog_changed(&mut self) {
        match self.paths.preferences() {
            Ok(preferences) => {
                if !self.settings {
                    self.settings_draft = preferences.clone();
                }
                self.preferences = preferences;
            }
            Err(error) => self.error = Some(format!("{error:#}")),
        }
        self.display_config = None;
        self.config_receiver = None;
        self.config_refresh_at = std::time::Instant::now();
    }
    /// Looks for Craft apps on GitHub now.
    pub(crate) fn refresh_catalog(&mut self, ctx: &egui::Context) {
        let (tx, rx) = std::sync::mpsc::channel();
        let root = self.paths.root.clone();
        let ctx = ctx.clone();
        self.catalog_message = "Looking for Craft apps on GitHub…".into();
        std::thread::spawn(move || {
            let result = catalog::refresh(&root)
                .map(|_| ())
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.catalog_receiver = Some(rx);
    }
    /// Loads icons for apps without a bundled one, off the interface thread.
    fn request_icons(&mut self, ctx: &egui::Context) {
        let missing: Vec<_> = model::apps()
            .into_iter()
            .chain(model::sources())
            .filter(|name| {
                !self.icons.contains_key(name) && self.icons_requested.insert(name.clone())
            })
            .collect();
        if missing.is_empty() {
            return;
        }
        let paths = self.paths.clone();
        let tx = self.icon_sender.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            for name in missing {
                let bytes = icon_bytes(&paths, &name);
                if tx.send((name, bytes)).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
    }
    /// Polls background work and keeps the snapshot of disk state fresh.
    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(receiver) = &self.catalog_receiver {
            if let Ok(result) = receiver.try_recv() {
                self.catalog_receiver = None;
                match result {
                    Ok(()) => {
                        self.catalog_message.clear();
                        self.catalog_changed();
                    }
                    Err(error) => {
                        self.catalog_message = format!("Could not refresh the app list: {error}")
                    }
                }
            }
        }
        while let Ok((name, bytes)) = self.icon_receiver.try_recv() {
            let handle = bytes
                .and_then(|bytes| texture(ctx, &name, &bytes))
                .unwrap_or_else(|| placeholder(ctx, &name));
            self.icons.insert(name, handle);
        }
        self.request_icons(ctx);
        if let Some(receiver) = &self.versions_receiver {
            match receiver.try_recv() {
                Ok(result) => {
                    self.versions_receiver = None;
                    self.versions_result = Some(result);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.versions_receiver = None;
                    self.versions_result =
                        Some(Err("Version list could not be loaded. Try again.".into()));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
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
        // Builds do not change installs or backups, so only release jobs pause the
        // snapshot; either kind finishing refreshes it.
        let busy = self.job.state.lock().unwrap().busy;
        let building = self.build_job.state.lock().unwrap().busy;
        if (self.operation_was_busy && !busy) || (self.build_was_busy && !building) {
            self.config_refresh_at = std::time::Instant::now();
        }
        self.operation_was_busy = busy;
        self.build_was_busy = building;
        if let Some(receiver) = &self.config_receiver {
            if let Ok(result) = receiver.try_recv() {
                match result {
                    Ok(snapshot) => {
                        if self.display_config.is_none() {
                            for app in &snapshot.config.apps {
                                if let Some(check) = snapshot.checks.get(
                                    &craft_apps_manager::hourly::key(&self.preferences, &app.name),
                                ) {
                                    if check.installed == app.version && !app.path.is_empty() {
                                        self.release_checks.entry(app.name.clone()).or_insert((
                                            check.installed.clone(),
                                            Ok(check.latest.clone()),
                                        ));
                                    }
                                }
                            }
                        }
                        self.alternates = snapshot.alternates;
                        self.display_config = Some(snapshot.config);
                        self.display_backups = snapshot.backups;
                        self.display_sources = snapshot.sources;
                        self.display_builds = snapshot.builds;
                        self.display_launch = snapshot.launch;
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
                let result = (|| -> Result<Snapshot> {
                    let config = paths.config()?;
                    let alternates = paths.alternate_installations(&config)?;
                    let backups = model::apps()
                        .into_iter()
                        .map(|name| backups::list(&paths, &name).map(|items| (name, items)))
                        .collect::<Result<_>>()?;
                    let checks = craft_apps_manager::hourly::read(&paths)?;
                    // Display-only extras: an unreadable file just leaves its row empty.
                    let sources = craft_apps_manager::files::read_or_default(
                        &paths.at("sources/source-index.json"),
                    )
                    .unwrap_or_default();
                    let builds = model::apps()
                        .into_iter()
                        .filter_map(|name| {
                            let folder = builder::history(&paths, &name)?;
                            let info = craft_apps_manager::files::read_json(
                                &folder.join("build-info.json"),
                            )
                            .ok();
                            Some((name, (folder, info)))
                        })
                        .collect();
                    let launch = model::apps()
                        .into_iter()
                        .filter_map(|name| {
                            apps::settings(&paths, &name)
                                .ok()
                                .map(|settings| (name, settings))
                        })
                        .collect();
                    Ok(Snapshot {
                        config,
                        alternates,
                        backups,
                        checks,
                        sources,
                        builds,
                        launch,
                    })
                })();
                let _ = tx.send(result.map_err(|e| format!("{e:#}")));
                ctx.request_repaint();
            });
        }
        if self.apps_startup_pending && !self.job.state.lock().unwrap().busy {
            if let Some(config) = &self.display_config {
                let apps = updates::installed_check_targets(config)
                    .into_iter()
                    .map(|app| (app.name, app.version))
                    .collect();
                self.apps_startup_pending = false;
                self.check_apps(ctx, apps);
            }
        }
        if let Some(receiver) = &self.periodic_receiver {
            if let Ok(result) = receiver.try_recv() {
                self.periodic_receiver = None;
                self.app_check_at = Instant::now();
                match result {
                    Ok(checks) => {
                        for (key, check) in checks {
                            let prefix = format!(
                                "{}:{}:",
                                self.preferences.release_format, self.preferences.architecture
                            );
                            if let Some(app) = key.strip_prefix(&prefix) {
                                self.release_checks
                                    .insert(app.to_owned(), (check.installed, Ok(check.latest)));
                            }
                        }
                    }
                    Err(error) => self
                        .job
                        .log(&format!("Background app check failed: {error}")),
                }
            }
        }
        if !self.builder
            && self.preferences.check_installed_apps_periodically
            && self.app_check_at.elapsed()
                >= Duration::from_secs(self.preferences.app_check_interval_minutes * 60)
            && self.periodic_receiver.is_none()
            && self.checking_apps.is_empty()
            && !self.job.state.lock().unwrap().busy
            && !self.build_job.state.lock().unwrap().busy
            && !self.release_pending()
            && self.manager_plan.is_none()
            && self.display_config.is_some()
        {
            let (tx, rx) = std::sync::mpsc::channel();
            self.periodic_receiver = Some(rx);
            let paths = self.paths.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let job = Job::new(
                    paths.at("runtime/background-app-check.log"),
                    &Default::default(),
                );
                let result = hourly::run_periodic(&paths, &job).and_then(|_| hourly::read(&paths));
                let _ = tx.send(result.map_err(|e| format!("{e:#}")));
                ctx.request_repaint();
            });
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
                            format!("Version {} is available.", available.version);
                        self.manager_available = Some(available);
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
    }
    fn screenshot(&mut self, ctx: &egui::Context) {
        let Ok(path) = std::env::var("CRAFT_SCREENSHOT_TO") else {
            return;
        };
        self.capture_frame += 1;
        if self.capture_frame >= 10 && (self.builder || self.display_config.is_some()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        ctx.input(|i| {
            for event in &i.events {
                if let egui::Event::Screenshot { image, .. } = event {
                    let rgba: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
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
    /// Cancel button rules shared by every place that can stop the release job.
    fn cancel_button(&mut self, ui: &mut egui::Ui, stage: &str) {
        let requested = self.job.cancel.load(Ordering::Relaxed);
        if theme::btn(if requested {
            "Cancel requested…"
        } else {
            "Cancel"
        })
        .enabled(!requested && !authorizing_package(stage))
        .show(ui)
        .on_hover_text("Downloads can be cancelled. An authorized Linux package transaction must finish to keep the package database consistent.")
        .clicked()
        {
            self.job.cancel.store(true, Ordering::Relaxed);
        }
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        theme::apply(ctx, self.preferences.theme);
        ctx.data_mut(|data| {
            data.insert_temp(egui::Id::new("active-dialogs"), Vec::<egui::Id>::new())
        });
        self.poll(ctx);
        self.screenshot(ctx);
        let mut state = self.job.state.lock().unwrap().clone();
        state.busy |= self.release_pending();
        let building = self.build_job.state.lock().unwrap().busy;
        if ctx.input(|i| i.viewport().close_requested()) && (state.busy || building) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.closing = true;
            self.release_plan = None;
            self.job.cancel.store(true, Ordering::Relaxed);
            self.build_job.cancel.store(true, Ordering::Relaxed);
        }
        if self.closing && !state.busy && !building {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.builder {
            self.builder_view(ctx, &state);
        } else {
            self.top_bar(ctx, &state);
            self.status_bar(ctx, &state);
            self.sidebar(ctx, &state);
            match self.page {
                Page::Overview => self.overview(ctx, &state),
                Page::App => self.app_page(ctx, &state),
            }
        }
        self.settings_ui(ctx);
        self.dialogs(ctx);
        restore_dialog_focus(ctx);
        if state.busy || building {
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
    width: f32,
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
    let width = width.min(ctx.screen_rect().width() - 48.0);
    // No inner margin: each dialog draws its own header, body and footer bands,
    // with their own padding and hairlines, edge to edge inside the 1pt border.
    egui::Modal::new(id)
        .backdrop_color(egui::Color32::from_black_alpha(150))
        .frame(
            egui::Frame::new()
                .fill(theme::palette().panel)
                .stroke(egui::Stroke::new(1.0_f32, theme::palette().border_strong))
                .corner_radius(egui::CornerRadius::same(12))
                .inner_margin(egui::Margin::ZERO)
                .shadow(egui::Shadow {
                    offset: [0, 12],
                    blur: 32,
                    spread: 0,
                    color: egui::Color32::from_black_alpha(110),
                }),
        )
        .show(ctx, |ui| {
            ui.set_width(width - 2.0);
            ui.spacing_mut().item_spacing.y = 0.0;
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
