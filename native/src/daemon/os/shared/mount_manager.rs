use std::collections::HashMap;
use std::process::Child;
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::mount::{MountConfig, MountId, MountRecovery, MountSnapshot, MountStatus};
use crate::vfs::BackendHandle;

use super::ipc_host::ShareHost;

// A remote callback is allowed to run for five minutes. Eject must not kill a
// host while Dokany is still legitimately completing that durable flush.
pub(super) const STOP_GRACE: Duration = Duration::from_secs(330);

#[path = "mount_manager_state.rs"]
mod entry_state;
#[path = "mount_manager_host.rs"]
mod host;
#[path = "mount_manager_start_cache.rs"]
mod start_cache;
#[path = "mount_manager_capabilities.rs"]
mod write_capabilities;

use entry_state::{finish_clean, finish_exit, random_token, removable, snapshot};

#[derive(Clone)]
pub(super) struct MountManager {
    state: Arc<Mutex<HashMap<String, MountEntry>>>,
    lifecycle: Arc<Mutex<()>>,
    registry_error: Arc<Mutex<Option<String>>>,
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
    child: Option<Child>,
    backend_stream_active: bool,
    registry_recorded: bool,
    recovery: MountRecovery,
}

impl Default for MountManager {
    fn default() -> Self {
        let (entries, registry_error) = match super::mount_registry::load() {
            Ok(configs) => {
                let recovery = start_cache::audit_registered(&configs);
                let entries = configs
                    .into_iter()
                    .zip(recovery)
                    .map(|(config, recovery)| {
                        let key = config.id.as_str().to_string();
                        (
                            key,
                            MountEntry {
                                config,
                                status: MountStatus::Failed {
                                    detail: "Laufwerk nach Daemon-Neustart zur Wiederherstellung bereit; bitte erneut versuchen".into(),
                                },
                                backend: None,
                                capabilities: None,
                                launch_token: None,
                                session_token: None,
                                backend_token: None,
                                control: None,
                                child: None,
                                backend_stream_active: false,
                                registry_recorded: true,
                                recovery,
                            },
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
        }
    }
}

impl MountManager {
    pub(super) fn start(
        &self,
        config: MountConfig,
        host: &ShareHost,
    ) -> Result<MountSnapshot, String> {
        config
            .validate()
            .map_err(|error| format!("Ungueltige Laufwerk-Konfiguration: {error}"))?;
        let _lifecycle = self.lifecycle_guard()?;
        #[cfg(windows)]
        self.ensure_registry_ready()?;
        self.start_locked(config, host, false)
    }

    fn start_locked(
        &self,
        config: MountConfig,
        host: &ShareHost,
        registry_recorded: bool,
    ) -> Result<MountSnapshot, String> {
        let key = config.id.as_str().to_string();
        start_cache::insert(self, &key, config.clone(), registry_recorded)?;

        #[cfg(not(windows))]
        {
            let _ = host;
            if let Ok(mut state) = self.state_guard() {
                state.remove(&key);
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "unsupported platform: Virtuelle Laufwerke werden nur unter Windows unterstuetzt",
            )
            .to_string());
        }

        #[cfg(windows)]
        {
            let cache_root = match start_cache::prepare(self, &key, &config.id) {
                Ok(path) => path,
                Err(error) => return self.fail_start(&key, error),
            };
            let backend = match super::mount_source::resolve(&config, host).and_then(|backend| {
                super::rooted_backend::RootedBackend::new(
                    backend,
                    config.source.root(),
                    config.mode,
                    config.root_security,
                )
                .map_err(|error| format!("Laufwerk-Wurzel absichern: {error}"))
            }) {
                Ok(backend) => backend,
                Err(error) => return self.fail_start(&key, error),
            };
            let capabilities =
                super::ipc_protocol::MountBackendCapabilities::from_backend(&backend);
            if config.mode == crate::mount::MountMode::ReadWrite {
                let staged_write = capabilities.staged_write();
                if !staged_write.supports_mounted_writes() {
                    return self.fail_start(
                        &key,
                        write_capabilities::missing(backend.scheme(), staged_write),
                    );
                }
            }
            if !registry_recorded {
                if let Err(error) = super::mount_registry::upsert(&config) {
                    return self.fail_start(&key, format!("Laufwerk-Registry sichern: {error}"));
                }
                let mut state = self.state_guard()?;
                let entry = state
                    .get_mut(&key)
                    .ok_or_else(|| "Laufwerk-Start wurde abgebrochen".to_string())?;
                entry.registry_recorded = true;
            }
            let launch_token = match random_token() {
                Ok(token) => token,
                Err(error) => return self.fail_start(&key, error),
            };
            let Some(ipc_addr) = super::ipc_storage::read_ipc_addr() else {
                return self.fail_start(&key, "Laufwerk-IPC-Adresse ist nicht verfuegbar".into());
            };
            if !ipc_addr.ip().is_loopback() {
                return self.fail_start(&key, "Laufwerk-IPC-Adresse ist nicht lokal".into());
            }
            {
                let mut state = self.state_guard()?;
                let entry = state
                    .get_mut(&key)
                    .ok_or_else(|| "Laufwerk-Start wurde abgebrochen".to_string())?;
                entry.backend = Some(backend);
                entry.capabilities = Some(capabilities);
                entry.launch_token = Some(launch_token.clone());
            }
            match super::mount_process::spawn(&config.id, &launch_token, ipc_addr, &cache_root) {
                Ok(child) => {
                    let mut state = self.state_guard()?;
                    let entry = state
                        .get_mut(&key)
                        .ok_or_else(|| "Laufwerk-Start wurde abgebrochen".to_string())?;
                    entry.child = Some(child);
                    Ok(snapshot(entry))
                }
                Err(error) => self.fail_start(&key, format!("Laufwerk-Host starten: {error}")),
            }
        }
    }

    fn fail_start(&self, key: &str, detail: String) -> Result<MountSnapshot, String> {
        let mut state = self.state_guard()?;
        let entry = state
            .get_mut(key)
            .ok_or_else(|| "Laufwerk-Start wurde abgebrochen".to_string())?;
        entry.backend = None;
        entry.capabilities = None;
        entry.launch_token = None;
        entry.status = MountStatus::Failed { detail };
        Ok(snapshot(entry))
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
            entry.control.clone()
        };
        if let Some(control) = control {
            let _ = control.send(());
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
        let Some(child) = entry.child.as_mut() else {
            if entry.recovery.is_clean() {
                finish_clean(entry);
            }
            return Ok(Some(snapshot(entry)));
        };
        match child.try_wait() {
            Ok(Some(exit)) => {
                entry.child = None;
                finish_exit(entry, exit);
                Ok(Some(snapshot(entry)))
            }
            Ok(None) => Ok(None),
            Err(error) => {
                entry.status = MountStatus::Failed {
                    detail: format!("Laufwerk-Hoststatus lesen: {error}"),
                };
                Ok(None)
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
            finish_exit(entry, exit);
            return Ok(snapshot(entry));
        }
        let mut state = self.state_guard()?;
        let entry = state
            .get_mut(key)
            .ok_or_else(|| "Das Laufwerk wird nicht verwaltet".to_string())?;
        if entry.recovery.is_clean() {
            finish_clean(entry);
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
        // Registry updates are read-modify-write transactions. Share the same
        // lifecycle lock as Start/Stop/Retry so a clean-exit removal cannot
        // race and overwrite a newly persisted mount.
        let Ok(_lifecycle) = self.lifecycle.try_lock() else {
            return;
        };
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        for entry in state.values_mut() {
            let Some(child) = entry.child.as_mut() else {
                continue;
            };
            match child.try_wait() {
                Ok(Some(exit)) => {
                    entry.child = None;
                    finish_exit(entry, exit);
                }
                Ok(None) => {}
                Err(error) => {
                    entry.status = MountStatus::Failed {
                        detail: format!("Laufwerk-Hoststatus lesen: {error}"),
                    };
                }
            }
        }
        for entry in state.values_mut() {
            if entry.child.is_none()
                && !entry.backend_stream_active
                && entry.recovery.is_clean()
                && matches!(entry.status, MountStatus::Unmounted)
            {
                finish_clean(entry);
            }
        }
        // `Unmounted` is reported before the host process returns. Retain its
        // Child handle until try_wait observes the exit so finish_clean owns
        // registry removal and runtime teardown exactly once.
        state.retain(|_, entry| !removable(entry));
    }

    pub(super) fn stop_all(&self) {
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
            let _ = self.stop(&id);
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
