use craft_apps_manager::{builds, catalog, installations, model, storage};

#[test]
fn canonical_models_and_legacy_paths_are_the_same_types() {
    let _: Option<model::Paths> = None::<storage::paths::Paths>;
    let _: Option<model::Preferences> = None::<storage::preferences::Preferences>;
    let _: Option<model::Theme> = None::<storage::preferences::Theme>;
    let _: Option<model::BuilderPreferences> = None::<builds::preferences::BuilderPreferences>;
    let _: Option<model::Source> = None::<builds::types::Source>;
    let _: Option<model::BuildInfo> = None::<builds::types::BuildInfo>;
    let _: Option<model::Installed> = None::<installations::inventory::Installed>;
    let _: Option<model::Config> = None::<installations::inventory::Config>;
    let _: Option<model::Asset> = None::<catalog::types::Asset>;
    let _: Option<model::Release> = None::<catalog::types::Release>;
    let _: fn(&str) -> String = catalog::metadata::title;
    let _: fn(&model::Installed, &model::Preferences) -> bool =
        installations::architecture::installed_architecture_matches;
    let _: fn(&std::path::Path, &str) -> Option<std::path::PathBuf> =
        installations::executables::installed_executable;
}

#[test]
fn installation_actions_and_release_plans_keep_legacy_types() {
    use craft_apps_manager::{apps, updates};
    let _: Option<apps::LaunchSettings> = None::<installations::launch::LaunchSettings>;
    let _: Option<updates::ReleasePlan> = None::<installations::releases::ReleasePlan>;
    let _: Option<updates::ReleaseEntry> = None::<installations::releases::ReleaseEntry>;
    let _: Option<updates::ReleaseSummary> = None::<installations::releases::ReleaseSummary>;
    let _: fn(&model::Paths, &str) -> anyhow::Result<()> = installations::launch::launch;
    let _: fn(&model::Paths, &str) -> anyhow::Result<()> = installations::actions::uninstall;
}

#[test]
fn canonical_operations_keep_existing_worker_and_builder_types() {
    use craft_apps_manager::{builder, jobs, operations};
    let _: Option<jobs::Job> = None::<operations::jobs::Job>;
    let _: Option<jobs::State> = None::<operations::jobs::State>;
    let _: fn(&model::Paths, &str, bool, &jobs::Job) -> anyhow::Result<()> = builds::build;
    let _: fn(&model::Paths, &str, bool, &operations::jobs::Job) -> anyhow::Result<()> =
        builder::build;
}

#[test]
fn self_update_facades_keep_the_original_protocol_types() {
    use craft_apps_manager::{manager_updates, self_update, update_revisions};
    let _: Option<self_update::Plan> = None::<self_update::legacy::Plan>;
    let _: Option<self_update::Available> = None::<self_update::legacy::Available>;
    let _: Option<manager_updates::api::ExpectedIdentity> =
        None::<self_update::service::api::ExpectedIdentity>;
    let _: Option<manager_updates::api::UpdateStatus> =
        None::<self_update::service::api::UpdateStatus>;
    let _: Option<manager_updates::api::CheckTrigger> =
        None::<self_update::service::api::CheckTrigger>;
    let _: Option<update_revisions::Envelope> = None::<self_update::revisions::Envelope>;
    #[cfg(feature = "service-updater")]
    let _: Option<manager_updates::model::CheckOutcome> =
        None::<self_update::service::model::CheckOutcome>;
}
