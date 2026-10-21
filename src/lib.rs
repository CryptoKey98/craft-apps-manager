pub mod builds;
pub mod catalog;
pub mod installations;
pub mod model;
pub mod network;
pub mod operations;
pub mod platform;
pub mod scheduling;
pub mod self_update;
pub mod storage;

// Compatibility exports retain existing source paths. Implementations live in the feature modules above.
pub use installations::identity as app_identity;
pub mod apps {
    pub use crate::installations::actions::*;
    pub use crate::installations::launch::*;
}
pub use builds as builder;
pub use builds::profiles;
pub use builds::toolchains as tools;
pub use catalog::discovery;
pub use installations::backups;
pub(crate) use installations::cleanup as installer_cleanup;
pub use installations::packages::native as installers;
pub use installations::packages::selection as downloads;
pub use installations::portable;
pub use installations::releases as updates;
pub use installations::relocation;
pub use operations::jobs;
pub use scheduling::checks as hourly;
pub use scheduling::system as scheduler;
pub use self_update::revisions as update_revisions;
pub use self_update::service as manager_updates;
pub use storage::files;
pub use storage::portable as portable_mode;
pub use storage::settings;

pub use installations::labels as installation_labels;
pub use installations::options as install_options;
#[cfg(any(target_os = "windows", target_os = "linux"))]
pub use operations::elevation as batch_admin;
