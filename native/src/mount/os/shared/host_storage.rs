//! Vault drives in the shared host flow (DRV-M7; contracts V1, S2-V2 to
//! S2-V4, S2-V10).
//!
//! Storage: a drive over the cleartext view of a vault (`Scheme::Vault`)
//! opens its engine with the sealed storage of its spool key, which the
//! daemon hands over on request (`Vault::SpoolKey`, checked with the host's
//! session token); every other drive keeps the plain storage and the bytes
//! of every earlier release. Without the key (the vault locked meanwhile)
//! the host mounts nothing and reports how many sealed changes wait for the
//! unlock (`MountRecovery::RequiresUnlock`); its journal stays untouched.
//!
//! Lock: the daemon is the lock participant of the drive; its order reaches
//! the control loop (`HostControl::lock_order`). The host publishes what it
//! can and drops clean cached content (`MountFilesystem::prepare_lock`). A
//! lock someone asked for without „Änderungen verschlüsselt aufbewahren“
//! stops there when changes remain: the drive stays mounted and the answer
//! says why. Otherwise the drive unmounts like at an unmount request, what
//! is left stays sealed in the journal, and the host answers only after its
//! volume is gone. A host that misses the deadline is aborted by the daemon;
//! its journal keeps the sealed changes as well.
use std::path::Path;
use std::time::Instant;

use crate::mount::{
    LockReadiness, MountEngine, MountFilesystem, MountId, MountRecovery, MountStorage,
};
use crate::vault::{SpoolKey, VAULT_LOCKED_CODE};

use super::super::host_platform::HostPlatform;
use super::{finish, report_failure, report_failure_with_recovery, Ended, HostControl, HostExit};

/// What the daemon answered to `Vault::SpoolKey`.
pub(crate) enum SpoolKeyAnswer {
    Key(SpoolKey),
    /// The vault is locked (code `vault_locked`); the text names the site.
    Locked(String),
    Failed(String),
}

/// A lock of the drive's vault, as the daemon ordered it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LockOrder {
    /// When the daemon aborts the drive.
    pub(crate) deadline: Instant,
    /// Forced lock or „Sperren, Änderungen verschlüsselt aufbewahren“:
    /// unpublished changes stay sealed and the drive unmounts anyway.
    pub(crate) keep_unsaved: bool,
}

/// The host's answer to a lock order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LockAnswer {
    /// Changes, deletes and namespace intents still unpublished.
    pub(crate) pending: usize,
    /// The volume is gone; `false` when the lock was refused.
    pub(crate) unmounted: bool,
    /// Why publishing stopped, when it did.
    pub(crate) detail: Option<String>,
}

/// The storage of this run. `Err` carries the failure, already reported.
pub(super) fn storage_for(
    control: &dyn HostControl,
    vault: bool,
    spool_root: &Path,
    id: &MountId,
) -> Result<MountStorage, String> {
    if !vault {
        return Ok(MountStorage::plain());
    }
    match control.spool_key() {
        SpoolKeyAnswer::Key(key) => crate::vault::sealed_mount_storage(&key).map_err(|error| {
            report_failure(
                control,
                &format!("the vault drive storage could not be prepared: {error}"),
            )
        }),
        SpoolKeyAnswer::Locked(text) => {
            let recovery = crate::mount::spool::audit_pending(spool_root, id)
                .map_or(MountRecovery::Unknown, MountRecovery::awaiting_unlock);
            Err(report_failure_with_recovery(
                control,
                &format!("{VAULT_LOCKED_CODE}: {text}"),
                recovery,
            ))
        }
        SpoolKeyAnswer::Failed(text) => Err(report_failure(
            control,
            &format!("the vault drive key is unavailable: {text}"),
        )),
    }
}

/// Publishes and drops clean content for a lock: what remains, and
/// whether the drive unmounts.
pub(crate) fn prepare_for_lock(engine: &MountEngine, order: &LockOrder) -> (LockReadiness, bool) {
    let readiness = MountFilesystem::prepare_lock(engine).unwrap_or_else(|error| LockReadiness {
        pending: engine
            .dirty_entries()
            .map_or(1, |entries| entries.len().max(1)),
        detail: Some(error.to_string()),
    });
    let unmount = order.keep_unsaved || readiness.pending == 0;
    (readiness, unmount)
}

/// One turn of the control loop for a lock order; `Some` when the drive
/// ended for the lock.
pub(super) fn lock_turn(
    platform: &mut dyn HostPlatform,
    control: &dyn HostControl,
    engine: &MountEngine,
) -> Option<Ended> {
    let order = control.lock_order()?;
    let (readiness, unmount) = prepare_for_lock(engine, &order);
    if !unmount {
        control.answer_lock(LockAnswer {
            pending: readiness.pending,
            unmounted: false,
            detail: readiness.detail,
        });
        return None;
    }
    let ended = finish(platform, control, engine, None, HostExit::Stopped);
    // Closing the volume publishes what its open files still held.
    let pending = engine
        .dirty_entries()
        .map_or(readiness.pending, |entries| entries.len());
    control.answer_lock(LockAnswer {
        pending,
        unmounted: true,
        detail: readiness.detail,
    });
    Some(ended)
}

#[cfg(test)]
#[path = "host_storage_tests.rs"]
mod tests;
