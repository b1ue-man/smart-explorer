use crate::bisync::{DeletePolicy, Direction};
use crate::syncjobs::{SyncJob, Trigger};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use super::ipc::{start_listener, ShareHost};
use super::job_supervisor::{EnqueueStatus, JobSupervisor};
use super::platform;
use super::state::{
    cadence_secs, clear_heartbeat, clear_stop, log, now_secs, paused, stop_requested_checked,
    write_heartbeat,
};

const FALLBACK_TICK_SECS: u64 = 15;

/// A cheap signature of a local subtree: (file count, newest mtime ms, total
/// size). Any add/modify/delete changes at least one component.
fn tree_sig(root: &std::path::Path) -> (u64, i64, u64) {
    let mut count = 0u64;
    let mut newest = 0i64;
    let mut bytes = 0u64;
    let mut stack = vec![root.to_path_buf()];
    let mut budget = 1_000_000u32; // bounded hint; the sync engine does a full checked walk
    while let Some(d) = stack.pop() {
        let rd = match std::fs::read_dir(&d) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for e in rd.flatten() {
            if budget == 0 {
                return (count, newest, bytes);
            }
            budget -= 1;
            let md = match std::fs::symlink_metadata(e.path()) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if platform::metadata_is_link_like(&md) {
                continue;
            }
            if md.is_dir() {
                stack.push(e.path());
            } else {
                count = count.saturating_add(1);
                bytes = bytes.saturating_add(md.len());
                if let Ok(t) = md.modified() {
                    if let Ok(d) = t.duration_since(std::time::UNIX_EPOCH) {
                        let ms = i64::try_from(d.as_millis()).unwrap_or(i64::MAX);
                        if ms > newest {
                            newest = ms;
                        }
                    }
                }
            }
        }
    }
    (count, newest, bytes)
}

/// Local filesystem root of an endpoint string, if it is a local path (not a
/// remote URL we can't watch).
fn local_root(endpoint: &str) -> Option<std::path::PathBuf> {
    if endpoint.contains("://") {
        return None;
    }
    let p = std::path::PathBuf::from(endpoint);
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

/// Lightweight remote-change signal for realtime mirror jobs. It is only a
/// wakeup hint; the sync engine still validates the persisted cursor/state.
fn remote_change_token(job: &SyncJob) -> Option<String> {
    if job.delete_policy != DeletePolicy::Mirror {
        return None;
    }
    let endpoint = match job.direction {
        Direction::AtoB => &job.source,
        Direction::BtoA => &job.target,
        Direction::Both => return None,
    };
    if local_root(endpoint).is_some() {
        return None;
    }
    let (backend, root) = crate::connect::resolve_endpoint(endpoint).ok()?;
    if !backend.supports_changes() {
        return None;
    }
    backend.current_change_cursor(&root).ok().flatten()
}

/// Set of currently-present removable-drive descriptors ("LETTER|LABEL|SERIAL").
fn current_drives() -> HashSet<String> {
    platform::removable_drives()
        .into_iter()
        .map(|d| format!("{}|{}|{}", d.letter, d.label, d.serial))
        .collect()
}

/// Does a drive descriptor match a job's `connect_match` (empty = any removable;
/// otherwise a case-insensitive `*?` wildcard tested against letter, label and
/// serial)?
pub(crate) fn drive_matches(pat: &str, descriptor: &str) -> bool {
    let pat = pat.trim();
    if pat.is_empty() {
        return true;
    }
    let parts: Vec<&str> = descriptor.split('|').collect();
    parts.iter().any(|p| wildcard_ci(pat, p))
}

/// Minimal case-insensitive glob (`*` and `?`).
pub(crate) fn wildcard_ci(pat: &str, s: &str) -> bool {
    let p: Vec<char> = pat.to_lowercase().chars().collect();
    let t: Vec<char> = s.to_lowercase().chars().collect();
    fn m(p: &[char], t: &[char]) -> bool {
        match p.first() {
            None => t.is_empty(),
            Some('*') => m(&p[1..], t) || (!t.is_empty() && m(p, &t[1..])),
            Some('?') => !t.is_empty() && m(&p[1..], &t[1..]),
            Some(&c) => !t.is_empty() && t[0] == c && m(&p[1..], &t[1..]),
        }
    }
    m(&p, &t)
}

/// The headless loop.
pub fn run_daemon() {
    let Some(_instance_guard) = platform::acquire_daemon_instance_guard(Duration::from_secs(20))
    else {
        return;
    };
    if let Err(error) = clear_stop() {
        log(&format!(
            "daemon refused to start: stop control could not be cleared: {error}"
        ));
        return;
    }
    log("daemon started");
    write_heartbeat();
    let share_host = ShareHost::new();
    if let Err(e) = start_listener(share_host.clone()) {
        log(&format!("background worker IPC failed: {e}"));
    }
    // Publish the lightweight control plane before starting Iroh. A terminal
    // client can now observe the daemon immediately even when relay discovery
    // makes the initial Share load take several seconds.
    if let Err(error) = share_host.reload_now() {
        log(&format!("share worker initial load failed: {error}"));
    }
    let mut job_supervisor = JobSupervisor::new();
    let mut sync_enabled = crate::autostart::is_enabled();

    // A daemon may have been started only for a Share session. Scheduled sync
    // work is permitted exclusively after the user enabled background sync.
    let startup_controls = scheduling_controls();
    if sync_enabled && startup_controls.permit_mutation {
        enqueue_startup_jobs(&mut job_supervisor);
    }

    // Per-job real-time state and the last-seen drive set.
    let mut rt_sig: HashMap<String, String> = HashMap::new();
    let mut rt_dirty_since: HashMap<String, i64> = HashMap::new();
    let mut seen_drives = current_drives();

    loop {
        let controls = scheduling_controls();
        if stop_requested() {
            stop_daemon(&mut job_supervisor);
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
                    enqueue_startup_jobs(&mut job_supervisor);
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
        if stop_requested() {
            stop_daemon(&mut job_supervisor);
            return;
        }
        let now = now_secs();
        let configured_jobs = load_configured_jobs();

        if sync_enabled && controls.permit_mutation {
            // 1) Timer jobs (interval + calendar), gated by active-hours in due().
            for job in configured_jobs.iter().filter(|j| j.due(now)) {
                enqueue_job(&mut job_supervisor, job);
                if stop_requested() {
                    break;
                }
            }

            // 2) Real-time jobs: watch local endpoints, run after the change settles.
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
                                enqueue_job(&mut job_supervisor, job);
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

            // 3) On-connect jobs: run when a matching removable drive appears.
            let drives = current_drives();
            if drives != seen_drives {
                for d in drives.difference(&seen_drives) {
                    for job in configured_jobs.iter().filter(|j| {
                        j.enabled && j.trigger == Trigger::OnConnect && j.active_now(now)
                    }) {
                        if drive_matches(&job.connect_match, d) {
                            log(&format!("device connected → '{}'", job.name));
                            enqueue_job(&mut job_supervisor, job);
                        }
                    }
                }
                seen_drives = drives;
            }
        }

        write_heartbeat();
        // Sleep one tick in 2 s slices so a stop request is honoured promptly.
        let tick = controls.tick_secs;
        let mut slept = 0;
        while slept < tick {
            if stop_requested() {
                break;
            }
            if crate::autostart::is_enabled() != sync_enabled {
                break;
            }
            std::thread::sleep(Duration::from_secs(2));
            if stop_requested() {
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
            share_host.tick();
            write_heartbeat();
            slept += 2;
        }
    }
}

fn enqueue_startup_jobs(supervisor: &mut JobSupervisor) {
    for job in load_configured_jobs()
        .into_iter()
        .filter(|job| job.enabled && job.trigger == Trigger::OnStartup)
    {
        if stop_requested() || !crate::autostart::is_enabled() {
            break;
        }
        enqueue_job(supervisor, &job);
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

fn stop_requested() -> bool {
    match stop_requested_checked() {
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

fn enqueue_job(supervisor: &mut JobSupervisor, job: &SyncJob) {
    if stop_requested() {
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

fn stop_daemon(supervisor: &mut JobSupervisor) {
    log("daemon stopping (stop requested or unreadable stop control)");
    for error in supervisor.cancel_and_join() {
        log(&error);
    }
    if let Err(error) = clear_stop() {
        log(&format!("stop control could not be cleared: {error}"));
    }
    clear_heartbeat();
}

fn poll_jobs(supervisor: &mut JobSupervisor) {
    for error in supervisor.poll() {
        log(&error);
    }
}
