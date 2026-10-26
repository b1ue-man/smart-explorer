//! The end of vault drives and their return after an unlock (DRV-M7;
//! S2-V1, S2-V6, S2-V7, C3.3 of the vault strand).
//!
//! - `MountTerminated { site, mount_id, reason }`: every end of a vault
//!   drive's host is classified (`user_unmount`, `external_unmount`,
//!   `host_exit`, `lock`) and handed to the vault side on a thread of its
//!   own: the drive's consumer goes, its folder's thumbnails go, and unless
//!   the lock ended it the vault locks when nothing else holds the session.
//! - A vault that is locked when a drive starts defers the drive instead of
//!   failing it: status `Failed` with the code `vault_locked`, and the
//!   number of sealed changes as `MountRecovery::RequiresUnlock`. Decision
//!   (U06 question 1): a drive restored at login waits for the unlock and
//!   mounts then, like the drives of other devices wait for Share.
//! - A lock keeps the drive (decision, U06 question 2): its registry record
//!   stays, its status says it waits for the unlock, and every unlock of the
//!   vault mounts it again, which publishes its sealed changes. An explicit
//!   unmount still forgets the drive, and a worker stop keeps only drives
//!   that restore at login, as for every drive.
//! - The hooks into the vault side (lock participant, unlock watch) are
//!   installed once, at the first start of a vault drive.
//! - Where a vault drive is mounted reaches the plaintext boundary of the
//!   vault strand (`vault::publish_vault_mount` when the host reports its
//!   folder or letter, `vault::withdraw_vault_mount` at its end; U20).
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use serde::Serialize;

use crate::mount::{MountConfig, MountRecovery, MountSnapshot, MountSource, MountStatus};
use crate::vault::VAULT_LOCKED_CODE;

use super::super::ipc_host::ShareHost;
use super::lock_participant::{LockDesk, MountLockParticipant};
use super::{registered_entry, snapshot, MountEntry, MountManager};

/// The vault drives of one worker: the lock desk and the hooks.
#[derive(Default)]
pub(super) struct VaultDrives {
    installed: AtomicBool,
    pub(super) desk: LockDesk,
}

/// Why a vault drive ended (`MountTerminated`, contract V2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::daemon) enum MountEndReason {
    UserUnmount,
    ExternalUnmount,
    HostExit,
    Lock,
}

/// The end of a vault drive (S2-V6).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(in crate::daemon) struct MountTerminated {
    pub(in crate::daemon) site: String,
    pub(in crate::daemon) mount_id: String,
    pub(in crate::daemon) reason: MountEndReason,
}

/// The site of a vault drive.
pub(super) fn vault_site(config: &MountConfig) -> Option<&str> {
    match &config.source {
        MountSource::Vault { site_id } => Some(site_id.as_str()),
        _ => None,
    }
}

/// A status text that carries the code `vault_locked` (CLI
/// `failed:vault_locked`, `requires-unlock:<n>` with sealed changes).
pub(in crate::daemon) fn is_vault_locked(detail: &str) -> bool {
    detail
        .strip_prefix(VAULT_LOCKED_CODE)
        .is_some_and(|rest| rest.starts_with(": "))
}

/// The status of a vault drive that waits for the unlock of its vault.
pub(super) fn waiting_status(site: &str, pending: Option<u32>) -> MountStatus {
    let changes = match pending {
        Some(count) => format!("; {count} versiegelte Änderung(en) werden dann gespeichert"),
        None => String::new(),
    };
    MountStatus::Failed {
        detail: format!(
            "{VAULT_LOCKED_CODE}: Tresor gesperrt (se vault unlock {site}); das Laufwerk wird nach dem Entsperren wieder eingebunden{changes}"
        ),
    }
}

/// The end of the host of `entry`, decided before its exit is booked.
pub(super) fn terminated(entry: &MountEntry, exit_ok: bool) -> Option<MountTerminated> {
    let site = vault_site(&entry.config)?;
    let reason = if entry.lock_end {
        MountEndReason::Lock
    } else if entry.stop_requested {
        MountEndReason::UserUnmount
    } else if exit_ok && matches!(entry.status, MountStatus::Unmounted) {
        MountEndReason::ExternalUnmount
    } else {
        MountEndReason::HostExit
    };
    Some(MountTerminated {
        site: site.to_string(),
        mount_id: entry.config.id.as_str().to_string(),
        reason,
    })
}

/// Mounted roots of vault drives by mount id: site and folder or letter.
fn mounted_roots() -> &'static Mutex<HashMap<String, (String, PathBuf)>> {
    static ROOTS: OnceLock<Mutex<HashMap<String, (String, PathBuf)>>> = OnceLock::new();
    ROOTS.get_or_init(Default::default)
}

/// A host reported its status: a mounted vault drive is published to the
/// plaintext boundary (index, Share roots, clipboard, transfer, cleanup).
pub(super) fn reported(config: &MountConfig, status: &MountStatus) {
    let Some(site) = vault_site(config) else {
        return;
    };
    let root = match status {
        MountStatus::MountedAt { path } => path.clone(),
        MountStatus::Mounted { drive } | MountStatus::Conflict { drive, .. } => {
            PathBuf::from(format!("{}:\\", drive.get()))
        }
        _ => return,
    };
    crate::vault::publish_vault_mount(site, &root);
    mounted_roots()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(config.id.as_str().to_string(), (site.to_string(), root));
}

/// The mounted root of drive `mount_id` leaves the plaintext boundary.
fn withdraw(mount_id: &str) {
    let known = mounted_roots()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(mount_id);
    if let Some((site, root)) = known {
        crate::vault::withdraw_vault_mount(&site, &root);
    }
}

/// After the exit is booked: a lock keeps the drive waiting for the next
/// unlock; every end reaches the vault side through `notify`.
pub(super) fn after_exit(
    entry: &mut MountEntry,
    end: MountTerminated,
    notify: impl FnOnce(MountTerminated, Option<PathBuf>),
) {
    withdraw(&end.mount_id);
    entry.stop_requested = false;
    if end.reason == MountEndReason::Lock {
        entry.lock_end = false;
        let pending = entry.lock_pending.take().filter(|count| *count > 0);
        if let (Some(count), false) = (pending, entry.recovery.is_clean()) {
            entry.recovery = MountRecovery::RequiresUnlock { pending: count };
        }
        entry.status = waiting_status(&end.site, entry.recovery.pending_unlock());
    }
    let place = entry.config.location_directory().map(PathBuf::from);
    notify(end, place);
}

/// Hands an end to the vault side without holding the drive table.
pub(super) fn notify_vault(end: MountTerminated, place: Option<PathBuf>) {
    let spawned = std::thread::Builder::new()
        .name("vault-drive-end".into())
        .spawn(move || {
            crate::vault::vault_drive_bridge().ended(
                &end.site,
                &end.mount_id,
                place.as_deref(),
                end.reason == MountEndReason::Lock,
            );
        });
    if let Err(error) = spawned {
        super::super::state::log(&format!("Tresor-Laufwerksende nicht gemeldet: {error}"));
    }
}

/// Whether `entry` waits for the unlock of `site`.
fn waits_for_unlock(entry: &MountEntry, site: &str) -> bool {
    vault_site(&entry.config) == Some(site)
        && entry.registry_recorded
        && entry.child.is_none()
        && !entry.backend_stream_active
        && (entry.recovery.pending_unlock().is_some()
            || matches!(&entry.status, MountStatus::Failed { detail } if is_vault_locked(detail)))
}

impl MountManager {
    /// Installs the lock participant and the unlock watch once.
    pub(super) fn install_vault_hooks(&self, host: &ShareHost) {
        if self.vault.installed.swap(true, Ordering::SeqCst) {
            return;
        }
        crate::vault::register().add_participant(Arc::new(MountLockParticipant {
            manager: self.clone(),
        }));
        let manager = self.clone();
        let host = host.clone();
        crate::vault::vault_drive_bridge().watch(Arc::new(move |site: &str| {
            manager.restore_vault(site, &host);
        }));
    }

    /// A drive whose vault is locked at its start waits for the unlock.
    pub(super) fn defer_until_unlock(
        &self,
        key: &str,
        cause: String,
        pending: Option<usize>,
    ) -> Result<MountSnapshot, String> {
        let mut state = self.state_guard()?;
        let entry = state
            .get_mut(key)
            .ok_or_else(|| "Laufwerk-Start wurde abgebrochen".to_string())?;
        entry.backend = None;
        entry.capabilities = None;
        entry.launch_token = None;
        entry.pending_host_failure = None;
        entry.host_terminal_failure = false;
        entry.recovery = pending.map_or(MountRecovery::Unknown, MountRecovery::awaiting_unlock);
        let tail = if entry.registry_recorded {
            "; das Laufwerk wird nach dem Entsperren eingebunden"
        } else {
            ""
        };
        let inventory = if pending.is_none() {
            "; Anzahl offener Änderungen bis zum Entsperren unbekannt; Cache bleibt unverändert erhalten"
        } else {
            ""
        };
        entry.status = MountStatus::Failed {
            detail: format!("{cause}{tail}{inventory}"),
        };
        Ok(snapshot(entry))
    }

    /// Mounts every drive of `site` that waits for its unlock (the unlock
    /// watch of the vault side calls it on a thread of its own).
    pub(in crate::daemon) fn restore_vault(&self, site: &str, host: &ShareHost) {
        let candidates: Vec<String> = match self.state_guard() {
            Ok(state) => state
                .iter()
                .filter(|(_, entry)| waits_for_unlock(entry, site))
                .map(|(key, _)| key.clone())
                .collect(),
            Err(_) => return,
        };
        for key in candidates {
            let Ok(_lifecycle) = self.lifecycle_guard() else {
                return;
            };
            let taken = {
                let Ok(mut state) = self.state_guard() else {
                    return;
                };
                match state.get(&key) {
                    Some(entry) if waits_for_unlock(entry, site) => state
                        .remove(&key)
                        .map(|entry| (entry.config, entry.recovery)),
                    _ => None,
                }
            };
            let Some((config, recovery)) = taken else {
                continue;
            };
            if let Err(error) = self.start_locked(config.clone(), host, true) {
                if let Ok(mut state) = self.state_guard() {
                    let detail = format!("Wiederherstellung nach dem Entsperren: {error}");
                    state
                        .entry(key)
                        .or_insert_with(|| registered_entry(config, recovery, detail));
                }
            }
        }
    }
}

/// Linux: a vault drive appears only below the vault place on tmpfs
/// (S2-V7, decision S3 OE2); a folder elsewhere is refused before the start.
#[cfg(target_os = "linux")]
pub(super) fn vault_place_refusal(config: &MountConfig) -> Option<String> {
    use crate::mount::os::linux_os::mount_point::{place_kind, PlaceKind};
    let folder = config.location_directory()?;
    (vault_site(config).is_some() && place_kind(folder) != PlaceKind::Vault).then(|| {
        "Tresor-Laufwerke brauchen einen Arbeitsspeicher-Einhängeort unter $XDG_RUNTIME_DIR/smart-explorer/Tresore".to_string()
    })
}

#[cfg(test)]
#[path = "mount_termination_tests.rs"]
mod tests;
