use anyhow::{Context, Result};
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
mod operations_view;
mod overview;
mod settings;
mod sidebar;
mod theme;
mod tray;
mod whats_new;

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
struct InstallationSelection {
    app: String,
    config: model::Config,
    preferences: model::Preferences,
}

/// Everything the window shows that is read from disk, refreshed off the UI thread.
struct Snapshot {
    config: model::Config,
    alternates: Vec<model::Installed>,
    backups: BTreeMap<String, Vec<backups::Backup>>,
    checks: craft_apps_manager::hourly::Checks,
    sources: BTreeMap<String, model::Source>,
    builds: BTreeMap<String, (PathBuf, Option<model::BuildInfo>)>,
    launch: BTreeMap<String, apps::LaunchSettings>,
    build_cache: std::collections::BTreeSet<String>,
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
    alternate_architecture: Option<model::Installed>,
    /// The newer release, when a check found one for the installed version.
    update: Option<String>,
    /// The last check for the installed version: Ok(None) means up to date.
    check: Option<ReleaseCheck>,
    checking: bool,
    loaded: bool,
}

enum CustomAppAddition {
    Added(Box<catalog::Entry>),
    Choose(Box<catalog::DownloadChoice>),
    Setup(Box<CustomAppSetup>),
}

struct CustomAppSetup {
    entry: catalog::Entry,
    release: model::Release,
    preferences: model::Preferences,
    architecture_confirmed: bool,
    selected: String,
    choosing: bool,
}

type BuildCacheSnapshot = (PathBuf, String, bool);

type SharedWindowSnapshot = (
    PathBuf,
    Option<(model::Theme, bool)>,
    std::collections::HashMap<String, String>,
);

pub struct App {
    icons: BTreeMap<String, egui::TextureHandle>,
    icon_receiver: std::sync::mpsc::Receiver<(String, Option<Vec<u8>>)>,
    icon_sender: std::sync::mpsc::Sender<(String, Option<Vec<u8>>)>,
    icons_requested: std::collections::BTreeSet<String>,
    catalog_receiver: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
    catalog_message: String,
    new_apps: Option<craft_apps_manager::discovery::NewApps>,
    new_app_fades: BTreeMap<String, Instant>,
    custom_link: String,
    custom_message: String,
    custom_failed: bool,
    custom_receiver: Option<std::sync::mpsc::Receiver<Result<CustomAppAddition, String>>>,
    custom_download_choice: Option<Box<catalog::DownloadChoice>>,
    custom_download_index: usize,
    custom_download_confirmed: bool,
    custom_setup: Option<Box<CustomAppSetup>>,
    custom_resume_install: Option<(String, Option<String>)>,
    custom_resume_versions: Option<String>,
    custom_apps_draft: Vec<catalog::Entry>,
    remove_custom_confirm: Option<String>,
    download_availability: BTreeMap<(String, String, String), Option<bool>>,
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
    whats_new_open: bool,
    whats_new_animation_started: Option<f64>,
    preferences: Preferences,
    build_preferences: BuilderPreferences,
    settings: bool,
    settings_jump: Option<usize>,
    selection: bool,
    visibility_open: bool,
    hide_notice_open: bool,
    hide_notice_seen: bool,
    visibility_draft: Vec<String>,
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
    tray: Option<tray::Tray>,
    tray_wake: tray::Wake,
    tray_attempted: bool,
    tray_error: Option<String>,
    tray_hidden: bool,
    tray_notice_shown: bool,
    tray_probe_at: Instant,
    tray_probe: Option<std::sync::mpsc::Receiver<bool>>,
    capture_frame: usize,
    launch_settings_open: bool,
    launch_build_options: bool,
    build_options_open: bool,
    build_choice: Option<PathBuf>,
    build_choices: Vec<(PathBuf, model::BuildInfo)>,
    build_app_cache: Option<bool>,
    build_cache_receiver: Option<std::sync::mpsc::Receiver<BuildCacheSnapshot>>,
    confirm_build: Option<String>,
    build_parent: String,
    build_shortcut: bool,
    build_source_fresh: bool,
    build_folder_receiver: Option<std::sync::mpsc::Receiver<Result<Option<PathBuf>, String>>>,
    delete_build_confirm: bool,
    clean_build_confirm: bool,
    remove_source_confirm: Option<String>,
    launch_draft: apps::LaunchSettings,
    launch_executables_key: Option<(String, String)>,
    launch_executables: Option<Result<Vec<String>, String>>,
    launch_executables_receiver: Option<std::sync::mpsc::Receiver<Result<Vec<String>, String>>>,
    launch_arguments: String,
    confirm_move: Option<model::Installed>,
    move_parent: String,
    move_running: Option<bool>,
    move_probe: Option<std::sync::mpsc::Receiver<std::result::Result<bool, String>>>,
    move_probe_at: std::time::Instant,
    move_folder: Option<std::sync::mpsc::Receiver<std::result::Result<Option<PathBuf>, String>>>,
    confirm_uninstall: bool,
    delete_profile: bool,
    confirm_install: Option<String>,
    install_selected_tag: Option<String>,
    additional_installation: bool,
    install_draft_app: Option<String>,
    install_location_context: Option<(String, String, String, bool)>,
    install_destination_preview: dialogs::DestinationPreview,
    install_parent: String,
    install_shortcut: bool,
    install_result: Option<Result<updates::ReleasePlan, String>>,
    install_receiver: Option<std::sync::mpsc::Receiver<Result<updates::ReleasePlan, String>>>,
    folder_receiver: Option<std::sync::mpsc::Receiver<Result<Option<PathBuf>, String>>>,
    install_error: Option<String>,
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
    token_status: craft_apps_manager::network::TokenStatus,
    token_check_started: bool,
    token_receiver: Option<std::sync::mpsc::Receiver<craft_apps_manager::network::TokenStatus>>,
    token_draft: String,
    token_reveal: bool,
    token_message: String,
    token_action_receiver:
        Option<std::sync::mpsc::Receiver<Result<craft_apps_manager::network::TokenStatus, String>>>,
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
    installation_selection:
        Option<std::sync::mpsc::Receiver<Result<InstallationSelection, String>>>,
    display_backups: BTreeMap<String, Vec<backups::Backup>>,
    display_sources: BTreeMap<String, model::Source>,
    display_builds: BTreeMap<String, (PathBuf, Option<model::BuildInfo>)>,
    display_build_cache: std::collections::BTreeSet<String>,
    display_launch: BTreeMap<String, apps::LaunchSettings>,
    config_refresh_at: std::time::Instant,
    operation_was_busy: bool,
    operation_completed_at: Option<std::time::Instant>,
    alternates: Vec<model::Installed>,
    release_plan: Option<updates::ReleasePlan>,
    plan_receiver: Option<std::sync::mpsc::Receiver<Result<updates::ReleasePlan, (bool, String)>>>,
    operation_records: Vec<craft_apps_manager::operations::Operation>,
    operation_refresh: Instant,
    shared_window_receiver: Option<std::sync::mpsc::Receiver<SharedWindowSnapshot>>,
    shared_build_logs: std::collections::HashMap<String, String>,
    activity_open: bool,
    activity_history: bool,
    activity_anchor: Option<egui::Pos2>,
    activity_notifications: operations_view::ActivityNotifications,
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
        let preview_new_app = args
            .windows(2)
            .find(|args| args[0] == "--preview-new-app")
            .map(|args| args[1].as_str());
        if let Some(app) = preview_new_app {
            model::valid_app(app)?;
        }
        let new_apps = if builder {
            None
        } else {
            Some(craft_apps_manager::discovery::NewApps::load(
                &paths.root,
                &catalog::all(),
                preview_new_app,
            )?)
        };
        let startup_operations = craft_apps_manager::operations::Manager::new(&paths.root).list();
        let active_build = startup_operations
            .iter()
            .find(|op| Job::builder_operation(op, &op.resource));
        let app = args
            .windows(2)
            .find(|a| a[0] == "--app")
            .map(|a| a[1].clone())
            .or_else(|| {
                (!builder)
                    .then(|| active_build.map(|op| op.resource.clone()))
                    .flatten()
            })
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
        let mut activity_notifications = operations_view::ActivityNotifications::new(&paths.root);
        activity_notifications.observe(&startup_operations);
        let resume_build_page = !builder && active_build.is_some_and(|op| op.resource == app);
        let mut window = Self {
            icons,
            icon_receiver,
            icon_sender,
            icons_requested: Default::default(),
            catalog_receiver,
            catalog_message: String::new(),
            new_apps,
            new_app_fades: BTreeMap::new(),
            custom_link: String::new(),
            custom_message: String::new(),
            custom_failed: false,
            custom_receiver: None,
            custom_download_choice: None,
            custom_download_index: 0,
            custom_download_confirmed: false,
            custom_setup: None,
            custom_resume_install: None,
            custom_resume_versions: None,
            custom_apps_draft: Vec::new(),
            remove_custom_confirm: None,
            download_availability: BTreeMap::new(),
            root_text: paths.root.display().to_string(),
            tools_text: paths.tools.display().to_string(),
            whats_new_animation_started: None,
            whats_new_open: !builder
                && whats_new::should_show(&paths.root, env!("CARGO_PKG_VERSION")),
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
            page: if resume_build_page || std::env::args().any(|a| a == "--preview-details") {
                Page::App
            } else {
                Page::Overview
            },
            search: String::new(),
            settings: std::env::args().any(|a| a == "--preview-settings" || a == "--preview-apps"),
            settings_jump: None,
            selection: std::env::args().any(|a| a == "--preview-apps"),
            visibility_open: false,
            hide_notice_open: false,
            hide_notice_seen: false,
            visibility_draft: Vec::new(),
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
            tray: None,
            tray_wake: tray::Wake::new(cc),
            tray_attempted: false,
            tray_error: None,
            tray_hidden: false,
            tray_notice_shown: false,
            tray_probe_at: Instant::now(),
            tray_probe: None,
            capture_frame: 0,
            launch_settings_open: false,
            launch_build_options: false,
            build_options_open: false,
            build_choice: None,
            build_choices: Vec::new(),
            build_app_cache: None,
            build_cache_receiver: None,
            confirm_build: None,
            build_parent: String::new(),
            build_shortcut: false,
            build_source_fresh: false,
            build_folder_receiver: None,
            delete_build_confirm: false,
            clean_build_confirm: false,
            remove_source_confirm: None,
            launch_draft: Default::default(),
            launch_executables_key: None,
            launch_executables: None,
            launch_executables_receiver: None,
            launch_arguments: String::new(),
            confirm_move: None,
            move_parent: String::new(),
            move_running: None,
            move_probe: None,
            move_probe_at: std::time::Instant::now(),
            move_folder: None,
            confirm_uninstall: false,
            delete_profile: false,
            confirm_install: None,
            install_selected_tag: None,
            additional_installation: false,
            install_draft_app: None,
            install_location_context: None,
            install_destination_preview: dialogs::DestinationPreview::default(),
            install_parent: String::new(),
            install_shortcut: true,
            install_result: None,
            install_receiver: None,
            folder_receiver: None,
            install_error: None,
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
            token_status: craft_apps_manager::network::TokenStatus::NotVerified,
            token_check_started: false,
            token_receiver: None,
            token_draft: String::new(),
            token_reveal: false,
            token_message: String::new(),
            token_action_receiver: None,
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
            installation_selection: None,
            display_backups: Default::default(),
            display_sources: Default::default(),
            display_builds: Default::default(),
            display_build_cache: Default::default(),
            display_launch: Default::default(),
            config_refresh_at: std::time::Instant::now(),
            operation_was_busy: false,
            operation_completed_at: None,
            alternates: Vec::new(),
            release_plan: None,
            plan_receiver: None,
            activity_open: false,
            activity_history: false,
            activity_anchor: None,
            activity_notifications,
            operation_records: startup_operations,
            operation_refresh: Instant::now() - Duration::from_secs(2),
            shared_window_receiver: None,
            shared_build_logs: Default::default(),
        };
        window.previews();
        Ok(window)
    }
    /// Opens a dialog or state directly, for screenshots of each part of the window:
    /// `--preview-dialog install|uninstall|launch|cleanup|manager|error|notice` and
    /// `--preview-build running|done|failed`. `--preview-progress` with
    /// `--preview-details` shows the progress on the app page.
    fn refresh_build_cache(&mut self) {
        self.build_app_cache = None;
        let paths = self.paths.clone();
        let app = self.app.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        self.build_cache_receiver = Some(receiver);
        std::thread::spawn(move || {
            let app_cache = builder::has_app_cache(&paths, &app).unwrap_or(false);
            let _ = sender.send((paths.root, app, app_cache));
        });
    }
    fn open_build_options(&mut self) {
        match builder::build_options(&self.paths, &self.app) {
            Ok(options) => {
                self.launch_arguments = options.launch.arguments.join("\n");
                self.launch_draft = options.launch;
                self.build_choices = builder::completed_builds(&self.paths, &self.app);
                self.build_choice = options
                    .selected_build
                    .filter(|folder| self.build_choices.iter().any(|(path, _)| path == folder));
                self.delete_build_confirm = false;
                self.clean_build_confirm = false;
                self.build_options_open = true;
                self.refresh_build_cache();
            }
            Err(error) => self.result(Err(error)),
        }
    }
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
                self.open_build_options();
            }
            Some("build-confirmation") => { self.build_parent = builder::output_root(&self.paths, &self.app).parent().unwrap().display().to_string(); self.confirm_build = Some(self.app.clone()); },
            Some("activity") | Some("activity-details") => self.activity_open = true,
            Some("activity-unread") => {
                self.activity_open = true;
                self.activity_notifications.observe(&[]);
            },
            Some("activity-history") => { self.activity_open = true; self.activity_history = true; },
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
        self.plan_receiver.is_some()
            || self.release_plan.is_some()
            || self.installation_selection.is_some()
    }
    fn start(&mut self, action: &str) {
        if action == "build" && self.confirm_build.is_none() {
            self.start_build(action);
            return;
        }
        // Starting a job replaces self.job, which would orphan a running one.
        if self.release_pending() || self.job.snapshot().busy {
            return;
        }
        // Cleaning and building share the builder lock.
        if matches!(action, "clean" | "clean-build" | "remove-source")
            && self.build_job.snapshot().busy
        {
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
        let parent = PathBuf::from(&self.build_parent);
        let shortcut = self.build_shortcut;
        self.job.describe(&action, &app);
        self.job.spawn(move |job| match action.as_str() {
            "releases" => updates::releases(&paths, &job, false),
            "install-app" => updates::install_app(&paths, &app, &job),
            "uninstall-app" => {
                apps::uninstall_with_profile(&paths, &app, delete_profile)?;
                job.report(
                    "App data",
                    if delete_profile {
                        "Deletion requested and removal completed"
                    } else {
                        "Kept"
                    },
                );
                Ok(())
            }
            "sources" => updates::sources(&paths, &paths.preferences()?.selected_sources, &job),
            "source-app" => updates::sources(&paths, &[app], &job),
            "remove-source" => updates::remove_source(&paths, &app),
            "clean-build" => {
                let folder =
                    builder::selected_build(&paths, &app)?.context("No completed local build")?;
                job.stage(
                    "Cleaning build files",
                    None,
                    "Keeping compiled builds and downloaded sources",
                );
                builder::clean_local(&paths, &app, &folder)
            }
            "build" => builder::build_to(&paths, &app, latest, Some(&parent), shortcut, &job),
            "setup" => tools::setup(&paths, &app, &job),
            "clear" => backups::clear(&paths),
            "clear-app" => backups::clear_app(&paths, &app),
            "clean" => builder::clean(&paths),
            _ => unreachable!(),
        });
    }
    /// Builds or sets up tools for the selected app, beside any release job.
    fn start_build(&mut self, action: &str) {
        if action == "build" && self.confirm_build.is_none() {
            self.build_parent = builder::output_root(&self.paths, &self.app)
                .parent()
                .unwrap()
                .display()
                .to_string();
            self.build_shortcut = false;
            self.build_source_fresh = false;
            self.confirm_build = Some(self.app.clone());
            return;
        }
        if self.build_job.snapshot().busy
            || self.cleaning()
            || self
                .operation_records
                .iter()
                .any(|operation| Job::builder_operation(operation, &self.app))
        {
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
        let parent = PathBuf::from(&self.build_parent);
        let shortcut = self.build_shortcut;
        self.build_job.describe(&action, &app);
        self.build_job.spawn(move |job| match action.as_str() {
            "build" => builder::build_to(&paths, &app, latest, Some(&parent), shortcut, &job),
            "setup" => tools::setup(&paths, &app, &job),
            _ => unreachable!(),
        });
    }
    fn begin_custom_install(&mut self, app: &str, tag: Option<String>, ctx: &egui::Context) {
        if self.custom_receiver.is_some() || self.custom_download_choice.is_some() {
            return;
        }
        self.install_error = None;
        self.custom_resume_install = Some((app.to_string(), tag.clone()));
        let root = self.paths.root.clone();
        let app = app.to_string();
        let preferences = self.app_preferences(&app);
        let ctx = ctx.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.custom_receiver = Some(rx);
        std::thread::spawn(move || {
            let result =
                catalog::prepare_custom_download(&root, &app, &preferences, tag.as_deref())
                    .map(|entry| CustomAppAddition::Added(Box::new(entry)))
                    .or_else(
                        |error| match error.downcast_ref::<catalog::DownloadChoice>() {
                            Some(choice) => Ok(CustomAppAddition::Choose(Box::new(choice.clone()))),
                            None => Err(format!("{error:#}")),
                        },
                    );
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }
    fn finish_custom_selection(&mut self, entry: &catalog::Entry, ctx: &egui::Context) {
        if let Some(app) = self.custom_resume_versions.take() {
            self.catalog_changed();
            self.app = app;
            self.open_versions_prepared(ctx);
        } else if let Some((app, tag)) = self.custom_resume_install.take() {
            self.catalog_changed();
            if self.confirm_install.as_deref() == Some(&app) {
                if let Some(tag) = tag {
                    self.install_selected_tag = Some(tag.clone());
                    if self.additional_installation {
                        self.load_install_prepared(&app, ctx);
                    } else {
                        self.review_version_prepared(app, tag);
                    }
                } else {
                    self.load_install_prepared(&app, ctx);
                }
            }
        } else if self.settings {
            self.custom_app_added(entry);
        }
    }
    fn load_install(&mut self, app: &str, ctx: &egui::Context) {
        if !self.additional_installation {
            if let Err(error) = self.use_installed_version_options(app) {
                self.install_error = Some(format!("{error:#}"));
                return;
            }
        }
        if catalog::get(app).is_some_and(|entry| entry.repository.contains('/')) {
            self.begin_custom_install(app, self.install_selected_tag.clone(), ctx);
        } else {
            self.load_install_prepared(app, ctx);
        }
    }
    fn load_install_prepared(&mut self, app: &str, ctx: &egui::Context) {
        self.install_error = None;
        let paths = self.paths.clone();
        let app = app.to_string();
        let ctx = ctx.clone();
        let preferences = self.build_preferences.clone();
        let additional = self.additional_installation;
        let tag = self.install_selected_tag.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.install_receiver = Some(rx);
        std::thread::spawn(move || {
            let job = Job::new(paths.at("logs/install-preview.log"), &preferences);
            let result = match (additional, tag.as_deref()) {
                (true, Some(tag)) => updates::plan_additional_version(&paths, &job, &app, tag),
                (true, None) => updates::plan_additional(&paths, &job, &app),
                (false, Some(tag)) => updates::plan_version(&paths, &job, &app, tag),
                (false, None) => updates::plan_app(&paths, &job, &app),
            }
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }
    fn run_install_plan(&mut self, app: String, plan: updates::ReleasePlan) {
        let paths = self.paths.clone();
        self.confirm_install = None;
        self.additional_installation = false;
        self.install_draft_app = None;
        self.install_result = None;
        self.app = app.clone();
        self.job = Job::new(paths.at("logs/updates.log"), &self.build_preferences);
        self.job.history(&self.job.log_path);
        self.job.describe(
            if plan.installer_downgrade() {
                "Downgrade"
            } else {
                "Install / update"
            },
            &app,
        );
        self.job_target = Some((app, "install-app".into()));
        self.job_action = "install-app".into();
        self.failure_dismissed = false;
        self.operation_was_busy = true;
        self.job
            .spawn(move |job| updates::execute_plan(&paths, &plan, &job));
    }
    fn open_versions(&mut self, ctx: &egui::Context) {
        if self.job.snapshot().busy || self.release_pending() {
            return;
        }
        let app = self.app.clone();
        if catalog::get(&app).is_some_and(|entry| entry.repository.contains('/')) {
            if self.custom_receiver.is_some() || self.custom_download_choice.is_some() {
                return;
            }
            self.begin_custom_install(&app, None, ctx);
            self.custom_resume_install = None;
            self.custom_resume_versions = Some(app.clone());
            self.versions_app = Some(app);
            self.versions_result = None;
        } else {
            self.open_versions_prepared(ctx);
        }
    }
    fn open_versions_prepared(&mut self, ctx: &egui::Context) {
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
    fn review_version(&mut self, app: String, tag: String, ctx: &egui::Context) {
        if self.job.snapshot().busy || self.release_pending() {
            return;
        }
        if let Err(error) = self.use_installed_version_options(&app) {
            self.result(Err(error));
            return;
        }
        if catalog::get(&app).is_some_and(|entry| entry.repository.contains('/')) {
            self.confirm_install = Some(app.clone());
            self.install_draft_app = Some(app.clone());
            self.versions_app = None;
            self.begin_custom_install(&app, Some(tag), ctx);
        } else {
            self.review_version_prepared(app, tag);
        }
    }
    fn review_version_prepared(&mut self, app: String, tag: String) {
        if self.job.snapshot().busy || self.release_pending() {
            return;
        }
        if let Err(error) = self.use_installed_version_options(&app) {
            self.result(Err(error));
            return;
        }
        self.install_shortcut = true;
        self.install_draft_app = Some(app.clone());
        self.confirm_install = Some(app.clone());
        self.install_error = None;
        self.install_selected_tag = Some(tag.clone());
        self.versions_app = None;
        let paths = self.paths.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.install_receiver = Some(rx);
        // Selecting a version only prepares a plan; installation starts after approval.
        let preferences = self.build_preferences.clone();
        std::thread::spawn(move || {
            let job = Job::new(paths.at("logs/install-preview.log"), &preferences);
            let result = updates::plan_version(&paths, &job, &app, &tag);
            let _ = tx.send(
                result
                    .as_ref()
                    .map(Clone::clone)
                    .map_err(|e| format!("{e:#}")),
            );
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
        self.job.describe("Review updates", "Application library");
        self.job.spawn(move |job| {
            let result = updates::plan_releases(&paths, &job);
            let _ = tx.send(result_for_display(&result));
            result.map(|_| ())
        });
    }
    /// True while a release job cleans build files, which blocks builds.
    fn cleaning(&self) -> bool {
        matches!(self.job_action.as_str(), "clean" | "clean-build") && self.job.snapshot().busy
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
        self.remove_custom_confirm = None;
        self.custom_apps_draft = catalog::custom_apps(&self.paths.root);
        self.custom_message.clear();
        self.custom_failed = false;
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
        self.mark_app_seen(app);
        if self.app != app {
            self.launch_settings_open = false;
            self.confirm_uninstall = false;
        }
        self.app = app.into();
        self.page = Page::App;
    }
    fn app_preferences(&self, app: &str) -> model::Preferences {
        let preferred = craft_apps_manager::install_options::preferred(&self.preferences, app);
        if !self.additional_installation && self.confirm_install.as_deref() != Some(app) {
            if let Some(actual) = self
                .display_config
                .as_ref()
                .and_then(|config| {
                    config
                        .apps
                        .iter()
                        .find(|copy| copy.name == app && !copy.path.is_empty())
                })
                .and_then(|copy| {
                    craft_apps_manager::install_options::for_installed(&self.preferences, copy).ok()
                })
            {
                return actual;
            }
        }
        if self
            .preferences
            .app_install_options
            .contains_key(&craft_apps_manager::install_options::identity(app))
        {
            return preferred;
        }
        self.display_config
            .as_ref()
            .and_then(|config| config.apps.iter().find(|copy| copy.name == app))
            .and_then(|copy| {
                craft_apps_manager::install_options::for_installed(&self.preferences, copy).ok()
            })
            .unwrap_or(preferred)
    }
    fn use_installed_version_options(&mut self, app: &str) -> anyhow::Result<()> {
        let installed = self.display_config.as_ref().and_then(|config| {
            config
                .apps
                .iter()
                .find(|copy| copy.name == app && !copy.path.is_empty())
        });
        if let Some(installed) = installed {
            let options = craft_apps_manager::install_options::InstallOptions::installed(installed)
                .ok_or_else(|| anyhow::anyhow!("Installation format or architecture is unknown. Confirm the existing installation before changing versions."))?;
            let preferred = craft_apps_manager::install_options::preferred(&self.preferences, app);
            if preferred.release_format != options.release_format
                || preferred.architecture != options.architecture
            {
                self.save_install_options(app, options)?;
            }
        }
        Ok(())
    }
    fn save_install_options(
        &mut self,
        app: &str,
        options: craft_apps_manager::install_options::InstallOptions,
    ) -> anyhow::Result<()> {
        options.validate()?;
        let mut preferences = self.paths.read_preferences()?;
        preferences
            .app_install_options
            .insert(craft_apps_manager::install_options::identity(app), options);
        craft_apps_manager::files::write_json(
            &self.paths.at("manager-settings.json"),
            &preferences,
        )?;
        self.preferences = preferences;
        self.download_availability.clear();
        Ok(())
    }
    fn installer_architecture_blocker(&self, app: &str) -> Option<&model::Installed> {
        self.display_config.as_ref()?.apps.iter().find(|copy| {
            copy.name == app
                && model::installer_architecture_conflict(copy, &self.app_preferences(app))
        })
    }
    fn install_available(&mut self, app: &str) -> bool {
        let key = (
            app.to_string(),
            self.app_preferences(app).release_format.clone(),
            self.app_preferences(app).architecture.clone(),
        );
        let preferences = self.app_preferences(app);
        *self
            .download_availability
            .entry(key)
            .or_insert_with(|| updates::cached_download_available(&self.paths, app, &preferences))
            != Some(false)
    }
    fn unavailable_download_message(&self, app: &str) -> String {
        if let Some(copy) = self.installer_architecture_blocker(app) {
            return model::installer_architecture_conflict_message(
                copy,
                &self.app_preferences(app),
            );
        }
        format!(
            "No {} download is available for {}.",
            self.app_preferences(app).release_format,
            model::architecture_label(&self.app_preferences(app).architecture)
        )
    }
    fn status(&self, app: &str) -> AppStatus {
        let config = self.display_config.as_ref();
        let installed = config
            .and_then(|c| c.apps.iter().find(|a| a.name == app))
            .filter(|a| !a.version.is_empty() && !a.path.is_empty())
            .cloned();
        let alternate_architecture = None;
        let matching_format = installed.is_some();
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
            alternate: self
                .alternates
                .iter()
                .find(|a| {
                    a.name == app && installed.as_ref().is_none_or(|copy| copy.path != a.path)
                })
                .cloned(),
            alternate_architecture,
            installed,
            checking: self.checking_apps.contains(app),
            loaded: config.is_some(),
        }
    }
    /// Re-reads settings after the app list changed, adopting newly found apps.
    fn is_new_app(&self, app: &str) -> bool {
        catalog::get(app).is_some_and(|entry| {
            self.new_apps
                .as_ref()
                .is_some_and(|tracker| tracker.is_new(&entry))
        })
    }

    fn mark_app_seen(&mut self, app: &str) {
        if let (Some(tracker), Some(entry)) = (&mut self.new_apps, catalog::get(app)) {
            let was_new = tracker.is_new(&entry);
            if let Err(error) = tracker.mark_seen(&entry) {
                self.error = Some(format!("Could not save the new-app indicator: {error:#}"));
            } else if was_new {
                self.new_app_fades.insert(app.to_owned(), Instant::now());
            }
        }
    }

    fn new_app_opacity(&self, app: &str) -> f32 {
        if self.is_new_app(app) {
            return 1.0;
        }
        self.new_app_fades.get(app).map_or(0.0, |started| {
            let t = (started.elapsed().as_secs_f32() / 0.28).clamp(0.0, 1.0);
            1.0 - t * t * (3.0 - 2.0 * t)
        })
    }

    fn new_app_rank(&self, app: &str) -> usize {
        catalog::get(app)
            .and_then(|entry| {
                self.new_apps
                    .as_ref()
                    .map(|tracker| tracker.discovery_rank(&entry))
            })
            .unwrap_or(usize::MAX)
    }

    fn catalog_changed(&mut self) {
        if let Some(tracker) = &mut self.new_apps {
            if let Err(error) = tracker.refresh(&self.paths.root, &catalog::all()) {
                self.error = Some(format!("Could not refresh new-app indicators: {error:#}"));
            }
        }
        self.download_availability.clear();
        if !self.builder && catalog::get(&self.app).is_none() {
            self.page = Page::Overview;
            self.confirm_install = None;
            self.versions_app = None;
        }
        match self.paths.preferences() {
            Ok(preferences) => {
                if !self.settings {
                    self.settings_draft = preferences.clone();
                }
                self.preferences = preferences;
            }
            Err(error) => self.error = Some(format!("{error:#}")),
        }
        // Keep the last installed-app snapshot visible during the refresh. Clearing
        // it makes a metadata-only version lookup temporarily look uninstalled,
        // changing action buttons and restarting sidebar/home layout animations.
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
    fn observed_build_state(&self, app: &str) -> craft_apps_manager::jobs::State {
        let (local, local_app) = if self.builder {
            (self.job.snapshot(), self.app.as_str())
        } else {
            (self.build_job.snapshot(), self.build_app.as_str())
        };
        let mut observed =
            Job::observed_builder_state(&local, local_app, &self.operation_records, app);
        self.apply_shared_build_log(&mut observed, app);
        observed
    }
    fn apply_shared_build_log(&self, state: &mut craft_apps_manager::jobs::State, app: &str) {
        let local_busy = if self.builder {
            self.job.snapshot().busy
        } else {
            self.build_app == app && self.build_job.snapshot().busy
        };
        if local_busy {
            return;
        }
        if let Some(operation) = Job::latest_builder_operation(&self.operation_records, app) {
            if operation.owner != std::process::id() {
                if let Some(log) = self.shared_build_logs.get(&operation.id) {
                    state.log.clone_from(log);
                }
            }
        }
    }
    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(receiver) = &self.build_cache_receiver {
            match receiver.try_recv() {
                Ok((root, app, app_cache)) => {
                    self.build_cache_receiver = None;
                    if root == self.paths.root && app == self.app && self.build_options_open {
                        self.build_app_cache = Some(app_cache);
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.build_cache_receiver = None
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }

        if self.build_options_open
            && self.build_app_cache.is_none()
            && self.build_cache_receiver.is_none()
            && !self.job.snapshot().busy
        {
            self.refresh_build_cache();
        }
        if !self.settings {
            self.token_draft.clear();
            self.token_reveal = false;
        }
        if let Some(receiver) = &self.token_action_receiver {
            let result = match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Token operation stopped unexpectedly.".into()))
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                self.token_action_receiver = None;
                match result {
                    Ok(status) => {
                        self.token_status = status;
                        self.token_message = match status {
                            craft_apps_manager::network::TokenStatus::Connected => "Token applied and verified.",
                            craft_apps_manager::network::TokenStatus::Invalid => "GitHub rejected the active token. Replace it or check your environment variable.",
                            craft_apps_manager::network::TokenStatus::NotConfigured => "Saved token removed.",
                            _ => "Token applied, but GitHub could not be reached for verification.",
                        }.into();
                    }
                    Err(error) => {
                        self.token_message = error;
                        self.token_check_started = false;
                    }
                }
            }
        }
        if !self.token_check_started {
            self.token_check_started = true;
            if self.token_status != craft_apps_manager::network::TokenStatus::NotConfigured {
                let (tx, rx) = std::sync::mpsc::channel();
                self.token_receiver = Some(rx);
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(craft_apps_manager::network::verify_token());
                    ctx.request_repaint();
                });
            }
        }
        if let Some(receiver) = &self.token_receiver {
            match receiver.try_recv() {
                Ok(status) => {
                    self.token_status = status;
                    self.token_receiver = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.token_status = craft_apps_manager::network::TokenStatus::UnableToVerify;
                    self.token_receiver = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(receiver) = &self.installation_selection {
            let result = match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err(
                    "Installation selection stopped unexpectedly. Try again.".into(),
                )),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                self.installation_selection = None;
                ctx.animate_value_with_time(
                    egui::Id::new(("installation-actions-opacity", &self.app)),
                    0.55,
                    0.0,
                );
                self.config_receiver = None;
                self.config_refresh_at = std::time::Instant::now();
                match result {
                    Ok(selected) => {
                        self.display_config = Some(selected.config);
                        self.preferences = selected.preferences;
                        self.release_checks.remove(&selected.app);
                        self.download_availability.clear();
                        self.confirm_uninstall = false;
                        self.launch_settings_open = false;
                    }
                    Err(error) => self.error = Some(error),
                }
            }
        }
        if let Some(receiver) = &self.shared_window_receiver {
            match receiver.try_recv() {
                Ok((root, appearance, logs)) => {
                    self.shared_window_receiver = None;
                    if root == self.paths.root {
                        self.shared_build_logs = logs;
                        if let Some((_, badge)) = appearance {
                            self.preferences.hide_activity_badge = badge;
                            if !self.settings {
                                self.settings_draft.hide_activity_badge = badge;
                            }
                        }
                        if self.builder {
                            if let Some((theme, _)) = appearance {
                                self.preferences.theme = theme;
                                self.settings_draft.theme = theme;
                                theme::apply(ctx, theme);
                            }
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.shared_window_receiver = None;
                    self.build_job
                        .log("Window synchronization worker stopped unexpectedly.");
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.operation_refresh.elapsed() >= Duration::from_secs(1) {
            let records = self.job.operations.list();
            if self.shared_window_receiver.is_none() {
                let paths = self.paths.clone();
                let records: Vec<_> = Job::latest_builder_operation(&records, &self.app)
                    .filter(|op| op.owner != std::process::id())
                    .cloned()
                    .into_iter()
                    .collect();
                let context = ctx.clone();
                let (sender, receiver) = std::sync::mpsc::channel();
                self.shared_window_receiver = Some(receiver);
                std::thread::spawn(move || {
                    use std::io::{Read, Seek, SeekFrom};
                    let appearance = paths
                        .read_preferences()
                        .ok()
                        .map(|p| (p.theme, p.hide_activity_badge));
                    let mut logs = std::collections::HashMap::new();
                    for operation in records.iter().filter(|op| {
                        matches!(
                            op.kind.as_str(),
                            "Build" | "Source download" | "Set up tools"
                        )
                    }) {
                        if let Some(path) = operation.facts.get("Log file") {
                            // Read a bounded tail outside the UI thread, including completed builds.
                            if let Ok(mut file) = std::fs::File::open(path) {
                                if let Ok(metadata) = file.metadata() {
                                    let _ = file.seek(SeekFrom::Start(
                                        metadata.len().saturating_sub(200_000),
                                    ));
                                    let mut bytes = Vec::new();
                                    if file.take(200_000).read_to_end(&mut bytes).is_ok() {
                                        logs.insert(
                                            operation.id.clone(),
                                            String::from_utf8_lossy(&bytes).into_owned(),
                                        );
                                    }
                                }
                            }
                        }
                    }
                    let _ = sender.send((paths.root, appearance, logs));
                    context.request_repaint();
                });
            }
            self.activity_notifications.observe(&records);
            if records.iter().any(|record| {
                record.lifecycle.terminal()
                    && !self
                        .operation_records
                        .iter()
                        .any(|old| old.id == record.id && old.lifecycle.terminal())
            }) {
                self.config_refresh_at = Instant::now();
            }
            self.operation_records = records;
            self.operation_refresh = Instant::now();
        }
        if let Some(receiver) = &self.catalog_receiver {
            if let Some(result) =
                poll_worker_result(receiver, || "App list worker stopped unexpectedly.".into())
            {
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
        if let Some(result) = poll_custom_result(&mut self.custom_receiver) {
            match result {
                Ok(CustomAppAddition::Setup(setup)) => self.custom_setup = Some(setup),
                Ok(CustomAppAddition::Added(entry)) => self.finish_custom_selection(&entry, ctx),
                Ok(CustomAppAddition::Choose(choice)) => {
                    self.custom_message = "Choose a compatible download for this app.".into();
                    self.custom_failed = false;
                    self.custom_download_index = 0;
                    self.custom_download_confirmed = false;
                    self.custom_download_choice = Some(choice);
                }
                Err(error) => {
                    if self.custom_resume_versions.take().is_some() {
                        self.versions_result = Some(Err(error.clone()));
                    }
                    if self.custom_resume_install.take().is_some() {
                        self.install_result = Some(Err(error.clone()));
                    }
                    self.custom_message = error;
                    self.custom_failed = true;
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
        if let Some(receiver) = &self.install_receiver {
            match receiver.try_recv() {
                Ok(result) => {
                    self.install_receiver = None;
                    self.install_result = Some(result);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.install_receiver = None;
                    self.install_result =
                        Some(Err("Release details could not be loaded. Try again.".into()));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(receiver) = &self.folder_receiver {
            match receiver.try_recv() {
                Ok(result) => {
                    self.folder_receiver = None;
                    match result {
                        Ok(Some(path)) => self.install_parent = path.display().to_string(),
                        Ok(None) => {}
                        Err(error) => self.install_error = Some(error),
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.folder_receiver = None;
                    self.install_error =
                        Some("Folder picker closed unexpectedly. Enter the path directly.".into());
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
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
        let busy = self.job.snapshot().busy;
        let building = self.build_job.snapshot().busy;
        if self.operation_was_busy && !busy {
            if self.job_action == "clean-build" && self.build_options_open {
                self.refresh_build_cache();
            }
            self.catalog_changed();
        }
        if (self.operation_was_busy && !busy) || (self.build_was_busy && !building) {
            self.config_refresh_at = std::time::Instant::now();
        }
        if busy {
            self.operation_completed_at = None;
        } else if self.operation_was_busy && self.job.state.lock().unwrap().stage == "Complete" {
            self.operation_completed_at = Some(std::time::Instant::now());
        }
        self.operation_was_busy = busy;
        self.build_was_busy = building;
        if let Some(receiver) = &self.config_receiver {
            if let Some(result) = poll_worker_result(receiver, || {
                "Installation discovery worker stopped unexpectedly.".into()
            }) {
                match result {
                    Ok(snapshot) => {
                        let changed_versions: Vec<_> = snapshot
                            .config
                            .apps
                            .iter()
                            .filter(|app| !app.path.is_empty() && !app.version.is_empty())
                            .filter(|app| {
                                self.display_config.as_ref().is_some_and(|old| {
                                    old.apps.iter().any(|previous| {
                                        previous.name == app.name && previous.version != app.version
                                    })
                                })
                            })
                            .cloned()
                            .collect();
                        let mut unknown_versions = Vec::new();
                        for app in changed_versions {
                            // A check for the previous version must not suppress checking the new one.
                            self.checking_apps.remove(&app.name);
                            if let Some(result) = updates::cached_installation_check(
                                &self.paths,
                                &app,
                                &self.preferences,
                            ) {
                                self.release_checks.insert(
                                    app.name.clone(),
                                    (
                                        app.version.clone(),
                                        result.map_err(|error| format!("{error:#}")),
                                    ),
                                );
                            } else {
                                unknown_versions.push((app.name.clone(), app.version.clone()));
                            }
                        }
                        if !unknown_versions.is_empty() {
                            self.check_apps(ctx, unknown_versions);
                        }
                        if self.display_config.is_none() {
                            for app in &snapshot.config.apps {
                                if let Some(check) =
                                    snapshot.checks.get(&craft_apps_manager::hourly::key(
                                        &craft_apps_manager::install_options::for_installed(
                                            &self.preferences,
                                            app,
                                        )
                                        .unwrap_or_else(|_| self.app_preferences(&app.name)),
                                        &app.name,
                                    ))
                                {
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
                        self.display_build_cache = snapshot.build_cache;
                        self.display_launch = snapshot.launch;
                    }
                    Err(error) => self.error = Some(error),
                }
                self.config_receiver = None;
            }
        }
        if !busy
            && self.installation_selection.is_none()
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
                    let builds: BTreeMap<String, (PathBuf, Option<model::BuildInfo>)> =
                        model::apps()
                            .into_iter()
                            .filter_map(|name| {
                                let folder = builder::selected_build(&paths, &name).ok()??;
                                let info = craft_apps_manager::files::read_json(
                                    &folder.join("build-info.json"),
                                )
                                .ok();
                                Some((name, (folder, info)))
                            })
                            .collect();
                    let build_cache = builds
                        .iter()
                        .filter(|(app, (folder, _))| {
                            builder::has_local_cache(&paths, app, folder).unwrap_or(false)
                        })
                        .map(|(app, _)| app.clone())
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
                        build_cache,
                        launch,
                    })
                })();
                let _ = tx.send(result.map_err(|e| format!("{e:#}")));
                ctx.request_repaint();
            });
        }
        if !self.builder && self.apps_startup_pending && !self.job.snapshot().busy {
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
            if let Some(result) = poll_worker_result(receiver, || {
                "Background check worker stopped unexpectedly.".into()
            }) {
                self.periodic_receiver = None;
                self.app_check_at = Instant::now();
                match result {
                    Ok(checks) => {
                        for (key, check) in checks {
                            if let Some(app) = key.rsplit(':').next() {
                                if self
                                    .display_config
                                    .as_ref()
                                    .and_then(|config| {
                                        config.apps.iter().find(|copy| copy.name == app)
                                    })
                                    .is_some_and(|copy| {
                                        copy.version == check.installed
                                            && craft_apps_manager::install_options::for_installed(
                                                &self.preferences,
                                                copy,
                                            )
                                            .is_ok_and(|prefs| {
                                                craft_apps_manager::hourly::key(&prefs, app) == key
                                            })
                                    })
                                {
                                    self.release_checks.insert(
                                        app.to_owned(),
                                        (check.installed, Ok(check.latest)),
                                    );
                                }
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
            && !self.job.snapshot().busy
            && !self.build_job.snapshot().busy
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
                let result = job.tracked("Background update check", "Application library", |job| {
                    hourly::run_periodic(&paths, job).and_then(|_| hourly::read(&paths))
                });
                let _ = tx.send(result.map_err(|e| format!("{e:#}")));
                ctx.request_repaint();
            });
        }
        if self.manager_startup_pending {
            self.manager_startup_pending = false;
            self.check_manager(ctx);
        }
        if let Some(receiver) = &self.manager_receiver {
            if let Some(result) = poll_worker_result(receiver, || {
                "Manager update check stopped unexpectedly.".into()
            }) {
                match result {
                    Ok(Some(available)) => {
                        self.manager_message =
                            format!("Version {} is available.", available.display_version());
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
            if let Some(result) = poll_worker_result(receiver, || {
                (
                    false,
                    "Manager update preparation stopped unexpectedly.".into(),
                )
            }) {
                self.manager_plan = None;
                match result {
                    Ok(plan) => match self_update::launch(&plan) {
                        Ok(()) => {
                            self.closing = true;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
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
                if self.display_config.as_ref().is_some_and(|config| {
                    config
                        .apps
                        .iter()
                        .any(|installed| installed.name == app && installed.version != version)
                }) {
                    continue;
                }
                self.checking_apps.remove(&app);
                self.download_availability
                    .retain(|(name, _, _), _| name != &app);
                self.release_checks.insert(app, (version, result));
            }
        }
    }
    fn completed_progress_opacity(&self, ctx: &egui::Context) -> f32 {
        let Some(at) = self.operation_completed_at else {
            return 0.0;
        };
        let elapsed = at.elapsed().as_secs_f32();
        if elapsed >= 2.2 {
            return 0.0;
        }
        ctx.request_repaint_after(Duration::from_millis(16));
        let fade = ((elapsed - 2.0) / 0.2).clamp(0.0, 1.0);
        1.0 - fade * fade * (3.0 - 2.0 * fade)
    }
    fn screenshot(&mut self, ctx: &egui::Context) {
        let Ok(path) = std::env::var("CRAFT_SCREENSHOT_TO") else {
            return;
        };
        self.capture_frame += 1;
        if self.capture_frame >= 10
            && (self.builder || self.display_config.is_some())
            && (self.confirm_install.is_none() || self.install_result.is_some())
        {
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
        .enabled(!requested && self.job.cancellation_available() && !authorizing_package(stage))
        .show(ui)
        .on_hover_text("Downloads can be cancelled. An authorized Linux package transaction must finish to keep the package database consistent.")
        .clicked()
        {
            self.job.request_cancel();
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
        self.poll_tray(ctx);
        self.screenshot(ctx);
        let mut state = self.job.snapshot();
        state.busy |= self.release_pending();
        let building = self.build_job.snapshot().busy;
        let close_requested = ctx.input(|i| i.viewport().close_requested());
        if close_requested
            && tray::hide_on_close(
                self.preferences.close_to_tray,
                self.tray
                    .as_ref()
                    .is_some_and(|tray| tray.set_visible(true).is_ok()),
                self.closing,
            )
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.tray_hidden = true;
            if !self.tray_notice_shown {
                self.tray_notice_shown = true;
                std::thread::spawn(|| {
                    if let Ok(exe) = std::env::current_exe() {
                        let _ = platform::notify_with_title(&exe, "Craft Apps Manager is still running", "Use the tray icon to reopen the manager or choose Exit to quit. Periodic update checks continue when enabled.");
                    }
                });
            }
        } else if close_requested && (state.busy || building) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.closing = true;
            self.release_plan = None;
            self.job.request_cancel();
            self.build_job.request_cancel();
        }
        if self.closing && !state.busy && !building {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.builder {
            let observed = self.observed_build_state(&self.app);
            self.builder_view(ctx, &observed);
        } else {
            self.top_bar(ctx, &state);
            self.status_bar(ctx, &state);
            self.sidebar(ctx, &state);
            match self.page {
                Page::Overview => self.overview(ctx, &state),
                Page::App => self.app_page(ctx, &state),
            }
        }
        self.activity_panel(ctx);
        self.settings_ui(ctx);
        self.dialogs(ctx);
        self.whats_new_dialog(ctx);
        restore_dialog_focus(ctx);
        if state.busy
            || building
            || self
                .operation_records
                .iter()
                .any(|op| Job::builder_operation(op, &op.resource))
        {
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
/// A disconnected one-shot worker is a failure, not a request still loading.
fn poll_worker_result<T, E>(
    receiver: &std::sync::mpsc::Receiver<Result<T, E>>,
    disconnected: impl FnOnce() -> E,
) -> Option<Result<T, E>> {
    match receiver.try_recv() {
        Ok(result) => Some(result),
        Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err(disconnected())),
        Err(std::sync::mpsc::TryRecvError::Empty) => None,
    }
}
fn poll_custom_result(
    receiver: &mut Option<std::sync::mpsc::Receiver<Result<CustomAppAddition, String>>>,
) -> Option<Result<CustomAppAddition, String>> {
    match receiver.as_ref()?.try_recv() {
        Ok(result) => {
            *receiver = None;
            Some(result)
        }
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            *receiver = None;
            Some(Err(
                "Download lookup stopped unexpectedly. Click Add to try again.".into(),
            ))
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => None,
    }
}
#[cfg(test)]
mod outcome_tests {
    use super::*;
    #[test]
    fn worker_poll_distinguishes_pending_result_and_abandoned_work() {
        let (tx, rx) = std::sync::mpsc::channel::<Result<u32, String>>();
        assert!(poll_worker_result(&rx, || "stopped".into()).is_none());
        tx.send(Ok(42)).unwrap();
        drop(tx);
        assert_eq!(poll_worker_result(&rx, || "stopped".into()), Some(Ok(42)));
        assert_eq!(
            poll_worker_result(&rx, || "stopped".into()),
            Some(Err("stopped".into()))
        );
    }
    #[test]
    fn custom_lookup_failure_releases_add_for_retry() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut receiver = Some(rx);
        assert!(poll_custom_result(&mut receiver).is_none());
        assert!(receiver.is_some());
        tx.send(Err("No compatible portable download".into()))
            .unwrap();
        assert!(poll_custom_result(&mut receiver).unwrap().is_err());
        assert!(receiver.is_none());
        let (tx, rx) = std::sync::mpsc::channel();
        receiver = Some(rx);
        drop(tx);
        assert!(poll_custom_result(&mut receiver).unwrap().is_err());
        assert!(receiver.is_none());
    }
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
