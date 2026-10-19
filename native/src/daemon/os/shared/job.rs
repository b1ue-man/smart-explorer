//! One daemon attempt, from preparation to its persisted outcome.
use crate::syncjobs::{
    AttemptOutcome, AttemptReport, FailureKind, JobError, JobResult, RunCause, RunMark, Runner,
    SyncJob,
};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;

use super::hooks::{run_job_hook, HookPhase};
use super::state::{log, now_secs};

pub(crate) fn run_one(job: &SyncJob, cancel: &AtomicBool) {
    let _ = run_for(
        job,
        cancel,
        RunCause::Other,
        Arc::new(AtomicI64::new(now_secs())),
        now_secs(),
    );
}

pub(super) fn run_for(
    job: &SyncJob,
    cancel: &AtomicBool,
    cause: RunCause,
    progress: Arc<AtomicI64>,
    started: i64,
) -> AttemptOutcome {
    let _awake = crate::keep_awake::hold(crate::keep_awake::Reason::SyncRun);
    let mut admitted = false;
    let stored = crate::syncjobs::update_job_state(&job.id, |state| {
        if state
            .running_now(started)
            .is_some_and(|mark| mark.runner != Runner::Daemon)
        {
            return;
        }
        admitted = true;
        state.running = Some(RunMark {
            runner: Runner::Daemon,
            cause,
            started,
            alive: started,
            stalled_since: None,
        });
        state.last_attempt = Some(started);
        state.last_runner = Some(Runner::Daemon);
        state.last_cause = Some(cause);
    });
    let state = match stored {
        Ok(state) => state,
        Err(error) => {
            log(&format!(
                "job '{}' not started: state cannot be stored: {error}",
                job.id
            ));
            return AttemptOutcome::Failed(JobError {
                kind: FailureKind::Internal,
                message: error.to_string(),
            });
        }
    };
    if !admitted {
        return AttemptOutcome::Cancelled;
    }
    crate::bisync::job_log_line(
        &job.id,
        "Auslöser",
        &format!("{cause:?}: Hintergrunddienst startet den Lauf"),
    );
    let cursor = super::job_triggers::host_cursor(job);
    let (mut outcome, result) = attempt(job, &state, cancel, cause, progress);
    if !persist(job, started, cause, outcome.clone(), result) {
        outcome = AttemptOutcome::Failed(JobError { kind: FailureKind::Internal,
            message: "Ergebnis konnte nicht sicher gespeichert werden; erledigte Dateien bleiben in der Sync-Basis.".into() });
    }
    if outcome == AttemptOutcome::Success {
        super::own_writes::succeeded(&job.id, started);
    }
    if outcome == AttemptOutcome::Success
        && matches!(
            cause,
            RunCause::Verify | RunCause::Startup | RunCause::CatchUp
        )
    {
        if let Err(error) = crate::syncjobs::update_job_state(&job.id, |state| {
            state.last_verify = Some(now_secs());
            state.verify_cursor = cursor;
        }) {
            log(&format!("verification state for {}: {error}", job.id));
        }
    }
    outcome
}

fn attempt(
    job: &SyncJob,
    state: &crate::syncjobs::JobState,
    cancel: &AtomicBool,
    cause: RunCause,
    progress: Arc<AtomicI64>,
) -> (AttemptOutcome, JobResult) {
    if (super::platform::requires_storage_access(&job.source)
        || super::platform::requires_storage_access(&job.target))
        && super::host_state::storage_access() != Some(true)
    {
        return failed(
            FailureKind::Access,
            "Dateizugriff fehlt: Zugriff auf alle Dateien erlauben.".into(),
        );
    }
    if cancel.load(Ordering::Acquire) {
        return cancelled();
    }
    let volume = state
        .pending_trigger
        .as_ref()
        .filter(|pending| pending.kind == crate::syncjobs::PendingKind::Connect)
        .and_then(|pending| pending.volume.as_deref())
        .or_else(|| {
            (job.trigger == crate::syncjobs::Trigger::OnConnect
                && matches!(
                    cause,
                    RunCause::Connect | RunCause::Retry | RunCause::Confirmed
                ))
            .then(|| state.last_connect.as_ref().map(|mark| mark.volume.as_str()))
            .flatten()
        });
    if let Some(volume) = volume {
        if !super::connect_triggers::still_present(job, volume) {
            return failed(
                FailureKind::Unreachable,
                "Das auslösende Laufwerk ist nicht mehr am gespeicherten Ort angeschlossen.".into(),
            );
        }
    }
    if job.is_vault() {
        return vault_attempt(job, state, cancel, cause, &crate::vault::run_vault_job);
    }
    let mut prepared = match PreparedJob::new(job, now_secs()) {
        Ok(prepared) => prepared,
        Err(error) => return failed(FailureKind::Config, error),
    };
    if let Err(error) = run_job_hook(job, HookPhase::Before, None, cancel) {
        let result = if cancel.load(Ordering::Acquire) {
            cancelled()
        } else {
            failed(FailureKind::Hook, error)
        };
        return cleanup(job, result, cancel);
    }
    let (a, root_a) = match crate::connect::resolve_endpoint(&job.source) {
        Ok(value) => value,
        Err(error) => {
            return cleanup(
                job,
                failed(
                    super::job_triggers::connect_failure(&error),
                    format!("Quelle: {error}"),
                ),
                cancel,
            )
        }
    };
    if cancel.load(Ordering::Acquire) {
        return cleanup(job, cancelled(), cancel);
    }
    let (b, root_b) = match crate::connect::resolve_endpoint(&job.target) {
        Ok(value) => value,
        Err(error) => {
            return cleanup(
                job,
                failed(
                    super::job_triggers::connect_failure(&error),
                    format!("Ziel: {error}"),
                ),
                cancel,
            )
        }
    };
    if cancel.load(Ordering::Acquire) {
        return cleanup(job, cancelled(), cancel);
    }
    // Syntax was checked before hooks. Actual matching uses the same pair
    // case policy as the engine, including a case-insensitive remote target.
    prepared.ignore = match job
        .checked_glob_set_for(crate::bisync::pair_key_policy(&*a, &root_a, &*b, &root_b).fold_case)
    {
        Ok(ignore) => ignore,
        Err(error) => return cleanup(job, failed(FailureKind::Config, error), cancel),
    };
    let (min_size, max_size, after, before) = prepared.bounds;
    let filter = crate::bisync::WalkFilter {
        include_hidden: job.include_hidden,
        ignore: &prepared.ignore,
        min_size,
        max_size,
        after_mtime_ms: after,
        before_mtime_ms: before,
    };
    let settings = match run_settings(job, state, cause) {
        Ok(settings) => settings,
        Err(error) => return cleanup(job, failed(FailureKind::Internal, error), cancel),
    };
    super::own_writes::forget(&job.id);
    let observer = super::own_writes::Observer {
        job_id: job.id.clone(),
        a: a.clone(),
        b: b.clone(),
        root_a: root_a.clone(),
        root_b: root_b.clone(),
        source: job.source.clone(),
        target: job.target.clone(),
        generation: state.last_attempt.unwrap_or(0),
        progress,
    };
    let out = crate::bisync::run_with(crate::bisync::RunRequest {
        a: &*a,
        root_a: &root_a,
        b: &*b,
        root_b: &root_b,
        opts: prepared.opts,
        filter: &filter,
        cancel,
        settings,
        observer: Some(&observer),
    });
    let result = crate::syncjobs::classify_run(&out, cancel.load(Ordering::Acquire), now_secs());
    log(&format!(
        "ran '{}' [{}]: {}→ {}← {}del {}conf {}err: {}",
        job.name,
        job.trigger.as_str(),
        result.1.a_to_b,
        result.1.b_to_a,
        result.1.deleted,
        result.1.conflicts,
        result.1.errors,
        result.1.note
    ));
    if let Some(summary) = out.omissions.summary() {
        log(&format!(
            "sync '{}' completed with omissions: {summary}",
            job.name
        ));
    }
    finish(job, result, cancel)
}

/// Engine settings: own state, verify depth, a confirmed stop (consumed).
fn run_settings(
    job: &SyncJob,
    state: &crate::syncjobs::JobState,
    cause: RunCause,
) -> Result<crate::bisync::RunSettings, String> {
    let mut settings = crate::bisync::RunSettings::for_job(&job.id);
    if matches!(
        cause,
        RunCause::Verify | RunCause::Startup | RunCause::CatchUp
    ) {
        settings.depth = crate::bisync::ScanDepth::VerifySources;
    }
    if let Some(block) = state.blocked.as_ref().filter(|block| block.confirmed) {
        if let Some(confirmation) = crate::syncjobs::block_confirmation(&block.kind) {
            settings.confirmed.push(confirmation);
        }
        crate::syncjobs::update_job_state(&job.id, |state| {
            if let Some(current) = state
                .blocked
                .as_mut()
                .filter(|current| current.kind == block.kind && current.since == block.since)
            {
                current.confirmed = false;
            }
            if state.pending_trigger.as_ref().is_some_and(|p| {
                p.kind == crate::syncjobs::PendingKind::Confirmed
                    && p.since <= state.running.as_ref().map_or(i64::MIN, |mark| mark.started)
            }) {
                state.pending_trigger = None;
            }
        })
        .map_err(|error| error.to_string())?;
    }
    Ok(settings)
}

/// The vault sync (`vault::run_vault_job`; tests pass their own).
type VaultRunner<'a> =
    &'a dyn Fn(crate::vault::VaultRunRequest<'_>) -> crate::vault::VaultRunOutcome;

/// A vault job (`vault-jobs/`, `kind=vault`, C5.8) runs only through the
/// vault sync with its rules R1–R11 (`vault::run_vault_job`): always a full
/// scan over the encrypted storage, strict conflicts, no automatic conflict
/// resolution. Hooks and confirmed stops behave as for every job.
fn vault_attempt(
    job: &SyncJob,
    state: &crate::syncjobs::JobState,
    cancel: &AtomicBool,
    cause: RunCause,
    run_vault: VaultRunner<'_>,
) -> (AttemptOutcome, JobResult) {
    if let Err(error) = job.validate() {
        return failed(FailureKind::Config, error);
    }
    if let Err(error) = run_job_hook(job, HookPhase::Before, None, cancel) {
        let result = if cancel.load(Ordering::Acquire) {
            cancelled()
        } else {
            failed(FailureKind::Hook, error)
        };
        return cleanup(job, result, cancel);
    }
    let settings = match run_settings(job, state, cause) {
        Ok(settings) => settings,
        Err(error) => return cleanup(job, failed(FailureKind::Internal, error), cancel),
    };
    let run = run_vault(crate::vault::VaultRunRequest {
        job,
        dry_run: false,
        cancel,
        settings,
        observer: None,
    });
    let result = match run.unavailable {
        // R7/R1: the run did not start; nothing changed on either side.
        Some(reason) => failed(FailureKind::Unreachable, reason),
        None => {
            crate::syncjobs::classify_run(&run.outcome, cancel.load(Ordering::Acquire), now_secs())
        }
    };
    log(&format!("ran vault job '{}': {}", job.id, result.1.note));
    if let Some(site) = run.seeded {
        log(&format!("vault job '{}': copy {site} is complete", job.id));
    }
    finish(job, result, cancel)
}

/// The after hook of a finished attempt, the cleanup hook of a cancelled one.
fn finish(
    job: &SyncJob,
    result: (AttemptOutcome, JobResult),
    cancel: &AtomicBool,
) -> (AttemptOutcome, JobResult) {
    if matches!(result.0, AttemptOutcome::Cancelled) {
        cleanup(job, result, cancel)
    } else {
        let mut result = result;
        if let Err(error) = run_job_hook(job, HookPhase::After, Some(&result.0), cancel) {
            add_hook_failure(&mut result, error);
            if cancel.load(Ordering::Acquire) {
                result.0 = AttemptOutcome::Cancelled;
                return cleanup(job, result, cancel);
            }
        }
        result
    }
}

fn cleanup(
    job: &SyncJob,
    mut result: (AttemptOutcome, JobResult),
    cancel: &AtomicBool,
) -> (AttemptOutcome, JobResult) {
    if let Err(error) = run_job_hook(job, HookPhase::Cleanup, Some(&result.0), cancel) {
        add_hook_failure(&mut result, error);
    }
    result
}

fn add_hook_failure(result: &mut (AttemptOutcome, JobResult), error: String) {
    result.1.errors = result.1.errors.saturating_add(1);
    result.1.note = format!("{}; {error}", result.1.note);
    if matches!(result.0, AttemptOutcome::Success) {
        result.0 = AttemptOutcome::Failed(JobError {
            kind: FailureKind::Hook,
            message: error,
        });
    }
}

fn failed(kind: FailureKind, message: String) -> (AttemptOutcome, JobResult) {
    (
        AttemptOutcome::Failed(JobError {
            kind,
            message: message.clone(),
        }),
        JobResult {
            when: now_secs(),
            errors: 1,
            note: message,
            ..Default::default()
        },
    )
}

fn cancelled() -> (AttemptOutcome, JobResult) {
    (
        AttemptOutcome::Cancelled,
        JobResult {
            when: now_secs(),
            note: "abgebrochen".into(),
            ..Default::default()
        },
    )
}

pub(super) fn persist(
    job: &SyncJob,
    started: i64,
    cause: RunCause,
    outcome: AttemptOutcome,
    result: JobResult,
) -> bool {
    let finished = now_secs();
    let report = AttemptReport {
        runner: Runner::Daemon,
        cause,
        started,
        finished,
        outcome,
        result: Some(result),
    };
    if let Err(error) = crate::syncjobs::record_attempt(&job.id, &report) {
        log(&format!(
            "could not persist attempt for '{}': {error}",
            job.name
        ));
        return false;
    }
    true
}

struct PreparedJob {
    opts: crate::bisync::BisyncOptions,
    ignore: globset::GlobSet,
    bounds: (u64, u64, i64, i64),
}

impl PreparedJob {
    fn new(job: &SyncJob, now: i64) -> Result<Self, String> {
        job.validate()?;
        Ok(Self {
            opts: job.checked_opts(false)?,
            ignore: job.checked_glob_set()?,
            bounds: job.checked_filter_bounds(now)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn review_task_job_preparation_precedes_every_runtime_effect() {
        let mut job = SyncJob::new("job".into(), "/source".into(), "/target".into());
        job.max_delete_pct = 101;
        assert!(PreparedJob::new(&job, 1_700_000_000).is_err());
        job.max_delete_pct = 0;
        job.ignore = vec!["[".into()];
        assert!(PreparedJob::new(&job, 1_700_000_000).is_err());
    }

    #[test]
    fn vault_task_daemon_runs_vault_job_through_vault_sync() {
        use std::cell::Cell;
        let job = SyncJob::new_vault(
            "Tresor".into(),
            "0123456789abcdef0123456789abcdef",
            "fedcba9876543210fedcba9876543210",
        );
        let state = crate::syncjobs::JobState::default();
        let cancel = AtomicBool::new(false);
        let called = Cell::new(false);
        let runner = |request: crate::vault::VaultRunRequest<'_>| {
            called.set(true);
            assert_eq!(request.job.id, job.id);
            assert!(!request.dry_run);
            let mut outcome = crate::vault::VaultRunOutcome::default();
            outcome.outcome.stats.a_to_b = 2;
            outcome
        };
        let (outcome, result) = vault_attempt(&job, &state, &cancel, RunCause::Other, &runner);
        assert!(called.get());
        assert_eq!(outcome, AttemptOutcome::Success);
        assert_eq!(result.a_to_b, 2);
        // R7/R1: a run that could not start fails without touching a side.
        let refuse = |_: crate::vault::VaultRunRequest<'_>| crate::vault::VaultRunOutcome {
            unavailable: Some("Tresor-Job: anderer Tresor".into()),
            ..Default::default()
        };
        let (outcome, _) = vault_attempt(&job, &state, &cancel, RunCause::Other, &refuse);
        assert!(matches!(
            outcome,
            AttemptOutcome::Failed(JobError {
                kind: FailureKind::Unreachable,
                ..
            })
        ));
    }
}
