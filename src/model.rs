//! Compatibility paths for the feature-owned application records and helpers.
pub use crate::builds::preferences::BuilderPreferences;
pub use crate::builds::types::{BuildInfo, Source};
pub use crate::catalog::metadata::{
    apps, category, description, ordered_release_apps, repository, repository_name, sources, title,
    valid_app,
};
pub use crate::catalog::types::{Asset, Release};
pub use crate::installations::architecture::{
    architecture_label, installed_architecture_matches, installer_architecture_conflict,
    installer_architecture_conflict_message, release_arch, release_os, MANAGER_ARCH,
};
pub use crate::installations::executables::{
    build_executable_name, executable_name, executable_names, installed_executable, is_executable,
};
pub use crate::installations::inventory::{Config, Installed};
pub use crate::storage::paths::Paths;
pub use crate::storage::preferences::{Preferences, Theme};
