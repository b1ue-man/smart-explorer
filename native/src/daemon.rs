//! Headless background sync (#4). Started as `smart_explorer.exe --sync-daemon`
//! from a per-user logon autostart entry — see `autostart.rs`. It opens no
//! window: it loops, runs every *due* saved sync job (the schedule lives on the
//! job, `syncjobs::SyncJob::due`), writes a heartbeat the GUI can read, then
//! sleeps. Because the daemon is the *same exe*, a self-update swaps it too.
//!
//! Safety mirrors the interactive sync exactly (same `bisync::run`): only files
//! that actually changed move, both-sides-changed stays a conflict (nothing is
//! silently overwritten), changes are reversible. Unresolved conflicts are left
//! for the user to settle in the GUI — the daemon never guesses.

use crate::syncjobs::SyncJob;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// How often the loop wakes to look for due jobs.
const CHECK_SECS: u64 = 60;
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

fn write_heartbeat() {
    let _ = std::fs::write(heartbeat_path(), now_secs().to_string());
}

/// Seconds since the daemon last beat (None = never / unreadable).
pub fn last_heartbeat_age() -> Option<i64> {
    let s = std::fs::read_to_string(heartbeat_path()).ok()?;
    let t: i64 = s.trim().parse().ok()?;
    Some((now_secs() - t).max(0))
}

/// Best-effort "is a background daemon alive?" — true if it beat within a
/// couple of check cycles. Used by the GUI for its status line.
pub fn is_running() -> bool {
    last_heartbeat_age()
        .map(|a| a < (CHECK_SECS as i64) * 2 + 30)
        .unwrap_or(false)
}

/// Ask a running daemon to exit (it checks this sentinel each loop). Also used
/// by the GUI when the user turns background sync off.
pub fn request_stop() {
    let _ = std::fs::write(stop_path(), "stop");
}

/// Clear a stale stop request before (re)starting the daemon.
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

fn is_local(path: &str) -> bool {
    crate::app::is_local_style(path)
}

/// Run one due job to completion (synchronously). Saved jobs are local↔local
/// (remote endpoints need re-auth, which a headless process can't do), so we
/// skip anything else rather than fail.
fn run_one(job: &SyncJob) {
    if !is_local(&job.source) || !is_local(&job.target) {
        log(&format!("skip '{}' (remote endpoint not supported headless)", job.name));
        return;
    }
    let a = std::sync::Arc::new(crate::vfs::LocalBackend::new(&job.source));
    let b = std::sync::Arc::new(crate::vfs::LocalBackend::new(&job.target));
    let opts = crate::bisync::BisyncOptions {
        direction: job.direction,
        conflict: job.conflict,
        reversible: true,
        dry_run: false,
    };
    let cancel = AtomicBool::new(false);
    let gs = job.glob_set();
    let filter = crate::bisync::WalkFilter {
        include_hidden: job.include_hidden,
        ignore: &gs,
    };
    let out = crate::bisync::run(
        &*a, &job.source, &*b, &job.target, opts, job.retain_days, &cancel, &filter,
    );
    crate::syncjobs::mark_run(&job.id);
    log(&format!(
        "ran '{}': {}→ {}← {}del {}conf {}err",
        job.name,
        out.stats.a_to_b,
        out.stats.b_to_a,
        out.stats.deleted,
        out.conflicts.len(),
        out.errors.len()
    ));
}

/// The headless loop. Returns when a stop is requested, or immediately if
/// another daemon already appears to be running (fresh heartbeat).
pub fn run_daemon() {
    // Single-instance guard: a recent heartbeat means another daemon is alive
    // (only one logon-autostart entry exists; this also stops the GUI's
    // "enable" spawn from doubling up an already-running daemon). The window is
    // tiny and bisync is idempotent/conflict-safe regardless.
    if is_running() {
        return;
    }
    clear_stop();
    log("daemon started");
    write_heartbeat();
    loop {
        if stop_requested() {
            clear_stop();
            log("daemon stopping (stop requested)");
            let _ = std::fs::remove_file(heartbeat_path());
            return;
        }
        let now = now_secs();
        for job in crate::syncjobs::load().into_iter().filter(|j| j.due(now)) {
            run_one(&job);
            write_heartbeat();
            if stop_requested() {
                break;
            }
        }
        write_heartbeat();
        // Sleep in short slices so a stop request is honoured promptly.
        let mut slept = 0;
        while slept < CHECK_SECS {
            if stop_requested() {
                break;
            }
            std::thread::sleep(Duration::from_secs(2));
            slept += 2;
        }
    }
}
