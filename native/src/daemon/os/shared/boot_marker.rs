//! "On startup" jobs of the embedded worker run once per device boot rather
//! than once per worker start: the app process (and with it the worker) may
//! start many times between boots. The host supplies a boot marker (Android:
//! the boot count); the marker of the last startup pass is stored beside the
//! jobs. Desktop workers use the current logon identity.

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

/// Store startup triggers before claiming this boot/logon. An unreadable or
/// unwritable marker is logged and retried on a later scheduler pass.
pub(super) fn register_startup(jobs: &[crate::syncjobs::SyncJob]) {
    let current = super::platform::session_marker();
    let Some(current) = current.filter(|marker| !marker.trim().is_empty()) else {
        // A missing host boot marker is not a new logon on every scheduler
        // tick. The worker records this process-local fallback once.
        static REGISTERED: std::sync::atomic::AtomicBool =
            std::sync::atomic::AtomicBool::new(false);
        if REGISTERED.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        let mut stored = true;
        for job in jobs
            .iter()
            .filter(|job| job.enabled && job.trigger == crate::syncjobs::Trigger::OnStartup)
        {
            stored &= super::job_triggers::persist(
                &job.id,
                crate::syncjobs::PendingKind::Startup,
                super::state::now_secs(),
                None,
            );
        }
        if stored {
            REGISTERED.store(true, std::sync::atomic::Ordering::Release);
        }
        return;
    };
    let path = crate::support_dirs::sync_data_dir().join(STORED_MARKER_FILE);
    let stored = match read_optional(&path) {
        Ok(stored) => stored,
        Err(error) => {
            log(&format!("startup marker unreadable: {error}"));
            return;
        }
    };
    if !startup_pass_due(Some(&current), stored.as_deref()) {
        return;
    }
    // Durable triggers precede the marker. Pause/defer cannot lose a logon;
    // an unsuccessful write leaves this pass due.
    for job in jobs
        .iter()
        .filter(|job| job.enabled && job.trigger == crate::syncjobs::Trigger::OnStartup)
    {
        let now = super::state::now_secs();
        if let Err(error) = crate::syncjobs::update_job_state(&job.id, |state| {
            if state.pending_trigger.is_none() {
                state.pending_trigger = Some(crate::syncjobs::PendingTrigger {
                    kind: crate::syncjobs::PendingKind::Startup,
                    since: now,
                    volume: None,
                });
            }
        }) {
            log(&format!("startup trigger could not be stored: {error}"));
            return;
        }
    }
    if let Err(error) = write_control(&path, &current) {
        log(&format!("startup marker could not be stored: {error}"));
    }
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
