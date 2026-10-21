use crate::installations::backups::backup_path;
use crate::{
    backups, files,
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
mod selection;
pub use execution::*;
pub use planning::*;
pub use recovery::*;
pub use selection::*;

#[cfg(test)]
#[path = "planning_tests.rs"]
mod planning_tests;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
