//! Lifetime, durable run marks and attempt reports of mobile sync consumers.
use crate::mobile::{ApiError, TaskCtx};
use crate::syncjobs::{
    AttemptOutcome, AttemptReport, FailureKind, JobError, JobResult, RunCause, RunMark, Runner,
    SyncJob,
};
use std::sync::atomic::{AtomicI64, Ordering};

pub(super) struct Lease {
    job: String,
    started: i64,
    stop: std::sync::mpsc::Sender<()>,
    beat: Option<std::thread::JoinHandle<()>>,
    _storage: crate::daemon::StorageRunGuard,
    _awake: crate::keep_awake::KeepAwake,
}

impl Lease {
    pub(super) fn acquire(job: &SyncJob, ctx: &TaskCtx) -> Result<Self, ApiError> {
        let cancel = ctx.cancel_flag();
        let storage = crate::daemon::register_storage_run(&job.source, &job.target, &cancel);
        if storage.access_missing() {
            return Err(ApiError::new(
                "permission",
                "Dateizugriff fehlt: Zugriff auf alle Dateien erlauben.",
            ));
        }
        if ctx.cancelled() {
            return Err(ApiError::new("canceled", "Abgebrochen"));
        }
        let started = super::super::args::now_secs();
        let mut admitted = false;
        crate::syncjobs::update_job_state(&job.id, |state| {
            if state.running_now(started).is_some() {
                return;
            }
            admitted = true;
            state.running = Some(RunMark {
                runner: Runner::Android,
                cause: RunCause::Manual,
                started,
                alive: started,
                stalled_since: None,
            });
        })
        .map_err(|e| ApiError::new("internal", format!("Job-Zustand speichern: {e}")))?;
        if !admitted {
            return Err(ApiError::new("busy", "Der Job läuft bereits."));
        }
        let (stop, rx) = std::sync::mpsc::channel();
        let id = job.id.clone();
        let beat = std::thread::Builder::new()
            .name("android-sync-mark".into())
            .spawn(move || {
                while matches!(
                    rx.recv_timeout(std::time::Duration::from_secs(30)),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                ) {
                    let _ = crate::syncjobs::update_job_state(&id, |state| {
                        if let Some(mark) = state.running.as_mut().filter(|mark| {
                            mark.runner == Runner::Android && mark.started == started
                        }) {
                            mark.alive = super::super::args::now_secs();
                        }
                    });
                }
            });
        let mut lease = Self {
            job: job.id.clone(),
            started,
            stop,
            beat: None,
            _storage: storage,
            _awake: crate::keep_awake::hold(crate::keep_awake::Reason::SyncRun),
        };
        lease.beat = Some(
            beat.map_err(|e| ApiError::new("internal", format!("Laufmarkierung erneuern: {e}")))?,
        );
        Ok(lease)
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(beat) = self.beat.take() {
            let _ = beat.join();
        }
        let _ =
            crate::syncjobs::update_job_state(&self.job, |state| {
                if state.running.as_ref().is_some_and(|mark| {
                    mark.runner == Runner::Android && mark.started == self.started
                }) {
                    state.running = None;
                }
            });
    }
}

pub(super) fn api_outcome(error: &ApiError) -> AttemptOutcome {
    if matches!(error.kind, "canceled" | "busy") {
        return AttemptOutcome::Cancelled;
    }
    let kind = match error.kind {
        "invalid" => FailureKind::Config,
        "permission" | "access" => FailureKind::Access,
        "internal" => FailureKind::Internal,
        "hook" => FailureKind::Hook,
        _ => crate::syncjobs::classify_failure(&error.message),
    };
    AttemptOutcome::Failed(JobError {
        kind,
        message: error.message.clone(),
    })
}

pub(super) fn record(
    job: &SyncJob,
    started: i64,
    cause: RunCause,
    outcome: AttemptOutcome,
    result: Option<JobResult>,
) -> Result<(), ApiError> {
    crate::syncjobs::record_attempt(
        &job.id,
        &AttemptReport {
            runner: Runner::Android,
            cause,
            started,
            finished: super::super::args::now_secs(),
            outcome,
            result,
        },
    )
    .map_err(|e| {
        ApiError::new(
            "internal",
            format!("Laufergebnis konnte nicht gespeichert werden: {e}"),
        )
    })?;
    Ok(())
}

/// Progress uses only the authoritative completed actions; cancellation and
/// partial checkpoints remain the engine's responsibility.
pub(super) struct Observer<'a> {
    pub(super) ctx: &'a TaskCtx,
    pub(super) items: AtomicI64,
}
impl crate::bisync::ApplySink for Observer<'_> {
    fn completed(&self, action: crate::bisync::CompletedAction) {
        let items = self
            .items
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1)
            .max(0) as u64;
        self.ctx.progress(0, 0, items, 0);
        self.ctx.message(&action.rel);
    }
}

pub(super) fn consume_confirmation(
    job: &SyncJob,
    started: i64,
) -> Result<crate::bisync::RunSettings, ApiError> {
    let mut settings = crate::bisync::RunSettings::for_job(&job.id);
    crate::syncjobs::update_job_state(&job.id, |state| {
        take_confirmation(state, started, &mut settings)
    })
    .map_err(|e| ApiError::new("internal", format!("Bestätigung speichern: {e}")))?;
    Ok(settings)
}

fn take_confirmation(
    state: &mut crate::syncjobs::JobState,
    started: i64,
    settings: &mut crate::bisync::RunSettings,
) {
    let mut changed = false;
    if let Some(block) = state.blocked.as_mut().filter(|block| block.confirmed) {
        if let Some(confirmation) = crate::syncjobs::block_confirmation(&block.kind) {
            settings.confirmed.push(confirmation);
        }
        block.confirmed = false;
        changed = true;
    }
    if changed
        && state.pending_trigger.as_ref().is_some_and(|pending| {
            pending.kind == crate::syncjobs::PendingKind::Confirmed && pending.since <= started
        })
    {
        state.pending_trigger = None;
    }
}

pub(super) fn run_job(ctx: &TaskCtx, job: &SyncJob) -> Result<serde_json::Value, ApiError> {
    use crate::daemon::{run_job_hook, HookPhase};
    let initial = crate::syncjobs::load_job_state(&job.id)
        .map_err(|e| ApiError::new("internal", e.to_string()))?;
    let started = initial
        .running
        .as_ref()
        .map_or_else(super::super::args::now_secs, |mark| mark.started);
    let cause = if initial
        .blocked
        .as_ref()
        .is_some_and(|block| block.confirmed)
    {
        RunCause::Confirmed
    } else {
        RunCause::Manual
    };
    crate::syncjobs::update_job_state(&job.id, |state| {
        state.last_attempt = Some(started);
        state.last_runner = Some(Runner::Android);
        state.last_cause = Some(cause);
        if let Some(mark) = state.running.as_mut() {
            mark.cause = cause;
        }
    })
    .map_err(|e| ApiError::new("internal", format!("Versuch speichern: {e}")))?;
    let cancel = ctx.cancel_flag();
    let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        super::checked_settings(job, false)?;
        if ctx.cancelled() {
            return Err(ApiError::new("canceled", "Abgebrochen"));
        }
        run_job_hook(job, HookPhase::Before, None, &cancel)
            .map_err(|e| ApiError::new("hook", e))?;
        ctx.message("Verbinde…");
        let pair = super::open_pair_for(ctx, job)?;
        if ctx.cancelled() {
            return Err(ApiError::new("canceled", "Abgebrochen"));
        }
        let settings = consume_confirmation(job, started)?;
        ctx.message("Synchronisiere…");
        let out = super::run_bisync_with(ctx, job, &pair, false, settings)?;
        let (outcome, result) =
            crate::syncjobs::classify_run(&out, ctx.cancelled(), super::super::args::now_secs());
        let omitted = out.omissions.summary();
        let summary = match &outcome {
            AttemptOutcome::Blocked(block) => block.detail.clone(),
            AttemptOutcome::Failed(error) => error.message.clone(),
            _ => super::run_summary(
                &out,
                omitted.as_deref(),
                matches!(outcome, AttemptOutcome::Cancelled),
            ),
        };
        super::report_errors(ctx, &out.errors);
        let value = serde_json::json!({ "summary": summary, "aToB": result.a_to_b,
            "bToA": result.b_to_a, "deleted": result.deleted, "conflicts": result.conflicts,
            "errors": result.errors, "omitted": omitted, "blocked": matches!(outcome, AttemptOutcome::Blocked(_)) });
        super::sync_conflicts::store_recorded_run(&job.id, pair, out.conflicts, out.state);
        Ok::<_, ApiError>((outcome, Some(result), value))
    }));
    let mut attempt = match attempt {
        Ok(Ok(attempt)) => attempt,
        Ok(Err(error)) => (
            if ctx.cancelled() && error.kind != "invalid" {
                AttemptOutcome::Cancelled
            } else {
                api_outcome(&error)
            },
            None,
            serde_json::json!({ "summary": error.message }),
        ),
        Err(_) => (
            AttemptOutcome::Failed(JobError {
                kind: FailureKind::Internal,
                message: "Sync-Worker wurde unerwartet beendet; Zwischenstände bleiben erhalten."
                    .into(),
            }),
            None,
            serde_json::json!({ "summary": "Sync-Worker wurde unerwartet beendet" }),
        ),
    };
    let phase = if matches!(attempt.0, AttemptOutcome::Cancelled) || attempt.1.is_none() {
        HookPhase::Cleanup
    } else {
        HookPhase::After
    };
    if let Err(message) = run_job_hook(job, phase, Some(&attempt.0), &cancel) {
        ctx.error("", &message);
        if let Some(result) = attempt.1.as_mut() {
            result.errors = result.errors.saturating_add(1);
            result.note = format!("{}; {message}", result.note);
            attempt.2["errors"] = serde_json::json!(result.errors);
        }
        if matches!(attempt.0, AttemptOutcome::Success) {
            attempt.0 = AttemptOutcome::Failed(JobError {
                kind: FailureKind::Hook,
                message,
            });
        }
    }
    if ctx.cancelled() && !matches!(attempt.0, AttemptOutcome::Blocked(_)) {
        attempt.0 = AttemptOutcome::Cancelled;
    }
    record(job, started, cause, attempt.0.clone(), attempt.1)?;
    if let Some(summary) = attempt.2["summary"].as_str() {
        ctx.message(summary);
    }
    ctx.set_failure_result(attempt.2.clone());
    match attempt.0 {
        AttemptOutcome::Cancelled => Err(ApiError::new(
            "canceled",
            "Abgebrochen; erledigte Dateien bleiben gespeichert.",
        )),
        AttemptOutcome::Failed(error) => Err(ApiError::new("sync_failed", error.message)),
        _ => Ok(attempt.2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn android_sync_confirmation_consumed_once_preserves_later_change() {
        let mut state = crate::syncjobs::JobState {
            blocked: Some(crate::syncjobs::Blocked {
                kind: crate::syncjobs::BlockKind::DeleteLimit {
                    deletions: 3,
                    limit: 2,
                },
                confirmed: true,
                ..Default::default()
            }),
            pending_trigger: Some(crate::syncjobs::PendingTrigger {
                kind: crate::syncjobs::PendingKind::Change,
                since: 101,
                volume: None,
            }),
            ..Default::default()
        };
        let mut first = crate::bisync::RunSettings::for_job("job");
        take_confirmation(&mut state, 100, &mut first);
        assert_eq!(first.confirmed.len(), 1);
        assert_eq!(state.pending_trigger.as_ref().unwrap().since, 101);
        let mut second = crate::bisync::RunSettings::for_job("job");
        take_confirmation(&mut state, 100, &mut second);
        assert!(second.confirmed.is_empty());
        state.blocked.as_mut().unwrap().confirmed = true;
        state.pending_trigger.as_mut().unwrap().kind = crate::syncjobs::PendingKind::Confirmed;
        take_confirmation(&mut state, 101, &mut second);
        assert!(state.pending_trigger.is_none());
    }
    #[test]
    fn android_sync_start_errors_keep_the_shared_failure_kind() {
        for (code, kind) in [
            ("permission", FailureKind::Access),
            ("invalid", FailureKind::Config),
            ("hook", FailureKind::Hook),
        ] {
            assert!(matches!(api_outcome(&ApiError::new(code, "Grund")),
                AttemptOutcome::Failed(error) if error.kind == kind));
        }
        assert!(matches!(
            api_outcome(&ApiError::new("canceled", "Stopp")),
            AttemptOutcome::Cancelled
        ));
    }
}
