//! Startup sequence and scheduling loop of the background worker, shared by
//! the desktop `--sync-daemon` process (`run_daemon` reads a pending handoff
//! from its environment) and the embedded worker thread (no handoff).

use crate::syncjobs::{PendingKind, RunCause, SyncJob};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use super::boot_marker;
use super::catch_up::CatchUpGate;
use super::handoff::{
    acquire_instance_guard, claim_handoff_after_singleton, discard_stop_after_singleton,
    stop_requested_checked_for, wait_for_handoff_activation,
};
use super::ipc::{start_listener, ShareHost};
use super::ipc_listener::IpcListener;
use super::job_supervisor::{EnqueueStatus, JobSupervisor};
use super::live;
use super::schedule::new_generation;
use super::state::{
    clear_heartbeat, log, now_secs, pause_reason, scheduling_controls, write_heartbeat, PauseReason,
};

const HANDOFF_ACTIVATION_TIMEOUT: Duration = Duration::from_secs(2);
const DAEMON_HANDOFF_TIMEOUT: Duration = Duration::from_secs(300);

/// A version-upgrade handoff of the desktop worker process: the replacement's
/// own generation and the one it retires (both validated by the caller).
pub(crate) struct Handoff {
    pub(crate) generation: String,
    pub(crate) retiring_generation: Option<String>,
}

/// The headless loop. `None` starts a fresh generation; `Some` first waits for
/// the retiring instance to activate and release the singleton.
pub(crate) fn run_daemon_with(handoff: Option<Handoff>) {
    let (generation, retiring_generation, handoff) = match handoff {
        Some(Handoff {
            generation,
            retiring_generation,
        }) => (generation, retiring_generation, true),
        None => match new_generation() {
            Ok(generation) => (generation, None, false),
            Err(error) => {
                log(&format!("daemon generation failed: {error}"));
                return;
            }
        },
    };
    if handoff && !wait_for_handoff_activation(&generation, HANDOFF_ACTIVATION_TIMEOUT) {
        return;
    }
    let Some(_instance_guard) = acquire_instance_guard(
        handoff,
        &generation,
        retiring_generation.as_deref(),
        DAEMON_HANDOFF_TIMEOUT,
    ) else {
        return;
    };
    if handoff {
        match claim_handoff_after_singleton(&generation) {
            Ok(true) => {}
            Ok(false) => {
                log("daemon handoff was superseded before singleton acquisition");
                return;
            }
            Err(error) => {
                log(&format!(
                    "daemon handoff control could not be verified: {error}"
                ));
                return;
            }
        }
    } else if let Err(error) = discard_stop_after_singleton() {
        log(&format!(
            "daemon refused to start: stop control could not be cleared: {error}"
        ));
        return;
    }
    log("daemon started");
    write_heartbeat();
    let share_host = ShareHost::new(generation);
    // Dropping the handle (any return, even a panic) ends the accept loop.
    let listener = match start_listener(share_host.clone()) {
        Ok(listener) => listener,
        Err(e) => {
            log(&format!("background worker IPC failed: {e}"));
            clear_heartbeat();
            return;
        }
    };
    // Publish the lightweight control plane before starting Iroh. A terminal
    // client can now observe the daemon immediately even when relay discovery
    // makes the initial Share load take several seconds.
    if let Err(error) = share_host.reload_now() {
        log(&format!("share worker initial load failed: {error}"));
    }
    // Ping remains available during initialization, but clients only accept
    // this generation as ready after all synchronous identity/profile loading
    // has released the Share host state.
    share_host.mark_initialized();
    // In-process callers treat a published generation like a ready Ping.
    let _serving = live::serve(&share_host);
    // The embedded worker beats once per tick; the desktop process keeps its
    // 2 s beat for the GUI status line.
    let embedded = live::is_embedded();
    let mut job_supervisor = JobSupervisor::new();
    let mut sync_enabled = crate::autostart::is_enabled();

    let mut realtime = super::realtime::Realtime::new();
    let mut connections = super::connect_triggers::Connections::new();
    let mut broken = HashMap::new();
    let mut timer_since = None;

    loop {
        let controls = scheduling_controls();
        if stop_requested(share_host.generation()) {
            stop_daemon(&mut job_supervisor, &share_host, &listener);
            return;
        }
        let enabled_now = crate::autostart::is_enabled();
        if enabled_now != sync_enabled {
            sync_enabled = enabled_now;
            connections = super::connect_triggers::Connections::new();
            if sync_enabled {
                log("background sync enabled");
            } else {
                log("background sync disabled; canceling scheduled work");
                for error in job_supervisor.cancel_and_join() {
                    log(&error);
                }
            }
        }
        if sync_enabled && controls.permit_mutation {
            poll_jobs(&mut job_supervisor);
        } else if sync_enabled {
            for error in job_supervisor.cancel_and_join() {
                log(&error);
            }
        }
        share_host.tick();
        if stop_requested(share_host.generation()) {
            stop_daemon(&mut job_supervisor, &share_host, &listener);
            return;
        }
        let now = now_secs();
        let configured_jobs = load_configured_jobs(&mut broken);
        let stale: HashSet<String> = job_supervisor
            .active_ids()
            .into_iter()
            .filter(|id| {
                !configured_jobs
                    .iter()
                    .any(|job| job.id == *id && job.enabled)
            })
            .map(str::to_string)
            .collect();
        job_supervisor.cancel_jobs(&stale);
        realtime.refresh(&configured_jobs, sync_enabled);
        if sync_enabled {
            boot_marker::register_startup(&configured_jobs);
            connections.poll(&configured_jobs, now);
        }
        let events = realtime.ready(&configured_jobs);

        // A host that defers scheduling (Android while its own periodic
        // worker owns background runs) holds these enqueues only; running
        // jobs and catch-up runs continue.
        if controls.may_schedule(sync_enabled) {
            let states = crate::syncjobs::load_job_states(&configured_jobs);
            for job in &configured_jobs {
                let Some(state) = states
                    .get(&job.id)
                    .filter(|state| state.load_error.is_none())
                else {
                    continue;
                };
                let mut cause = super::due::due_now(job, state, now, timer_since);
                if cause.is_none()
                    && state.blocked.is_none()
                    && state.consecutive_failures == 0
                    && job.enabled
                    && job.active_now(now)
                {
                    cause = events.get(&job.id).copied();
                }
                if cause.is_none()
                    && state.blocked.is_none()
                    && state.consecutive_failures == 0
                    && job.enabled
                    && job.active_now(now)
                    && job.trigger == crate::syncjobs::Trigger::RealTime
                    && job.verify_interval_secs > 0
                    && state.last_verify.map_or(true, |at| {
                        now.saturating_sub(at)
                            >= i64::try_from(job.verify_interval_secs).unwrap_or(i64::MAX)
                    })
                {
                    super::job_triggers::persist(&job.id, PendingKind::Verify, now, None);
                    cause = Some(RunCause::Verify);
                }
                if let Some(cause) = cause {
                    enqueue_job(&mut job_supervisor, job, cause, share_host.generation());
                }
            }
            timer_since = Some(now);
        }
        service_catch_up(&mut job_supervisor, sync_enabled, controls.permit_mutation);

        super::problem_notify::notify(&configured_jobs, now);
        write_heartbeat();
        // Sleep one tick in 2 s slices so a stop request is honoured promptly.
        let tick = if sync_enabled {
            controls.tick_secs.min(2)
        } else {
            controls.tick_secs
        };
        let mut slept = 0;
        while slept < tick {
            if stop_requested(share_host.generation()) {
                break;
            }
            if crate::autostart::is_enabled() != sync_enabled {
                break;
            }
            std::thread::sleep(Duration::from_secs(2));
            if stop_requested(share_host.generation()) {
                break;
            }
            // A toggle during the sleep is applied by the outer loop first, so
            // a pending catch-up is never judged by the stale state.
            if crate::autostart::is_enabled() != sync_enabled {
                break;
            }
            let live_controls = scheduling_controls();
            if sync_enabled && !live_controls.permit_mutation {
                for error in job_supervisor.cancel_and_join() {
                    log(&error);
                }
                break;
            }
            if live_controls.tick_secs != tick
                || live_controls.permit_mutation != controls.permit_mutation
                || live_controls.defer_scheduling != controls.defer_scheduling
            {
                break;
            }
            if sync_enabled && live_controls.permit_mutation {
                poll_jobs(&mut job_supervisor);
            }
            service_catch_up(
                &mut job_supervisor,
                sync_enabled,
                live_controls.permit_mutation,
            );
            share_host.tick();
            if !embedded {
                write_heartbeat();
            }
            slept += 2;
        }
    }
}

fn service_catch_up(supervisor: &mut JobSupervisor, sync_enabled: bool, permit_mutation: bool) {
    live::service_catch_up(supervisor, || {
        if !sync_enabled {
            CatchUpGate::Closed("Hintergrund-Sync ist ausgeschaltet".into())
        } else if permit_mutation {
            CatchUpGate::Open
        } else {
            CatchUpGate::Closed(blocked_message())
        }
    });
}

fn blocked_message() -> String {
    match pause_reason() {
        Ok(Some(PauseReason::Manual)) => "Hintergrund-Sync ist pausiert".into(),
        Ok(Some(PauseReason::BatterySaver)) => "Automatische Pause: Energiesparmodus".into(),
        Ok(Some(PauseReason::MeteredNetwork)) => "Automatische Pause: getaktetes Netz".into(),
        Ok(None) => "Hintergrund-Sync ist blockiert (siehe Worker-Protokoll)".into(),
        Err(error) => format!("Pausenstatus nicht lesbar: {error}"),
    }
}

fn stop_requested(generation: &str) -> bool {
    match stop_requested_checked_for(generation) {
        Ok(requested) => requested,
        Err(error) => {
            log(&format!(
                "daemon stopping: stop control could not be read safely: {error}"
            ));
            true
        }
    }
}

fn load_configured_jobs(broken: &mut HashMap<String, String>) -> Vec<SyncJob> {
    match crate::syncjobs::load_report() {
        Ok(report) => {
            let mut invalid_notices = Vec::new();
            for invalid in report.broken {
                let mut notice_job = SyncJob::new(invalid.id.clone(), String::new(), String::new());
                notice_job.id = invalid.id.clone();
                invalid_notices.push(notice_job);
                if broken.get(&invalid.id) == Some(&invalid.error) {
                    continue;
                }
                log(&format!(
                    "job '{}' cannot be loaded: {}",
                    invalid.id, invalid.error
                ));
                let mut job = SyncJob::new(invalid.id.clone(), String::new(), String::new());
                job.id = invalid.id.clone();
                super::job::persist(
                    &job,
                    now_secs(),
                    RunCause::Other,
                    crate::syncjobs::AttemptOutcome::Failed(crate::syncjobs::JobError {
                        kind: crate::syncjobs::FailureKind::Config,
                        message: invalid.error.clone(),
                    }),
                    crate::syncjobs::JobResult {
                        when: now_secs(),
                        errors: 1,
                        note: invalid.error.clone(),
                        ..Default::default()
                    },
                );
                broken.insert(invalid.id, invalid.error);
            }
            super::problem_notify::notify(&invalid_notices, now_secs());
            for job in &report.jobs {
                if crate::syncjobs::load_job_state(&job.id)
                    .ok()
                    .is_some_and(|state| state.load_error.is_some())
                {
                    // Store/quarantine once, retaining an explicit safety stop
                    // rather than inferring that the old state had no block.
                    if let Err(error) = crate::syncjobs::update_job_state(&job.id, |_| {}) {
                        log(&format!(
                            "job state '{}' cannot recover safely: {error}",
                            job.id
                        ));
                    }
                }
                if let Some(previous) = broken.remove(&job.id) {
                    let _ = crate::syncjobs::update_job_state(&job.id, |state| {
                        if state.last_error.as_ref().is_some_and(|error| {
                            error.kind == crate::syncjobs::FailureKind::Config
                                && error.message == previous
                        }) {
                            state.last_error = None;
                            state.consecutive_failures = 0;
                            state.retry_at = None;
                        }
                    });
                }
            }
            report.jobs
        }
        Err(error) => {
            log(&format!(
                "scheduled sync blocked: jobs cannot be loaded: {error}"
            ));
            Vec::new()
        }
    }
}

fn enqueue_job(supervisor: &mut JobSupervisor, job: &SyncJob, cause: RunCause, generation: &str) {
    if stop_requested(generation) {
        return;
    }
    if matches!(cause, RunCause::Interval | RunCause::Calendar) {
        super::job_triggers::persist(&job.id, PendingKind::Other, now_secs(), None);
    }
    match supervisor.enqueue_cause(job, cause) {
        Ok(EnqueueStatus::Started | EnqueueStatus::Queued) => {
            log(&format!("job queued '{}'", job.name));
        }
        Ok(EnqueueStatus::AlreadyScheduled | EnqueueStatus::RecentlyAttempted) => {}
        Err(error) => log(&error),
    }
}

fn stop_daemon(supervisor: &mut JobSupervisor, share_host: &ShareHost, listener: &IpcListener) {
    log("daemon stopping (stop requested or unreadable stop control)");
    // No new IPC client is admitted while the worker winds down.
    listener.shutdown();
    share_host.shutdown_lan();
    share_host.stop_mounts();
    for error in supervisor.cancel_and_join() {
        log(&error);
    }
    clear_heartbeat();
}

fn poll_jobs(supervisor: &mut JobSupervisor) {
    for error in supervisor.poll() {
        log(&error);
    }
}
