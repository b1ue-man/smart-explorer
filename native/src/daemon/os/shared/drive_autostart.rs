//! The login entry that restores drives (DRV-M3, strand S2 C2 of plan
//! 2026-10-17): `~/.config/autostart/smart-explorer-drives.desktop` under
//! Linux, the Run value `SmartExplorerDrives` under Windows. Both start
//! `<program> --sync-daemon --drive-restore` (`run_start_reason.rs`).
//!
//! The entry exists exactly while a registered drive restores at login
//! (`MountConfig::restores_at_login`: „Beim Anmelden automatisch einbinden“
//! or the device tree). The registry calls `reconcile` after every change and
//! the worker once at its start, which also brings the program path of an
//! existing entry up to date. An entry the desktop switched off stays off.
//! Android has no login entry: the app starts its own worker.
use std::io;

use crate::mount::MountConfig;

/// Whether any registered drive needs the login entry.
pub(crate) fn restore_needed(configs: &[MountConfig]) -> bool {
    configs.iter().any(MountConfig::restores_at_login)
}

/// One login entry: the platform adapter or a test double.
pub(crate) trait RestoreEntry {
    fn present(&self) -> bool;
    /// Creates the entry or brings its program path up to date; an entry the
    /// desktop switched off is left as it is.
    fn write(&self) -> io::Result<()>;
    fn remove(&self) -> io::Result<()>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EntryChange {
    Written,
    Removed,
    Unchanged,
}

/// Makes the entry follow the need: written while needed, removed otherwise.
pub(crate) fn reconcile_with(entry: &dyn RestoreEntry, needed: bool) -> io::Result<EntryChange> {
    if needed {
        entry.write().map(|()| EntryChange::Written)
    } else if entry.present() {
        entry.remove().map(|()| EntryChange::Removed)
    } else {
        Ok(EntryChange::Unchanged)
    }
}

/// After a registry change. A failure is logged; it never undoes the change
/// of the drive itself.
pub(crate) fn reconcile(configs: &[MountConfig]) {
    if let Err(error) = reconcile_with(&PlatformEntry, restore_needed(configs)) {
        super::super::state::log(&format!("drive restore login entry: {error}"));
    }
}

/// At the start of the worker. A registry that cannot be read leaves the
/// entry as it is.
pub(crate) fn reconcile_registered() {
    match super::super::mount_registry::load() {
        Ok(configs) => reconcile(&configs),
        Err(error) => super::super::state::log(&format!(
            "drive restore login entry not checked: registry unreadable: {error}"
        )),
    }
}

struct PlatformEntry;

impl RestoreEntry for PlatformEntry {
    fn present(&self) -> bool {
        platform::present()
    }

    fn write(&self) -> io::Result<()> {
        platform::write()
    }

    fn remove(&self) -> io::Result<()> {
        platform::remove()
    }
}

#[cfg(not(target_os = "android"))]
mod platform {
    pub(super) fn present() -> bool {
        crate::autostart::drive_restore_registered()
    }

    pub(super) fn write() -> std::io::Result<()> {
        crate::autostart::register_drive_restore()
    }

    pub(super) fn remove() -> std::io::Result<()> {
        crate::autostart::unregister_drive_restore()
    }
}

/// Android: no login entry (module comment).
#[cfg(target_os = "android")]
mod platform {
    pub(super) fn present() -> bool {
        false
    }

    pub(super) fn write() -> std::io::Result<()> {
        Ok(())
    }

    pub(super) fn remove() -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{reconcile_with, restore_needed, EntryChange, RestoreEntry};
    use crate::mount::{BackendRoot, DriveSelection, MountConfig, MountId, MountMode, MountSource};
    use std::cell::RefCell;
    use std::io;

    /// A login entry in memory; `switched_off` stands for an entry the
    /// desktop disabled (`Hidden=true`, `StartupApproved`).
    #[derive(Default)]
    struct FakeEntry {
        present: RefCell<bool>,
        switched_off: bool,
        writes: RefCell<usize>,
        removals: RefCell<usize>,
    }

    impl RestoreEntry for FakeEntry {
        fn present(&self) -> bool {
            *self.present.borrow()
        }

        fn write(&self) -> io::Result<()> {
            *self.writes.borrow_mut() += 1;
            if !self.switched_off {
                *self.present.borrow_mut() = true;
            }
            Ok(())
        }

        fn remove(&self) -> io::Result<()> {
            *self.removals.borrow_mut() += 1;
            *self.present.borrow_mut() = false;
            Ok(())
        }
    }

    fn drive(id: &str, source: MountSource, auto_remount: bool) -> MountConfig {
        MountConfig::new(
            MountId::parse(id).unwrap(),
            source,
            DriveSelection::Automatic,
            MountMode::ReadOnly,
            id,
        )
        .unwrap()
        .with_auto_remount(auto_remount)
    }

    fn saved(id: &str, auto_remount: bool) -> MountConfig {
        drive(
            id,
            MountSource::SavedRemote {
                account: "sftp:prod".into(),
                root: BackendRoot::parse("/srv").unwrap(),
            },
            auto_remount,
        )
    }

    #[test]
    fn drive_task_autostart_entry_follows_need() {
        let entry = FakeEntry::default();
        // No drive: nothing to restore, no entry is created.
        assert!(!restore_needed(&[]));
        assert_eq!(
            reconcile_with(&entry, false).unwrap(),
            EntryChange::Unchanged
        );
        assert!(!entry.present());
        // Drives without the option keep the behaviour of v0.5.182.
        let plain = [saved("plain", false)];
        assert!(!restore_needed(&plain));
        assert_eq!(
            reconcile_with(&entry, restore_needed(&plain)).unwrap(),
            EntryChange::Unchanged
        );
        // „Beim Anmelden automatisch einbinden“ on one drive creates it.
        let mut drives = [saved("plain", false), saved("login", true)];
        assert!(restore_needed(&drives));
        assert_eq!(
            reconcile_with(&entry, restore_needed(&drives)).unwrap(),
            EntryChange::Written
        );
        assert!(entry.present());
        // Every later change refreshes it while it is needed (program path).
        assert_eq!(
            reconcile_with(&entry, restore_needed(&drives)).unwrap(),
            EntryChange::Written
        );
        assert_eq!(*entry.writes.borrow(), 2);
        // Switching the option off on the last such drive removes it.
        drives[1].auto_remount = false;
        assert!(!restore_needed(&drives));
        assert_eq!(
            reconcile_with(&entry, restore_needed(&drives)).unwrap(),
            EntryChange::Removed
        );
        assert!(!entry.present());
        // The device tree restores at login without the option (S2 C2).
        let devices = [drive("devices", MountSource::Devices, false)];
        assert!(restore_needed(&devices));
        assert_eq!(reconcile_with(&entry, true).unwrap(), EntryChange::Written);
        assert!(entry.present());
        // Removing the last drive that needs it removes the entry.
        assert_eq!(reconcile_with(&entry, false).unwrap(), EntryChange::Removed);
        assert_eq!(*entry.removals.borrow(), 2);
        // A vault drive restores only with the option.
        let vault = drive(
            "vault",
            MountSource::Vault {
                site_id: "site-1".into(),
            },
            false,
        );
        assert!(!restore_needed(&[vault.clone()]));
        assert!(restore_needed(&[vault.with_auto_remount(true)]));
        // An entry the desktop switched off stays off: the adapter is asked
        // to write, and it keeps the switched-off entry as it is.
        let switched_off = FakeEntry {
            switched_off: true,
            ..FakeEntry::default()
        };
        assert_eq!(
            reconcile_with(&switched_off, true).unwrap(),
            EntryChange::Written
        );
        assert!(!switched_off.present());
    }
}
