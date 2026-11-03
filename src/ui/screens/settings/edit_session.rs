//! Pure Settings draft lifecycle. Persistence and immediate maintenance actions live elsewhere.
use super::token_draft::{TokenChange, TokenDraft};
use anyhow::{bail, Result};
use craft_apps_manager::{
    builds::preferences::BuilderPreferences,
    catalog::Entry,
    storage::{
        config::{ConfigPatch, FieldPatch},
        preferences::Preferences,
    },
};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionOrigin {
    Settings(u64),
    ReleaseReview,
}

#[derive(Clone, Copy, Debug)]
pub enum CloseRoute {
    Cancel,
    TitleBar,
    Escape,
    Outside,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Locations {
    pub root: PathBuf,
    pub tools: PathBuf,
}

/// Intentionally neither Debug nor serializable: contains an in-memory secret.
pub struct SettingsDrafts {
    pub manager: Preferences,
    pub builder: BuilderPreferences,
    pub custom_apps: Vec<Entry>,
    pub locations: Locations,
    pub scheduler_enabled: bool,
    pub token: TokenDraft,
}

/// Immutable data to hand to the save worker. Secrets are kept outside config.
pub struct EditSnapshot {
    pub session_id: u64,
    pub config: ConfigPatch,
    pub locations: Option<(Locations, Locations)>,
    pub scheduler: Option<(bool, bool)>,
    pub token: Option<TokenChange>,
}

pub struct SettingsEditSession {
    id: u64,
    base_document: Value,
    base_projection: Value,
    base_locations: Locations,
    base_scheduler: bool,
    drafts: Option<SettingsDrafts>,
    saving: bool,
}

impl SettingsEditSession {
    pub fn begin(
        base_document: Value,
        manager: Preferences,
        builder: BuilderPreferences,
        custom_apps: Vec<Entry>,
        locations: Locations,
        scheduler_enabled: bool,
    ) -> Result<Self> {
        let base_projection = projection(&manager, &builder, &custom_apps)?;
        Ok(Self {
            id: NEXT_SESSION.fetch_add(1, Ordering::Relaxed),
            base_document,
            base_projection,
            base_locations: locations.clone(),
            base_scheduler: scheduler_enabled,
            drafts: Some(SettingsDrafts {
                manager,
                builder,
                custom_apps,
                locations,
                scheduler_enabled,
                token: TokenDraft::default(),
            }),
            saving: false,
        })
    }

    /// Compare the repository/package definition, not unrelated preferences.
    pub fn custom_app_context_changed(&self, document: &Value, app: &str) -> bool {
        let entry = |document: &Value| -> Option<Value> {
            document
                .get("custom_apps")?
                .as_array()?
                .iter()
                .find(|entry| entry.get("key").and_then(Value::as_str) == Some(app))
                .cloned()
        };
        entry(&self.base_document) != entry(document)
    }

    pub fn id(&self) -> u64 {
        self.id
    }
    pub fn drafts(&self) -> Option<&SettingsDrafts> {
        self.drafts.as_ref()
    }
    pub fn drafts_mut(&mut self) -> Option<&mut SettingsDrafts> {
        if self.saving {
            None
        } else {
            self.drafts.as_mut()
        }
    }

    /// All close routes share the same invalidation and secret disposal.
    pub fn discard(&mut self, _route: CloseRoute) {
        if let Some(draft) = &mut self.drafts {
            draft.token.discard();
        }
        self.drafts = None;
        self.saving = false;
    }

    pub fn accept_nested_edit(
        &mut self,
        origin: SelectionOrigin,
        edit: impl FnOnce(&mut SettingsDrafts),
    ) -> bool {
        if origin != SelectionOrigin::Settings(self.id) {
            return false;
        }
        let Some(drafts) = self.drafts_mut() else {
            return false;
        };
        edit(drafts);
        true
    }

    pub fn accept_custom_result(&mut self, session_id: u64, registrations: Vec<Entry>) -> bool {
        self.accept_nested_edit(SelectionOrigin::Settings(session_id), |d| {
            d.custom_apps = registrations
        })
    }

    pub fn snapshot_for_save(&self) -> Result<EditSnapshot> {
        let Some(drafts) = &self.drafts else {
            bail!("Settings session is closed");
        };
        let current = projection(&drafts.manager, &drafts.builder, &drafts.custom_apps)?;
        let mut fields = Vec::new();
        dirty_fields(
            &self.base_projection,
            &current,
            &self.base_document,
            &mut Vec::new(),
            &mut fields,
        );
        Ok(EditSnapshot {
            session_id: self.id,
            config: ConfigPatch { fields },
            locations: (self.base_locations != drafts.locations)
                .then(|| (self.base_locations.clone(), drafts.locations.clone())),
            scheduler: (self.base_scheduler != drafts.scheduler_enabled)
                .then_some((self.base_scheduler, drafts.scheduler_enabled)),
            token: drafts.token.change(),
        })
    }

    pub fn freeze_for_save(&mut self) -> Result<EditSnapshot> {
        if self.saving {
            bail!("Settings save is already in progress");
        }
        let snapshot = self.snapshot_for_save()?;
        self.saving = true;
        Ok(snapshot)
    }

    pub fn resume_after_failure(&mut self) {
        self.saving = false;
    }

    /// Clear only successful stages before exposing a partial failure for retry.
    /// The config document must be the authoritative committed worker result.
    pub fn acknowledge_saved(
        &mut self,
        config_document: Option<Value>,
        locations_saved: bool,
        scheduler_saved: bool,
        token_saved: bool,
    ) -> Result<()> {
        if !self.saving {
            bail!("Settings save is not in progress");
        }
        let Some(drafts) = &mut self.drafts else {
            bail!("Settings session is closed");
        };
        if let Some(document) = config_document {
            self.base_projection =
                projection(&drafts.manager, &drafts.builder, &drafts.custom_apps)?;
            self.base_document = document;
        }
        if locations_saved {
            self.base_locations = drafts.locations.clone();
        }
        if scheduler_saved {
            self.base_scheduler = drafts.scheduler_enabled;
        }
        if token_saved {
            drafts.token.discard();
        }
        Ok(())
    }
}

fn projection(
    manager: &Preferences,
    builder: &BuilderPreferences,
    custom_apps: &[Entry],
) -> Result<Value> {
    Ok(
        serde_json::json!({ "manager": serde_json::to_value(manager)?, "builder": serde_json::to_value(builder)?, "custom_apps": serde_json::to_value(custom_apps)? }),
    )
}

fn at_path<'a>(root: &'a Value, path: &[String]) -> Option<&'a Value> {
    path.iter()
        .try_fold(root, |value, key| value.as_object()?.get(key))
}

/// Compare typed projections to avoid treating defaulted/unknown properties as edits.
/// Use the raw snapshot for before-values so absent defaults merge correctly.
fn dirty_fields(
    base: &Value,
    current: &Value,
    raw: &Value,
    path: &mut Vec<String>,
    out: &mut Vec<FieldPatch>,
) {
    if base == current {
        return;
    }
    if let (Some(before), Some(after)) = (base.as_object(), current.as_object()) {
        let keys: BTreeSet<_> = before.keys().chain(after.keys()).collect();
        for key in keys {
            path.push(key.clone());
            match (before.get(key), after.get(key)) {
                (Some(b), Some(a)) => dirty_fields(b, a, raw, path, out),
                (_, after) => out.push(FieldPatch {
                    path: path.clone(),
                    before: at_path(raw, path).cloned(),
                    after: after.cloned(),
                }),
            }
            path.pop();
        }
    } else {
        out.push(FieldPatch {
            path: path.clone(),
            before: at_path(raw, path).cloned(),
            after: Some(current.clone()),
        });
    }
}

#[cfg(test)]
#[path = "edit_session_tests.rs"]
mod tests;
