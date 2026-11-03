//! Worker-side handoff of a prepared manager update.

use anyhow::Result;
use craft_apps_manager::{jobs::Job, manager_updates::bridge};
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Failure {
    Cancelled,
    Paused,
    Failed(String),
}

#[derive(Debug)]
struct Paused;
impl std::fmt::Display for Paused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            "Manager update paused while another operation is running. Check again to continue.",
        )
    }
}
impl std::error::Error for Paused {}

/// Runs inside the existing update job, keeping it busy through helper startup.
/// The backend invokes the guard after all validation, immediately before spawn.
pub(super) fn run(
    job: &Job,
    build_job: &Job,
    prepare: impl FnOnce() -> Result<PathBuf>,
    launch: impl FnOnce(&Path, &dyn Fn() -> Result<()>) -> Result<()>,
) -> Result<()> {
    job.check()?;
    let plan = prepare()?;
    job.stage(
        "Preparing restart",
        None,
        "Verifying manager and update helper",
    );
    launch(&plan, &|| {
        job.check()?;
        if build_job.snapshot().busy
            || bridge::foreground_busy(&job.operations.list(), job.operation_id().as_deref())
        {
            return Err(Paused.into());
        }
        // A cancellation can arrive while reading the shared operation records.
        job.check()
    })
}

pub(super) fn display_result(result: &Result<()>) -> Result<(), Failure> {
    match result {
        Ok(()) => Ok(()),
        Err(error) if craft_apps_manager::jobs::is_cancelled(error) => Err(Failure::Cancelled),
        Err(error) if error.is::<Paused>() => Err(Failure::Paused),
        Err(error) => Err(Failure::Failed(format!("{error:#}"))),
    }
}

/// A result can arrive before the existing Job persists its terminal record.
/// Keep polling without blocking, and close the UI only after that job settles.
pub(super) fn poll_result(
    job: &Job,
    receiver: &std::sync::mpsc::Receiver<Result<(), Failure>>,
) -> Option<Result<(), Failure>> {
    if job.snapshot().busy {
        return None;
    }
    super::poll_worker_result(receiver, || {
        Failure::Failed("Manager update worker stopped unexpectedly.".into())
    })
}

#[cfg(test)]
#[path = "manager_handoff_tests.rs"]
mod tests;
