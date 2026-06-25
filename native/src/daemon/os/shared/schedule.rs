use crate::syncjobs::{SyncJob, Trigger};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

use super::ipc::{start_listener, ShareHost};
use super::job::run_one;
use super::platform;
use super::state::{
    cadence_secs, clear_heartbeat, clear_stop, log, now_secs, paused, stop_requested,
    write_heartbeat,
};

/// A cheap signature of a local subtree: (file count, newest mtime ms, total
/// size). Any add/modify/delete changes at least one component.
fn tree_sig(root: &std::path::Path) -> (u64, i64, u64) {
    let mut count = 0u64;
    let mut newest = 0i64;
    let mut bytes = 0u64;
    let mut stack = vec![root.to_path_buf()];
    let mut budget = 200_000u32; // guard against pathological trees
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
            let md = match e.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            if md.is_dir() {
                stack.push(e.path());
            } else {
                count += 1;
                bytes += md.len();
                if let Ok(t) = md.modified() {
                    if let Ok(d) = t.duration_since(std::time::UNIX_EPOCH) {
                        let ms = d.as_millis() as i64;
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
    let Some(_instance_guard) = acquire_daemon_instance_guard(Duration::from_secs(20)) else {
        return;
    };
    clear_stop();
    log("daemon started");
    write_heartbeat();
    let share_host = ShareHost::new();
    if let Err(e) = start_listener(share_host.clone()) {
        log(&format!("background worker IPC failed: {e}"));
    }
    let (job_done_tx, job_done_rx) = channel::<String>();
    let mut active_jobs: HashSet<String> = HashSet::new();

    // On-startup jobs run once now (subject to pause).
    if !paused() {
        for job in crate::syncjobs::load()
            .into_iter()
            .filter(|j| j.enabled && j.trigger == Trigger::OnStartup)
        {
            if stop_requested() {
                break;
            }
            spawn_job(&job, &mut active_jobs, &job_done_tx);
        }
    }

    // Per-job real-time state and the last-seen drive set.
    let mut rt_sig: HashMap<String, (u64, i64, u64)> = HashMap::new();
    let mut rt_dirty_since: HashMap<String, i64> = HashMap::new();
    let mut seen_drives = current_drives();

    loop {
        drain_finished_jobs(&mut active_jobs, &job_done_rx);
        share_host.tick();
        if stop_requested() {
            clear_stop();
            log("daemon stopping (stop requested)");
            clear_heartbeat();
            return;
        }
        let now = now_secs();
        let jobs = crate::syncjobs::load();

        if !paused() {
            // 1) Timer jobs (interval + calendar), gated by active-hours in due().
            for job in jobs.iter().filter(|j| j.due(now)) {
                spawn_job(job, &mut active_jobs, &job_done_tx);
                if stop_requested() {
                    break;
                }
            }

            // 2) Real-time jobs: watch local endpoints, run after the change settles.
            for job in jobs
                .iter()
                .filter(|j| j.enabled && j.trigger == Trigger::RealTime && j.active_now(now))
            {
                let roots: Vec<std::path::PathBuf> = [&job.source, &job.target]
                    .iter()
                    .filter_map(|e| local_root(e))
                    .collect();
                if roots.is_empty() {
                    continue; // nothing local to watch
                }
                let sig = roots.iter().fold((0u64, 0i64, 0u64), |a, r| {
                    let s = tree_sig(r);
                    (a.0 + s.0, a.1.max(s.1), a.2 + s.2)
                });
                match rt_sig.get(&job.id) {
                    Some(&prev) if prev == sig => {
                        // Unchanged since last tick - run if a pending change has settled.
                        if let Some(&since) = rt_dirty_since.get(&job.id) {
                            if now - since >= job.rt_debounce_secs as i64 {
                                spawn_job(job, &mut active_jobs, &job_done_tx);
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
                    for job in jobs.iter().filter(|j| {
                        j.enabled && j.trigger == Trigger::OnConnect && j.active_now(now)
                    }) {
                        if drive_matches(&job.connect_match, d) {
                            log(&format!("device connected → '{}'", job.name));
                            spawn_job(job, &mut active_jobs, &job_done_tx);
                        }
                    }
                }
                seen_drives = drives;
            }
        }

        write_heartbeat();
        // Sleep one tick in 2 s slices so a stop request is honoured promptly.
        let tick = cadence_secs();
        let mut slept = 0;
        while slept < tick {
            if stop_requested() {
                break;
            }
            std::thread::sleep(Duration::from_secs(2));
            drain_finished_jobs(&mut active_jobs, &job_done_rx);
            share_host.tick();
            write_heartbeat();
            slept += 2;
        }
    }
}

#[cfg(windows)]
struct DaemonInstanceGuard(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for DaemonInstanceGuard {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Threading::ReleaseMutex(self.0);
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(windows)]
fn acquire_daemon_instance_guard(timeout: Duration) -> Option<DaemonInstanceGuard> {
    let deadline = Instant::now() + timeout;
    loop {
        match try_acquire_daemon_mutex() {
            Ok(Some(guard)) => return Some(guard),
            Ok(None) if stop_requested() && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(250));
            }
            Ok(None) => return None,
            Err(e) => {
                log(&format!("daemon single-instance lock failed: {e}"));
                return None;
            }
        }
    }
}

#[cfg(windows)]
fn try_acquire_daemon_mutex() -> std::io::Result<Option<DaemonInstanceGuard>> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;

    let name: Vec<u16> = std::ffi::OsStr::new(r"Local\SmartExplorerSyncDaemon")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let handle = CreateMutexW(std::ptr::null_mut(), 1, name.as_ptr());
        if handle.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            CloseHandle(handle);
            return Ok(None);
        }
        Ok(Some(DaemonInstanceGuard(handle)))
    }
}

#[cfg(not(windows))]
struct DaemonInstanceGuard;

#[cfg(not(windows))]
fn acquire_daemon_instance_guard(_timeout: Duration) -> Option<DaemonInstanceGuard> {
    Some(DaemonInstanceGuard)
}

fn spawn_job(job: &SyncJob, active: &mut HashSet<String>, done: &Sender<String>) {
    if active.contains(&job.id) {
        return;
    }
    active.insert(job.id.clone());
    let job = job.clone();
    let done = done.clone();
    log(&format!("job queued '{}'", job.name));
    std::thread::Builder::new()
        .name(format!("daemon-job-{}", job.id))
        .spawn(move || {
            run_one(&job);
            let _ = done.send(job.id);
        })
        .ok();
}

fn drain_finished_jobs(active: &mut HashSet<String>, done: &Receiver<String>) {
    while let Ok(id) = done.try_recv() {
        active.remove(&id);
        write_heartbeat();
    }
}
