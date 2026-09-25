//! "On startup" jobs of the embedded worker run once per device boot rather
//! than once per worker start: the app process (and with it the worker) may
//! start many times between boots. The host supplies a boot marker (Android:
//! the boot count); the marker of the last startup pass is stored beside the
//! jobs. The desktop worker process never consults it.

use super::state::{log, read_optional, write_control};

const STORED_MARKER_FILE: &str = "startup-boot-marker";

/// Whether the startup pass for the host's current boot is still due. Without
/// a host marker every worker start counts, as on the desktop.
pub(super) fn startup_pass_due(current: Option<&str>, stored: Option<&str>) -> bool {
    match current.map(str::trim).filter(|marker| !marker.is_empty()) {
        None => true,
        Some(current) => stored.map(str::trim) != Some(current),
    }
}

/// Decide the startup pass for this boot and record it when due. A marker that
/// cannot be read or stored is logged; the pass then runs (running a sync job
/// once more is safe, silently skipping a boot is not).
pub(super) fn claim_startup_pass() -> bool {
    let current = crate::support_dirs::host().map(|host| host.boot_marker.trim());
    let Some(current) = current.filter(|marker| !marker.is_empty()) else {
        return true;
    };
    let path = crate::support_dirs::sync_data_dir().join(STORED_MARKER_FILE);
    let stored = read_optional(&path).unwrap_or_else(|error| {
        log(&format!("startup boot marker unreadable: {error}"));
        None
    });
    if !startup_pass_due(Some(current), stored.as_deref()) {
        log("startup jobs already ran for this device boot");
        return false;
    }
    if let Err(error) = write_control(&path, current) {
        log(&format!("startup boot marker could not be stored: {error}"));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::startup_pass_due;

    #[test]
    fn android_task_startup_pass_runs_once_per_boot_marker() {
        assert!(startup_pass_due(None, None));
        assert!(startup_pass_due(None, Some("17")));
        assert!(startup_pass_due(Some(""), Some("17")));
        assert!(startup_pass_due(Some("17"), None));
        assert!(!startup_pass_due(Some("17"), Some("17")));
        assert!(!startup_pass_due(Some(" 17\n"), Some("17\n")));
        assert!(startup_pass_due(Some("18"), Some("17")));
    }
}
