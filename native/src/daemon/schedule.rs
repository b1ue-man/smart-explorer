use crate::syncjobs::Trigger;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use super::job::run_one;
use super::platform;
use super::state::{
    cadence_secs, clear_heartbeat, clear_stop, is_running, log, now_secs, paused, stop_requested,
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
    // Handoff: if a stop was requested (e.g. an updated GUI cycling the daemon),
    // wait for the previous instance to exit before taking over. Otherwise the
    // single-instance guard applies - a fresh heartbeat means one already runs.
    if stop_requested() {
        for _ in 0..15 {
            if !is_running() {
                break;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        clear_stop();
    } else if is_running() {
        return;
    }
    clear_stop();
    log("daemon started");
    write_heartbeat();

    // On-startup jobs run once now (subject to pause).
    if !paused() {
        for job in crate::syncjobs::load()
            .into_iter()
            .filter(|j| j.enabled && j.trigger == Trigger::OnStartup)
        {
            if stop_requested() {
                break;
            }
            run_one(&job);
            write_heartbeat();
        }
    }

    // Per-job real-time state and the last-seen drive set.
    let mut rt_sig: HashMap<String, (u64, i64, u64)> = HashMap::new();
    let mut rt_dirty_since: HashMap<String, i64> = HashMap::new();
    let mut seen_drives = current_drives();

    loop {
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
                run_one(job);
                write_heartbeat();
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
                                run_one(job);
                                rt_dirty_since.remove(&job.id);
                                write_heartbeat();
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
                            run_one(job);
                            write_heartbeat();
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
            slept += 2;
        }
    }
}
