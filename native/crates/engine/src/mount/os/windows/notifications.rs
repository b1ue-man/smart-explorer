//! DLL notifications are issued only by the filesystem-owning host loop.
//! Draining, invalidation and path mapping are the shared change pump
//! (`os/shared/change_pump.rs`, DRV-M2); this sink turns a callback path
//! into the `X:\…` form of `DokanNotify*`.
use std::io;

use super::super::shared::change_pump::{ChangePump, ChangeSink};
use super::DokanyFileSystem;
use crate::mount::{DriveLetter, MountFilesystem};

#[derive(Default)]
pub(super) struct HostNotifications {
    pump: ChangePump,
}

impl HostNotifications {
    /// The volume harnesses hold the engine itself.
    #[cfg(test)]
    pub(super) fn deliver(
        &mut self,
        engine: &crate::mount::MountEngine,
        filesystem: &DokanyFileSystem,
        drive: DriveLetter,
    ) -> io::Result<()> {
        self.deliver_from(engine, filesystem, drive)
    }

    pub(super) fn deliver_from(
        &mut self,
        source: &dyn MountFilesystem,
        filesystem: &DokanyFileSystem,
        drive: DriveLetter,
    ) -> io::Result<()> {
        self.pump
            .deliver(source, &mut DokanySink { filesystem, drive })
    }
}

struct DokanySink<'a> {
    filesystem: &'a DokanyFileSystem,
    drive: DriveLetter,
}

impl ChangeSink for DokanySink<'_> {
    fn created(&mut self, callback_path: &str, is_directory: bool) -> io::Result<()> {
        let path = mounted_path(self.drive, callback_path)?;
        accepted(self.filesystem.notify_create(&path, is_directory))
    }

    fn deleted(&mut self, callback_path: &str, is_directory: bool) -> io::Result<()> {
        let path = mounted_path(self.drive, callback_path)?;
        accepted(self.filesystem.notify_delete(&path, is_directory))
    }

    fn modified(&mut self, callback_path: &str) -> io::Result<()> {
        let path = mounted_path(self.drive, callback_path)?;
        accepted(self.filesystem.notify_update(&path))
    }
}

fn accepted(delivered: bool) -> io::Result<()> {
    if delivered {
        Ok(())
    } else {
        // Retain this bounded batch; a transient delivery failure must not
        // turn an already drained change into a lost notification.
        Err(io::Error::other(
            "Dokany remote-change notification was not accepted",
        ))
    }
}

fn mounted_path(drive: DriveLetter, callback: &str) -> io::Result<Vec<u16>> {
    let mut path = format!("{}:{callback}", drive.get())
        .encode_utf16()
        .collect::<Vec<_>>();
    if path.len() >= 32_767 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "notification pathname is too long",
        ));
    }
    path.push(0);
    Ok(path)
}
