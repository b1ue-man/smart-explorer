//! Evidence-based retries after login failures and cleanup of dead run marks.
use super::*;
use crate::syncjobs::{JobError, RunCause, RunMark, Runner};

const NOW: i64 = 1_900_000_000;

fn job(id: &str, source: &str) -> SyncJob {
    let mut job = SyncJob::new(id.into(), source.into(), "C:/Users/test/Vault".into());
    job.id = id.into();
    job.name = id.into();
    job
}

fn auth_failed(at: i64) -> JobState {
    JobState {
        consecutive_failures: 7,
        last_attempt: Some(at),
        last_error: Some(JobError {
            kind: FailureKind::Auth,
            message: "HTTP 400: invalid_grant".into(),
        }),
        ..JobState::default()
    }
}

#[test]
fn sync_transparency_task_changed_credentials_allow_one_retry() {
    let notebook = job("notebook", "gdrive:///Notebook");
    let jobs = vec![notebook.clone()];
    let states = BTreeMap::from([("notebook".to_string(), auth_failed(100))]);
    let found = evidence(&notebook, &states["notebook"], &jobs, &states, Some(200));
    assert_eq!(found.as_ref().map(|(at, _)| *at), Some(200));
    // Older than the failed attempt: no evidence.
    assert!(evidence(&notebook, &states["notebook"], &jobs, &states, Some(50)).is_none());
    // The same evidence is not used twice.
    let mut used = auth_failed(100);
    used.recheck = Some(Recheck {
        evidence: 200,
        reason: String::new(),
        pending: false,
    });
    assert!(evidence(&notebook, &used, &jobs, &states, Some(200)).is_none());
}

#[test]
fn sync_transparency_task_later_success_on_the_same_drive_account_is_evidence() {
    let notebook = job("notebook", "gdrive:///Notebook");
    let vault = job("vault", "GDRIVE:///Vaults/Private");
    let nas = job("nas", "sftp://nas/backup");
    let jobs = vec![notebook.clone(), vault, nas];
    let mut states = BTreeMap::from([("notebook".to_string(), auth_failed(100))]);
    states.insert(
        "nas".into(),
        JobState {
            last_success: Some(900),
            ..JobState::default()
        },
    );
    // An SFTP success says nothing about the Drive login.
    assert!(evidence(&notebook, &states["notebook"], &jobs, &states, None).is_none());
    states.insert(
        "vault".into(),
        JobState {
            last_success: Some(300),
            ..JobState::default()
        },
    );
    let (at, reason) = evidence(&notebook, &states["notebook"], &jobs, &states, None).unwrap();
    assert_eq!(at, 300);
    assert!(
        reason.contains("vault") && reason.contains("Google-Konto"),
        "{reason}"
    );
    // Password backends never get evidence from other jobs (lockout risk).
    let sftp = job("sftp", "sftp://nas/photos");
    let sftp_states = BTreeMap::from([
        ("sftp".to_string(), auth_failed(100)),
        (
            "nas".to_string(),
            JobState {
                last_success: Some(900),
                ..JobState::default()
            },
        ),
    ]);
    let both = vec![sftp.clone(), job("nas", "sftp://nas/backup")];
    assert!(evidence(&sftp, &sftp_states["sftp"], &both, &sftp_states, None).is_none());
}

#[test]
fn sync_transparency_task_only_login_failures_wait_for_evidence() {
    let notebook = job("notebook", "gdrive:///Notebook");
    let mut state = auth_failed(100);
    state.last_error.as_mut().unwrap().kind = FailureKind::Unreachable;
    let jobs = vec![notebook.clone()];
    let states = BTreeMap::from([("notebook".to_string(), state.clone())]);
    assert!(evidence(&notebook, &state, &jobs, &states, Some(500)).is_none());
}

#[test]
fn sync_transparency_task_due_and_admission_follow_a_pending_recheck() {
    let mut notebook = job("notebook", "gdrive:///Notebook");
    notebook.trigger = crate::syncjobs::Trigger::Interval;
    notebook.interval_min = 60;
    let mut state = auth_failed(NOW - 600);
    assert_eq!(
        super::super::due::due_now(&notebook, &state, NOW, None),
        None
    );
    assert_eq!(super::super::due::next_due(&notebook, &state, NOW), None);
    state.recheck = Some(Recheck {
        evidence: NOW - 10,
        reason: "Anmeldedaten wurden geändert".into(),
        pending: true,
    });
    assert_eq!(
        super::super::due::due_now(&notebook, &state, NOW, None),
        Some(RunCause::Retry)
    );
    assert_eq!(
        super::super::due::next_due(&notebook, &state, NOW),
        Some(NOW)
    );
    state.recheck.as_mut().unwrap().pending = false;
    assert_eq!(
        super::super::due::due_now(&notebook, &state, NOW, None),
        None
    );
}

#[test]
fn sync_transparency_task_dead_run_marks_become_interrupted_with_a_verification_run() {
    let id = format!(
        "sync_transparency_task_mark_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let job = job(&id, "gdrive:///Notebook");
    let mark = RunMark {
        runner: Runner::Desktop,
        cause: RunCause::Manual,
        started: NOW - 4_000,
        alive: NOW - 3_000,
        stalled_since: None,
    };
    crate::syncjobs::update_job_state(&id, |state| state.running = Some(mark.clone())).unwrap();
    let jobs = vec![job];
    // A run of this service is never touched.
    let states = crate::syncjobs::load_job_states(&jobs);
    assert!(!refresh(&jobs, &states, NOW, &HashSet::from([id.clone()])));
    assert_eq!(
        crate::syncjobs::load_job_state(&id).unwrap().running,
        Some(mark.clone())
    );
    assert!(refresh(&jobs, &states, NOW, &HashSet::new()));
    let state = crate::syncjobs::load_job_state(&id).unwrap();
    assert!(state.running.is_none());
    let interrupted = state.interrupted.expect("dead run is recorded");
    assert_eq!(
        (interrupted.started, interrupted.alive),
        (mark.started, mark.alive)
    );
    assert_eq!(
        state.pending_trigger.map(|trigger| trigger.kind),
        Some(PendingKind::Verify)
    );
    let log = crate::bisync::read_job_log(&id, None).unwrap().text;
    assert!(log.contains("Unterbrochen"), "{log}");
    // A live mark (renewed within the stale window) stays.
    let live = RunMark {
        alive: NOW - 10,
        ..mark
    };
    crate::syncjobs::update_job_state(&id, |state| state.running = Some(live.clone())).unwrap();
    let states = crate::syncjobs::load_job_states(&jobs);
    assert!(!refresh(&jobs, &states, NOW, &HashSet::new()));
    crate::syncjobs::remove_job_state(&id).unwrap();
    if let Some(path) = crate::bisync::job_log_path(&id) {
        let _ = std::fs::remove_file(path);
    }
}
