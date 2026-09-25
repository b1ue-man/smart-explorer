//! Android: "background sync enabled" is an app-private flag file, and the
//! background worker is a thread of the app process. Starting it again at
//! boot is the host's job (boot receiver / foreground service), so there is
//! no login entry and no process to launch or hand off to.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

const SYNC_ENABLED_FLAG: &str = "android-sync-enabled";

fn flag_path() -> PathBuf {
    crate::support_dirs::sync_data_dir().join(SYNC_ENABLED_FLAG)
}

pub fn is_enabled() -> bool {
    flag_path().exists()
}

pub fn enable() -> io::Result<()> {
    std::fs::write(flag_path(), b"1")
}

pub fn disable() -> io::Result<()> {
    match std::fs::remove_file(flag_path()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Start the embedded worker thread without waiting. A start failure is
/// written to the worker log by `ensure_embedded_daemon` and reported again
/// by the next readiness check.
pub fn spawn_daemon_now() {
    let _ = crate::daemon::ensure_embedded_daemon(Duration::ZERO);
}

/// There is no older process to retire in-process: a "replacement" only makes
/// sure the embedded worker thread runs. Generations are managed by the worker.
pub fn spawn_daemon_handoff_checked(
    _generation: &str,
    _retiring_generation: Option<&str>,
) -> io::Result<()> {
    crate::daemon::ensure_embedded_daemon(Duration::ZERO)
        .map(|_| ())
        .map_err(io::Error::other)
}
