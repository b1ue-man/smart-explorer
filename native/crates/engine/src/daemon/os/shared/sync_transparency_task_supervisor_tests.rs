//! Admission of evidence-based retries; heartbeat progress from log lines.
use super::*;
use crate::syncjobs::{JobState, Recheck};

fn auth_failed() -> JobState {
    JobState {
        consecutive_failures: 3,
        last_attempt: Some(100),
        last_error: Some(JobError {
            kind: FailureKind::Auth,
            message: "invalid_grant".into(),
        }),
        // An old probe deadline never authorizes a login attempt by itself.
        retry_at: Some(50),
        ..JobState::default()
    }
}

#[test]
fn sync_transparency_task_admission_allows_a_retry_only_with_pending_evidence() {
    let mut state = auth_failed();
    assert!(!admission_allowed(&state, RunCause::Retry, 1_000));
    state.recheck = Some(Recheck {
        evidence: 200,
        reason: "Anmeldedaten wurden geändert".into(),
        pending: true,
    });
    assert!(admission_allowed(&state, RunCause::Retry, 1_000));
    // Only the retry cause; a change trigger still waits for the user.
    assert!(!admission_allowed(&state, RunCause::Change, 1_000));
    state.recheck.as_mut().unwrap().pending = false;
    assert!(!admission_allowed(&state, RunCause::Retry, 1_000));
}

#[test]
fn sync_transparency_task_log_lines_count_as_progress() {
    let id = format!(
        "sync_transparency_task_progress_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let before = super::super::state::now_secs();
    crate::bisync::job_log_line(&id, "Gelesen", "/root: 3 Dateien, 0 Ordner, 12 ms");
    assert!(crate::bisync::last_activity(&id).is_some_and(|at| at >= before));
    if let Some(path) = crate::bisync::job_log_path(&id) {
        let _ = std::fs::remove_file(path);
    }
}
