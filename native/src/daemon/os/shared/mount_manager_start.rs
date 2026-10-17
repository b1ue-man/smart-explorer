//! The start path of a managed drive, moved out of `mount_manager.rs` without
//! a change of behaviour (DRV-M3, plan 2026-10-17 D1) before the restore at
//! login reuses it. DRV-M4 adds the Linux path here.
use crate::mount::{MountConfig, MountSnapshot, MountStatus};

use super::super::ipc_host::ShareHost;
#[cfg(windows)]
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
            let backend =
                match super::super::mount_source::resolve(&config, host).and_then(|backend| {
                    super::super::rooted_backend::RootedBackend::new(
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
            if !registry_recorded {
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
            match super::super::mount_process::spawn(
                &config.id,
                &launch_token,
                ipc_addr,
                &cache_root,
            ) {
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

    // Only the Windows start path fails this way until DRV-M4 adds Linux.
    #[cfg_attr(not(windows), allow(dead_code))]
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
}
