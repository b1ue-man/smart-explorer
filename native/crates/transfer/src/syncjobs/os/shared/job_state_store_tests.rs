//! Milestone tests of the job state store (RV1, T-JOBS).

use super::*;
use crate::syncjobs::{
    AttemptOutcome, Blocked, FailureKind, JobError, JobSide, PendingKind, PendingTrigger,
    ProblemKind, RunCause, Runner,
};

fn seed() -> JobState {
    empty_state()
}

fn report(outcome: AttemptOutcome, started: i64, finished: i64) -> AttemptReport {
    AttemptReport {
        runner: Runner::Daemon,
        cause: RunCause::Interval,
        started,
        finished,
        outcome,
        result: Some(JobResult {
            when: finished,
            note: "ok".into(),
            ..Default::default()
        }),
    }
}

fn record(dir: &Path, outcome: AttemptOutcome, started: i64, finished: i64) -> JobState {
    let report = report(outcome, started, finished);
    try_update_in(dir, "job", seed, |state| {
        super::super::job_state_policy::apply_attempt(state, &report);
        Ok(())
    })
    .unwrap()
}

fn failure(kind: FailureKind) -> AttemptOutcome {
    AttemptOutcome::Failed(JobError {
        kind,
        message: "Ziel nicht erreichbar".into(),
    })
}

#[test]
fn review_task_job_state_records_attempts_with_backoff_and_reset() {
    let directory = tempfile::tempdir().unwrap();
    let dir = directory.path();

    let state = record(dir, AttemptOutcome::Success, 100, 160);
    assert_eq!(state.last_success, Some(160));
    assert_eq!(state.last_attempt, Some(100));
    assert_eq!(state.last_runner, Some(Runner::Daemon));
    assert_eq!(load_in(dir, "job", seed).unwrap(), state);

    let state = record(dir, failure(FailureKind::Unreachable), 200, 210);
    assert_eq!(state.consecutive_failures, 1);
    assert_eq!(state.retry_at, Some(210 + 300));
    assert_eq!(state.last_success, Some(160));
    let state = record(dir, failure(FailureKind::Unreachable), 600, 610);
    assert_eq!(state.retry_at, Some(610 + 900));
    assert_eq!(state.problem(), None);
    let state = record(dir, failure(FailureKind::Run), 2_000, 2_010);
    assert_eq!(state.consecutive_failures, 3);
    assert_eq!(state.problem(), Some(ProblemKind::FailureSeries));

    let state = record(dir, failure(FailureKind::Auth), 3_000, 3_010);
    assert_eq!(state.retry_at, None, "login failures wait for the user");
    assert_eq!(state.problem(), Some(ProblemKind::NeedsAction));

    let state = record(dir, AttemptOutcome::Cancelled, 4_000, 4_010);
    assert_eq!(
        state.consecutive_failures, 4,
        "a cancellation counts nothing"
    );

    let state = record(dir, AttemptOutcome::Success, 5_000, 5_100);
    assert_eq!(state.consecutive_failures, 0);
    assert_eq!(state.last_error, None);
    assert_eq!(state.retry_at, None);
    assert_eq!(state.last_success, Some(5_100));
}

#[test]
fn review_task_job_state_triggers_survive_until_a_covering_run() {
    let directory = tempfile::tempdir().unwrap();
    let dir = directory.path();
    try_update_in(dir, "job", seed, |state| {
        state.pending_trigger = Some(PendingTrigger {
            kind: PendingKind::Change,
            since: 500,
            volume: None,
        });
        Ok(())
    })
    .unwrap();

    // Cancelled and failed attempts keep the trigger; a run that started
    // before the change does not cover it.
    assert!(record(dir, AttemptOutcome::Cancelled, 600, 610)
        .pending_trigger
        .is_some());
    assert!(record(dir, failure(FailureKind::Unreachable), 700, 710)
        .pending_trigger
        .is_some());
    assert!(record(dir, AttemptOutcome::Success, 400, 800)
        .pending_trigger
        .is_some());
    assert!(record(dir, AttemptOutcome::Success, 900, 950)
        .pending_trigger
        .is_none());
}

#[test]
fn review_task_job_state_block_confirmation_matches_the_shown_stop() {
    let directory = tempfile::tempdir().unwrap();
    let dir = directory.path();
    let kind = BlockKind::MassDelete {
        side: JobSide::B,
        deletions: 120,
        total: 200,
    };
    let block = Blocked {
        kind: kind.clone(),
        detail: "120 Löschungen".into(),
        since: 1_000,
        confirmed: false,
    };
    let state = record(dir, AttemptOutcome::Blocked(block.clone()), 1_000, 1_005);
    assert_eq!(state.problem(), Some(ProblemKind::Blocked));
    assert_eq!(state.retry_at, None);

    let other = BlockKind::MassDelete {
        side: JobSide::B,
        deletions: 121,
        total: 200,
    };
    let refused = try_update_in(dir, "job", seed, |state| {
        if super::super::job_state_policy::confirm(state, &other, 1_100) {
            Ok(())
        } else {
            Err(io::Error::new(io::ErrorKind::InvalidInput, "changed"))
        }
    });
    assert_eq!(refused.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    assert!(
        !load_in(dir, "job", seed)
            .unwrap()
            .blocked
            .unwrap()
            .confirmed
    );

    let confirmed = try_update_in(dir, "job", seed, |state| {
        assert!(super::super::job_state_policy::confirm(state, &kind, 1_200));
        Ok(())
    })
    .unwrap();
    assert!(confirmed.blocked.as_ref().unwrap().confirmed);
    assert_eq!(
        confirmed
            .pending_trigger
            .as_ref()
            .map(|trigger| trigger.kind),
        Some(PendingKind::Confirmed)
    );

    // The same stop again keeps its first time; a success clears it.
    let again = record(
        dir,
        AttemptOutcome::Blocked(Blocked {
            since: 1_300,
            ..block
        }),
        1_300,
        1_305,
    );
    assert_eq!(again.blocked.as_ref().unwrap().since, 1_000);
    assert!(!again.blocked.as_ref().unwrap().confirmed);
    assert_eq!(
        record(dir, AttemptOutcome::Success, 1_400, 1_500).blocked,
        None
    );
}

#[test]
fn review_task_job_state_unreadable_file_is_set_aside() {
    let directory = tempfile::tempdir().unwrap();
    let dir = directory.path();
    std::fs::write(state_path(dir, "job"), b"{ not json").unwrap();

    let loaded = load_in(dir, "job", seed).unwrap();
    assert!(loaded.load_error.is_some());
    assert_eq!(loaded.last_success, None);

    let written = record(dir, AttemptOutcome::Success, 10, 20);
    assert!(
        written.load_error.is_some(),
        "the caller learns about it once"
    );
    assert!(corrupt_path(dir, "job").exists());
    let reloaded = load_in(dir, "job", seed).unwrap();
    assert_eq!(reloaded.load_error, None);
    assert_eq!(reloaded.last_success, Some(20));
}

#[test]
fn review_task_job_state_tolerates_newer_values_and_fields() {
    let body = r#"{
        "version": 7,
        "consecutive_failures": 2,
        "last_error": {"kind": "quantum_flux", "message": "neu"},
        "blocked": {"kind": {"type": "future_stop", "extra": 1}, "detail": "x", "since": 5},
        "pending_trigger": {"kind": "telepathy", "since": 9},
        "watch": {"detection": {"mode": "hologram"}, "since": 3},
        "future_field": [1, 2, 3]
    }"#;
    let state: JobState = serde_json::from_str(body).unwrap();
    assert_eq!(state.consecutive_failures, 2);
    assert_eq!(state.last_error.unwrap().kind, FailureKind::Other);
    assert_eq!(state.blocked.unwrap().kind, BlockKind::Other);
    assert_eq!(state.pending_trigger.unwrap().kind, PendingKind::Other);
    assert_eq!(
        state.watch.unwrap().detection,
        crate::syncjobs::ChangeDetection::Other
    );
}

#[test]
fn review_task_job_state_lock_keeps_every_concurrent_update() {
    let directory = tempfile::tempdir().unwrap();
    let dir = directory.path().to_path_buf();
    let writers: Vec<_> = (0..4)
        .map(|_| {
            let dir = dir.clone();
            std::thread::spawn(move || {
                for _ in 0..25 {
                    try_update_in(&dir, "job", seed, |state| {
                        state.consecutive_failures += 1;
                        Ok(())
                    })
                    .unwrap();
                }
            })
        })
        .collect();
    for writer in writers {
        writer.join().unwrap();
    }
    assert_eq!(
        load_in(&dir, "job", seed).unwrap().consecutive_failures,
        100
    );
}

#[test]
fn review_task_problem_notices_are_throttled_per_problem() {
    use super::super::job_state_policy::{build_notice, notice_due, problem_key};
    let mut state = empty_state();
    state.last_error = Some(JobError {
        kind: FailureKind::Auth,
        message: "Anmeldung abgelehnt.".into(),
    });
    state.consecutive_failures = 1;
    let key = problem_key(&state).unwrap();
    assert_eq!(key, "needs_action:auth");
    assert!(notice_due(None, &key, 1_000));
    let notified = Notified {
        key: key.clone(),
        at: 1_000,
    };
    assert!(!notice_due(Some(&notified), &key, 1_000 + 3_600));
    assert!(notice_due(Some(&notified), &key, 1_000 + 86_400));
    assert!(notice_due(Some(&notified), "failures:run", 1_100));

    let job = SyncJob::new("Fotos".into(), "/a".into(), "/b".into());
    let notice = build_notice(&job, &state, ProblemKind::NeedsAction);
    assert!(notice.title.contains("Fotos"));
    assert!(notice.text.contains("Anmeldung"));
}

#[test]
fn review_task_legacy_seed_ignores_implausible_clocks() {
    let mut job = SyncJob::new("x".into(), "/a".into(), "/b".into());
    job.last_run = 1_000;
    assert_eq!(
        legacy_seed(Some(&job), "unknown-id", 2_000).last_success,
        Some(1_000)
    );
    job.last_run = 2_000 + 3 * 86_400;
    assert_eq!(
        legacy_seed(Some(&job), "unknown-id", 2_000).last_success,
        None
    );
}
