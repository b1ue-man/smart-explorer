//! C08 exercises persisted loader recovery without removing foreign safety stops.
use super::sync_reliability_task_old_jobs_tests::{assert_noop, local_pair, reload, EndpointFixtures, SavedJob};
use crate::syncjobs::{
    AttemptOutcome, AttemptReport, BlockKind, Blocked, FailureKind, JobError, JobResult,
    PendingKind, RunCause, Runner,
};
use std::sync::atomic::AtomicBool;

fn store_failure(saved: &SavedJob, message: &str, kind: FailureKind, cause: RunCause, runner: Runner) {
    let now = super::state::now_secs() - 10;
    crate::syncjobs::record_attempt(&saved.id, &AttemptReport {
        runner, cause, started: now, finished: now,
        outcome: AttemptOutcome::Failed(JobError { kind, message: message.into() }),
        result: Some(JobResult { when: now, errors: 1, note: message.into(), ..Default::default() }),
    }).unwrap();
}

#[test]
fn sync_reliability_task_old_jobs_modern_loader_recovers_only_after_actual_repair_and_restart() {
    let (_temp, a, b) = local_pair();
    std::fs::write(format!("{a}/keep.txt"), b"owned baseline remains").unwrap();
    let saved = SavedJob::old(&a, &b, "interval");
    saved.run();
    let original_job = saved.load();
    let local_a = crate::vfs::LocalBackend::new(&a);
    let local_b = crate::vfs::LocalBackend::new(&b);
    let key = saved.key(&local_a, &a, &local_b, &b);
    let baseline = crate::bisync::baseline_file(&key).unwrap();
    let before = std::fs::read(&baseline).unwrap();
    assert!(super::job_triggers::persist(&saved.id, PendingKind::Other, 1, None));
    std::fs::write(saved.path(), saved.original.replace("retries=2", "retries=not-a-number")).unwrap();
    assert!(!super::run_loop::sync_reliability_task_reload_after_restart().iter().any(|job| job.id == saved.id));
    let broken = crate::syncjobs::load_job_state(&saved.id).unwrap();
    assert_eq!(broken.last_error.as_ref().unwrap().kind, FailureKind::Config);
    assert!(broken.last_error.as_ref().unwrap().message.starts_with("Sync-Jobdatei konnte nicht geladen werden: "));
    assert_eq!(super::due::due_now(&original_job, &broken, super::state::now_secs(), None), None);
    assert!(!super::run_loop::sync_reliability_task_reload_after_restart().iter().any(|job| job.id == saved.id));
    saved.repair();
    let job = reload(&saved);
    let recovered = crate::syncjobs::load_job_state(&saved.id).unwrap();
    assert!(recovered.last_error.is_none());
    assert_eq!(recovered.consecutive_failures, 0);
    assert_eq!(recovered.pending_trigger, broken.pending_trigger);
    assert_eq!(recovered.last_success, broken.last_success);
    assert_eq!(std::fs::read(&baseline).unwrap(), before);
    assert_eq!(super::due::due_now(&job, &recovered, super::state::now_secs(), None), Some(RunCause::Interval));
    assert_noop(&saved.run());
    assert_eq!(saved.key(&local_a, &a, &local_b, &b), key);
}

#[test]
fn sync_reliability_task_old_jobs_legacy_loader_requires_persisted_origin_proof() {
    let (_temp, a, b) = local_pair();
    for diagnostic in ["path", "Löschschutz-Migration: saved eligibility unavailable", "Permission denied (os error 13)"] {
        let saved = SavedJob::old(&a, &b, "interval");
        let job = saved.load();
        let message = if diagnostic == "path" {
            format!("invalid sync job configuration {}: retries is not a valid number", saved.path().display())
        } else { diagnostic.to_string() };
        store_failure(&saved, &message, FailureKind::Config, RunCause::Other, Runner::Daemon);
        assert!(super::job_triggers::persist(&saved.id, PendingKind::Other, 1, None));
        let before = crate::syncjobs::load_job_state(&saved.id).unwrap();
        reload(&saved);
        let recovered = crate::syncjobs::load_job_state(&saved.id).unwrap();
        assert!(recovered.last_error.is_none(), "{diagnostic}: {recovered:?}");
        assert_eq!(recovered.pending_trigger, before.pending_trigger);
        assert_eq!(super::due::due_now(&job, &recovered, super::state::now_secs(), None), Some(RunCause::Interval));
    }
    for (runner, cause, same_note) in [
        (Runner::Desktop, RunCause::Other, true),
        (Runner::Daemon, RunCause::Manual, true),
        (Runner::Daemon, RunCause::Other, false),
    ] {
        let saved = SavedJob::old(&a, &b, "interval");
        saved.load();
        store_failure(&saved, "Permission denied (os error 13)", FailureKind::Config, cause, runner);
        if !same_note {
            crate::syncjobs::update_job_state(&saved.id, |state| state.last_result.as_mut().unwrap().note = "a different attempt".into()).unwrap();
        }
        let before = crate::syncjobs::load_job_state(&saved.id).unwrap();
        let job = reload(&saved);
        assert_eq!(crate::syncjobs::load_job_state(&saved.id).unwrap(), before);
        assert_eq!(super::due::due_now(&job, &before, super::state::now_secs() + 86_400, None), None);
    }
}

#[test]
fn sync_reliability_task_old_jobs_auth_access_and_preparation_errors_stay_blocked() {
    let (_temp, a, b) = local_pair();
    for (message, expected) in [
        ("Drive-Konto wurde gewechselt; gespeicherte Ordnerbindung gehört zu einem anderen Konto", FailureKind::Auth),
        ("Permission denied (os error 13)", FailureKind::Access),
    ] {
        let source = "gdrive:///protected";
        let endpoints = EndpointFixtures::new(Vec::new());
        endpoints.insert_failure(source, message);
        let saved = SavedJob::old(source, &b, "interval");
        super::job::run_one(&saved.load(), &AtomicBool::new(false));
        let before = crate::syncjobs::load_job_state(&saved.id).unwrap();
        assert_eq!(before.last_error.as_ref().unwrap().kind, expected);
        assert!(before.retry_at.is_none());
        let job = reload(&saved);
        assert_eq!(crate::syncjobs::load_job_state(&saved.id).unwrap(), before);
        assert_eq!(super::due::due_now(&job, &before, super::state::now_secs() + 86_400, None), None);
        let mut supervisor = super::job_supervisor::JobSupervisor::new();
        supervisor.enqueue_cause(&job, RunCause::Retry).unwrap();
        assert_eq!(supervisor.completion(&job.id), (AttemptOutcome::Cancelled, false));
        assert_eq!(crate::syncjobs::load_job_state(&saved.id).unwrap(), before);
    }
    let saved = SavedJob::old(&a, &b, "interval");
    let mut invalid = saved.load();
    invalid.filter_min_size_kb = 100;
    invalid.filter_max_size_kb = 1;
    super::job::run_one(&invalid, &AtomicBool::new(false));
    let before = crate::syncjobs::load_job_state(&saved.id).unwrap();
    assert_eq!(before.last_error.as_ref().unwrap().kind, FailureKind::Config);
    let job = reload(&saved);
    assert_eq!(crate::syncjobs::load_job_state(&saved.id).unwrap(), before);
    assert_eq!(super::due::due_now(&job, &before, super::state::now_secs() + 86_400, None), None);
}

#[test]
fn sync_reliability_task_old_jobs_loader_preserves_state_owner_and_safety_stops() {
    let (_temp, a, b) = local_pair();
    let saved = SavedJob::old(&a, &b, "interval");
    saved.load();
    let corrupt = b"{historical state is interrupted";
    std::fs::create_dir_all(saved.state_path().parent().unwrap()).unwrap();
    std::fs::write(saved.state_path(), corrupt).unwrap();
    let job = reload(&saved);
    let state = crate::syncjobs::load_job_state(&saved.id).unwrap();
    assert_eq!(std::fs::read(saved.state_path().with_extension("json.corrupt")).unwrap(), corrupt);
    assert_eq!(state.blocked.as_ref().unwrap().kind, BlockKind::Other);
    assert_eq!(super::due::due_now(&job, &state, super::state::now_secs(), None), None);
    let mut supervisor = super::job_supervisor::JobSupervisor::new();
    supervisor.enqueue_cause(&job, RunCause::Interval).unwrap();
    assert_eq!(supervisor.completion(&job.id), (AttemptOutcome::Cancelled, false));
    assert_eq!(crate::syncjobs::load_job_state(&saved.id).unwrap(), state);
    store_failure(&saved, "Sync-Jobdatei konnte nicht geladen werden: historical failure", FailureKind::Config, RunCause::Other, Runner::Daemon);
    reload(&saved);
    let protected = crate::syncjobs::load_job_state(&saved.id).unwrap();
    assert_eq!(protected.blocked, state.blocked);
    assert!(protected.last_error.is_none());
    assert_eq!(super::due::due_now(&job, &protected, super::state::now_secs(), None), None);

    let live = SavedJob::old(&a, &b, "interval");
    live.load();
    store_failure(&live, "Sync-Jobdatei konnte nicht geladen werden: historical failure", FailureKind::Config, RunCause::Other, Runner::Daemon);
    crate::syncjobs::update_job_state(&live.id, |state| {
        let now = super::state::now_secs();
        state.running = Some(crate::syncjobs::RunMark { runner: Runner::Desktop, cause: RunCause::Manual,
            started: now, alive: now, stalled_since: None });
        state.blocked = Some(Blocked { kind: BlockKind::ReplicaMissing { side: crate::syncjobs::JobSide::B },
            since: now, detail: "owner/replica stop".into(), confirmed: false });
    }).unwrap();
    let before = crate::syncjobs::load_job_state(&live.id).unwrap();
    reload(&live);
    assert_eq!(crate::syncjobs::load_job_state(&live.id).unwrap(), before);
}

#[test]
fn sync_reliability_task_old_jobs_transient_drive_identity_failure_keeps_retry() {
    let (_temp, _, b) = local_pair();
    let endpoints = EndpointFixtures::new(Vec::new());
    for message in [
        "Drive account identity request failed: HTTP status 429",
        "Drive account identity request failed: status code 503",
        "Drive account identity response is invalid: unexpected JSON",
        "Drive account identity response has no permissionId",
    ] {
        endpoints.insert_failure("gdrive:///retry", message);
        let saved = SavedJob::old("gdrive:///retry", &b, "interval");
        super::job::run_one(&saved.load(), &AtomicBool::new(false));
        let before = crate::syncjobs::load_job_state(&saved.id).unwrap();
        assert_eq!(before.last_error.as_ref().unwrap().kind, FailureKind::Unreachable);
        let job = reload(&saved);
        let state = crate::syncjobs::load_job_state(&saved.id).unwrap();
        assert_eq!(state, before);
        assert_eq!(super::due::due_now(&job, &state, state.retry_at.unwrap(), None), Some(RunCause::Retry));
    }
}
