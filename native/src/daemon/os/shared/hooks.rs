//! Before, after and cleanup commands of a sync job (RV1, contract V4), one
//! implementation for the background worker and for manual runs (desktop
//! window, Android facade). Contract stage: the commands run as before
//! (`cmd /C` or `sh -c`, without cancellation); the cleanup command needs the
//! job field requested from K3 and is skipped until then.

use std::sync::atomic::AtomicBool;

use crate::syncjobs::{AttemptOutcome, SyncJob};

/// Which command of a job runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookPhase {
    /// `run_before`: before any side is opened; a failure fails the attempt
    /// (`FailureKind::Hook`).
    Before,
    /// `run_after`: after a finished run, also with conflicts, errors or a
    /// block; not after a cancellation.
    After,
    /// The cleanup command: after a cancellation or a run that could not
    /// start, once the before command succeeded (undo it).
    Cleanup,
}

/// Runs the job's command for `phase`; a job without that command is `Ok`.
/// The command learns the job and, after a run, its outcome through
/// environment variables; it is ended when `cancel` is set. `Err` carries a
/// German reason for the job line.
pub fn run_job_hook(
    job: &SyncJob,
    phase: HookPhase,
    outcome: Option<&AttemptOutcome>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let _ = (outcome, cancel);
    let command = match phase {
        HookPhase::Before => job.run_before.trim(),
        HookPhase::After => job.run_after.trim(),
        HookPhase::Cleanup => return Ok(()),
    };
    if command.is_empty() {
        return Ok(());
    }
    super::job::run_cmd(command)
}
