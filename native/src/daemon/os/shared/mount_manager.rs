use std::collections::HashMap;
use std::io;
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::mount::{MountConfig, MountId, MountRecovery, MountSnapshot, MountStatus};
use crate::vfs::BackendHandle;

use super::ipc_host::ShareHost;

// A remote callback is allowed to run for five minutes. Eject must not kill a
// host while Dokany is still legitimately completing that durable flush.
pub(super) const STOP_GRACE: Duration = Duration::from_secs(330);
/// Status of a registered drive after a worker start until its restore or a
/// Retry (text of v0.5.182).
const RESTART_DETAIL: &str =
    "Laufwerk nach Daemon-Neustart zur Wiederherstellung bereit; bitte erneut versuchen";

#[path = "mount_manager_state.rs"]
mod entry_state;
#[path = "mount_manager_host.rs"]
mod host;
// DRV-M7: the lock participant of vault drives and its channel.
#[path = "mount_lock_participant.rs"]
pub(in crate::daemon) mod lock_participant;
#[path = "mount_restore.rs"]
mod restore;
#[path = "mount_manager_start_cache.rs"]
mod start_cache;
#[path = "mount_manager_start.rs"]
mod start_path;
// DRV-M7: the end of vault drives and their return after an unlock.
#[path = "mount_termination.rs"]
pub(in crate::daemon) mod termination;
#[path = "mount_manager_capabilities.rs"]
mod write_capabilities;

use entry_state::{
    clear_backend_runtime, finish_clean, finish_exit, random_token, removable, snapshot,
};

#[derive(Clone)]
pub(super) struct MountManager {
    state: Arc<Mutex<HashMap<String, MountEntry>>>,
    lifecycle: Arc<Mutex<()>>,
    registry_error: Arc<Mutex<Option<String>>>,
    /// DRV-M3: registry records that survive a clean end (`mount_restore.rs`).
    retention: Arc<Mutex<restore::Retention>>,
    /// DRV-M7: lock orders and hooks of vault drives (`mount_termination.rs`).
    vault: Arc<termination::VaultDrives>,
}

struct MountEntry {
    config: MountConfig,
    status: MountStatus,
    backend: Option<BackendHandle>,
    capabilities: Option<super::ipc_protocol::MountBackendCapabilities>,
    launch_token: Option<String>,
    session_token: Option<String>,
    backend_token: Option<String>,
    control: Option<SyncSender<()>>,
    child: Option<super::mount_host_process::MountHostProcess>,
    backend_stream_active: bool,
    // Daemon-side failures stay pending until process finalization. A public
    // terminal detail while a child exists is therefore host-reported.
    pending_host_failure: Option<String>,
    // Only a host-reported Failed status carrying an explicit recovery
    // boundary is terminal. Callback failures deliberately omit that boundary.
    host_terminal_failure: bool,
    registry_recorded: bool,
    recovery: MountRecovery,
    /// DRV-M7: the host ends for a lock of its vault, with this many
    /// unpublished changes once it answered.
    lock_end: bool,
    lock_pending: Option<u32>,
    /// DRV-M7: the daemon asked the host to stop (`MountTerminated`).
    stop_requested: bool,
}

impl Default for MountManager {
    fn default() -> Self {
        Self::from_registry(super::mount_registry::load(), start_cache::audit_registered)
    }
}

impl MountManager {
    /// The drive table of a starting worker: every registered drive waits
    /// for its restore (`mount_restore.rs`) or a Retry, with the recovery
    /// state of the cache audit.
    fn from_registry(
        loaded: io::Result<Vec<MountConfig>>,
        audit: impl FnOnce(&[MountConfig]) -> Vec<MountRecovery>,
    ) -> Self {
        let (entries, registry_error) = match loaded {
            Ok(configs) => {
                let recovery = audit(&configs);
                let entries = configs
                    .into_iter()
                    .zip(recovery)
                    .map(|(config, recovery)| {
                        let key = config.id.as_str().to_string();
                        (
                            key,
                            registered_entry(config, recovery, RESTART_DETAIL.into()),
                        )
                    })
                    .collect();
                (entries, None)
            }
            Err(error) => {
                let detail = format!("Laufwerk-Registry laden: {error}");
                super::state::log(&detail);
                (HashMap::new(), Some(detail))
            }
        };
        Self {
            state: Arc::new(Mutex::new(entries)),
            lifecycle: Arc::new(Mutex::new(())),
            registry_error: Arc::new(Mutex::new(registry_error)),
            retention: Arc::new(Mutex::new(restore::Retention::default())),
            vault: Arc::new(termination::VaultDrives::default()),
        }
    }

    pub(super) fn start(
        &self,
        config: MountConfig,
        host: &ShareHost,
    ) -> Result<MountSnapshot, String> {
        config
            .validate()
            .map_err(|error| format!("Ungueltige Laufwerk-Konfiguration: {error}"))?;
        let _lifecycle = self.lifecycle_guard()?;
        // Windows and Linux record the drive in the registry; a registry
        // that failed to load is never written over.
        #[cfg(any(windows, target_os = "linux"))]
        self.ensure_registry_ready()?;
        self.start_locked(config, host, false)
    }

    pub(super) fn list(&self) -> Result<Vec<MountSnapshot>, String> {
        self.ensure_registry_ready()?;
        let state = self.state_guard()?;
        let mut mounts: Vec<_> = state.values().map(snapshot).collect();
        mounts.sort_by(|left, right| left.config.id.as_str().cmp(right.config.id.as_str()));
        Ok(mounts)
    }

    pub(super) fn stop(&self, id: &MountId) -> Result<MountSnapshot, String> {
        let _lifecycle = self.lifecycle_guard()?;
        let stopped = self.stop_locked(id)?;
        if matches!(&stopped.status, MountStatus::Unmounted) {
            let mut state = self.state_guard()?;
            if state.get(id.as_str()).is_some_and(removable) {
                state.remove(id.as_str());
            }
        }
        Ok(stopped)
    }

    fn stop_locked(&self, id: &MountId) -> Result<MountSnapshot, String> {
        let key = id.as_str();
        let control = {
            let mut state = self.state_guard()?;
            let entry = state
                .get_mut(key)
                .ok_or_else(|| "Das Laufwerk wird nicht verwaltet".to_string())?;
            if entry.child.is_none() {
                if entry.backend_stream_active {
                    return Ok(snapshot(entry));
                }
                if entry.recovery.requires_retention() {
                    return Ok(snapshot(entry));
                }
                if entry.registry_recorded && self.retention().keeps(entry) {
                    // DRV-M3: a drive that restores at login stays registered
                    // across a worker stop or an update stop.
                    return Ok(snapshot(entry));
                }
                if entry.registry_recorded {
                    match super::mount_registry::remove(&entry.config.id) {
                        Ok(()) => entry.registry_recorded = false,
                        Err(error) => {
                            entry.status = MountStatus::Failed {
                                detail: format!("Laufwerk-Registry bereinigen: {error}"),
                            };
                            return Ok(snapshot(entry));
                        }
                    }
                }
                entry.status = MountStatus::Unmounted;
                return Ok(snapshot(entry));
            }
            if !matches!(
                &entry.status,
                MountStatus::Failed { .. }
                    | MountStatus::RuntimeUnavailable { .. }
                    | MountStatus::Conflict { .. }
            ) {
                entry.status = MountStatus::Unmounting;
            }
            entry.stop_requested = true;
            entry.control.clone()
        };
        if let Some(control) = control {
            // Never wait here. The channel holds one stop, and a full channel
            // already carries it. Its reader is the control thread of the
            // host, which can be the caller itself (`ipc.rs`,
            // `serve_mount_control` stops the drive when the host closes the
            // connection): a blocking send would wait for a thread that is
            // not reading, with the lifecycle lock held.
            let _ = control.try_send(());
        }

        let deadline = Instant::now() + STOP_GRACE;
        loop {
            if let Some(done) = self.observe_exit(key)? {
                return Ok(done);
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        self.force_stop(key)
    }

    fn observe_exit(&self, key: &str) -> Result<Option<MountSnapshot>, String> {
        let mut state = self.state_guard()?;
        let entry = state
            .get_mut(key)
            .ok_or_else(|| "Das Laufwerk wird nicht verwaltet".to_string())?;
        let mut retention = self.retention();
        let Some(child) = entry.child.as_mut() else {
            if entry.recovery.is_clean() {
                let keep = retention.keeps(entry);
                finish_clean(entry, keep);
            }
            return Ok(Some(snapshot(entry)));
        };
        let observation = child.try_wait();
        match observation {
            Ok(Some(exit)) => {
                finish_exit(entry, exit, &mut retention);
                Ok(Some(snapshot(entry)))
            }
            Ok(None) => Ok(None),
            Err(error) => {
                entry_state::terminate_after_observation_error(entry, &error, &mut retention);
                Ok(Some(snapshot(entry)))
            }
        }
    }

    fn force_stop(&self, key: &str) -> Result<MountSnapshot, String> {
        let child = {
            let mut state = self.state_guard()?;
            let entry = state
                .get_mut(key)
                .ok_or_else(|| "Das Laufwerk wird nicht verwaltet".to_string())?;
            entry.child.take()
        };
        if let Some(mut child) = child {
            if let Err(error) = child.kill() {
                match child.try_wait() {
                    Ok(Some(exit)) => {
                        let mut state = self.state_guard()?;
                        let entry = state
                            .get_mut(key)
                            .ok_or_else(|| "Das Laufwerk wird nicht verwaltet".to_string())?;
                        finish_exit(entry, exit, &mut self.retention());
                        return Ok(snapshot(entry));
                    }
                    Ok(None) => {}
                    Err(observe_error) => {
                        if let Ok(mut state) = self.state_guard() {
                            if let Some(entry) = state.get_mut(key) {
                                entry.child = Some(child);
                            }
                        }
                        return Err(format!(
                            "Laufwerk-Host beenden: {error}; Abschlussstatus lesen: {observe_error}"
                        ));
                    }
                }
                if let Ok(mut state) = self.state_guard() {
                    if let Some(entry) = state.get_mut(key) {
                        entry.child = Some(child);
                    }
                }
                return Err(format!("Laufwerk-Host beenden: {error}"));
            }
            let exit = match child.wait() {
                Ok(exit) => exit,
                Err(error) => {
                    if let Ok(mut state) = self.state_guard() {
                        if let Some(entry) = state.get_mut(key) {
                            entry.child = Some(child);
                        }
                    }
                    return Err(format!("Laufwerk-Host abwarten: {error}"));
                }
            };
            let mut state = self.state_guard()?;
            let entry = state
                .get_mut(key)
                .ok_or_else(|| "Das Laufwerk wird nicht verwaltet".to_string())?;
            finish_exit(entry, exit, &mut self.retention());
            return Ok(snapshot(entry));
        }
        let mut state = self.state_guard()?;
        let entry = state
            .get_mut(key)
            .ok_or_else(|| "Das Laufwerk wird nicht verwaltet".to_string())?;
        if entry.recovery.is_clean() {
            let keep = self.retention().keeps(entry);
            finish_clean(entry, keep);
        }
        Ok(snapshot(entry))
    }

    pub(super) fn retry(&self, id: &MountId, host: &ShareHost) -> Result<MountSnapshot, String> {
        let _lifecycle = self.lifecycle_guard()?;
        let config = {
            let state = self.state_guard()?;
            let entry = state
                .get(id.as_str())
                .ok_or_else(|| "Das Laufwerk wird nicht verwaltet".to_string())?;
            entry.config.clone()
        };
        let _ = self.stop_locked(id)?;
        let registry_recorded = {
            let mut state = self.state_guard()?;
            let entry = state
                .get(id.as_str())
                .ok_or_else(|| "Das Laufwerk wird nicht verwaltet".to_string())?;
            if entry.child.is_some() || entry.backend_stream_active {
                return Err(
                    "Die vorherige Laufwerk-Generation beendet noch einen Remote-Aufruf; Retry ist erst danach sicher"
                        .into(),
                );
            }
            let registry_recorded = entry.registry_recorded;
            state.remove(id.as_str());
            registry_recorded
        };
        self.start_locked(config, host, registry_recorded)
    }

    pub(super) fn tick(&self) {
        // Serialize registry cleanup with the Start/Stop/Retry lifecycle.
        let Ok(_lifecycle) = self.lifecycle.try_lock() else {
            return;
        };
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let mut retention = self.retention();
        for entry in state.values_mut() {
            let Some(child) = entry.child.as_mut() else {
                continue;
            };
            let observation = child.try_wait();
            match observation {
                Ok(Some(exit)) => {
                    finish_exit(entry, exit, &mut retention);
                }
                Ok(None) => {}
                Err(error) => {
                    entry_state::terminate_after_observation_error(entry, &error, &mut retention);
                }
            }
        }
        for entry in state.values_mut() {
            if entry.child.is_none()
                && !entry.backend_stream_active
                && entry.recovery.is_clean()
                && matches!(entry.status, MountStatus::Unmounted)
            {
                let keep = retention.keeps(entry);
                finish_clean(entry, keep);
            }
        }
        // `Unmounted` is reported before the host process returns. Retain its
        // Child handle until try_wait observes the exit so finish_clean owns
        // registry removal and runtime teardown exactly once.
        state.retain(|_, entry| !removable(entry));
    }

    /// Worker stop. DRV-M3: drives that restore at login keep their
    /// registry record, so the next worker mounts them again.
    pub(super) fn stop_all(&self) {
        self.retention().begin_shutdown();
        let ids: Vec<_> = self
            .state
            .lock()
            .map(|state| {
                state
                    .values()
                    .map(|entry| entry.config.id.clone())
                    .collect()
            })
            .unwrap_or_default();
        for id in ids {
            // The worker log names the drive a stop waits for, and why one
            // did not end cleanly.
            super::state::log(&format!("daemon stop: ending drive {id}"));
            if let Err(error) = self.stop(&id) {
                super::state::log(&format!("daemon stop: drive {id} did not end: {error}"));
            }
        }
    }

    fn state_guard(&self) -> Result<MutexGuard<'_, HashMap<String, MountEntry>>, String> {
        self.state
            .lock()
            .map_err(|_| "Laufwerk-Verwaltung ist gesperrt".to_string())
    }

    fn lifecycle_guard(&self) -> Result<MutexGuard<'_, ()>, String> {
        self.lifecycle
            .lock()
            .map_err(|_| "Laufwerk-Lifecycle ist gesperrt".to_string())
    }

    /// Lock order: `state` before `retention`. The book holds flags and ids
    /// only, so a poisoned lock keeps its values.
    fn retention(&self) -> MutexGuard<'_, restore::Retention> {
        self.retention
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn ensure_registry_ready(&self) -> Result<(), String> {
        let error = self
            .registry_error
            .lock()
            .map_err(|_| "Laufwerk-Registry-Status ist gesperrt".to_string())?;
        match error.as_ref() {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }
}

/// A registered drive that does not run.
fn registered_entry(config: MountConfig, recovery: MountRecovery, detail: String) -> MountEntry {
    MountEntry {
        config,
        status: MountStatus::Failed { detail },
        backend: None,
        capabilities: None,
        launch_token: None,
        session_token: None,
        backend_token: None,
        control: None,
        child: None,
        backend_stream_active: false,
        pending_host_failure: None,
        host_terminal_failure: false,
        registry_recorded: true,
        recovery,
        lock_end: false,
        lock_pending: None,
        stop_requested: false,
    }
}
