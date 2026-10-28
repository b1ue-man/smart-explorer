//! The start path of a managed drive, moved out of `mount_manager.rs` without
//! a change of behaviour (DRV-M3, plan 2026-10-17 D1) before the restore at
//! login reuses it.
//!
//! DRV-M4 adds Linux: the folder of the drive is resolved once (the
//! configured one, else the first free default below `~/SmartExplorer`) and
//! stored with the drive, so it stays the same at every mount and no two
//! drives share one; the backend is rooted with the POSIX name rules, and
//! the FUSE host starts. Only Android starts no drive (it offers folders
//! through its documents provider, DRV-M8).
//!
//! DRV-M7 adds vault drives (`MountSource::Vault`): the hooks into the vault
//! side are installed at the first one; a locked vault defers the drive
//! until its unlock (`mount_termination.rs`); a running drive holds the
//! vault session; under Linux it appears only below the vault place.
use crate::mount::{MountConfig, MountId, MountSnapshot, MountStatus};
use std::sync::Arc;

use super::super::ipc_host::ShareHost;
#[cfg(any(windows, target_os = "linux"))]
use super::{random_token, write_capabilities};
use super::{snapshot, start_cache, MountManager};

impl MountManager {
    pub(in crate::daemon) fn start(
        &self,
        config: MountConfig,
        host: &ShareHost,
    ) -> Result<MountSnapshot, String> {
        self.start_retained(config, host, None)
    }

    pub(in crate::daemon) fn start_retained(
        &self,
        config: MountConfig,
        host: &ShareHost,
        lease: Option<Arc<dyn Send + Sync>>,
    ) -> Result<MountSnapshot, String> {
        config
            .validate()
            .map_err(|error| format!("Ungueltige Laufwerk-Konfiguration: {error}"))?;
        let _lifecycle = self.lifecycle_guard()?;
        // Windows and Linux record the drive in the registry; a registry
        // that failed to load is never written over.
        #[cfg(any(windows, target_os = "linux"))]
        self.ensure_registry_ready()?;
        self.start_locked_retained(config, host, false, lease, start_cache::StartEntry::New)
    }

    pub(super) fn retry(&self, id: &MountId, host: &ShareHost) -> Result<MountSnapshot, String> {
        self.retry_retained(id, host, None)
    }

    pub(in crate::daemon) fn retry_retained(
        &self,
        id: &MountId,
        host: &ShareHost,
        lease: Option<Arc<dyn Send + Sync>>,
    ) -> Result<MountSnapshot, String> {
        self.retry_with(id, |manager, config, registry_recorded| {
            manager.start_locked_retained(
                config,
                host,
                registry_recorded,
                lease,
                start_cache::StartEntry::Retry,
            )
        })
    }

    fn retry_with(
        &self,
        id: &MountId,
        start: impl FnOnce(&Self, MountConfig, bool) -> Result<MountSnapshot, String>,
    ) -> Result<MountSnapshot, String> {
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
            let state = self.state_guard()?;
            let entry = state
                .get(id.as_str())
                .ok_or_else(|| "Das Laufwerk wird nicht verwaltet".to_string())?;
            start_cache::ensure_retryable(entry)?;
            entry.registry_recorded
        };
        let result = start(self, config, registry_recorded);
        if let Err(detail) = &result {
            let mut state = self.state_guard()?;
            if let Some(entry) = state.get_mut(id.as_str()) {
                // A clean Stop may have set Unmounted before a preflight
                // failed. Keep its Retry action; richer failures (including
                // vault_locked) and their recovery evidence remain intact.
                if entry.child.is_none()
                    && !entry.backend_stream_active
                    && matches!(entry.status, MountStatus::Unmounted)
                {
                    entry.status = MountStatus::Failed {
                        detail: detail.clone(),
                    };
                }
            }
        }
        result
    }

    /// Starts a drive; the caller holds the lifecycle lock.
    /// `registry_recorded` says that the registry already holds `config`
    /// (Retry, restore at login), so it is not written again.
    pub(super) fn start_locked(
        &self,
        config: MountConfig,
        host: &ShareHost,
        registry_recorded: bool,
    ) -> Result<MountSnapshot, String> {
        self.start_locked_retained(
            config,
            host,
            registry_recorded,
            None,
            start_cache::StartEntry::New,
        )
    }

    fn start_locked_retained(
        &self,
        mut config: MountConfig,
        host: &ShareHost,
        registry_recorded: bool,
        lease: Option<Arc<dyn Send + Sync>>,
        admission: start_cache::StartEntry,
    ) -> Result<MountSnapshot, String> {
        if super::super::mount_registry::inactive::is_inactive(&config.id)
            .map_err(|error| format!("Laufwerk-Recovery-Eigentümer lesen: {error}"))?
        {
            return Err(
                "Laufwerk liegt im inaktiven Recoverybestand; zuerst ausdrücklich reaktivieren"
                    .into(),
            );
        }
        if let crate::mount::MountSource::Vault { site_id } = &config.source {
            let policy = crate::vault::mount_policy(site_id)?;
            if policy.read_only {
                config.mode = crate::mount::MountMode::ReadOnly;
            }
            // Current explicit consent wins over a stale strict config. A legacy
            // Trusted flag alone never grants consent to a newly resolved root.
            if policy.trusted {
                config.root_security = crate::mount::MountRootSecurity::Trusted;
            }
        }
        let key = config.id.as_str().to_string();
        match admission {
            start_cache::StartEntry::New => {
                start_cache::insert(self, &key, config.clone(), registry_recorded, lease)
            }
            start_cache::StartEntry::Retry => {
                start_cache::restart(self, &key, config.clone(), registry_recorded, lease)
            }
        }?;
        self.forget_update_stop(&config.id);
        let _start_end = StartEnd {
            manager: self,
            key: &key,
        };
        let vault_site = super::termination::vault_site(&config).map(str::to_string);
        if vault_site.is_some() {
            self.install_vault_hooks(host);
        }

        #[cfg(not(any(windows, target_os = "linux")))]
        {
            let _ = host;
            if let Ok(mut state) = self.state_guard() {
                state.remove(&key);
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "unsupported platform: Virtuelle Laufwerke werden nur unter Windows und Linux unterstuetzt",
            )
            .to_string());
        }

        #[cfg(any(windows, target_os = "linux"))]
        {
            let (cache_root, pending) = match start_cache::prepare(self, &key, &config.id) {
                Ok(prepared) => prepared,
                Err(error) => return self.fail_start(&key, error),
            };
            #[cfg(windows)]
            let folder_added = false;
            #[cfg(target_os = "linux")]
            let (config, folder_added) = match self.with_folder(&key, config) {
                Ok(placed) => placed,
                Err(error) => return self.fail_start(&key, error),
            };
            let backend = match backend_of(&config, host, &cache_root) {
                Ok(backend) => backend,
                Err(error) if super::termination::is_vault_locked(&error) => {
                    return self.defer_until_unlock(&key, error, pending)
                }
                Err(error) => return self.fail_start(&key, error),
            };
            let capabilities =
                super::super::ipc_protocol::MountBackendCapabilities::from_backend(&backend);
            // The device tree opens its devices one by one (DRV-M6): their
            // write guarantees are checked per device, not here.
            if config.mode == crate::mount::MountMode::ReadWrite && !config.source.is_device_tree()
            {
                let staged_write = capabilities.staged_write();
                if !staged_write.supports_mounted_writes() {
                    return self.fail_start(
                        &key,
                        write_capabilities::missing(backend.scheme(), staged_write),
                    );
                }
            }
            if !registry_recorded || folder_added {
                if let Err(error) = super::super::mount_registry::upsert(&config) {
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
            let Some(ipc_addr) = super::super::ipc_storage::read_ipc_addr() else {
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
            // A running vault drive holds the session of its vault (C3.3).
            if let Some(site) = &vault_site {
                let place = config.location_directory().map(std::path::PathBuf::from);
                if let Err(error) = crate::vault::vault_drive_bridge().started(site, &key, place) {
                    let cause = format!("{}: {error}", crate::vault::VAULT_LOCKED_CODE);
                    return self.defer_until_unlock(&key, cause, pending);
                }
            }
            let spawned = spawn_host(&config, &launch_token, ipc_addr, &cache_root);
            if let (Err(_), Some(site)) = (&spawned, &vault_site) {
                // No host ran: the consumer goes without the lock rule.
                crate::vault::vault_drive_bridge().ended(site, &key, None, true);
            }
            match spawned {
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

    // Android starts no drive host, so it never fails a start this way.
    #[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
    pub(super) fn fail_start(&self, key: &str, detail: String) -> Result<MountSnapshot, String> {
        let mut state = self.state_guard()?;
        let entry = state
            .get_mut(key)
            .ok_or_else(|| "Laufwerk-Start wurde abgebrochen".to_string())?;
        if entry.child.is_none() {
            entry.startup_lease = None;
        }
        entry.backend = None;
        entry.capabilities = None;
        entry.launch_token = None;
        entry.pending_host_failure = None;
        entry.host_terminal_failure = false;
        entry.status = MountStatus::Failed { detail };
        Ok(snapshot(entry))
    }

    /// The folder of a Linux drive; `true` when it was resolved now and
    /// must be stored. The table entry carries it from here on.
    #[cfg(target_os = "linux")]
    fn with_folder(&self, key: &str, config: MountConfig) -> Result<(MountConfig, bool), String> {
        use crate::mount::os::linux_os::mount_point;
        use std::path::{Path, PathBuf};

        let others: Vec<PathBuf> = {
            let state = self.state_guard()?;
            state
                .iter()
                .filter(|(other, _)| other.as_str() != key)
                .filter_map(|(_, entry)| entry.config.location_directory().map(Path::to_path_buf))
                .collect()
        };
        let used = |folder: &Path| others.iter().any(|other| other == folder);
        if let Some(folder) = config.location_directory() {
            if used(folder) {
                return Err(format!(
                    "Der Einhängeort {} gehört schon zu einem anderen Laufwerk",
                    folder.display()
                ));
            }
            if let Some(refusal) = super::termination::vault_place_refusal(&config) {
                return Err(refusal);
            }
            return Ok((config, false));
        }
        let root = match config.source {
            crate::mount::MountSource::Vault { .. } => mount_point::vaults_root(),
            _ => mount_point::drives_root(),
        }
        .map_err(|error| format!("Einhängeort: {error}"))?;
        let folder = mount_point::default_location(&root, &config.label, |candidate| {
            used(candidate) || !mount_point::folder_is_free(candidate)
        })
        .map_err(|error| format!("Einhängeort: {error}"))?;
        let config = config
            .with_location_directory(Some(folder))
            .map_err(|error| format!("Einhängeort: {error}"))?;
        let mut state = self.state_guard()?;
        let entry = state
            .get_mut(key)
            .ok_or_else(|| "Laufwerk-Start wurde abgebrochen".to_string())?;
        entry.config = config.clone();
        Ok((config, true))
    }
}

/// The backend the host is handed: the rooted backend of the source, or for
/// the device tree the placeholder of the handshake.
#[cfg(any(windows, target_os = "linux"))]
fn backend_of(
    config: &MountConfig,
    host: &ShareHost,
    cache: &std::path::Path,
) -> Result<crate::vfs::BackendHandle, String> {
    if config.source.is_device_tree() {
        return Ok(super::super::drive_modules::devices_mount::placeholder_backend());
    }
    super::super::mount_source::resolve(config, host).and_then(|backend| {
        super::super::mount_recovery_identity::authorize(config, &backend, cache)?;
        if let crate::mount::MountSource::Vault { site_id } = &config.source {
            let policy = crate::vault::mount_policy(site_id)?;
            crate::vault::note_mount_binding(config.id.as_str(), policy.binding);
        }
        rooted(backend, config).map_err(|error| format!("Laufwerk-Wurzel absichern: {error}"))
    })
}

#[cfg(windows)]
fn rooted(
    backend: crate::vfs::BackendHandle,
    config: &MountConfig,
) -> std::io::Result<crate::vfs::BackendHandle> {
    super::super::rooted_backend::RootedBackend::open_windows(
        backend,
        config.source.root(),
        config.mode,
        config.root_security,
    )
}

/// Linux drives show every name a POSIX system can hold (DRV-M1, DRV-M4).
#[cfg(target_os = "linux")]
fn rooted(
    backend: crate::vfs::BackendHandle,
    config: &MountConfig,
) -> std::io::Result<crate::vfs::BackendHandle> {
    super::super::rooted_backend::RootedBackend::new_in(
        backend,
        config.source.root(),
        config.mode,
        config.root_security,
        crate::mount::PathDialect::Posix,
    )
}

#[cfg(windows)]
fn spawn_host(
    config: &MountConfig,
    launch_token: &str,
    ipc_addr: std::net::SocketAddr,
    cache_root: &std::path::Path,
) -> std::io::Result<super::super::mount_host_process::MountHostProcess> {
    super::super::mount_process::spawn(&config.id, launch_token, ipc_addr, cache_root)
}

#[cfg(target_os = "linux")]
fn spawn_host(
    config: &MountConfig,
    launch_token: &str,
    ipc_addr: std::net::SocketAddr,
    cache_root: &std::path::Path,
) -> std::io::Result<super::super::mount_host_process::MountHostProcess> {
    let folder = config.location_directory().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "das Laufwerk hat keinen Einhängeort",
        )
    })?;
    super::super::mount_process::spawn(&config.id, launch_token, ipc_addr, cache_root, folder)
}

/// A failed/deferred synchronous start owns no child. Never retain its lease
/// forever; failures with a live or unobserved child remain protected.
struct StartEnd<'a> {
    manager: &'a MountManager,
    key: &'a str,
}
impl Drop for StartEnd<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.manager.state_guard() {
            if let Some(entry) = state.get_mut(self.key) {
                if entry.child.is_none() {
                    entry.startup_lease = None;
                }
            }
        }
    }
}

/// Shared start fence: an early backend stream outlives a failed host, while
/// authenticated Mounted releases every copy together, including idle streams.
#[derive(Clone)]
pub(super) struct StartupLease {
    retained: Arc<std::sync::Mutex<Option<Arc<dyn Send + Sync>>>>,
}
impl StartupLease {
    pub(super) fn new(retained: Arc<dyn Send + Sync>) -> Self {
        Self {
            retained: Arc::new(std::sync::Mutex::new(Some(retained))),
        }
    }
    pub(super) fn mounted(&self) {
        self.retained
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
    }
}

#[cfg(test)]
#[path = "batches_mount_retry_regression_task_tests.rs"]
mod batches_mount_retry_regression_task_tests;
