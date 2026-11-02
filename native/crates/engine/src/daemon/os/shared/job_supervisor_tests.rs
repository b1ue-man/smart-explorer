use super::{EnqueueStatus, JobRunner, JobSupervisor, ThreadSpawner};
use crate::syncjobs::SyncJob;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn job(id: &str) -> SyncJob {
    let mut job = SyncJob::new(id.to_string(), "/source".into(), "/target".into());
    job.id = id.to_string();
    job
}

fn thread_spawner() -> ThreadSpawner {
    Arc::new(|name, task| std::thread::Builder::new().name(name).spawn(task))
}

fn poll_until_idle(supervisor: &mut JobSupervisor) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !supervisor.is_idle() && Instant::now() < deadline {
        assert!(supervisor.poll().is_empty());
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(supervisor.is_idle());
}

#[test]
fn serializes_all_daemon_jobs_globally() {
    let running = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let completed = Arc::new(AtomicUsize::new(0));
    let runner: JobRunner = {
        let running = running.clone();
        let maximum = maximum.clone();
        let completed = completed.clone();
        Arc::new(move |_, _| {
            let now = running.fetch_add(1, Ordering::SeqCst) + 1;
            maximum.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(20));
            running.fetch_sub(1, Ordering::SeqCst);
            completed.fetch_add(1, Ordering::SeqCst);
        })
    };
    let mut supervisor = JobSupervisor::with_hooks(runner, thread_spawner());

    assert_eq!(
        supervisor.enqueue(&job("one")).unwrap(),
        EnqueueStatus::Started
    );
    assert_eq!(
        supervisor.enqueue(&job("two")).unwrap(),
        EnqueueStatus::Queued
    );
    poll_until_idle(&mut supervisor);

    assert_eq!(completed.load(Ordering::SeqCst), 2);
    assert_eq!(maximum.load(Ordering::SeqCst), 1);
}

#[test]
fn stop_cancels_and_joins_active_job_without_starting_pending_work() {
    let started = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(AtomicBool::new(false));
    let runner: JobRunner = {
        let started = started.clone();
        let finished = finished.clone();
        Arc::new(move |_, cancel| {
            started.fetch_add(1, Ordering::SeqCst);
            while !cancel.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(1));
            }
            finished.store(true, Ordering::Release);
        })
    };
    let mut supervisor = JobSupervisor::with_hooks(runner, thread_spawner());
    supervisor.enqueue(&job("active")).unwrap();
    supervisor.enqueue(&job("pending")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    while started.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
        std::thread::yield_now();
    }

    assert!(supervisor.cancel_and_join().is_empty());
    assert!(finished.load(Ordering::Acquire));
    assert_eq!(started.load(Ordering::SeqCst), 1);
    assert!(supervisor.is_idle());
}

#[test]
fn spawn_failure_is_returned_and_does_not_reserve_the_job() {
    let runner: JobRunner = Arc::new(|_, _| {});
    let spawner: ThreadSpawner = Arc::new(|_, _| Err(io::Error::other("injected spawn failure")));
    let mut supervisor = JobSupervisor::with_hooks(runner, spawner);

    let error = supervisor.enqueue(&job("failed")).unwrap_err();
    assert!(error.contains("injected spawn failure"));
    assert!(supervisor.is_idle());
    assert_eq!(supervisor.enqueue(&job("failed")).unwrap_err(), error);
}

#[test]
fn completed_job_is_not_requeued_in_a_tight_loop() {
    let runner: JobRunner = Arc::new(|_, _| {});
    let mut supervisor = JobSupervisor::with_hooks(runner, thread_spawner());
    assert_eq!(
        supervisor.enqueue(&job("cooldown")).unwrap(),
        EnqueueStatus::Started
    );
    poll_until_idle(&mut supervisor);
    assert_eq!(
        supervisor.enqueue(&job("cooldown")).unwrap(),
        EnqueueStatus::RecentlyAttempted
    );
}

#[test]
fn android_task_cancel_jobs_stops_only_the_selected_work() {
    let started = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let runner: JobRunner = {
        let started = started.clone();
        Arc::new(move |job, cancel| {
            started.lock().unwrap().push(job.id.clone());
            if job.id == "active" {
                while !cancel.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        })
    };
    let mut supervisor = JobSupervisor::with_hooks(runner, thread_spawner());
    supervisor.enqueue(&job("active")).unwrap();
    supervisor.enqueue(&job("queued")).unwrap();
    supervisor.enqueue(&job("other")).unwrap();
    assert_eq!(supervisor.active_job_id(), Some("active"));
    assert_eq!(supervisor.active_job_name(), Some("active"));

    let selected = ["active", "queued"].map(String::from).into_iter().collect();
    supervisor.cancel_jobs(&selected);
    assert_eq!(supervisor.take_completed(), vec!["queued".to_string()]);
    poll_until_idle(&mut supervisor);

    assert_eq!(
        supervisor.take_completed(),
        vec!["active".to_string(), "other".to_string()]
    );
    assert_eq!(
        *started.lock().unwrap(),
        vec!["active".to_string(), "other".to_string()]
    );
    assert_eq!(
        supervisor.enqueue(&job("queued")).unwrap(),
        EnqueueStatus::RecentlyAttempted
    );
}

#[test]
fn review_task_queued_jobs_reload_the_current_configuration_and_skip_disabled_jobs() {
    let (entered_tx, entered_rx) = crossbeam_channel::bounded(1);
    let (release_tx, release_rx) = crossbeam_channel::bounded(1);
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let runner: JobRunner = {
        let seen = seen.clone();
        Arc::new(move |job, _| {
            if job.id == "active" {
                entered_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            } else {
                seen.lock()
                    .unwrap()
                    .push((job.name.clone(), job.target.clone()));
            }
        })
    };
    let jobs = Arc::new(std::sync::Mutex::new(vec![
        job("active"),
        job("queued"),
        job("disabled"),
    ]));
    let mut supervisor = JobSupervisor::with_hooks(runner, thread_spawner());
    supervisor.loader = {
        let jobs = jobs.clone();
        Arc::new(move |id| {
            Ok(jobs
                .lock()
                .unwrap()
                .iter()
                .find(|job| job.id == id)
                .cloned())
        })
    };
    supervisor.enqueue(&job("active")).unwrap();
    entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(
        supervisor.enqueue(&job("queued")).unwrap(),
        EnqueueStatus::Queued
    );
    assert_eq!(
        supervisor.enqueue(&job("disabled")).unwrap(),
        EnqueueStatus::Queued
    );
    {
        let mut jobs = jobs.lock().unwrap();
        jobs[1].name = "new name".into();
        jobs[1].target = "sftp://current/target".into();
        jobs[2].enabled = false;
    }
    release_tx.send(()).unwrap();
    poll_until_idle(&mut supervisor);
    assert_eq!(
        *seen.lock().unwrap(),
        vec![("new name".to_string(), "sftp://current/target".to_string())]
    );
    assert_eq!(
        supervisor.completion("disabled"),
        (crate::syncjobs::AttemptOutcome::Cancelled, false)
    );
}

#[test]
fn review_task_independent_jobs_overlap_with_a_bound_and_duplicate_ids_do_not() {
    let running = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let (release_tx, release_rx) = crossbeam_channel::bounded(2);
    let runner: JobRunner = {
        let running = running.clone();
        let maximum = maximum.clone();
        Arc::new(move |_, cancel| {
            let now = running.fetch_add(1, Ordering::SeqCst) + 1;
            maximum.fetch_max(now, Ordering::SeqCst);
            while !cancel.load(Ordering::Acquire) {
                if release_rx.recv_timeout(Duration::from_millis(10)).is_ok() {
                    break;
                }
            }
            running.fetch_sub(1, Ordering::SeqCst);
        })
    };
    let mut supervisor = JobSupervisor::with_hooks(runner, thread_spawner());
    supervisor.capacity = 2;
    supervisor.loader = Arc::new(|id| {
        let mut configured = job(id);
        configured.target = format!("/{id}/target");
        Ok(Some(configured))
    });
    assert_eq!(
        supervisor.enqueue(&job("one")).unwrap(),
        EnqueueStatus::Started
    );
    assert_eq!(
        supervisor.enqueue(&job("two")).unwrap(),
        EnqueueStatus::Started
    );
    assert_eq!(
        supervisor.enqueue(&job("one")).unwrap(),
        EnqueueStatus::AlreadyScheduled
    );
    assert_eq!(
        supervisor.enqueue(&job("three")).unwrap(),
        EnqueueStatus::Queued
    );
    let deadline = Instant::now() + Duration::from_secs(1);
    while maximum.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(maximum.load(Ordering::SeqCst), 2);
    release_tx.send(()).unwrap();
    release_tx.send(()).unwrap();
    supervisor.cancel_jobs(&["three".to_string()].into_iter().collect());
    poll_until_idle(&mut supervisor);
}

#[test]
fn review_task_equal_locator_pairs_wait_before_hooks_even_when_reversed() {
    let running = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let (entered_tx, entered_rx) = crossbeam_channel::bounded(2);
    let (release_tx, release_rx) = crossbeam_channel::bounded(1);
    let runner: JobRunner = {
        let running = running.clone();
        let maximum = maximum.clone();
        Arc::new(move |job, _| {
            maximum.fetch_max(running.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
            entered_tx.send(job.id.clone()).unwrap();
            if job.id == "one" {
                release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            }
            running.fetch_sub(1, Ordering::SeqCst);
        })
    };
    let mut supervisor = JobSupervisor::with_hooks(runner, thread_spawner());
    supervisor.capacity = 2;
    supervisor.loader = Arc::new(|id| {
        let mut configured = job(id);
        if id == "two" {
            std::mem::swap(&mut configured.source, &mut configured.target);
        }
        Ok(Some(configured))
    });
    supervisor.enqueue(&job("one")).unwrap();
    assert_eq!(
        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        "one"
    );
    assert_eq!(
        supervisor.enqueue(&job("two")).unwrap(),
        EnqueueStatus::Queued
    );
    assert!(supervisor.poll().is_empty());
    assert_eq!(supervisor.active_ids(), vec!["one"]);
    assert!(entered_rx.try_recv().is_err());
    release_tx.send(()).unwrap();
    poll_until_idle(&mut supervisor);
    assert_eq!(
        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        "two"
    );
    assert_eq!(maximum.load(Ordering::SeqCst), 1);
}

#[test]
fn review_task_admission_respects_new_errors_and_confirmations() {
    use crate::syncjobs::{JobState, RunCause};
    let mut state = JobState {
        consecutive_failures: 1,
        retry_at: Some(110),
        ..JobState::default()
    };
    assert!(!super::admission_allowed(&state, RunCause::Interval, 120));
    assert!(!super::admission_allowed(&state, RunCause::Retry, 100));
    assert!(super::admission_allowed(&state, RunCause::Retry, 110));
    state.last_error = Some(crate::syncjobs::JobError {
        kind: crate::syncjobs::FailureKind::Auth,
        message: "login".into(),
    });
    assert!(!super::admission_allowed(&state, RunCause::Retry, 200));
    state.retry_at = None;
    assert!(!super::admission_allowed(&state, RunCause::Retry, 200));
    state.pending_trigger = Some(crate::syncjobs::PendingTrigger {
        kind: crate::syncjobs::PendingKind::Confirmed,
        since: 200,
        volume: None,
    });
    assert!(super::admission_allowed(&state, RunCause::Confirmed, 200));
}
