//! Job state: evidence-based retries of `needs_user` failures, interrupted
//! runs and the job log's result line.
use super::job_state::{
    AttemptOutcome, AttemptReport, FailureKind, Interrupted, JobError, JobState, Recheck, RunCause,
    Runner,
};
use super::job_state_policy::{apply_attempt, attempt_text};
use super::results::JobResult;

fn report(outcome: AttemptOutcome, started: i64) -> AttemptReport {
    AttemptReport {
        runner: Runner::Daemon,
        cause: RunCause::Retry,
        started,
        finished: started + 5,
        outcome,
        result: Some(JobResult {
            when: started + 5,
            ..Default::default()
        }),
    }
}

fn auth() -> AttemptOutcome {
    AttemptOutcome::Failed(JobError {
        kind: FailureKind::Auth,
        message: "HTTP 400: invalid_grant".into(),
    })
}

fn waiting() -> JobState {
    JobState {
        consecutive_failures: 7,
        last_attempt: Some(100),
        last_error: Some(JobError {
            kind: FailureKind::Auth,
            message: "invalid_grant".into(),
        }),
        recheck: Some(Recheck {
            evidence: 200,
            reason: "Anmeldedaten wurden geändert".into(),
            pending: true,
        }),
        interrupted: Some(Interrupted {
            runner: Runner::Desktop,
            started: 50,
            alive: 60,
            detected: 300,
        }),
        ..JobState::default()
    }
}

#[test]
fn sync_transparency_task_recheck_is_used_once_and_survives_cancellation() {
    let mut state = waiting();
    apply_attempt(&mut state, &report(AttemptOutcome::Cancelled, 400));
    assert!(state
        .recheck
        .as_ref()
        .is_some_and(|recheck| recheck.pending));
    assert!(
        state.interrupted.is_none(),
        "a finished attempt replaces the interruption note"
    );
    apply_attempt(&mut state, &report(auth(), 500));
    let recheck = state
        .recheck
        .as_ref()
        .expect("the used evidence stays recorded");
    assert!(!recheck.pending);
    assert_eq!(recheck.evidence, 200);
    assert_eq!(state.consecutive_failures, 8);
    assert_eq!(state.retry_at, None, "no automatic login attempts");
}

#[test]
fn sync_transparency_task_success_clears_error_recheck_and_interruption() {
    let mut state = waiting();
    apply_attempt(&mut state, &report(AttemptOutcome::Success, 400));
    assert!(state.last_error.is_none());
    assert!(state.recheck.is_none());
    assert!(state.interrupted.is_none());
    assert_eq!(state.consecutive_failures, 0);
    assert_eq!(state.last_success, Some(405));
}

#[test]
fn sync_transparency_task_result_line_names_outcome_runner_and_series() {
    let mut state = JobState::default();
    let failed = report(
        AttemptOutcome::Failed(JobError {
            kind: FailureKind::Unreachable,
            message: "Ziel nicht erreichbar".into(),
        }),
        1_000,
    );
    apply_attempt(&mut state, &failed);
    let text = attempt_text(&failed, &state);
    assert!(
        text.contains("Fehler (Unreachable): Ziel nicht erreichbar"),
        "{text}"
    );
    assert!(text.contains("Daemon (Retry)"), "{text}");
    assert!(text.contains("1 Fehler in Folge"), "{text}");
    assert!(text.contains("nächster automatischer Versuch"), "{text}");
    let ok = report(AttemptOutcome::Success, 2_000);
    apply_attempt(&mut state, &ok);
    assert!(attempt_text(&ok, &state).starts_with("Erfolg"));
}

#[test]
fn sync_transparency_task_old_state_files_without_new_fields_still_load() {
    let state: JobState = serde_json::from_str(
        r#"{"version":1,"consecutive_failures":2,"last_error":{"kind":"auth","message":"x"}}"#,
    )
    .unwrap();
    assert!(state.recheck.is_none() && state.interrupted.is_none());
    let text = serde_json::to_string(&waiting()).unwrap();
    let back: JobState = serde_json::from_str(&text).unwrap();
    assert_eq!(back, waiting());
}

#[test]
fn sync_transparency_task_recorded_attempt_and_saved_edit_reach_the_job_log() {
    let id = format!(
        "sync_transparency_task_state_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    super::job_state_store::record_attempt(&id, &report(auth(), 1_000)).unwrap();
    let log = crate::bisync::read_job_log(&id, None).unwrap().text;
    assert!(
        log.contains("Ergebnis") && log.contains("invalid_grant"),
        "{log}"
    );
    // Saving the job is new evidence: exactly one retry is armed.
    super::job_state_store::recheck_after_edit(&id);
    let state = super::job_state_store::load_job_state(&id).unwrap();
    let recheck = state.recheck.expect("edit arms one retry");
    assert!(recheck.pending);
    assert_eq!(recheck.reason, "Einstellungen wurden gespeichert");
    let log = crate::bisync::read_job_log(&id, None).unwrap().text;
    assert!(log.contains("Wiederholung"), "{log}");
    super::job_state_store::remove_job_state(&id).unwrap();
    if let Some(path) = crate::bisync::job_log_path(&id) {
        let _ = std::fs::remove_file(path);
    }
}
