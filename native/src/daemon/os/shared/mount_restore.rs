//! The restore at login and its option (DRV-M3, strand S2 C2 and C4 of plan
//! 2026-10-17).
//!
//! A drive with „Beim Anmelden automatisch einbinden“, and the device tree,
//! restores at login (`MountConfig::restores_at_login`). Every worker start
//! mounts such drives again after the cache audit of
//! `MountManager::from_registry`, through the start path of Retry with the
//! registry record kept (`start_locked(…, true)`), so the registry is not
//! rewritten. Drives of other devices (`MountSource::uses_share`) wait while
//! the worker is restricted (decision S2 K7, `run_start_reason.rs`); its
//! promotion restores them. Every other drive keeps the behaviour of
//! v0.5.182: it waits for a Retry.
//!
//! `Retention` decides which registry records survive a clean end: a drive
//! that restores at login keeps its record across a worker stop
//! (`stop_all`) and an update stop (exit 75, contract V8); an explicit
//! unmount forgets every drive, as before.
use std::collections::HashSet;
use std::io;

use crate::mount::{MountConfig, MountId, MountRecovery, MountSnapshot, MountStatus};

use super::super::ipc_host::ShareHost;
use super::{registered_entry, snapshot, MountEntry, MountManager};

/// Status of a drive of another device while the worker waits for Share.
pub(super) const DEFERRED_DETAIL: &str = "Wartet auf Smart Explorer Share: Laufwerke anderer Geräte werden eingebunden, sobald Smart Explorer, se oder der Autostart des Hintergrund-Syncs Share startet";
/// Prefix of the status of a drive whose restore failed to start.
pub(super) const RESTORE_FAILED_PREFIX: &str = "Wiederherstellung beim Start: ";

#[derive(Default)]
pub(super) struct Retention {
    shutting_down: bool,
    /// Drives that ended on an update stop and keep their record.
    kept: HashSet<String>,
}

impl Retention {
    /// Whether a clean end keeps the registry record of `entry`.
    pub(super) fn keeps(&self, entry: &MountEntry) -> bool {
        entry.config.restores_at_login()
            && (self.shutting_down || self.kept.contains(entry.config.id.as_str()))
    }

    /// An update stop (exit 75): the next worker mounts the drive again.
    pub(super) fn keep_after_update_stop(&mut self, entry: &MountEntry) -> bool {
        if entry.config.restores_at_login() {
            self.kept.insert(entry.config.id.as_str().to_string());
        }
        self.keeps(entry)
    }

    pub(super) fn begin_shutdown(&mut self) {
        self.shutting_down = true;
    }

    fn forget(&mut self, id: &MountId) {
        self.kept.remove(id.as_str());
    }
}

impl MountManager {
    /// Mounts every registered drive that restores at login and does not
    /// run; `share_allowed` is false while the worker is restricted.
    pub(in crate::daemon) fn restore(
        &self,
        host: &ShareHost,
        share_allowed: bool,
    ) -> Vec<MountSnapshot> {
        if !crate::mount::drive_mount_supported() {
            return Vec::new();
        }
        self.restore_with(share_allowed, |manager, config| {
            manager.start_locked(config, host, true)
        })
    }

    pub(super) fn restore_with(
        &self,
        share_allowed: bool,
        mut start: impl FnMut(&Self, MountConfig) -> Result<MountSnapshot, String>,
    ) -> Vec<MountSnapshot> {
        let candidates = match self.state_guard() {
            Ok(state) => {
                let mut ids: Vec<_> = state
                    .values()
                    .filter(|entry| restorable(entry))
                    .map(|entry| entry.config.id.as_str().to_string())
                    .collect();
                ids.sort();
                ids
            }
            Err(_) => return Vec::new(),
        };
        let mut restored = Vec::new();
        for key in candidates {
            let Ok(_lifecycle) = self.lifecycle_guard() else {
                break;
            };
            let (config, recovery) = {
                let Ok(mut state) = self.state_guard() else {
                    break;
                };
                let Some(entry) = state.get_mut(&key) else {
                    continue;
                };
                // A Retry or an eject may have come first.
                if !restorable(entry) {
                    continue;
                }
                if !share_allowed && entry.config.source.uses_share() {
                    entry.status = MountStatus::Failed {
                        detail: DEFERRED_DETAIL.into(),
                    };
                    continue;
                }
                let taken = (entry.config.clone(), entry.recovery);
                state.remove(&key);
                taken
            };
            match start(self, config.clone()) {
                Ok(snapshot) => restored.push(snapshot),
                Err(error) => {
                    self.put_back(config, recovery, format!("{RESTORE_FAILED_PREFIX}{error}"))
                }
            }
        }
        restored
    }

    /// A start that failed before it kept an entry: the drive stays in the
    /// table with its record and the evidence of its journal.
    fn put_back(&self, config: MountConfig, recovery: MountRecovery, detail: String) {
        let Ok(mut state) = self.state_guard() else {
            return;
        };
        let key = config.id.as_str().to_string();
        state
            .entry(key)
            .or_insert_with(|| registered_entry(config, recovery, detail));
    }

    /// `se drive set`, the drive window: switches „Beim Anmelden automatisch
    /// einbinden“; a recorded drive is stored at once, which also updates
    /// the login entry (`mount_registry::upsert`).
    pub(in crate::daemon) fn set_options(
        &self,
        id: &MountId,
        auto_remount: bool,
    ) -> Result<MountSnapshot, String> {
        self.set_options_with(id, auto_remount, super::super::mount_registry::upsert)
    }

    pub(super) fn set_options_with(
        &self,
        id: &MountId,
        auto_remount: bool,
        persist: impl FnOnce(&MountConfig) -> io::Result<()>,
    ) -> Result<MountSnapshot, String> {
        let _lifecycle = self.lifecycle_guard()?;
        let mut state = self.state_guard()?;
        let entry = state
            .get_mut(id.as_str())
            .ok_or_else(|| "Das Laufwerk wird nicht verwaltet".to_string())?;
        if entry.config.auto_remount != auto_remount {
            let config = entry.config.clone().with_auto_remount(auto_remount);
            if entry.registry_recorded {
                persist(&config).map_err(|error| format!("Laufwerk-Registry sichern: {error}"))?;
            }
            entry.config = config;
        }
        Ok(snapshot(entry))
    }

    /// A new start of the drive: a later clean end decides afresh.
    pub(super) fn forget_update_stop(&self, id: &MountId) {
        self.retention().forget(id);
    }
}

/// A registered drive that restores at login and does not run.
fn restorable(entry: &MountEntry) -> bool {
    entry.config.restores_at_login()
        && entry.registry_recorded
        && entry.child.is_none()
        && !entry.backend_stream_active
        && matches!(
            entry.status,
            MountStatus::Failed { .. }
                | MountStatus::RuntimeUnavailable { .. }
                | MountStatus::Unmounted
        )
}

#[cfg(test)]
#[path = "mount_restore_tests.rs"]
mod tests;
