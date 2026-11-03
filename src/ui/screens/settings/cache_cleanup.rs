//! Immediate maintenance, independent of the editable Settings draft.
use super::*;
use std::sync::mpsc;

#[derive(Default)]
pub(in crate::ui) struct State {
    pub(super) confirm: bool,
    receiver: Option<mpsc::Receiver<Result<(), String>>>,
    message: String,
}

impl App {
    fn api_cache_cleanup_blocked(&self) -> bool {
        self.release_pending()
            || self.job.snapshot().busy
            || self.build_job.snapshot().busy
            || self.settings_save.is_some()
            || self.manager_plan.is_some()
            || self.confirm_clean
            || self.api_cache_cleanup.receiver.is_some()
            || self
                .operation_records
                .iter()
                .any(|operation| !operation.lifecycle.terminal())
    }

    pub(super) fn api_cache_cleanup_row(&mut self, ui: &mut egui::Ui) {
        let blocked = self.api_cache_cleanup_blocked();
        let clearing = self.api_cache_cleanup.receiver.is_some();
        let status = if clearing {
            "Clearing cache…".to_owned()
        } else if !self.api_cache_cleanup.message.is_empty() {
            self.api_cache_cleanup.message.clone()
        } else {
            String::new()
        };
        let cleanup = "Unused cache is cleared automatically after 30 days.";
        let detail = if status.is_empty() {
            format!("Saved app details and download options. {cleanup}")
        } else {
            format!("{status} {cleanup}")
        };
        group(ui, "GitHub Cache", |ui| {
            if row(ui, "Cached GitHub data", Some(&detail), 64.0, |ui| {
                if clearing {
                    ui.spinner();
                }
                btn(if clearing {
                    "Clearing…"
                } else {
                    "Clear cache"
                })
                .enabled(!blocked)
                .show(ui)
                .on_disabled_hover_text("Wait for active operations and maintenance to finish.")
                .on_hover_text("Removes cached GitHub data. Your apps and settings are kept.")
                .clicked()
            }) {
                self.api_cache_cleanup.confirm = true;
            }
        });
    }

    pub(super) fn poll_api_cache_cleanup(&mut self, ctx: &egui::Context) {
        let Some(receiver) = &self.api_cache_cleanup.receiver else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("Cache cleanup worker stopped unexpectedly.".into()))
            }
            Err(mpsc::TryRecvError::Empty) => None,
        };
        if let Some(result) = result {
            self.api_cache_cleanup.receiver = None;
            self.api_cache_cleanup.message = match result {
                Ok(()) => "Cache cleared.".into(),
                Err(error) => format!("Could not clear cache: {error}"),
            };
        } else {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    pub(super) fn api_cache_cleanup_dialog(&mut self, ctx: &egui::Context) {
        if !self.api_cache_cleanup.confirm {
            return;
        }
        let blocked = self.api_cache_cleanup_blocked();
        let dialog = modal(ctx, "Clear cache?", 440.0, |ui| {
            dialogs::title_bar(ui, "Clear cache?", |_| {});
            dialogs::band(ui, egui::Margin::symmetric(16, 16), 0.0, |ui| {
                ui.label("Clear cached GitHub data? It downloads again when needed. Your apps, settings, and GitHub token are kept.");
            });
            dialogs::footer(ui, |ui| {
                if dialogs::action(ui, "Clear cache", Kind::Danger, !blocked).clicked() {
                    self.api_cache_cleanup.confirm = false;
                    self.api_cache_cleanup.message.clear();
                    let root = self.paths.root.clone();
                    let (sender, receiver) = mpsc::channel();
                    self.api_cache_cleanup.receiver = Some(receiver);
                    let ctx = ctx.clone();
                    std::thread::spawn(move || {
                        let result = if craft_apps_manager::operations::Manager::new(&root)
                            .list()
                            .iter()
                            .any(|operation| !operation.lifecycle.terminal())
                        {
                            Err("Wait for active operations to finish.".into())
                        } else {
                            craft_apps_manager::network::clear_api_cache(&root)
                                .and_then(|()|craft_apps_manager::installations::release_catalog::store::clear(&root))
                                .map_err(|error| format!("{error:#}"))
                        };
                        let _ = sender.send(result);
                        ctx.request_repaint();
                    });
                }
                if dialogs::action(ui, "Cancel", Kind::Secondary, true).clicked() {
                    self.api_cache_cleanup.confirm = false;
                }
            });
        });
        if dialog.should_close() {
            self.api_cache_cleanup.confirm = false;
        }
    }
}
