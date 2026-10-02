//! One classification of a finished engine run for every runner (background
//! worker, desktop window, Android facade), so the job state counts runs the
//! same way everywhere (RV1, contract V4).

use crate::bisync::Outcome;

use super::job_state::{AttemptOutcome, BlockKind, Blocked, FailureKind, JobError};
use super::results::JobResult;

/// Error kind with which the engine reports a stop it decided itself (delete
/// guard) until the typed stop of V3 replaces it.
const ENGINE_STOP: &str = "abgebrochen";

/// Classifies a finished `bisync::run`. `canceled` = the runner's own cancel
/// flag was set (user, pause, worker stop, host). A stop the engine decided
/// itself is a block, never a cancellation, so it is not repeated every
/// minute. Errors count `stats.errors`, because the message list is capped.
pub fn classify_run(out: &Outcome, canceled: bool, finished: i64) -> (AttemptOutcome, JobResult) {
    let engine_stop = out.errors.iter().find(|(kind, _)| kind == ENGINE_STOP);
    let errors = out
        .stats
        .errors
        .max(u64::try_from(out.errors.len()).unwrap_or(u64::MAX));
    let status = if canceled || engine_stop.is_some() {
        "abgebrochen"
    } else if errors > 0 {
        "Fehler"
    } else if !out.conflicts.is_empty() {
        "Konflikte"
    } else {
        "ok"
    };
    let result = JobResult {
        when: finished,
        a_to_b: out.stats.a_to_b,
        b_to_a: out.stats.b_to_a,
        deleted: out.stats.deleted,
        conflicts: u64::try_from(out.conflicts.len()).unwrap_or(u64::MAX),
        errors,
        note: out.omissions.result_note(status),
    };
    let outcome = if canceled {
        AttemptOutcome::Cancelled
    } else if let Some((_, detail)) = engine_stop {
        AttemptOutcome::Blocked(Blocked {
            kind: BlockKind::Other,
            detail: detail.clone(),
            since: finished,
            confirmed: false,
        })
    } else if errors > 0 {
        AttemptOutcome::Failed(JobError {
            kind: FailureKind::Run,
            message: run_error_message(out, errors),
        })
    } else {
        AttemptOutcome::Success
    };
    (outcome, result)
}

fn run_error_message(out: &Outcome, errors: u64) -> String {
    match out.errors.first() {
        Some((path, message)) if errors == 1 => format!("{path}: {message}"),
        Some((path, message)) => format!("{errors} Fehler, zuerst {path}: {message}"),
        None => format!("{errors} Fehler"),
    }
}
