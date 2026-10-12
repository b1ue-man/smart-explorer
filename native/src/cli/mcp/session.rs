//! State of one `se mcp` process: its private session directory (job output
//! spools, guarded by a held lock file), the job registry, reused logins and
//! the path of the running `se` executable. Directories of sessions whose
//! process is gone (no lock holder) are removed at the next start.
use super::jobs::JobRegistry;
use super::targets::Targets;
use std::fs::File;
use std::path::{Path, PathBuf};

const LOCK_FILE: &str = "session.lock";

pub(super) struct Session {
    pub(super) directory: PathBuf,
    pub(super) jobs: JobRegistry,
    pub(super) targets: Targets,
    /// The installed `se` that serves this session; the `se` tool runs it.
    pub(super) executable: PathBuf,
    lock: std::sync::Mutex<Option<File>>,
}

impl Session {
    pub(super) fn open() -> Result<Self, String> {
        let sessions = sessions_dir();
        crate::support_dirs::ensure_private_dir(&sessions)
            .map_err(|error| format!("cannot create {}: {error}", sessions.display()))?;
        remove_stale_sessions(&sessions);
        let directory = sessions.join(format!(
            "{}-{:x}",
            std::process::id(),
            super::jobs::now_ms()
        ));
        crate::support_dirs::ensure_private_dir(&directory)
            .map_err(|error| format!("cannot create {}: {error}", directory.display()))?;
        let lock = crate::support_dirs::open_private_lock(&directory.join(LOCK_FILE))
            .map_err(|error| format!("cannot create the session lock: {error}"))?;
        lock.try_lock()
            .map_err(|error| format!("cannot lock the session directory: {error}"))?;
        // Already absolute; on Linux it is the resolved /proc/self/exe target.
        let executable = std::env::current_exe()
            .map_err(|error| format!("cannot determine the se executable: {error}"))?;
        Ok(Self {
            jobs: JobRegistry::new(&directory),
            targets: Targets::default(),
            directory,
            executable,
            lock: std::sync::Mutex::new(Some(lock)),
        })
    }

    /// Cancels running jobs, waits up to `grace` for them to stop and removes
    /// the session directory.
    pub(super) fn close(&self, grace: std::time::Duration) {
        let running = self.jobs.cancel_running();
        let deadline = std::time::Instant::now() + grace;
        while running.iter().any(|job| job.end().is_none()) && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        if let Ok(mut lock) = self.lock.lock() {
            drop(lock.take());
        }
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

pub(super) fn sessions_dir() -> PathBuf {
    crate::support_dirs::app_data_dir()
        .join("mcp")
        .join("sessions")
}

/// Removes session directories whose lock nobody holds anymore.
fn remove_stale_sessions(sessions: &Path) {
    let Ok(entries) = std::fs::read_dir(sessions) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_dir = entry
            .file_type()
            .map(|kind| kind.is_dir() && !kind.is_symlink())
            .unwrap_or(false);
        if !is_dir {
            continue;
        }
        let stale = match File::options()
            .read(true)
            .write(true)
            .open(path.join(LOCK_FILE))
        {
            Ok(lock) => {
                let free = lock.try_lock().is_ok();
                drop(lock);
                free
            }
            // A directory without its lock file never got a live owner.
            Err(error) => error.kind() == std::io::ErrorKind::NotFound,
        };
        if stale {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{remove_stale_sessions, LOCK_FILE};
    use std::fs::File;

    #[test]
    fn mcp_task_only_unlocked_session_directories_are_removed() {
        let root = tempfile::tempdir().unwrap();
        let live = root.path().join("live");
        let stale = root.path().join("stale");
        std::fs::create_dir_all(&live).unwrap();
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::write(stale.join(LOCK_FILE), b"").unwrap();
        std::fs::write(stale.join("j1.stdout"), b"old").unwrap();
        let held = File::create(live.join(LOCK_FILE)).unwrap();
        held.try_lock().unwrap();
        remove_stale_sessions(root.path());
        assert!(live.exists());
        assert!(!stale.exists());
        drop(held);
    }
}
