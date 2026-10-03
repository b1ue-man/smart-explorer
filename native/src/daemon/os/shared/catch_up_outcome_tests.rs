//! Acceptance for catch-up outcomes and host-deferred durable triggers.
use super::*;
use crate::syncjobs::{FailureKind, JobError, PendingKind, PendingTrigger};
use std::collections::HashMap;

struct OutcomeQueue {
    completed: Vec<String>,
    outcomes: HashMap<String, (AttemptOutcome, bool)>,
}
impl CatchUpQueue for OutcomeQueue {
    fn admit(&mut self, job: &SyncJob) -> Result<EnqueueStatus, String> {
        self.completed.push(job.id.clone()); Ok(EnqueueStatus::Started)
    }
    fn cancel_jobs(&mut self, _: &HashSet<String>) {}
    fn take_completed(&mut self) -> Vec<String> { std::mem::take(&mut self.completed) }
    fn active_job_id(&self) -> Option<&str> { None }
    fn completion(&mut self, id: &str) -> (AttemptOutcome, bool) {
        self.outcomes.remove(id).unwrap_or((AttemptOutcome::Cancelled, false))
    }
}
fn job(id: &str, trigger: Trigger) -> SyncJob {
    let mut job = SyncJob::new(id.into(), "/a".into(), "/b".into());
    job.id = id.into(); job.trigger = trigger; job
}
fn failed(kind: FailureKind) -> (AttemptOutcome, bool) {
    (AttemptOutcome::Failed(JobError { kind, message: format!("{kind:?}") }), true)
}

#[test]
fn review_task_catch_up_aggregates_failures_and_retries_only_transient_errors() {
    let jobs: Vec<_> = ["success", "auth", "offline", "cancel"].into_iter().map(|id| job(id, Trigger::RealTime)).collect();
    let mut queue = OutcomeQueue { completed: Vec::new(), outcomes: [
        ("success".into(), (AttemptOutcome::Success, true)),
        ("auth".into(), failed(FailureKind::Auth)),
        ("offline".into(), failed(FailureKind::Unreachable)),
        ("cancel".into(), (AttemptOutcome::Cancelled, false)),
    ].into_iter().collect() };
    let mut book = CatchUpBook::new(); let id = book.request().unwrap();
    let report = book.service(&mut queue, &CatchUpGate::Open, Some(&Ok(jobs)), 1_800_000_000);
    let status = book.status(id).unwrap();
    assert!(status.finished && status.retry_suggested); assert_eq!(status.failed, 2);
    assert_eq!((report.records[0].ran, report.records[0].succeeded, report.records[0].failed), (3, 1, 2));
    let mut auth_only = OutcomeQueue { completed: Vec::new(), outcomes: [("auth".into(), failed(FailureKind::Auth))].into_iter().collect() };
    let id = book.request().unwrap();
    book.service(&mut auth_only, &CatchUpGate::Open, Some(&Ok(vec![job("auth", Trigger::RealTime)])), 1_800_000_000);
    assert!(!book.status(id).unwrap().retry_suggested);
}

#[test]
fn review_task_empty_cancelled_or_interrupted_windows_do_not_replace_the_last_run() {
    let mut queue = OutcomeQueue { completed: Vec::new(), outcomes: HashMap::new() };
    let mut book = CatchUpBook::new();
    book.request().unwrap();
    assert!(book.service(&mut queue, &CatchUpGate::Open, Some(&Ok(Vec::new())), 1_800_000_000).records.is_empty());
    book.request().unwrap();
    assert!(book.service(&mut queue, &CatchUpGate::Open, Some(&Ok(vec![job("cancel", Trigger::RealTime)])), 1_800_000_000).records.is_empty());
    let mut run = Run { id: 42, phase: Phase::Running, cancel: Cancel::Applied,
        admitted: vec![Admitted { id: "done".into(), name: "done".into(), done: true,
            owned: true, outcome: Some(AttemptOutcome::Success), ran: true }],
        skipped: Vec::new(), message: None, running_job: None, queued: 0 };
    let mut report = ServiceReport::default();
    run.finish("Abgebrochen".into(), &mut report); assert!(report.records.is_empty());
}

#[test]
fn review_task_catch_up_owns_saved_startup_and_connect_triggers_and_respects_auth_backoff() {
    let now = 1_800_000_000;
    let mut state = JobState { pending_trigger: Some(PendingTrigger {
        kind: PendingKind::Startup, since: now - 60, volume: None }), ..JobState::default() };
    assert_eq!(catch_up_cause(&job("start", Trigger::OnStartup), &state, now), Some(RunCause::Startup));
    state.pending_trigger = Some(PendingTrigger { kind: PendingKind::Connect, since: now - 60, volume: Some("USB-A".into()) });
    let job = job("connect", Trigger::OnConnect);
    assert_eq!(catch_up_cause(&job, &state, now), Some(RunCause::Connect));
    state.consecutive_failures = 1; state.last_error = Some(JobError { kind: FailureKind::Auth, message: "login".into() });
    assert_eq!(catch_up_cause(&job, &state, now), None);
    state.retry_at = Some(now + 300);
    assert_eq!(catch_up_cause(&job, &state, now), None);
    assert_eq!(catch_up_cause(&job, &state, now + 300), None);
    state.last_error = Some(JobError { kind: FailureKind::Unreachable, message: "offline".into() });
    assert_eq!(catch_up_cause(&job, &state, now + 300), Some(RunCause::Retry));
}
