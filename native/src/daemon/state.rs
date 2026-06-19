use super::platform;

/// Default tick (seconds) between schedule evaluations. Kept short so real-time
/// and on-connect jobs react within a few seconds; editable via `cadence.txt`.
const DEFAULT_TICK_SECS: u64 = 15;
/// Cap the log so it can't grow without bound.
const LOG_CAP_BYTES: u64 = 256 * 1024;

pub(crate) fn now_secs() -> i64 {
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
pub(crate) fn paused() -> bool {
    if pause_remaining().is_some() {
        return true;
    }
    let (battery, metered) = autopause_flags();
    (battery && platform::battery_saver_on()) || (metered && platform::on_metered_network())
}

pub(crate) fn write_heartbeat() {
    let _ = std::fs::write(heartbeat_path(), now_secs().to_string());
}

pub(crate) fn clear_heartbeat() {
    let _ = std::fs::remove_file(heartbeat_path());
}

/// Seconds since the daemon last beat (None = never / unreadable).
pub fn last_heartbeat_age() -> Option<i64> {
    let s = std::fs::read_to_string(heartbeat_path()).ok()?;
    let t: i64 = s.trim().parse().ok()?;
    Some((now_secs() - t).max(0))
}

/// Best-effort "is a background daemon alive?" - true if it beat within a couple
/// of tick cycles. Used by the GUI for its status line.
pub fn is_running() -> bool {
    last_heartbeat_age()
        .map(|a| a < (cadence_secs() as i64) * 2 + 30)
        .unwrap_or(false)
}

/// The last `lines` lines of the daemon log (for the GUI log viewer).
pub fn read_log_tail(lines: usize) -> String {
    match std::fs::read_to_string(log_path()) {
        Ok(s) => {
            let mut tail: Vec<&str> = s.lines().rev().take(lines).collect();
            tail.reverse();
            tail.join("\n")
        }
        Err(_) => "(noch kein Protokoll)".to_string(),
    }
}

pub fn request_stop() {
    let _ = std::fs::write(stop_path(), "stop");
}
pub fn clear_stop() {
    let _ = std::fs::remove_file(stop_path());
}
pub(crate) fn stop_requested() -> bool {
    stop_path().exists()
}

pub(crate) fn log(msg: &str) {
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
