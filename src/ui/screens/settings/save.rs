//! Settings side effects run on a worker; no backend method is called by drawing code.
use super::{edit_session::EditSnapshot, token_draft::TokenChange};
use craft_apps_manager::{model::Paths, network, storage::config::ConfigSnapshot};
use std::{path::PathBuf, sync::mpsc};

pub struct SaveRequest {
    pub paths: Paths,
    pub home: PathBuf,
    pub edit: EditSnapshot,
}

pub enum Stage<T> {
    NotRequested,
    Applied(T),
    Failed(String),
}

impl<T> Stage<T> {
    pub fn applied(&self) -> bool {
        matches!(self, Self::Applied(_))
    }
    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Failed(message) => Some(message),
            _ => None,
        }
    }
}

pub struct SaveOutcome {
    pub session_id: u64,
    pub config: Stage<ConfigSnapshot>,
    pub locations: Stage<()>,
    pub scheduler: Stage<bool>,
    pub token: Stage<network::TokenStatus>,
}

pub trait Backend: Send + 'static {
    fn validate(&mut self, request: &SaveRequest) -> anyhow::Result<()>;
    fn config(&mut self, request: &SaveRequest) -> anyhow::Result<ConfigSnapshot>;
    fn locations(&mut self, request: &SaveRequest) -> anyhow::Result<()>;
    fn scheduler(&mut self, request: &SaveRequest, value: bool) -> anyhow::Result<()>;
    fn token(&mut self, change: &TokenChange) -> anyhow::Result<network::TokenStatus>;
}

pub fn spawn(
    request: SaveRequest,
    mut backend: impl Backend,
    ctx: eframe::egui::Context,
) -> mpsc::Receiver<SaveOutcome> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let outcome = execute(&request, &mut backend);
        let _ = tx.send(outcome);
        ctx.request_repaint();
    });
    rx
}

fn execute(request: &SaveRequest, backend: &mut impl Backend) -> SaveOutcome {
    let mut outcome = SaveOutcome {
        session_id: request.edit.session_id,
        config: Stage::NotRequested,
        locations: Stage::NotRequested,
        scheduler: Stage::NotRequested,
        token: Stage::NotRequested,
    };
    if let Err(error) = backend.validate(request) {
        outcome.config = Stage::Failed(format!("{error:#}"));
        return outcome;
    }
    match backend.config(request) {
        Ok(snapshot) => outcome.config = Stage::Applied(snapshot),
        Err(error) => {
            outcome.config = Stage::Failed(format!("{error:#}"));
            return outcome;
        }
    }
    if request.edit.locations.is_some() {
        match backend.locations(request) {
            Ok(()) => outcome.locations = Stage::Applied(()),
            Err(error) => {
                outcome.locations = Stage::Failed(format!("{error:#}"));
                return outcome;
            }
        }
    }
    if let Some((_, value)) = request.edit.scheduler {
        outcome.scheduler = match backend.scheduler(request, value) {
            Ok(()) => Stage::Applied(value),
            Err(error) => Stage::Failed(format!("{error:#}")),
        };
    }
    if let Some(change) = &request.edit.token {
        outcome.token = match backend.token(change) {
            Ok(status) => Stage::Applied(status),
            Err(error) => Stage::Failed(format!("{error:#}")),
        };
    }
    outcome
}

pub struct SystemBackend;

impl Backend for SystemBackend {
    fn validate(&mut self, request: &SaveRequest) -> anyhow::Result<()> {
        if let Some((_, desired)) = &request.edit.locations {
            for path in [&desired.root, &desired.tools] {
                let path = craft_apps_manager::portable_mode::resolve_folder(&request.home, path)?;
                anyhow::ensure!(path.is_absolute(), "Folder paths must be absolute");
                anyhow::ensure!(
                    !path.exists() || path.is_dir(),
                    "Folder path is not a directory: {}",
                    path.display()
                );
            }
        }
        if let Some(TokenChange::Set(token)) = &request.edit.token {
            anyhow::ensure!(
                !token.trim().is_empty() && !token.chars().any(char::is_whitespace),
                "Enter a token without spaces or line breaks."
            );
        }
        Ok(())
    }

    fn config(&mut self, request: &SaveRequest) -> anyhow::Result<ConfigSnapshot> {
        use craft_apps_manager::storage::config::ConfigStore;
        let store = ConfigStore::new(request.paths.root.clone());
        store.commit(request.edit.config.clone())?;
        craft_apps_manager::catalog::load(&request.paths.root);
        // Adopt the newly saved catalog before publishing the ordered UI lists.
        // This normalizes ordering without selecting new apps for updates.
        request.paths.preferences()?;
        store.load()
    }

    fn locations(&mut self, request: &SaveRequest) -> anyhow::Result<()> {
        let Some((base, desired)) = &request.edit.locations else {
            return Ok(());
        };
        let _lock = craft_apps_manager::platform::Lock::take("Local\\CraftAppsManager")?;
        let file = request.home.join("data-root.json");
        if std::fs::symlink_metadata(&file).is_ok() {
            anyhow::ensure!(
                !craft_apps_manager::files::linked(&file)?,
                "Linked folder-location file"
            );
        }
        let current: crate::ui::Locations = craft_apps_manager::files::read_or_default(&file)?;
        let resolve = |path: &std::path::Path| {
            craft_apps_manager::portable_mode::resolve_folder(&request.home, path)
        };
        let current_root = resolve(current.root.as_deref().unwrap_or(&request.paths.root))?;
        let current_tools = resolve(current.tools.as_deref().unwrap_or(&request.paths.tools))?;
        let desired_root = resolve(&desired.root)?;
        let desired_tools = resolve(&desired.tools)?;
        anyhow::ensure!(
            (current_root == resolve(&base.root)? && current_tools == resolve(&base.tools)?)
                || (current_root == desired_root && current_tools == desired_tools),
            "Folder locations changed in another window. Reopen Settings before changing them."
        );
        craft_apps_manager::files::write_json(
            &file,
            &crate::ui::Locations {
                root: Some(desired_root),
                tools: Some(desired_tools),
            },
        )
    }

    fn scheduler(&mut self, request: &SaveRequest, value: bool) -> anyhow::Result<()> {
        craft_apps_manager::scheduler::set(&request.paths, false, value)
    }

    fn token(&mut self, change: &TokenChange) -> anyhow::Result<network::TokenStatus> {
        match change {
            TokenChange::Set(value) => network::save_token(value)?,
            TokenChange::Remove => network::remove_saved_token()?,
        }
        // HTTP verification is deliberately not part of the durable save barrier.
        Ok(if network::token_source().is_some() {
            network::TokenStatus::NotVerified
        } else {
            network::TokenStatus::NotConfigured
        })
    }
}

#[cfg(test)]
#[path = "save_tests.rs"]
mod tests;
