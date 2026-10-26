//! State of one `se mcp` process: its private session directory (job output
//! spools, guarded by a held lock file), the job registry, reused logins, its
//! persistent shell sessions and the path of the running `se` executable.
//! Directories of sessions whose process is gone (no lock holder) are removed
//! at the next start.
use super::jobs::JobRegistry;
use super::shell_registry::ShellRegistry;
use super::targets::Targets;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const LOCK_FILE: &str = "session.lock";

pub(super) struct Session {
    pub(super) directory: PathBuf,
    pub(super) jobs: JobRegistry,
    pub(super) targets: Arc<Targets>,
    /// Shell sessions this server opened or joined (`exec` with `session`).
    pub(super) shells: Arc<ShellRegistry>,
    /// The installed `se` that serves this session; the `se` tool runs it.
    pub(super) executable: PathBuf,
    pub(super) hello: std::sync::Mutex<Option<crate::mcp_launcher::RuntimeHelloReply>>,
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
        let active_runtime = crate::mcp_launcher::current_runtime();
        let executable = active_runtime.as_ref().map(|r| r.executable.clone()).unwrap_or(executable);
        let record = match active_runtime {
            Some(runtime) => serde_json::json!({"installation_id":runtime.installation.manifest.installation_id,"runtime_sha256":runtime.contract.runtime_sha256,"ipc_contract_id":runtime.contract.ipc_contract_id,"shared_state_contract_id":runtime.contract.shared_state_contract_id,"generation":runtime.installation.manifest.generation,"product_version":env!("CARGO_PKG_VERSION"),"executable":executable,"pid":std::process::id(),"entrypoint":if crate::mcp_launcher::migration_entry() {"installed_cli_migration"}else{"managed_launcher"},"update_retention_eligible":!crate::mcp_launcher::migration_entry()}),
            None => serde_json::json!({"state":"legacy","executable":executable,"pid":std::process::id()}),
        };
        std::fs::write(directory.join("runtime.v1.json"),record.to_string()).map_err(|e|format!("cannot record the MCP runtime identity: {e}"))?;
        Ok(Self {
            jobs: JobRegistry::new(&directory),
            targets: Arc::default(),
            shells: Arc::default(),
            directory,
            executable,
            lock: std::sync::Mutex::new(Some(lock)),
            hello: std::sync::Mutex::new(None),
        })
    }

    pub(super) fn negotiate_runtime(&self) -> Result<(), String> {
        if let Some(runtime) = crate::mcp_launcher::current_runtime() {
            let hello = crate::daemon::runtime_hello(&runtime)?;
            *self.hello.lock().map_err(|_| "runtime_hello_poisoned")? = Some(hello);
        }
        Ok(())
    }

    /// Cancels running jobs, waits up to `grace` for them to stop, ends own
    /// shell sessions and leaves joined ones, and removes the session
    /// directory.
    pub(super) fn close(&self, grace: std::time::Duration) {
        let running = self.jobs.cancel_running();
        let deadline = std::time::Instant::now() + grace;
        while running.iter().any(|job| job.end().is_none()) && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        self.shells.close_all();
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
