//! Startup sequence and scheduling loop of the background worker, shared by
//! the desktop `--sync-daemon` process (`run_daemon` reads a pending handoff
//! from its environment) and the embedded worker thread (no handoff).

use crate::syncjobs::{SyncJob, Trigger};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use super::boot_marker;
use super::catch_up::CatchUpGate;
use super::handoff::{
    acquire_instance_guard, claim_handoff_after_singleton, discard_stop_after_singleton,
    stop_requested_checked_for, wait_for_handoff_activation,
};
use super::ipc::{start_listener, ShareHost};
use super::job_supervisor::{EnqueueStatus, JobSupervisor};
use super::live;
use super::schedule::{
    current_drives, drive_matches, local_root, new_generation, remote_change_token, tree_sig,
};
use super::state::{
    cadence_secs, clear_heartbeat, log, now_secs, pause_reason, paused, write_heartbeat,
    PauseReason,
};

const FALLBACK_TICK_SECS: u64 = 15;
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
    if let Err(e) = start_listener(share_host.clone()) {
        log(&format!("background worker IPC failed: {e}"));
        clear_heartbeat();
        return;
    }
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
    let mut job_supervisor = JobSupervisor::new();
    let mut sync_enabled = crate::autostart::is_enabled();

    // A daemon may have been started only for a Share session. Scheduled sync
    // work is permitted exclusively after the user enabled background sync.
    let startup_controls = scheduling_controls();
    if sync_enabled && startup_controls.permit_mutation {
        enqueue_startup_jobs(&mut job_supervisor, share_host.generation());
    }

    // Per-job real-time state and the last-seen drive set.
    let mut rt_sig: HashMap<String, String> = HashMap::new();
    let mut rt_dirty_since: HashMap<String, i64> = HashMap::new();
    let mut seen_drives = current_drives();

    loop {
        let controls = scheduling_controls();
        if stop_requested(share_host.generation()) {
            stop_daemon(&mut job_supervisor, &share_host);
            return;
        }
        let enabled_now = crate::autostart::is_enabled();
        if enabled_now != sync_enabled {
            sync_enabled = enabled_now;
            rt_sig.clear();
            rt_dirty_since.clear();
            seen_drives = current_drives();
            if sync_enabled {
                log("background sync enabled");
                if controls.permit_mutation {
                    enqueue_startup_jobs(&mut job_supervisor, share_host.generation());
                }
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
            stop_daemon(&mut job_supervisor, &share_host);
            return;
        }
        let now = now_secs();
        let configured_jobs = load_configured_jobs();

        if sync_enabled && controls.permit_mutation {
            // 1) Timer jobs (interval + calendar), gated by active-hours in due().
            for job in configured_jobs.iter().filter(|j| j.due(now)) {
                enqueue_job(&mut job_supervisor, job, share_host.generation());
                if stop_requested(share_host.generation()) {
                    break;
                }
            }
            // 2) Real-time jobs: watch local endpoints, run after the change settles.
            enqueue_realtime_jobs(
                &mut job_supervisor,
                &configured_jobs,
                now,
                share_host.generation(),
                &mut rt_sig,
                &mut rt_dirty_since,
            );
            // 3) On-connect jobs: run when a matching removable drive appears.
            enqueue_connect_jobs(
                &mut job_supervisor,
                &configured_jobs,
                now,
                share_host.generation(),
                &mut seen_drives,
            );
        }
        service_catch_up(&mut job_supervisor, sync_enabled, controls.permit_mutation);

        write_heartbeat();
        // Sleep one tick in 2 s slices so a stop request is honoured promptly.
        let tick = controls.tick_secs;
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
            write_heartbeat();
            slept += 2;
        }
    }
}

fn enqueue_realtime_jobs(
    supervisor: &mut JobSupervisor,
    configured_jobs: &[SyncJob],
    now: i64,
    generation: &str,
    rt_sig: &mut HashMap<String, String>,
    rt_dirty_since: &mut HashMap<String, i64>,
) {
    for job in configured_jobs
        .iter()
        .filter(|j| j.enabled && j.trigger == Trigger::RealTime && j.active_now(now))
    {
        let roots: Vec<std::path::PathBuf> = [&job.source, &job.target]
            .iter()
            .filter_map(|e| local_root(e))
            .collect();
        let remote_token = remote_change_token(job);
        if roots.is_empty() && remote_token.is_none() {
            continue; // nothing watchable
        }
        let sig = roots.iter().fold((0u64, 0i64, 0u64), |a, r| {
            let s = tree_sig(r);
            (a.0 + s.0, a.1.max(s.1), a.2 + s.2)
        });
        let sig = format!(
            "{}:{}:{}:{}",
            sig.0,
            sig.1,
            sig.2,
            remote_token.as_deref().unwrap_or("")
        );
        match rt_sig.get(&job.id) {
            Some(prev) if prev == &sig => {
                // Unchanged since last tick - run if a pending change has settled.
                if let Some(&since) = rt_dirty_since.get(&job.id) {
                    if now - since >= job.rt_debounce_secs as i64 {
                        enqueue_job(supervisor, job, generation);
                        rt_dirty_since.remove(&job.id);
                    }
                }
            }
            Some(_) => {
                // Changed this tick - (re)start the settle timer.
                rt_dirty_since.insert(job.id.clone(), now);
                rt_sig.insert(job.id.clone(), sig);
            }
            None => {
                // First sighting - record baseline, don't run.
                rt_sig.insert(job.id.clone(), sig);
            }
        }
    }
}

fn enqueue_connect_jobs(
    supervisor: &mut JobSupervisor,
    configured_jobs: &[SyncJob],
    now: i64,
    generation: &str,
    seen_drives: &mut HashSet<String>,
) {
    let drives = current_drives();
    if drives != *seen_drives {
        for d in drives.difference(seen_drives) {
            for job in configured_jobs
                .iter()
                .filter(|j| j.enabled && j.trigger == Trigger::OnConnect && j.active_now(now))
            {
                if drive_matches(&job.connect_match, d) {
                    log(&format!("device connected → '{}'", job.name));
                    enqueue_job(supervisor, job, generation);
                }
            }
        }
        *seen_drives = drives;
    }
}

fn enqueue_startup_jobs(supervisor: &mut JobSupervisor, generation: &str) {
    // The embedded worker restarts with its app process; its startup pass
    // follows device boots instead.
    if live::is_embedded() && !boot_marker::claim_startup_pass() {
        return;
    }
    for job in load_configured_jobs()
        .into_iter()
        .filter(|job| job.enabled && job.trigger == Trigger::OnStartup)
    {
        if stop_requested(generation) || !crate::autostart::is_enabled() {
            break;
        }
        enqueue_job(supervisor, &job, generation);
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

struct SchedulingControls {
    permit_mutation: bool,
    tick_secs: u64,
}

fn scheduling_controls() -> SchedulingControls {
    let tick_secs = match cadence_secs() {
        Ok(value) => value,
        Err(error) => {
            log(&format!(
                "scheduled sync blocked: cadence control could not be read: {error}"
            ));
            return SchedulingControls {
                permit_mutation: false,
                tick_secs: FALLBACK_TICK_SECS,
            };
        }
    };
    match paused() {
        Ok(is_paused) => SchedulingControls {
            permit_mutation: !is_paused,
            tick_secs,
        },
        Err(error) => {
            log(&format!(
                "scheduled sync blocked: pause control could not be read: {error}"
            ));
            SchedulingControls {
                permit_mutation: false,
                tick_secs,
            }
        }
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

fn load_configured_jobs() -> Vec<SyncJob> {
    match crate::syncjobs::load() {
        Ok(jobs) => jobs,
        Err(error) => {
            log(&format!(
                "scheduled sync blocked: saved jobs could not be loaded: {error}"
            ));
            Vec::new()
        }
    }
}

fn enqueue_job(supervisor: &mut JobSupervisor, job: &SyncJob, generation: &str) {
    if stop_requested(generation) {
        return;
    }
    match supervisor.enqueue(job) {
        Ok(EnqueueStatus::Started | EnqueueStatus::Queued) => {
            log(&format!("job queued '{}'", job.name));
        }
        Ok(EnqueueStatus::AlreadyScheduled | EnqueueStatus::RecentlyAttempted) => {}
        Err(error) => log(&error),
    }
}

fn stop_daemon(supervisor: &mut JobSupervisor, share_host: &ShareHost) {
    log("daemon stopping (stop requested or unreadable stop control)");
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
