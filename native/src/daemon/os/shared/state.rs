use super::platform;
use std::io::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};

/// Default tick (seconds) between schedule evaluations. Kept short so real-time
/// and on-connect jobs react within a few seconds; editable via `cadence.txt`.
const DEFAULT_TICK_SECS: u64 = 15;
/// Cap the log so it can't grow without bound.
const LOG_CAP_BYTES: u64 = 256 * 1024;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn sync_dir() -> std::path::PathBuf {
    crate::support_dirs::sync_data_dir()
}

fn heartbeat_path() -> std::path::PathBuf {
    sync_dir().join("daemon.heartbeat")
}
pub(super) fn stop_path() -> std::path::PathBuf {
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
pub fn cadence_secs() -> io::Result<u64> {
    match read_optional(&cadence_path())? {
        Some(value) => parse_cadence(&value),
        None => Ok(DEFAULT_TICK_SECS),
    }
}

pub fn set_cadence_secs(v: u64) -> io::Result<()> {
    write_control(&cadence_path(), &v.clamp(2, 3600).to_string())
}

/// Pause all background syncs until `unix_ts` (0 or absent = not paused;
/// `i64::MAX` = indefinitely).
pub fn pause_until(unix_ts: i64) -> io::Result<()> {
    write_control(&pause_path(), &unix_ts.to_string())
}
pub fn pause_for_secs(secs: i64) -> io::Result<()> {
    let until = now_secs()
        .checked_add(secs.max(0))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "pause duration overflow"))?;
    pause_until(until)
}
pub fn pause_indefinite() -> io::Result<()> {
    pause_until(i64::MAX)
}
pub fn resume() -> io::Result<()> {
    remove_control(&pause_path())
}
/// Seconds remaining on a manual pause (None = not paused; Some(i64::MAX) = forever).
pub fn pause_remaining() -> io::Result<Option<i64>> {
    let Some(value) = read_optional(&pause_path())? else {
        return Ok(None);
    };
    let ts = parse_i64_control(&value, "pause deadline")?;
    if ts == i64::MAX {
        return Ok(Some(i64::MAX));
    }
    let rem = ts.saturating_sub(now_secs());
    if rem > 0 {
        Ok(Some(rem))
    } else {
        Ok(None)
    }
}

/// Auto-pause toggles (`battery`, `metered`) persisted as `b,m` 0/1 flags.
pub fn autopause_flags() -> io::Result<(bool, bool)> {
    match read_optional(&autopause_path())? {
        Some(value) => parse_autopause(&value),
        None => Ok((false, false)),
    }
}
pub fn set_autopause_flags(battery: bool, metered: bool) -> io::Result<()> {
    write_control(
        &autopause_path(),
        &format!("{},{}", battery as u8, metered as u8),
    )
}

/// Should background syncs hold off right now? (manual pause OR an enabled
/// auto-pause condition is currently true.)
pub(crate) fn paused() -> io::Result<bool> {
    if pause_remaining()?.is_some() {
        return Ok(true);
    }
    let (battery, metered) = autopause_flags()?;
    Ok((battery && platform::battery_saver_on()) || (metered && platform::on_metered_network()))
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
    let cadence = cadence_secs().unwrap_or(DEFAULT_TICK_SECS);
    last_heartbeat_age()
        .map(|a| a < (cadence as i64) * 2 + 30)
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

pub fn request_stop() -> io::Result<()> {
    write_control(&stop_path(), "stop")
}

pub(crate) fn log(msg: &str) {
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

pub(super) fn read_optional(path: &std::path::Path) -> io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn parse_cadence(value: &str) -> io::Result<u64> {
    value
        .trim()
        .parse::<u64>()
        .map(|value| value.clamp(2, 3600))
        .map_err(|error| invalid_data("cadence", error))
}

fn parse_i64_control(value: &str, label: &str) -> io::Result<i64> {
    value
        .trim()
        .parse::<i64>()
        .map_err(|error| invalid_data(label, error))
}

fn parse_autopause(value: &str) -> io::Result<(bool, bool)> {
    let values: Vec<&str> = value.trim().split(',').collect();
    if values.len() != 2 || values.iter().any(|value| !matches!(*value, "0" | "1")) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "auto-pause flags must be exactly two comma-separated 0/1 values",
        ));
    }
    Ok((values[0] == "1", values[1] == "1"))
}

fn invalid_data(label: &str, error: impl std::fmt::Display) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("invalid {label}: {error}"),
    )
}

pub(super) fn write_control(path: &std::path::Path, value: &str) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "control path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temp = parent.join(format!(
        ".{}.{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("control"),
        std::process::id(),
        sequence
    ));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(value.as_bytes())?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        platform::atomic_replace(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

fn remove_control(path: &std::path::Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_controls_are_errors_instead_of_disabling_safety() {
        assert_eq!(
            parse_cadence("not-a-number").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            parse_i64_control("?", "pause deadline").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            parse_autopause("1").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            parse_autopause("1,maybe").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn control_replacement_preserves_complete_values() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("control");
        write_control(&path, "first").unwrap();
        write_control(&path, "second").unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "second");
    }
}
