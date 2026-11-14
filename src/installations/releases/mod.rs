use crate::{
    files,
    jobs::Job,
    model::{Asset, Paths, Preferences, Release},
    network::Network,
    platform,
};
use anyhow::{bail, Context, Result};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub use crate::builds::sources::{remove_source, sources};

mod execution;
mod planning;
mod recovery;
mod scope;
mod staging;
pub use scope::{check_installer_scope_preview, downgrade_scope_decision, downgrade_scope_error};
pub(crate) use staging::StagedPackages;
mod selection;
mod validation;
pub use execution::*;
pub use planning::*;
pub use recovery::*;
pub use selection::*;
mod check_batch;
pub use check_batch::check_installation_batch;

#[cfg(test)]
#[path = "planning_tests.rs"]
mod planning_tests;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
