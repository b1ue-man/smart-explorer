//! Headless background sync (#4). Started as `smart_explorer.exe --sync-daemon`
//! from a per-user logon autostart entry — see `autostart.rs`. It opens no
//! window: it loops on a short tick, runs every *due* saved sync job, reacts to
//! the event triggers (on-startup, real-time change, device/USB connect), writes
//! a heartbeat the GUI can read, then sleeps. Because the daemon is the *same
//! exe*, a self-update swaps it too.
//!
//! Safety mirrors the interactive sync exactly (same `bisync::run`): only files
//! that actually changed move, both-sides-changed stays a conflict (nothing is
//! silently overwritten), changes are reversible. Unresolved conflicts are left
//! for the user to settle in the GUI — the daemon never guesses.

use crate::syncjobs::{SyncJob, Trigger};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// Default tick (seconds) between schedule evaluations. Kept short so real-time
/// and on-connect jobs react within a few seconds; editable via `cadence.txt`.
const DEFAULT_TICK_SECS: u64 = 15;
/// Cap the log so it can't grow without bound.
const LOG_CAP_BYTES: u64 = 256 * 1024;

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn sync_dir() -> std::path::PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let d = base.join("smart_explorer").join("sync");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn heartbeat_path() -> std::path::PathBuf {
    sync_dir().join("daemon.heartbeat")
}
fn stop_path() -> std::path::PathBuf {
    sync_dir().join("daemon.stop")
}
fn log_path() -> std::path::PathBuf {
    sync_dir().join("daemon.log")
}
fn cadence_path() -> std::path::PathBuf {
    sync_dir().join("cadence.txt")
}
fn pause_path() -> std::path::PathBuf {
    sync_dir().join("pause.until")
}
fn autopause_path() -> std::path::PathBuf {
    sync_dir().join("autopause.txt")
}

/// Tick length in seconds (clamped 2..=3600). Editable by the GUI.
pub fn cadence_secs() -> u64 {
    std::fs::read_to_string(cadence_path())
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(|v| v.clamp(2, 3600))
        .unwrap_or(DEFAULT_TICK_SECS)
}

pub fn set_cadence_secs(v: u64) {
    let _ = std::fs::write(cadence_path(), v.clamp(2, 3600).to_string());
}

// ── pause control (shared with the GUI) ──────────────────────────────────────

/// Pause all background syncs until `unix_ts` (0 or absent = not paused;
/// `i64::MAX` = indefinitely).
pub fn pause_until(unix_ts: i64) {
    let _ = std::fs::write(pause_path(), unix_ts.to_string());
}
pub fn pause_for_secs(secs: i64) {
    pause_until(now_secs() + secs.max(0));
}
pub fn pause_indefinite() {
    pause_until(i64::MAX);
}
pub fn resume() {
    let _ = std::fs::remove_file(pause_path());
}
/// Seconds remaining on a manual pause (None = not paused; Some(i64::MAX) = forever).
pub fn pause_remaining() -> Option<i64> {
    let ts: i64 = std::fs::read_to_string(pause_path()).ok()?.trim().parse().ok()?;
    if ts == i64::MAX {
        return Some(i64::MAX);
    }
    let rem = ts - now_secs();
    if rem > 0 {
        Some(rem)
    } else {
        None
    }
}

/// Auto-pause toggles (`battery`, `metered`) persisted as `b,m` 0/1 flags.
pub fn autopause_flags() -> (bool, bool) {
    std::fs::read_to_string(autopause_path())
        .ok()
        .and_then(|s| {
            let mut it = s.trim().split(',');
            Some((it.next()? == "1", it.next().unwrap_or("0") == "1"))
        })
        .unwrap_or((false, false))
}
pub fn set_autopause_flags(battery: bool, metered: bool) {
    let _ = std::fs::write(
        autopause_path(),
        format!("{},{}", battery as u8, metered as u8),
    );
}

/// Should background syncs hold off right now? (manual pause OR an enabled
/// auto-pause condition is currently true.)
fn paused() -> bool {
    if pause_remaining().is_some() {
        return true;
    }
    let (battery, metered) = autopause_flags();
    (battery && power::battery_saver_on()) || (metered && power::on_metered_network())
}

fn write_heartbeat() {
    let _ = std::fs::write(heartbeat_path(), now_secs().to_string());
}

/// Seconds since the daemon last beat (None = never / unreadable).
pub fn last_heartbeat_age() -> Option<i64> {
    let s = std::fs::read_to_string(heartbeat_path()).ok()?;
    let t: i64 = s.trim().parse().ok()?;
    Some((now_secs() - t).max(0))
}

/// Best-effort "is a background daemon alive?" — true if it beat within a couple
/// of tick cycles. Used by the GUI for its status line.
pub fn is_running() -> bool {
    last_heartbeat_age()
        .map(|a| a < (cadence_secs() as i64) * 2 + 30)
        .unwrap_or(false)
}

pub fn request_stop() {
    let _ = std::fs::write(stop_path(), "stop");
}
pub fn clear_stop() {
    let _ = std::fs::remove_file(stop_path());
}
fn stop_requested() -> bool {
    stop_path().exists()
}

fn log(msg: &str) {
    use std::io::Write;
    if std::fs::metadata(log_path()).map(|m| m.len()).unwrap_or(0) > LOG_CAP_BYTES {
        let _ = std::fs::write(log_path(), "");
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
    {
        let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let _ = writeln!(f, "{} {}", ts, msg);
    }
}

/// Run one job to completion (synchronously). Endpoints are resolved the same
/// way the GUI does — local paths directly, remote URLs by re-opening the
/// matching saved connection (credentials live in the OS keyring).
fn run_one(job: &SyncJob) {
    let (a, root_a) = match crate::connect::resolve_endpoint(&job.source) {
        Ok(x) => x,
        Err(e) => {
            log(&format!("skip '{}': source {}", job.name, e));
            return;
        }
    };
    let (b, root_b) = match crate::connect::resolve_endpoint(&job.target) {
        Ok(x) => x,
        Err(e) => {
            log(&format!("skip '{}': target {}", job.name, e));
            return;
        }
    };
    let opts = job.opts(false);
    let cancel = AtomicBool::new(false);
    let gs = job.glob_set();
    let (min_size, max_size, after, before) = job.filter_bounds(now_secs());
    let filter = crate::bisync::WalkFilter {
        include_hidden: job.include_hidden,
        ignore: &gs,
        min_size,
        max_size,
        after_mtime_ms: after,
        before_mtime_ms: before,
    };
    let out = crate::bisync::run(&*a, &root_a, &*b, &root_b, opts, &cancel, &filter);
    crate::syncjobs::mark_run(&job.id);
    log(&format!(
        "ran '{}' [{}]: {}→ {}← {}del {}conf {}err",
        job.name,
        job.trigger.as_str(),
        out.stats.a_to_b,
        out.stats.b_to_a,
        out.stats.deleted,
        out.conflicts.len(),
        out.errors.len()
    ));
}

// ── real-time change detection (local-side mtime/count signature) ────────────

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

// ── on-connect (removable drive arrival) ─────────────────────────────────────

/// Set of currently-present removable-drive descriptors ("LETTER|LABEL|SERIAL").
fn current_drives() -> HashSet<String> {
    drives::removable()
        .into_iter()
        .map(|d| format!("{}|{}|{}", d.letter, d.label, d.serial))
        .collect()
}

/// Does a drive descriptor match a job's `connect_match` (empty = any removable;
/// otherwise a case-insensitive `*?` wildcard tested against letter, label and
/// serial)?
fn drive_matches(pat: &str, descriptor: &str) -> bool {
    let pat = pat.trim();
    if pat.is_empty() {
        return true;
    }
    let parts: Vec<&str> = descriptor.split('|').collect();
    parts.iter().any(|p| wildcard_ci(pat, p))
}

/// Minimal case-insensitive glob (`*` and `?`).
fn wildcard_ci(pat: &str, s: &str) -> bool {
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
    if is_running() {
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
            let _ = std::fs::remove_file(heartbeat_path());
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
                        // Unchanged since last tick — run if a pending change has settled.
                        if let Some(&since) = rt_dirty_since.get(&job.id) {
                            if now - since >= job.rt_debounce_secs as i64 {
                                run_one(job);
                                rt_dirty_since.remove(&job.id);
                                write_heartbeat();
                            }
                        }
                    }
                    Some(_) => {
                        // Changed this tick — (re)start the settle timer.
                        rt_dirty_since.insert(job.id.clone(), now);
                        rt_sig.insert(job.id.clone(), sig);
                    }
                    None => {
                        // First sighting — record baseline, don't run.
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

// ── platform helpers ─────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct DriveInfo {
    pub letter: String,
    pub label: String,
    pub serial: String,
}

#[cfg(windows)]
mod drives {
    use super::DriveInfo;
    use std::os::windows::ffi::OsStrExt;

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect()
    }

    pub fn removable() -> Vec<DriveInfo> {
        use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumeInformationW};
        // GetDriveTypeW returns a plain u32; DRIVE_REMOVABLE == 2.
        const DRIVE_REMOVABLE: u32 = 2;
        let mut out = Vec::new();
        let mask = unsafe { windows::Win32::Storage::FileSystem::GetLogicalDrives() };
        for i in 0..26u32 {
            if mask & (1 << i) == 0 {
                continue;
            }
            let letter = (b'A' + i as u8) as char;
            let root = format!("{}:\\", letter);
            let rootw = wide(&root);
            let dtype = unsafe { GetDriveTypeW(windows::core::PCWSTR(rootw.as_ptr())) };
            if dtype != DRIVE_REMOVABLE {
                continue;
            }
            let mut name = [0u16; 261];
            let mut serial: u32 = 0;
            let label = unsafe {
                if GetVolumeInformationW(
                    windows::core::PCWSTR(rootw.as_ptr()),
                    Some(&mut name),
                    Some(&mut serial),
                    None,
                    None,
                    None,
                )
                .is_ok()
                {
                    let len = name.iter().position(|&c| c == 0).unwrap_or(0);
                    String::from_utf16_lossy(&name[..len])
                } else {
                    String::new()
                }
            };
            out.push(DriveInfo {
                letter: format!("{}:", letter),
                label,
                serial: format!("{:08X}", serial),
            });
        }
        out
    }
}

#[cfg(not(windows))]
mod drives {
    use super::DriveInfo;
    pub fn removable() -> Vec<DriveInfo> {
        Vec::new()
    }
}

#[cfg(windows)]
mod power {
    pub fn battery_saver_on() -> bool {
        use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
        let mut st = SYSTEM_POWER_STATUS::default();
        unsafe {
            if GetSystemPowerStatus(&mut st).is_ok() {
                // SystemStatusFlag bit0 = "battery saver on" (Windows 10+).
                st.SystemStatusFlag & 0x01 != 0
            } else {
                false
            }
        }
    }
    pub fn on_metered_network() -> bool {
        // Best-effort: metered detection needs WinRT NetworkInformation, which we
        // don't pull in here. Treated as "not metered" until that lands.
        false
    }
}

#[cfg(not(windows))]
mod power {
    pub fn battery_saver_on() -> bool {
        false
    }
    pub fn on_metered_network() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_matches() {
        assert!(wildcard_ci("*", "anything"));
        assert!(wildcard_ci("backup*", "BACKUP_DRIVE"));
        assert!(wildcard_ci("E:", "e:"));
        assert!(wildcard_ci("????", "ABCD"));
        assert!(!wildcard_ci("backup?", "backup"));
        assert!(!wildcard_ci("x*", "yz"));
    }

    #[test]
    fn drive_matching() {
        assert!(drive_matches("", "E:|STICK|1A2B")); // empty = any
        assert!(drive_matches("STICK", "E:|STICK|1A2B")); // by label
        assert!(drive_matches("E:", "E:|STICK|1A2B")); // by letter
        assert!(drive_matches("1A2B", "E:|STICK|1A2B")); // by serial
        assert!(drive_matches("back*", "F:|Backup|99")); // wildcard label
        assert!(!drive_matches("nope", "E:|STICK|1A2B"));
    }
}
