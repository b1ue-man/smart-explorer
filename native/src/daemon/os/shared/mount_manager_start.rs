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
use crate::mount::{MountConfig, MountSnapshot, MountStatus};

use super::super::ipc_host::ShareHost;
#[cfg(any(windows, target_os = "linux"))]
use super::{random_token, write_capabilities};
use super::{snapshot, start_cache, MountManager};

impl MountManager {
    /// Starts a drive; the caller holds the lifecycle lock.
    /// `registry_recorded` says that the registry already holds `config`
    /// (Retry, restore at login), so it is not written again.
    pub(super) fn start_locked(
        &self,
        config: MountConfig,
        host: &ShareHost,
        registry_recorded: bool,
    ) -> Result<MountSnapshot, String> {
        let key = config.id.as_str().to_string();
        self.forget_update_stop(&config.id);
        start_cache::insert(self, &key, config.clone(), registry_recorded)?;

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
            let cache_root = match start_cache::prepare(self, &key, &config.id) {
                Ok(path) => path,
                Err(error) => return self.fail_start(&key, error),
            };
            #[cfg(windows)]
            let folder_added = false;
            #[cfg(target_os = "linux")]
            let (config, folder_added) = match self.with_folder(&key, config) {
                Ok(placed) => placed,
                Err(error) => return self.fail_start(&key, error),
            };
            let backend =
                match super::super::mount_source::resolve(&config, host).and_then(|backend| {
                    rooted(backend, &config)
                        .map_err(|error| format!("Laufwerk-Wurzel absichern: {error}"))
                }) {
                    Ok(backend) => backend,
                    Err(error) => return self.fail_start(&key, error),
                };
            let capabilities =
                super::super::ipc_protocol::MountBackendCapabilities::from_backend(&backend);
            if config.mode == crate::mount::MountMode::ReadWrite {
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
            match spawn_host(&config, &launch_token, ipc_addr, &cache_root) {
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

#[cfg(windows)]
fn rooted(
    backend: crate::vfs::BackendHandle,
    config: &MountConfig,
) -> std::io::Result<crate::vfs::BackendHandle> {
    super::super::rooted_backend::RootedBackend::new(
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
