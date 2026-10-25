//! Descriptor-relative recovery with verified private capture before deletion.
use std::{ffi::OsStr, fs::File, io, os::unix::io::AsRawFd, path::Path};
use crate::local_access::{ConfinedDirectory, DirectoryHandle};

pub(crate) fn recover_transfer_child(
    parent: &ConfinedDirectory,
    name: &OsStr,
    expected_identity: &str,
    destination: Option<&OsStr>,
) -> io::Result<()> {
    let expected = parent.entry(name)?;
    if crate::copy::local_file_identity_handle(&expected)? != expected_identity {
        return Err(changed());
    }
    if let Some(destination) = destination {
        // No byte is destroyed by restore. A child exchanged in the narrow
        // check/rename interval is detected afterwards and put back without
        // replacing a subsequently created entry.
        parent.rename(name, parent, destination, false)?;
        let reached = parent.entry(destination)?;
        if crate::copy::local_file_identity_handle(&reached)? != expected_identity {
            return Err(match parent.rename(destination, parent, name, false) {
                Ok(()) => changed(),
                Err(error) => io::Error::new(io::ErrorKind::InvalidData,
                    format!("transfer recovery source changed; retained at {destination:?}; rollback failed: {error}")),
            });
        }
        return parent.file().sync_all();
    }
    // Reopen the held descriptor, never the replaceable user path. The
    // existing LocalAccess capture puts the entry into an exclusive 0700
    // directory and validates it there before any deletion can occur.
    let anchor = format!("/proc/self/fd/{}/.", parent.file().as_raw_fd());
    let directory = DirectoryHandle::open_root(Path::new(&anchor))?;
    let mut captured = directory.quarantine_regular_child(name, &expected)?;
    let verified = crate::copy::local_file_identity_handle(captured.file()).and_then(|identity| {
        if identity == expected_identity { Ok(()) } else { Err(changed()) }
    });
    if let Err(cause) = verified {
        let retained = captured.retained_location();
        return Err(match captured.restore() {
            Ok(()) => cause,
            Err(error) => io::Error::new(cause.kind(),
                format!("{cause}; transfer cleanup retained at {}; rollback failed: {error}", retained.display())),
        });
    }
    captured.dispose().map_err(|error| io::Error::new(error.kind(), format!(
        "transfer cleanup at {}: {error}", captured.retained_location().display(),
    )))?;
    parent.file().sync_all()
}

fn changed() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "transfer recovery file identity changed")
}

pub(crate) fn recover_transfer_child_live(
    parent: &ConfinedDirectory, name: &OsStr, expected: &File,
    destination: Option<&OsStr>,
) -> io::Result<()> {
    let identity = super::file_identity(expected)?;
    let current = parent.entry(name)?;
    let class = crate::local_access::classify_open_file(&current)?;
    if !current.metadata()?.is_file() || class.link_like || class.special
        || super::file_identity(&current)? != identity { return Err(changed()); }
    if let Some(destination) = destination {
        parent.rename(name, parent, destination, false)?;
        if super::file_identity(&parent.entry(destination)?)? != identity {
            return Err(match parent.rename(destination, parent, name, false) {
                Ok(()) => changed(),
                Err(error) => io::Error::new(io::ErrorKind::InvalidData, format!(
                    "live transfer source changed; retained at {destination:?}; rollback failed: {error}")),
            });
        }
        return parent.file().sync_all();
    }
    let anchor = format!("/proc/self/fd/{}/.", parent.file().as_raw_fd());
    let directory = DirectoryHandle::open_root(Path::new(&anchor))?;
    // The original File stays alive through capture and disposal. The private
    // capture verifies this same live inode before permitting any deletion.
    let mut captured = directory.quarantine_regular_child(name, expected)?;
    captured.dispose().map_err(|error| io::Error::new(error.kind(), format!(
        "live transfer cleanup retained at {}: {error}", captured.retained_location().display(),
    )))?;
    parent.file().sync_all()
}
