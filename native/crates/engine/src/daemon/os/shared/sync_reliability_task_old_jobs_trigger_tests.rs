//! C08 durable triggers and catch-up use the real saved-job supervisor.
use super::sync_reliability_task_old_jobs_tests::{local_pair, reload, SavedJob};
use crate::syncjobs::{AttemptOutcome, AttemptReport, PendingKind, RunCause, Runner, Trigger};

#[test]
fn sync_reliability_task_old_jobs_trigger_classes_keep_durable_wakeups() {
    let (_temp, a, b) = local_pair();
    let now = super::state::now_secs();
    let mut jobs = Vec::new();
    let saved: Vec<_> = ["interval", "calendar", "realtime", "onstartup", "onconnect"]
        .into_iter()
        .map(|trigger| SavedJob::old(&a, &b, trigger))
        .collect();
    for fixture in &saved {
        let job = fixture.load();
        let mut state = crate::syncjobs::load_job_state(&job.id).unwrap();
        match job.trigger {
            Trigger::Interval => assert_eq!(
                super::due::due_now(&job, &state, now, None),
                Some(RunCause::Interval)
            ),
            Trigger::Calendar => assert_eq!(
                super::due::due_now(&job, &state, now, None),
                Some(RunCause::Calendar)
            ),
            Trigger::RealTime => {
                assert!(super::job_triggers::persist_change(&job.id, now - 10));
                state = crate::syncjobs::load_job_state(&job.id).unwrap();
                assert_eq!(super::due::next_due(&job, &state, now), Some(now));
                let filter = super::job_triggers::filter(&job, false).unwrap();
                assert!(!filter.admits(&crate::watch::WatchEntry {
                    rel: "file.skip",
                    is_dir: None
                }));
                assert!(filter.admits(&crate::watch::WatchEntry {
                    rel: "node_modules/ordinary",
                    is_dir: None
                }));
            }
            Trigger::OnStartup | Trigger::OnConnect => {
                let (kind, cause) = if job.trigger == Trigger::OnStartup {
                    (PendingKind::Startup, RunCause::Startup)
                } else {
                    (PendingKind::Connect, RunCause::Connect)
                };
                let volume = (kind == PendingKind::Connect).then(|| {
                    serde_json::to_string(&(a.clone(), "C08 disk", "C08 volume")).unwrap()
                });
                if let Some(volume) = &volume {
                    assert!(super::connect_triggers::matches(&job, volume));
                    let remote = crate::syncjobs::SyncJob::new(
                        "remote".into(),
                        "sftp://host/source".into(),
                        "gdrive:///source".into(),
                    );
                    assert!(!super::connect_triggers::matches(&remote, volume));
                }
                assert!(super::job_triggers::persist(
                    &job.id,
                    kind,
                    now - 10,
                    volume
                ));
                state = crate::syncjobs::load_job_state(&job.id).unwrap();
                assert_eq!(super::due::due_now(&job, &state, now, None), Some(cause));
                crate::syncjobs::record_attempt(
                    &job.id,
                    &AttemptReport {
                        runner: Runner::Daemon,
                        cause,
                        started: now,
                        finished: now,
                        outcome: AttemptOutcome::Cancelled,
                        result: None,
                    },
                )
                .unwrap();
                assert_eq!(
                    crate::syncjobs::load_job_state(&job.id)
                        .unwrap()
                        .pending_trigger,
                    state.pending_trigger
                );
                reload(fixture);
                assert_eq!(
                    super::due::due_now(
                        &job,
                        &crate::syncjobs::load_job_state(&job.id).unwrap(),
                        now,
                        None
                    ),
                    Some(cause)
                );
            }
            _ => unreachable!(),
        }
        jobs.push(job);
    }
    let selected = super::catch_up::select_catch_up_jobs(&jobs, now);
    assert_eq!(
        selected.iter().map(|job| job.trigger).collect::<Vec<_>>(),
        [Trigger::Interval, Trigger::Calendar, Trigger::RealTime]
    );
    assert!(!super::boot_marker::startup_pass_due(
        Some("same-session"),
        Some("same-session")
    ));
    assert!(super::boot_marker::startup_pass_due(
        Some("new-session"),
        Some("same-session")
    ));
}

#[test]
fn sync_reliability_task_old_jobs_catch_up_finishes_real_saved_job_bytes() {
    let (_temp, a, b) = local_pair();
    std::fs::write(
        format!("{a}/catch-up.txt"),
        b"normal saved job via real catch-up queue",
    )
    .unwrap();
    let saved = SavedJob::old(&a, &b, "interval");
    let jobs = Ok(vec![saved.load()]);
    let mut supervisor = super::job_supervisor::JobSupervisor::new();
    let mut book = super::catch_up::CatchUpBook::new();
    let id = book.request().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !book.status(id).unwrap().finished && std::time::Instant::now() < deadline {
        assert!(supervisor.poll().is_empty());
        book.service(
            &mut supervisor,
            &super::catch_up::CatchUpGate::Open,
            Some(&jobs),
            super::state::now_secs(),
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(book.status(id).unwrap().finished);
    assert_eq!(book.status(id).unwrap().failed, 0);
    assert_eq!(book.status(id).unwrap().admitted, 1);
    assert_eq!(
        std::fs::read(format!("{b}/catch-up.txt")).unwrap(),
        b"normal saved job via real catch-up queue"
    );
    assert_eq!(
        crate::syncjobs::load_job_state(&saved.id)
            .unwrap()
            .last_cause,
        Some(RunCause::CatchUp)
    );
}
