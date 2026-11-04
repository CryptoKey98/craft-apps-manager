use anyhow::{Context, Result};
use craft_apps_manager::{
    apps, builder, catalog, hourly,
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

mod dialogs;
mod manager_handoff;
mod resources;
mod screens;
mod widgets;
use resources::{app_icon, fonts, icon_bytes, placeholder, texture};
use widgets::operations_view;
mod theme;
mod tray;
mod whats_new;

/// Per-app package work, including children of Update All. Queued items are not installing yet.
fn sidebar_install_operation<'a>(
    records: &'a [craft_apps_manager::operations::Operation],
    app: &str,
) -> Option<&'a craft_apps_manager::operations::Operation> {
    use craft_apps_manager::operations::Lifecycle;
    records
        .iter()
        .filter(|record| {
            record.resource == app
                && is_installation_operation(record)
                && matches!(
                    record.lifecycle,
                    Lifecycle::Running | Lifecycle::CancellationRequested
                )
        })
        .max_by_key(|record| (record.parent.is_some(), record.heartbeat))
}

/// App-page progress must use the selected child's measurements, not the batch totals.
fn app_installation_state(
    records: &[craft_apps_manager::operations::Operation],
    app: &str,
    local: &craft_apps_manager::jobs::State,
    local_operation: Option<&str>,
) -> Option<craft_apps_manager::jobs::State> {
    let operation = sidebar_install_operation(records, app)?;
    let local_operation = local_operation?;
    if !local.busy
        || (operation.id != local_operation && operation.parent.as_deref() != Some(local_operation))
    {
        return None;
    }
    let mut state = local.clone();
    state.busy = true;
    state.stage = operation.stage.clone();
    state.detail = operation.detail.clone();
    state.progress = operation.progress;
    Some(state)
}

fn is_installation_operation(record: &craft_apps_manager::operations::Operation) -> bool {
    matches!(
        record.kind.as_str(),
        "Install" | "Update" | "Install / update" | "Downgrade"
    )
}

/// Read committed facts only: detecting/reconstructing installations during an install
/// could mistake temporary installer state for a removed application.
fn completed_install_inventory(paths: &Paths) -> Result<Option<model::Config>, String> {
    let config = paths
        .read_inventory()
        .map_err(|error| format!("{error:#}"))?;
    if config
        .as_ref()
        .is_some_and(|config| std::path::Path::new(&config.apps_root) != paths.root)
    {
        return Err("Installation inventory points to a different data folder.".into());
    }
    Ok(config)
}

type ReleaseCheck = Result<Option<String>, String>;
type CheckMessage = (u64, model::Installed, ReleaseCheck);
struct InstallationSelection {
    app: String,
    config: model::Config,
    preferences: model::Preferences,
    check: Option<ReleaseCheck>,
}

fn installation_check_id(
    paths: &model::Paths,
    copy: &model::Installed,
    generation: u64,
) -> egui::Id {
    egui::Id::new((
        "installation-update-status",
        &paths.root,
        generation,
        model::repository(&copy.name),
        &copy.name,
        &copy.version,
        &copy.install_kind,
        &copy.architecture,
        &copy.package_type,
        craft_apps_manager::installation_labels::path_identity(&copy.path),
    ))
}

/// Everything the window shows that is read from disk, refreshed off the UI thread.
struct Snapshot {
    config: model::Config,
    alternates: Vec<model::Installed>,
    rollbacks: BTreeMap<
        String,
        Result<craft_apps_manager::installations::rollbacks::RollbackStatus, String>,
    >,
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
    settings_token_entry: bool,
    settings_token_focus: bool,
    token_help_open: bool,
    selection: bool,
    visibility_open: bool,
    hide_notice_open: bool,
    hide_notice_seen: bool,
    visibility_draft: Vec<String>,
    source_selection: bool,
    selection_draft: Vec<String>,
    selection_origin: Option<screens::settings::edit_session::SelectionOrigin>,
    settings_edit: Option<screens::settings::edit_session::SettingsEditSession>,
    settings_load: Option<std::sync::mpsc::Receiver<Result<screens::settings::Loaded, String>>>,
    settings_save: Option<std::sync::mpsc::Receiver<screens::settings::save::SaveOutcome>>,
    custom_settings_session: Option<u64>,
    settings_draft: Preferences,
    build_draft: BuilderPreferences,
    auto: bool,
    auto_draft: bool,
    error: Option<String>,
    selection_notice: Option<String>,
    root_text: String,
    tools_text: String,
    confirm_clean: bool,
    api_cache_cleanup: screens::settings::cache_cleanup::State,
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
    uninstall_profiles: dialogs::ProfilePreview,
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
    compatibility_info: Option<dialogs::CompatibilityInfo>,
    release_checks: BTreeMap<String, (String, ReleaseCheck)>,
    check_receiver: std::sync::mpsc::Receiver<CheckMessage>,
    check_sender: std::sync::mpsc::Sender<CheckMessage>,
    checking_apps: std::collections::BTreeSet<String>,
    check_generation: u64,
    apps_startup_pending: bool,
    app_check_at: Instant,
    periodic_receiver: Option<std::sync::mpsc::Receiver<Result<hourly::Checks, String>>>,
    manager_receiver: Option<
        std::sync::mpsc::Receiver<
            Result<craft_apps_manager::manager_updates::coordinator::ManagerCheck, String>,
        >,
    >,
    manager_available: Option<self_update::Available>,
    manager_message: String,
    manager_startup_pending: bool,
    token_status: craft_apps_manager::network::TokenStatus,
    token_check_started: bool,
    token_receiver: Option<std::sync::mpsc::Receiver<craft_apps_manager::network::TokenStatus>>,
    token_verification_session: Option<u64>,
    token_saved_verification_pending: bool,
    token_draft: screens::settings::TokenDraft,
    token_reveal: bool,
    token_message: String,
    manager_plan: Option<std::sync::mpsc::Receiver<Result<(), manager_handoff::Failure>>>,
    confirm_self_update: bool,
    rollback_dialog: Option<dialogs::rollback::RollbackDialog>,
    display_rollbacks: BTreeMap<
        String,
        Result<craft_apps_manager::installations::rollbacks::RollbackStatus, String>,
    >,
    display_config: Option<model::Config>,
    inventory_receiver: Option<std::sync::mpsc::Receiver<Result<Option<model::Config>, String>>>,
    inventory_refresh_pending: bool,
    config_receiver: Option<std::sync::mpsc::Receiver<Result<Snapshot, String>>>,
    installation_selection:
        Option<std::sync::mpsc::Receiver<Result<InstallationSelection, String>>>,
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
    // Release ownership after the other window resources have been dropped.
    instance: Option<crate::app::instance::Instance>,
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
        // Unit-test windows must not silently contact production services or
        // read real credentials. Explicit worker behavior has separate tests.
        let allow_startup_network = !cfg!(test);
        let manager_startup_pending = allow_startup_network
            && !builder
            && (preferences.check_manager_on_startup
                || craft_apps_manager::manager_updates::bridge::restart_requested()
                || craft_apps_manager::manager_updates::coordinator::needs_bridge_check(&paths));
        let apps_startup_pending =
            allow_startup_network && !builder && preferences.check_installed_apps_on_startup;
        let (check_sender, check_receiver) = std::sync::mpsc::channel();
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
        let catalog_receiver = (allow_startup_network
            && preferences.check_catalog_on_startup
            && std::env::args().all(|a| !a.starts_with("--preview")))
        .then(|| {
            let (tx, rx) = std::sync::mpsc::channel();
            let root = paths.root.clone();
            let ctx = cc.egui_ctx.clone();
            std::thread::spawn(move || {
                let result = catalog::refresh_for(&root, catalog::DiscoveryTrigger::Startup)
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
            instance: None,
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
                && whats_new::should_show(
                    &paths.root,
                    env!("CARGO_PKG_VERSION"),
                    craft_apps_manager::update_revisions::BUILD_REVISION,
                ),
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
            settings_token_entry: false,
            settings_token_focus: false,
            token_help_open: false,
            selection: std::env::args().any(|a| a == "--preview-apps"),
            visibility_open: false,
            hide_notice_open: false,
            hide_notice_seen: false,
            visibility_draft: Vec::new(),
            source_selection: false,
            selection_draft: preferences.selected_apps.clone(),
            selection_origin: None,
            settings_edit: None,
            settings_load: None,
            settings_save: None,
            custom_settings_session: None,
            settings_draft: preferences.clone(),
            build_draft: build_preferences.clone(),
            preferences,
            build_preferences,
            auto,
            auto_draft: auto,
            error: self_update::startup_message(),
            selection_notice: None,
            confirm_clean: false,
            api_cache_cleanup: Default::default(),
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
            uninstall_profiles: Default::default(),
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
            compatibility_info: None,
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
            token_check_started: !allow_startup_network,
            token_receiver: None,
            token_verification_session: None,
            token_saved_verification_pending: false,
            token_draft: screens::settings::TokenDraft::default(),
            token_reveal: false,
            token_message: String::new(),
            manager_plan: None,
            confirm_self_update: false,
            rollback_dialog: None,
            display_rollbacks: Default::default(),
            display_config: None,
            inventory_receiver: None,
            inventory_refresh_pending: false,
            config_receiver: None,
            installation_selection: None,
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
            Some("cleanup") => self.confirm_clean = true,
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
        self.check_manager_trigger(
            ctx,
            craft_apps_manager::manager_updates::check_state::CheckTrigger::Manual,
        );
    }
    fn start_manager_update(
        &mut self,
        ctx: &egui::Context,
        available: self_update::Available,
        journey: Option<craft_apps_manager::manager_updates::bridge::Journey>,
    ) {
        if self.job.snapshot().busy
            || self.build_job.snapshot().busy
            || self.release_pending()
            || self.manager_plan.is_some()
            || craft_apps_manager::manager_updates::bridge::foreground_busy(
                &self.operation_records,
                None,
            )
        {
            self.manager_message = "Manager update paused while another operation is running. Check again to continue.".into();
            return;
        }
        let paths = self.paths.clone();
        let ctx = ctx.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.manager_plan = Some(rx);
        self.job = Job::interactive(self.paths.at("logs/updates.log"), &self.build_preferences);
        self.job_target = None;
        self.job_action.clear();
        self.failure_dismissed = false;
        self.job.describe("Manager update download", "Manager");
        let build_job = self.build_job.clone();
        self.job.spawn_background(move |job| {
            let result = manager_handoff::run(
                &job,
                &build_job,
                || match journey {
                    Some(journey) => {
                        self_update::prepare_continuation(&paths, &available, journey, &job)
                    }
                    None => self_update::prepare(&paths, &available, &job),
                },
                |plan, guard| self_update::launch_with_guard(plan, guard),
            );
            if result.is_err() {
                craft_apps_manager::manager_updates::native_admin::cancel(paths.root.clone());
            }
            let _ = tx.send(manager_handoff::display_result(&result));
            ctx.request_repaint();
            result
        });
        self.manager_message = "Downloading and verifying manager…".into();
    }
    fn check_manager_trigger(
        &mut self,
        ctx: &egui::Context,
        trigger: craft_apps_manager::manager_updates::check_state::CheckTrigger,
    ) {
        if self.manager_receiver.is_some() || self.manager_plan.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.manager_receiver = Some(rx);
        self.manager_message = "Checking for manager updates…".into();
        let paths = self.paths.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result =
                craft_apps_manager::manager_updates::coordinator::check_status(&paths, trigger)
                    .map_err(|e| format!("{e:#}"));
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
            .filter_map(|(app, version)| {
                let installed = self
                    .display_config
                    .as_ref()?
                    .apps
                    .iter()
                    .find(|copy| {
                        copy.name == app && copy.version == version && !copy.path.is_empty()
                    })?
                    .clone();
                Some(installed)
            })
            .collect();
        self.check_installations(ctx, apps);
    }
    fn check_installations(&mut self, ctx: &egui::Context, apps: Vec<model::Installed>) {
        let apps: Vec<_> = apps
            .into_iter()
            .filter(|copy| self.checking_apps.insert(copy.name.clone()))
            .collect();
        if apps.is_empty() {
            return;
        }
        for copy in &apps {
            self.remember_installation_check(ctx, copy);
            let id = installation_check_id(&self.paths, copy, self.check_generation);
            ctx.data_mut(|data| {
                data.insert_temp(
                    egui::Id::new(("active-installation-check", &self.paths.root, &copy.name)),
                    id,
                )
            });
            self.release_checks.remove(&copy.name);
        }
        let tx = self.check_sender.clone();
        let generation = self.check_generation;
        let paths = self.paths.clone();
        let preferences = self.preferences.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            for installed in apps {
                let started = std::time::Instant::now();
                let result = updates::check_installation(&paths, &installed, &preferences)
                    .map_err(|e| format!("{e:#}"));
                // Keep fast cached checks visible long enough to acknowledge the click.
                std::thread::sleep(Duration::from_millis(750).saturating_sub(started.elapsed()));
                if tx.send((generation, installed, result)).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
    }
    fn remember_installation_check(&self, ctx: &egui::Context, copy: &model::Installed) {
        let id = installation_check_id(&self.paths, copy, self.check_generation);
        if !self.display_config.as_ref().is_some_and(|config| {
            config.apps.iter().any(|selected| {
                selected.name == copy.name
                    && installation_check_id(&self.paths, selected, self.check_generation) == id
            })
        }) {
            return;
        }
        if let Some((version, result)) = self.release_checks.get(&copy.name) {
            if version == &copy.version {
                ctx.data_mut(|data| data.insert_temp(id, result.clone()));
            }
        }
    }
    fn apply_installation_selection(
        &mut self,
        selected: InstallationSelection,
        ctx: &egui::Context,
    ) {
        if let Some(previous) = self
            .display_config
            .as_ref()
            .and_then(|config| config.apps.iter().find(|copy| copy.name == selected.app))
        {
            self.remember_installation_check(ctx, previous);
        }
        self.display_config = Some(selected.config);
        self.preferences = selected.preferences;
        self.release_checks.remove(&selected.app);
        if let Some(copy) = self.display_config.as_ref().and_then(|config| {
            config
                .apps
                .iter()
                .find(|copy| copy.name == selected.app && !copy.path.is_empty())
        }) {
            let id = installation_check_id(&self.paths, copy, self.check_generation);
            let check = selected
                .check
                .or_else(|| ctx.data(|data| data.get_temp::<ReleaseCheck>(id)));
            if let Some(check) = check {
                ctx.data_mut(|data| data.insert_temp(id, check.clone()));
                self.release_checks
                    .insert(selected.app, (copy.version.clone(), check));
            }
        }
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
        self.job = Job::interactive(paths.at(log), &self.build_preferences);
        self.job.enable_identity_choices();
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
            "clean" => builder::clean(&paths),
            _ => unreachable!(),
        });
    }
    /// Builds or sets up tools for the selected app, beside any release job.
    fn start_build(&mut self, action: &str) {
        if self.manager_plan.is_some() {
            return;
        }
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
                        self.review_version_prepared(app, tag, ctx);
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
        let config = self
            .display_config
            .clone()
            .filter(|config| config.apps.iter().any(|entry| entry.name == app));
        let (tx, rx) = std::sync::mpsc::channel();
        self.install_receiver = Some(rx);
        std::thread::spawn(move || {
            let job = Job::new(paths.at("logs/install-preview.log"), &preferences);
            let result = if let Some(config) = config {
                updates::plan_install_preview(
                    &paths,
                    &job,
                    &app,
                    tag.as_deref(),
                    additional,
                    config,
                )
            } else {
                match (additional, tag.as_deref()) {
                    (true, Some(tag)) => updates::plan_additional_version(&paths, &job, &app, tag),
                    (true, None) => updates::plan_additional(&paths, &job, &app),
                    (false, Some(tag)) => updates::plan_version(&paths, &job, &app, tag),
                    (false, None) => updates::plan_app(&paths, &job, &app),
                }
            }
            .and_then(|mut plan| {
                updates::check_installer_scope_preview(&paths, &mut plan, &job)?;
                Ok(plan)
            })
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }
    fn run_install_plan(&mut self, app: String, mut plan: updates::ReleasePlan) {
        let paths = self.paths.clone();
        let parent = std::path::PathBuf::from(&self.install_parent);
        self.confirm_install = None;
        self.additional_installation = false;
        self.install_draft_app = None;
        self.install_result = None;
        self.app = app.clone();
        self.job = Job::interactive(paths.at("logs/updates.log"), &self.build_preferences);
        self.job.enable_identity_choices();
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
        self.job.spawn_background(move |job| {
            job.history(&job.log_path);
            if plan.preferences.release_format == "portable" {
                let shortcut = plan.desktop_shortcut;
                updates::choose_destination(&paths, &mut plan, &parent, shortcut)?;
            }
            updates::execute_plan(&paths, &plan, &job)
        });
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
        self.load_versions_for(&self.app.clone(), ctx);
    }
    fn load_versions_for(&mut self, app: &str, ctx: &egui::Context) {
        // A popup reopening during a refresh reuses the in-flight request and
        // current results, rather than blanking or launching another worker.
        if self.versions_app.as_deref() == Some(app) && self.versions_receiver.is_some() {
            return;
        }
        let app = app.to_owned();
        let keep = self.versions_app.as_deref() == Some(app.as_str())
            && self
                .versions_result
                .as_ref()
                .is_some_and(|result| result.is_ok());
        self.versions_app = Some(app.clone());
        if !keep {
            self.versions_result = None;
        }
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
            if self.versions_app.as_deref() != Some(&app) {
                self.versions_app = None;
                self.versions_result = None;
            }
            self.begin_custom_install(&app, Some(tag), ctx);
        } else {
            self.review_version_prepared(app, tag, ctx);
        }
    }
    fn review_version_prepared(&mut self, app: String, tag: String, ctx: &egui::Context) {
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
        if self.versions_app.as_deref() != Some(&app) {
            self.versions_app = None;
            self.versions_result = None;
        }
        // Selecting a version only prepares a plan; execution always revalidates.
        self.load_install_prepared(&app, ctx);
    }
    fn begin_release_review(&mut self) {
        let paths = self.paths.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.plan_receiver = Some(rx);
        self.job = Job::interactive(paths.at("logs/updates.log"), &self.build_preferences);
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
        if self.settings_save.is_some() || self.settings_load.is_some() {
            return;
        }
        self.discard_settings(screens::settings::edit_session::CloseRoute::Cancel);
        self.settings = true;
        let paths = self.paths.clone();
        let home = self.home.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.settings_load = Some(rx);
        std::thread::spawn(move || {
            let result =
                screens::settings::Loaded::read(&paths, &home).map_err(|e| format!("{e:#}"));
            let _ = tx.send(result);
        });
    }
    /// Reuse the current edit session; navigation must not discard pending edits.
    fn open_token_settings(&mut self) {
        if self.settings_save.is_some() {
            return;
        }
        if !self.settings {
            self.open_settings();
        }
        if self.settings {
            self.settings_token_entry = true;
            self.settings_token_focus = true;
            self.settings_jump = None;
        }
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
    fn app_install_options(
        &self,
        app: &str,
    ) -> craft_apps_manager::install_options::InstallOptions {
        let installed = self
            .display_config
            .as_ref()
            .and_then(|config| config.apps.iter().find(|copy| copy.name == app));
        craft_apps_manager::install_options::for_selection(
            &self.preferences,
            app,
            installed,
            self.additional_installation || self.confirm_install.as_deref() == Some(app),
        )
    }
    fn app_preferences(&self, app: &str) -> model::Preferences {
        craft_apps_manager::install_options::apply(
            &self.preferences,
            &self.app_install_options(app),
        )
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
            let preferred =
                craft_apps_manager::install_options::preferred_options(&self.preferences, app);
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
        let base = preferences.clone();
        preferences
            .app_install_options
            .insert(craft_apps_manager::install_options::identity(app), options);
        self.preferences =
            craft_apps_manager::settings::save_preferences_patch(&self.paths, &base, &preferences)?;
        self.download_availability.clear();
        Ok(())
    }
    fn installer_architecture_blocker(&self, app: &str) -> Option<&model::Installed> {
        self.display_config.as_ref()?.apps.iter().find(|copy| {
            copy.name == app
                && model::installer_architecture_conflict(copy, &self.app_preferences(app))
        })
    }
    fn install_available(&mut self, app: &str, ctx: &egui::Context) -> bool {
        let options = self.app_install_options(app);
        let key = (
            app.to_string(),
            options.release_format.clone(),
            options.architecture.clone(),
        );
        if let Some(Some(available)) = self.download_availability.get(&key) {
            return *available;
        }
        let preferences = craft_apps_manager::install_options::apply(&self.preferences, &options);
        let ctx = ctx.clone();
        let available =
            updates::cached_download_available_ready(&self.paths, app, &preferences, move || {
                ctx.request_repaint()
            });
        // A cold preview is temporary; don't permanently cache the initial miss.
        if available.is_some() {
            self.download_availability.insert(key, available);
        }
        available != Some(false)
    }
    fn unavailable_download_message(&self, app: &str) -> String {
        if let Some(copy) = self.installer_architecture_blocker(app) {
            return model::installer_architecture_conflict_message(
                copy,
                &self.app_preferences(app),
            );
        }
        let options = self.app_install_options(app);
        format!(
            "No {} download is available for {}.",
            options.release_format,
            model::architecture_label(&options.architecture)
        )
    }
    fn refresh_changed_installations(&mut self, config: &model::Config, ctx: &egui::Context) {
        let changed_versions: Vec<_> = config
            .apps
            .iter()
            .filter(|app| !app.path.is_empty() && !app.version.is_empty())
            .filter(|app| {
                self.display_config.as_ref().is_some_and(|old| {
                    old.apps.iter().any(|previous| {
                        previous.name == app.name
                            && installation_check_id(&self.paths, previous, self.check_generation)
                                != installation_check_id(&self.paths, app, self.check_generation)
                    })
                })
            })
            .cloned()
            .collect();
        let mut unknown_versions = Vec::new();
        for app in changed_versions {
            if let Some(previous) = self
                .display_config
                .as_ref()
                .and_then(|config| config.apps.iter().find(|copy| copy.name == app.name))
            {
                self.remember_installation_check(ctx, previous);
            }
            // A check for the previous version must not suppress checking the new one.
            self.checking_apps.remove(&app.name);
            let cache_ctx = ctx.clone();
            if let Some(result) = updates::cached_installation_check_ready(
                &self.paths,
                &app,
                &self.preferences,
                move || cache_ctx.request_repaint(),
            ) {
                self.release_checks.insert(
                    app.name.clone(),
                    (
                        app.version.clone(),
                        result.map_err(|error| format!("{error:#}")),
                    ),
                );
            } else {
                unknown_versions.push(app);
            }
        }
        if !unknown_versions.is_empty() {
            self.check_installations(ctx, unknown_versions);
        }
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

    fn catalog_changed(&mut self) {
        if let Some(tracker) = &mut self.new_apps {
            if let Err(error) = tracker.refresh(&self.paths.root, &catalog::all()) {
                self.error = Some(format!("Could not refresh new-app indicators: {error:#}"));
            }
        }
        self.download_availability.clear();
        self.reconcile_catalog_selection();
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
        self.inventory_receiver = None;
        self.config_refresh_at = std::time::Instant::now();
    }

    fn reconcile_catalog_selection(&mut self) {
        if !self.builder && catalog::get(&self.app).is_none() {
            self.page = Page::Overview;
            self.confirm_install = None;
            self.versions_app = None;
            self.launch_settings_open = false;
            self.confirm_uninstall = false;
        }
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
            self.token_draft.discard();
            self.token_reveal = false;
        }
        self.poll_settings(ctx);
        if !self.token_check_started {
            self.token_check_started = true;
            if self.token_status != craft_apps_manager::network::TokenStatus::NotConfigured {
                let (tx, rx) = std::sync::mpsc::channel();
                self.token_receiver = Some(rx);
                self.token_verification_session = self.settings_edit.as_ref().map(|s| s.id());
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
                    if std::mem::take(&mut self.token_saved_verification_pending) {
                        match status {
                            craft_apps_manager::network::TokenStatus::Invalid => self.error = Some("The active token is invalid or was rejected by GitHub. Replace it in Settings or check your environment variable.".into()),
                            craft_apps_manager::network::TokenStatus::UnableToVerify => self.error = Some("The token was saved, but GitHub could not be reached to verify it.".into()),
                            _ => {}
                        }
                    }
                    if self.settings_edit.as_ref().map(|s| s.id())
                        == self.token_verification_session
                        && self.settings
                        && !self.token_draft.has_pending()
                    {
                        self.token_message = if screens::settings::verification_allows_close(status) {
                            if status == craft_apps_manager::network::TokenStatus::NotConfigured { "Saved token removed." } else { "Token saved and verified." }
                        } else if status == craft_apps_manager::network::TokenStatus::Invalid {
                            "The active token is invalid or was rejected by GitHub. Replace it or check your environment variable."
                        } else {
                            "Token was saved, but GitHub could not be reached for verification."
                        }.into();
                    }
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
                self.inventory_receiver = None;
                self.config_refresh_at = std::time::Instant::now();
                match result {
                    Ok(selected) => {
                        self.apply_installation_selection(selected, ctx);
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
                                if !self.settings {
                                    self.settings_draft.theme = theme;
                                }
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
                // Another window/process may have refreshed release metadata.
                // Reopen the list through the shared cache rather than keeping
                // a window-local list indefinitely.
                if self.versions_receiver.is_none()
                    && records.iter().any(|record| {
                        record.lifecycle.terminal()
                            && !record.kind.starts_with("Review ")
                            && !self
                                .operation_records
                                .iter()
                                .any(|old| old.id == record.id && old.lifecycle.terminal())
                    })
                {
                    self.versions_app = None;
                    if let Some(app) = self.confirm_install.clone() {
                        if self.install_receiver.is_none()
                            && self.custom_receiver.is_none()
                            && !self.job.snapshot().busy
                        {
                            self.load_install(&app, ctx);
                        }
                    }
                }
            }
            if records.iter().any(|record| {
                is_installation_operation(record)
                    && record.lifecycle.terminal()
                    && !self
                        .operation_records
                        .iter()
                        .any(|old| old.id == record.id && old.lifecycle.terminal())
            }) {
                self.inventory_refresh_pending = true;
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
            let valid = self.custom_settings_session.is_none_or(|id| {
                self.settings
                    && self
                        .settings_edit
                        .as_ref()
                        .is_some_and(|session| session.id() == id)
                    && self.settings_save.is_none()
            });
            if !valid {
                self.custom_settings_session = None;
            }
            if valid {
                match result {
                    Ok(CustomAppAddition::Setup(setup)) => self.custom_setup = Some(setup),
                    Ok(CustomAppAddition::Added(entry)) => {
                        self.finish_custom_selection(&entry, ctx)
                    }
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
        if let Some(receiver) = &self.inventory_receiver {
            if let Some(result) =
                poll_worker_result(receiver, || "Inventory reader stopped unexpectedly.".into())
            {
                if let Ok(Some(config)) = result {
                    self.refresh_changed_installations(&config, ctx);
                    self.display_config = Some(config);
                }
                self.inventory_receiver = None;
            }
        }
        // A completed batch child has already persisted its verified installation.
        // Adopt that inventory without waiting for the remaining apps or running discovery.
        if self.inventory_refresh_pending
            && self.inventory_receiver.is_none()
            && self.config_receiver.is_none()
            && self.installation_selection.is_none()
        {
            self.inventory_refresh_pending = false;
            let (sender, receiver) = std::sync::mpsc::channel();
            self.inventory_receiver = Some(receiver);
            let paths = self.paths.clone();
            let context = ctx.clone();
            std::thread::spawn(move || {
                let _ = sender.send(completed_install_inventory(&paths));
                context.request_repaint();
            });
        }
        // Builds do not change installations, so only release jobs pause the
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
                        self.refresh_changed_installations(&snapshot.config, ctx);
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
                        self.display_rollbacks = snapshot.rollbacks;
                        self.display_sources = snapshot.sources;
                        self.display_builds = snapshot.builds;
                        self.display_build_cache = snapshot.build_cache;
                        self.display_launch = snapshot.launch;
                    }
                    Err(error) => self.error = Some(error),
                }
                self.config_receiver = None;
                self.inventory_receiver = None;
            }
        }
        if !busy
            && self.inventory_receiver.is_none()
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
                    let rollbacks = config
                        .apps
                        .iter()
                        .map(|installed| {
                            (
                                installed.name.clone(),
                                craft_apps_manager::installations::rollbacks::status(
                                    &paths, installed,
                                )
                                .map_err(|e| format!("{e:#}")),
                            )
                        })
                        .collect();
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
                        rollbacks,
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
            self.check_manager_trigger(
                ctx,
                craft_apps_manager::manager_updates::check_state::CheckTrigger::Periodic(
                    self.preferences.app_check_interval_minutes,
                ),
            );
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
            let trigger = if craft_apps_manager::manager_updates::bridge::restart_requested()
                || craft_apps_manager::manager_updates::coordinator::needs_bridge_check(&self.paths)
            {
                craft_apps_manager::manager_updates::check_state::CheckTrigger::BridgeRestart
            } else {
                craft_apps_manager::manager_updates::check_state::CheckTrigger::Startup
            };
            self.check_manager_trigger(ctx, trigger);
        }
        if let Some(receiver) = &self.manager_receiver {
            if let Some(result) = poll_worker_result(receiver, || {
                "Manager update check stopped unexpectedly.".into()
            }) {
                match result {
                    Ok(result) => {
                        self.manager_message = result.message;
                        self.manager_available = result.available;
                        if let Some(message) = result.continuation_paused {
                            self.manager_message = message;
                        }
                        if let (Some(journey), Some(available)) =
                            (result.continuation, self.manager_available.clone())
                        {
                            self.start_manager_update(ctx, available, Some(journey));
                        }
                    }
                    Err(error) => {
                        self.manager_message = format!("Could not check for updates: {error}")
                    }
                }
                self.manager_receiver = None;
            }
        }
        if let Some(result) = self
            .manager_plan
            .as_ref()
            .and_then(|receiver| manager_handoff::poll_result(&self.job, receiver))
        {
            self.manager_plan = None;
            match result {
                Err(manager_handoff::Failure::Paused) => {
                    self.manager_message = "Manager update paused while another operation is running. Check again to continue.".into();
                }
                Err(manager_handoff::Failure::Cancelled) => {
                    self.manager_message = "Manager update cancelled.".into();
                }
                Ok(()) => {
                    self.closing = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Err(manager_handoff::Failure::Failed(error)) => {
                    self.manager_message = "Manager update failed.".into();
                    self.error = Some(error);
                }
            }
        }
        while let Ok((generation, installed, result)) = self.check_receiver.try_recv() {
            self.apply_release_check(ctx, generation, installed, result);
        }
    }
    fn apply_release_check(
        &mut self,
        ctx: &egui::Context,
        generation: u64,
        installed: model::Installed,
        result: ReleaseCheck,
    ) {
        if generation == self.check_generation {
            let id = installation_check_id(&self.paths, &installed, generation);
            let active_id = egui::Id::new((
                "active-installation-check",
                &self.paths.root,
                &installed.name,
            ));
            if ctx.data(|data| data.get_temp::<egui::Id>(active_id)) == Some(id) {
                self.checking_apps.remove(&installed.name);
                ctx.data_mut(|data| data.remove::<egui::Id>(active_id));
            }
            ctx.data_mut(|data| data.insert_temp(id, result.clone()));
            let selected_matches = self
                .display_config
                .as_ref()
                .and_then(|config| config.apps.iter().find(|copy| copy.name == installed.name))
                .is_some_and(|copy| installation_check_id(&self.paths, copy, generation) == id);
            if selected_matches {
                self.download_availability
                    .retain(|(name, _, _), _| name != &installed.name);
                self.release_checks
                    .insert(installed.name, (installed.version, result));
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
        if self
            .instance
            .as_ref()
            .is_some_and(|instance| instance.take_activation())
        {
            self.show_from_tray(ctx);
        }
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
        let rollback_saving = self
            .rollback_dialog
            .as_ref()
            .is_some_and(dialogs::rollback::RollbackDialog::save_pending);
        if close_requested && (self.settings_save.is_some() || rollback_saving) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        } else if close_requested
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
        if self.closing
            && !state.busy
            && !building
            && self.settings_save.is_none()
            && !rollback_saving
        {
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
    modal_faded(ctx, title, width, 1.0, content)
}
fn modal_faded(
    ctx: &egui::Context,
    title: impl Into<String>,
    width: f32,
    opacity: f32,
    content: impl FnOnce(&mut egui::Ui),
) -> egui::ModalResponse<()> {
    let opacity = opacity.clamp(0.0, 1.0);
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
        .backdrop_color(egui::Color32::from_black_alpha(
            (150.0 * opacity).round() as u8
        ))
        .frame(
            egui::Frame::new()
                .fill(theme::palette().panel.gamma_multiply(opacity))
                .stroke(egui::Stroke::new(
                    1.0_f32,
                    theme::palette().border_strong.gamma_multiply(opacity),
                ))
                .corner_radius(egui::CornerRadius::same(12))
                .inner_margin(egui::Margin::ZERO)
                .shadow(egui::Shadow {
                    offset: [0, 12],
                    blur: 32,
                    spread: 0,
                    color: egui::Color32::from_black_alpha((110.0 * opacity).round() as u8),
                }),
        )
        .show(ctx, |ui| {
            ui.multiply_opacity(opacity);
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
    fn sidebar_install_progress_tracks_only_running_package_operations() {
        use craft_apps_manager::operations::{Lifecycle, Manager};
        let root = std::env::temp_dir().join(format!("sidebar-progress-{}", std::process::id()));
        let manager = Manager::new(&root);
        let batch = manager.register("Bulk update", "Application library", None);
        batch.start();
        let first = manager.register("Update", "filmcraft", Some(batch.id()));
        let second = manager.register("Update", "photocraft", Some(batch.id()));
        first.start();
        let records = vec![batch.snapshot(), first.snapshot(), second.snapshot()];
        assert!(sidebar_install_operation(&records, "filmcraft").is_some());
        assert!(sidebar_install_operation(&records, "photocraft").is_none());
        first.finish(Lifecycle::Completed, "Installed".into());
        second.start();
        let records = vec![batch.snapshot(), first.snapshot(), second.snapshot()];
        assert!(sidebar_install_operation(&records, "filmcraft").is_none());
        assert!(sidebar_install_operation(&records, "photocraft").is_some());
        second.finish(Lifecycle::Failed, "Failed".into());
        let check = manager.register("Check", "filmcraft", None);
        check.start();
        assert!(
            sidebar_install_operation(&[second.snapshot(), check.snapshot()], "filmcraft")
                .is_none()
        );
        assert!(sidebar_install_operation(&[second.snapshot()], "photocraft").is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn app_page_batch_progress_uses_only_the_selected_running_child() {
        use craft_apps_manager::operations::Manager;
        let root = std::env::temp_dir().join(format!("app-page-progress-{}", std::process::id()));
        let manager = Manager::new(&root);
        let child = manager.register("Update", "filmcraft", Some("batch".into()));
        child.start();
        let mut record = child.snapshot();
        record.stage = "Extracting".into();
        record.detail = "Verified package".into();
        record.progress = Some(0.35);
        let local = craft_apps_manager::jobs::State {
            busy: true,
            stage: "Batch".into(),
            progress: Some(0.8),
            ..Default::default()
        };
        let records = vec![record];
        let progress =
            app_installation_state(&records, "filmcraft", &local, Some("batch")).unwrap();
        assert!(
            app_installation_state(&records, "filmcraft", &local, Some("another-job")).is_none()
        );
        assert!(app_installation_state(&records, "filmcraft", &local, None).is_none());
        assert!(progress.busy);
        assert_eq!(progress.stage, "Extracting");
        assert_eq!(progress.detail, "Verified package");
        assert_eq!(progress.progress, Some(0.35));
        assert!(app_installation_state(&records, "photocraft", &local, Some("batch")).is_none());
        let mut done = records;
        done[0].lifecycle = craft_apps_manager::operations::Lifecycle::Completed;
        assert!(app_installation_state(&done, "filmcraft", &local, Some("batch")).is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn completed_inventory_reads_committed_versions_without_discovery() {
        let root = std::env::temp_dir().join(format!("sidebar-inventory-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let paths = Paths::new(root.clone(), None);
        assert!(completed_install_inventory(&paths).unwrap().is_none());
        let config: model::Config = serde_json::from_value(serde_json::json!({
            "appsRoot": root.to_string_lossy(),
            "apps": [{"name":"filmcraft", "version":"2.0.0", "path":"not-yet-discoverable"}]
        }))
        .unwrap();
        craft_apps_manager::files::write_json(&root.join("installations.json"), &config).unwrap();
        let before = std::fs::read(root.join("installations.json")).unwrap();
        let read = completed_install_inventory(&paths).unwrap().unwrap();
        assert_eq!(read.apps[0].version, "2.0.0");
        assert_eq!(read.apps[0].path, "not-yet-discoverable");
        assert_eq!(
            before,
            std::fs::read(root.join("installations.json")).unwrap()
        );
        let mut wrong = config;
        wrong.apps_root = root
            .join("different-library")
            .to_string_lossy()
            .into_owned();
        craft_apps_manager::files::write_json(&root.join("installations.json"), &wrong).unwrap();
        assert!(completed_install_inventory(&paths).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

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
