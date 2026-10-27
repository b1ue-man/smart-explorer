//! Test support of the drive-host session flow (`screen_metadata_runtime_task_`
//! tests in `screen_metadata_runtime_task_tests.rs`): a drive entry exactly as
//! `start_locked_retained` leaves it right before the host process starts
//! (backend, capabilities, launch token, status `Mounting`), and a spool key
//! that stands in for the vault register of the worker process
//! (`VaultDriveBridge::spool_key`, covered by the `vault_task_spool_key_*`
//! tests of `vault/os/shared/spool_key.rs`).
use std::collections::HashMap;
use std::io;
use std::sync::{Mutex, OnceLock, PoisonError};

use crate::mount::{MountConfig, MountId, MountRecovery, MountSnapshot, MountStatus};
use crate::vault::SpoolKey;
use crate::vfs::BackendHandle;

use super::super::{registered_entry, snapshot, MountManager};

fn keys() -> &'static Mutex<HashMap<String, [u8; 32]>> {
    static KEYS: OnceLock<Mutex<HashMap<String, [u8; 32]>>> = OnceLock::new();
    KEYS.get_or_init(Mutex::default)
}

/// The spool key the worker answers for `mount_id`.
pub(in crate::daemon) fn inject_spool_key(mount_id: &MountId, raw: [u8; 32]) {
    keys()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(mount_id.as_str().to_string(), raw);
}

/// `None` for every mount no test injected a key for.
pub(super) fn injected_spool_key(mount_id: &str) -> Option<io::Result<SpoolKey>> {
    let raw = keys()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(mount_id)
        .copied()?;
    Some(SpoolKey::from_bytes(&raw))
}

impl MountManager {
    /// Adds `config` as a drive whose host may attach with `launch_token`.
    pub(in crate::daemon) fn arm_host_for_test(
        &self,
        config: MountConfig,
        backend: BackendHandle,
        launch_token: &str,
    ) -> Result<(), String> {
        let capabilities =
            crate::daemon::ipc_protocol::MountBackendCapabilities::from_backend(&backend);
        let mut entry = registered_entry(config, MountRecovery::Clean, String::new());
        entry.status = MountStatus::Mounting;
        entry.registry_recorded = false;
        entry.backend = Some(backend);
        entry.capabilities = Some(capabilities);
        entry.launch_token = Some(launch_token.to_string());
        let key = entry.config.id.as_str().to_string();
        self.state_guard()?.insert(key, entry);
        Ok(())
    }

    /// The table row of `id`, as `ListMounts` shows it.
    pub(in crate::daemon) fn snapshot_for_test(&self, id: &MountId) -> Option<MountSnapshot> {
        let state = self.state_guard().ok()?;
        state.get(id.as_str()).map(snapshot)
    }

    /// Drops the row of `id`; its control channel closes with it, so the
    /// worker's control thread and the host's control reader end.
    pub(in crate::daemon) fn forget_for_test(&self, id: &MountId) {
        if let Ok(mut state) = self.state_guard() {
            state.remove(id.as_str());
        }
    }
}
