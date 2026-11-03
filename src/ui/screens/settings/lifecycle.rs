//! Frame-thread publication of owned worker results and unified session disposal.
use super::{
    edit_session::{self, CloseRoute, SelectionOrigin},
    save,
};
use crate::ui::App;
use craft_apps_manager::{catalog, model, network::TokenStatus, storage::config::ConfigStore};
use eframe::egui;

pub(in crate::ui) struct Loaded {
    snapshot: craft_apps_manager::storage::config::ConfigSnapshot,
    manager: model::Preferences,
    builder: model::BuilderPreferences,
    custom: Vec<catalog::Entry>,
    locations: edit_session::Locations,
    scheduler: bool,
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;

#[cfg(all(test, target_os = "windows"))]
#[path = "tray_lifecycle_tests.rs"]
mod tray_tests;
impl Loaded {
    pub(in crate::ui) fn read(
        paths: &model::Paths,
        home: &std::path::Path,
    ) -> anyhow::Result<Self> {
        let snapshot = ConfigStore::new(paths.root.clone()).load()?;
        let locator: crate::ui::Locations =
            craft_apps_manager::files::read_or_default(&home.join("data-root.json"))?;
        let resolve =
            |path: &std::path::Path| craft_apps_manager::portable_mode::resolve_folder(home, path);
        let locations = edit_session::Locations {
            root: resolve(locator.root.as_deref().unwrap_or(&paths.root))?,
            tools: resolve(locator.tools.as_deref().unwrap_or(&paths.tools))?,
        };
        Ok(Self {
            manager: snapshot.manager()?,
            builder: snapshot.builder()?,
            custom: snapshot.custom_apps()?,
            snapshot,
            locations,
            scheduler: craft_apps_manager::scheduler::enabled(false),
        })
    }
}
impl App {
    pub(in crate::ui) fn discard_settings(&mut self, route: CloseRoute) {
        if self.settings_save.is_some() {
            return;
        }
        if let Some(mut session) = self.settings_edit.take() {
            session.discard(route);
        }
        self.settings = false;
        self.settings_token_entry = false;
        self.settings_token_focus = false;
        self.token_help_open = false;
        self.settings_load = None;
        self.token_draft.discard();
        self.token_reveal = false;
        self.token_message.clear();
        self.settings_draft = self.preferences.clone();
        self.build_draft = self.build_preferences.clone();
        self.auto_draft = self.auto;
        self.root_text = self.paths.root.display().to_string();
        self.tools_text = self.paths.tools.display().to_string();
        self.custom_apps_draft.clear();
        self.custom_link.clear();
        self.custom_message.clear();
        self.remove_custom_confirm = None;
        self.visibility_open = false;
        self.visibility_draft.clear();
        if matches!(self.selection_origin, Some(SelectionOrigin::Settings(_))) {
            self.selection = false;
            self.selection_origin = None;
            self.selection_draft.clear();
        }
        if self.custom_settings_session.take().is_some() {
            self.custom_receiver = None;
            self.custom_setup = None;
            self.custom_download_choice = None;
        }
    }
    pub(in crate::ui) fn poll_settings(&mut self, ctx: &egui::Context) {
        if let Some(receiver) = &self.settings_load {
            let result = crate::ui::poll_worker_result(receiver, || {
                "Settings could not be loaded: worker stopped.".into()
            });
            if let Some(result) = result {
                self.settings_load = None;
                match result {
                    Ok(loaded) => {
                        self.root_text = loaded.locations.root.display().to_string();
                        self.tools_text = loaded.locations.tools.display().to_string();
                        let session = edit_session::SettingsEditSession::begin(
                            loaded.snapshot.document,
                            loaded.manager.clone(),
                            loaded.builder.clone(),
                            loaded.custom.clone(),
                            loaded.locations,
                            loaded.scheduler,
                        );
                        match session {
                            Ok(session) => {
                                self.preferences = loaded.manager.clone();
                                self.settings_draft = loaded.manager;
                                self.build_preferences = loaded.builder.clone();
                                self.build_draft = loaded.builder;
                                self.custom_apps_draft = loaded.custom;
                                self.auto = loaded.scheduler;
                                self.auto_draft = loaded.scheduler;
                                self.settings_edit = Some(session);
                            }
                            Err(error) => {
                                self.discard_settings(CloseRoute::Cancel);
                                self.error = Some(format!("{error:#}"));
                            }
                        }
                    }
                    Err(error) => {
                        self.discard_settings(CloseRoute::Cancel);
                        self.error = Some(error);
                    }
                }
            }
        }
        let result = self.settings_save.as_ref().and_then(|receiver| match receiver.try_recv() {
            Ok(outcome) => Some(Ok(outcome)),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err("Settings save worker stopped unexpectedly. Reopen Settings to check the saved values.")),
        });
        let Some(result) = result else {
            return;
        };
        self.settings_save = None;
        // The save worker may have removed the currently displayed custom app.
        // Reconcile navigation even when a later, independent save stage failed.
        self.reconcile_catalog_selection();
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(message) => {
                if let Some(session) = &mut self.settings_edit {
                    session.resume_after_failure();
                    if let Some(drafts) = session.drafts_mut() {
                        self.token_draft = std::mem::take(&mut drafts.token);
                    }
                }
                self.error = Some(message.into());
                return;
            }
        };
        let Some(session) = &mut self.settings_edit else {
            return;
        };
        if session.id() != outcome.session_id {
            return;
        }
        let errors: Vec<_> = [
            outcome.config.error(),
            outcome.locations.error(),
            outcome.scheduler.error(),
            outcome.token.error(),
        ]
        .into_iter()
        .flatten()
        .map(str::to_owned)
        .collect();
        let credentials_changed = outcome.token.applied();
        let mut recheck_apps = Vec::new();
        let document = match outcome.config {
            save::Stage::Applied(snapshot) => {
                match (snapshot.manager(), snapshot.builder()) {
                    (Ok(manager), Ok(builder)) => {
                        let invalidated: std::collections::BTreeSet<_> = model::apps()
                            .into_iter()
                            .filter(|app| {
                                let previous = craft_apps_manager::install_options::preferred(
                                    &self.preferences,
                                    app,
                                );
                                let next =
                                    craft_apps_manager::install_options::preferred(&manager, app);
                                previous.release_format != next.release_format
                                    || previous.architecture != next.architecture
                                    || session.custom_app_context_changed(&snapshot.document, app)
                            })
                            .collect();
                        self.release_checks.retain(|app, (_, result)| {
                            catalog::get(app).is_some()
                                && !invalidated.contains(app)
                                && (!credentials_changed || result.is_ok())
                        });
                        if !invalidated.is_empty() || credentials_changed {
                            // Generation is shared across check workers. Restart pending checks
                            // so unrelated apps are not stranded when a changed context invalidates it.
                            if let Some(config) = &self.display_config {
                                recheck_apps = config
                                    .apps
                                    .iter()
                                    .filter(|copy| {
                                        (self.checking_apps.contains(&copy.name)
                                            || invalidated.contains(&copy.name))
                                            && catalog::get(&copy.name).is_some()
                                            && !copy.path.is_empty()
                                            && !copy.version.is_empty()
                                    })
                                    .map(|copy| (copy.name.clone(), copy.version.clone()))
                                    .collect();
                            }
                            self.check_generation += 1;
                            self.checking_apps.clear();
                            self.download_availability.clear();
                        }
                        self.preferences = manager;
                        self.build_preferences = builder;
                        self.config_receiver = None;
                        self.inventory_receiver = None;
                        self.config_refresh_at = std::time::Instant::now();
                    }
                    _ => {
                        self.error =
                            Some("Saved settings could not be displayed. Reopen Settings.".into());
                    }
                }
                Some(snapshot.document)
            }
            _ => None,
        };
        let scheduler_saved = outcome.scheduler.applied();
        if let save::Stage::Applied(value) = outcome.scheduler {
            self.auto = value;
        }
        let token_saved = outcome.token.applied();
        if let save::Stage::Applied(status) = outcome.token {
            self.token_saved_verification_pending = status != TokenStatus::NotConfigured;
            self.token_status = status;
            self.token_message = if status == TokenStatus::NotConfigured {
                "Saved token removed."
            } else {
                "Token saved. Checking GitHub…"
            }
            .into();
            self.token_check_started = false;
            self.token_receiver = None;
        }
        if let Err(error) = session.acknowledge_saved(
            document,
            outcome.locations.applied(),
            scheduler_saved,
            token_saved,
        ) {
            self.error = Some(format!("{error:#}"));
        }
        session.resume_after_failure();
        if let Some(drafts) = session.drafts_mut() {
            self.token_draft = std::mem::take(&mut drafts.token);
        }
        if errors.is_empty() {
            self.discard_settings(CloseRoute::Cancel);
        } else if !errors.is_empty() {
            self.error = Some(format!("Some settings could not be saved. Successful changes were kept. Click Save to retry the remaining changes.\n\n{}", errors.join("\n\n")));
        }
        if !recheck_apps.is_empty() {
            self.check_apps(ctx, recheck_apps);
        }
        ctx.request_repaint();
    }
}
