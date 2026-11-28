//! Descriptor-relative recovery with verified private capture before deletion.
//!
//! Removable media (2026-11-27):
//! - U1: FAT, exFAT, NTFS and FUSE drivers may take owner and mode from the
//!   mount (`uid=`, `dmask=`), and then refuse the owner-private capture
//!   folder. There the app's own stage or quarantine is removed directly:
//!   `unlinkat` relative to the held parent, right after its entry was
//!   confirmed to be the held object (device and inode). The private folder
//!   adds nothing on such a volume: the mount, not the entry, decides who may
//!   write each name on it. Other filesystems keep the refusal.
//! - U3: a restore without replacement where the filesystem lacks
//!   `RENAME_NOREPLACE` (FUSE without rename2, such as ntfs-3g: `EINVAL`)
//!   takes the way `publish_no_replace` takes, through the held parent.
use crate::local_access::{ConfinedDirectory, DirectoryHandle};
use std::{
    ffi::OsStr,
    fs::File,
    io,
    os::unix::{fs::MetadataExt, io::AsRawFd},
    path::{Path, PathBuf},
};

pub fn recover_transfer_child(
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
        rename_back(parent, name, destination)?;
        let reached = parent.entry(destination)?;
        if crate::copy::local_file_identity_handle(&reached)? != expected_identity {
            return Err(match rename_back(parent, destination, name) {
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
    let mut captured = match directory.quarantine_regular_child(name, &expected) {
        Ok(captured) => captured,
        Err(refusal) => return remove_uncapturable(parent, name, &expected, refusal),
    };
    let verified = crate::copy::local_file_identity_handle(captured.file()).and_then(|identity| {
        if identity == expected_identity {
            Ok(())
        } else {
            Err(changed())
        }
    });
    if let Err(cause) = verified {
        let retained = captured.retained_location();
        return Err(match captured.restore() {
            Ok(()) => cause,
            Err(error) => io::Error::new(
                cause.kind(),
                format!(
                    "{cause}; transfer cleanup retained at {}; rollback failed: {error}",
                    retained.display()
                ),
            ),
        });
    }
    captured.dispose().map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "transfer cleanup at {}: {error}",
                captured.retained_location().display(),
            ),
        )
    })?;
    parent.file().sync_all()
}

fn changed() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "transfer recovery file identity changed",
    )
}

pub fn recover_transfer_child_live(
    parent: &ConfinedDirectory,
    name: &OsStr,
    expected: &File,
    destination: Option<&OsStr>,
) -> io::Result<()> {
    let identity = super::file_identity(expected)?;
    let current = parent.entry(name)?;
    let class = crate::local_access::classify_open_file(&current)?;
    if !current.metadata()?.is_file()
        || class.link_like
        || class.special
        || super::file_identity(&current)? != identity
    {
        return Err(changed());
    }
    if let Some(destination) = destination {
        rename_back(parent, name, destination)?;
        if super::file_identity(&parent.entry(destination)?)? != identity {
            return Err(match rename_back(parent, destination, name) {
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
    let mut captured = match directory.quarantine_regular_child(name, expected) {
        Ok(captured) => captured,
        Err(refusal) => return remove_uncapturable(parent, name, expected, refusal),
    };
    captured.dispose().map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "live transfer cleanup retained at {}: {error}",
                captured.retained_location().display(),
            ),
        )
    })?;
    parent.file().sync_all()
}

/// U1: the capture was refused before anything moved (a folder that cannot
/// be owner-private, a rename the filesystem cannot make without replacing)
/// on a filesystem whose modes may come from the mount: the held entry is
/// removed directly. Any other refusal, or the same one on another
/// filesystem, is returned unchanged. A failed direct removal keeps the
/// refusal's kind, so no caller reads it as an already vanished entry.
fn remove_uncapturable(
    parent: &ConfinedDirectory,
    name: &OsStr,
    expected: &File,
    refusal: io::Error,
) -> io::Result<()> {
    let mount_modes = matches!(
        refusal.kind(),
        io::ErrorKind::PermissionDenied | io::ErrorKind::Unsupported
    ) && super::linux_fs_kind::magic(parent.file())
        .is_ok_and(super::linux_fs_kind::owner_modes_unstorable);
    if !mount_modes {
        return Err(refusal);
    }
    remove_own_file(parent, name, expected).map_err(|error| {
        io::Error::new(
            refusal.kind(),
            format!("{refusal}; direct removal of the own entry failed: {error}"),
        )
    })
}

/// Unlinks `name` in the held parent only while it names the held regular
/// file `expected` (same device and inode; the held handle keeps the inode
/// from being reused), then flushes the parent.
fn remove_own_file(parent: &ConfinedDirectory, name: &OsStr, expected: &File) -> io::Result<()> {
    let held = expected.metadata()?;
    let current = parent.entry(name)?.metadata()?;
    if !current.is_file() || current.dev() != held.dev() || current.ino() != held.ino() {
        return Err(changed());
    }
    parent.remove(name, false)?;
    parent.file().sync_all()
}

/// U3: `renameat2(RENAME_NOREPLACE)` relative to the held parent; where the
/// filesystem lacks the flag, `publish_no_replace` through the held
/// descriptor: the look at the destination, then the ladder of
/// `android_fs::rename_no_replace` (a create-only hard link, else a checked
/// rename only for the app's own randomly named stage as the source, the
/// residual risk documented there).
fn rename_back(parent: &ConfinedDirectory, from: &OsStr, to: &OsStr) -> io::Result<()> {
    match parent.rename(from, parent, to, false) {
        Err(error)
            if matches!(
                error.raw_os_error(),
                Some(libc::EINVAL | libc::ENOSYS | libc::EOPNOTSUPP)
            ) =>
        {
            rename_through_anchor(parent, from, to)
        }
        result => result,
    }
}

/// The fallback of `rename_back`: both names spelled below the held
/// parent's descriptor, so a renamed or replaced ancestor path is never
/// followed.
fn rename_through_anchor(parent: &ConfinedDirectory, from: &OsStr, to: &OsStr) -> io::Result<()> {
    let anchor = PathBuf::from(format!("/proc/self/fd/{}", parent.file().as_raw_fd()));
    super::linux_publish::publish_no_replace(&anchor.join(from), &anchor.join(to))
}

#[cfg(test)]
#[path = "linux_removable_tests.rs"]
mod removable_tests;
