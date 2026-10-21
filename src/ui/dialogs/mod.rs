//! Every modal the manager shows. Each keeps the behaviour of the dialog it replaces.
//!
//! `modal` draws the bare dialog card; the helpers here draw its bands the way the
//! mockups do: a title bar or an icon header, a padded body, and a footer with a
//! hairline above right-aligned buttons.
use super::theme::{self, btn, Icon, Kind};

use super::{modal, App};

use craft_apps_manager::{
    apps, backups as backup_store, jobs::Job, model, profiles, self_update as manager_update,
    updates,
};

use eframe::egui::{self, Color32, CornerRadius, FontId, RichText, Sense, Stroke, StrokeKind};
mod backups;
mod batch_updates;
mod builds;
mod cleanup;
mod custom_apps;
mod identity;
mod installation;
mod launch;
mod move_installation;
mod notices;
mod paths;
mod self_update;
mod shared;
mod uninstall;
pub(super) use paths::DestinationPreview;
use paths::*;
pub(super) use shared::*;

enum DialogFlow {
    Continue,
    Stop,
}

impl App {
    pub(super) fn dialogs(&mut self, ctx: &egui::Context) {
        if matches!(self.render_identity_choice(ctx), DialogFlow::Stop) {
            return;
        }
        if self.confirm_install.is_none() {
            self.additional_installation = false;
            self.install_selected_tag = None;
        }
        self.custom_setup_dialog(ctx);
        self.render_custom_choices(ctx);
        let busy = self.job.snapshot().busy;
        self.render_build_dialogs(ctx, busy);
        self.render_release_review(ctx);
        self.render_move_dialog(ctx);
        self.render_install_dialog(ctx);
        self.render_launch_dialogs(ctx);
        self.render_uninstall_dialog(ctx);
        self.render_batch_selection(ctx);
        self.render_restore_dialog(ctx, busy);
        self.render_self_update_dialog(ctx);
        self.render_cleanup_dialog(ctx);
        self.render_notices(ctx);
    }
}
