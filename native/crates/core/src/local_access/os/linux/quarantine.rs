//! Reversible capture and moves remain descriptor-relative. In contrast to
//! ordinary staging, security-sensitive restore never uses a checked rename.
use std::collections::hash_map::RandomState;
use std::ffi::{CString, OsStr, OsString};
use std::fs::File;
use std::hash::{BuildHasher, Hash, Hasher};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;

use super::{validate_name, DirectoryHandle};

const HELD_NAME: &str = "content";

/// An expected regular child captured in an exclusively created 0700 folder.
/// No path from this guard is an authorization to call a path-based deleter.
/// Call `restore` on cancellation/errors, or `move_to` for a checked trash
/// destination. A failed restore retains the content, never overwrites.
#[must_use = "restore the captured child or move it to a checked destination"]
pub struct QuarantinedChild {
    parent: DirectoryHandle,
    original: OsString,
    directory: DirectoryHandle,
    file: File,
    active: bool,
}

impl DirectoryHandle {
    pub fn quarantine_regular_child(
        &self,
        name: &OsStr,
        expected: &File,
    ) -> io::Result<QuarantinedChild> {
        validate_name(name)?;
        let expected_metadata = expected.metadata()?;
        super::super::regular_or_refusal(expected_metadata.file_type())?;
        if !same_object(&self.metadata_at(name)?, &expected_metadata) {
            return Err(changed());
        }
        // The fresh folder's permissions precede any content. Its handle is
        // kept even when the selected root or a parent is moved meanwhile.
        let directory = self.private_quarantine_directory()?;
        if let Err(cause) = rename_no_replace(self, name, &directory, OsStr::new(HELD_NAME)) {
            // Nothing was captured: the fresh folder goes again.
            return Err(self.discard_empty_capture(directory, cause));
        }
        let captured = directory
            .open_regular_child(OsStr::new(HELD_NAME))
            .and_then(|file| {
                if same_object(&file.metadata()?, &expected_metadata) {
                    Ok(file)
                } else {
                    Err(changed())
                }
            });
        match captured {
            Ok(file) => Ok(QuarantinedChild {
                parent: self.clone(),
                original: name.to_os_string(),
                directory,
                file,
                active: true,
            }),
            Err(cause) => match rename_no_replace(&directory, OsStr::new(HELD_NAME), self, name) {
                Ok(()) => Err(self.discard_empty_capture(directory, cause)),
                Err(restore) => Err(io::Error::new(
                    cause.kind(),
                    format!(
                        "{cause}; captured entry retained at {}: {restore}",
                        retained_path(&directory).display(),
                    ),
                )),
            },
        }
    }

    /// Removes a capture folder that holds nothing (again): exactly the
    /// opened object, and `rmdir` refuses it once anything was put inside.
    /// The cause stays the reported error; a failed removal is appended.
    fn discard_empty_capture(&self, directory: Self, cause: io::Error) -> io::Error {
        let Some(name) = directory.path.file_name().map(OsStr::to_os_string) else {
            return cause;
        };
        let kept = retained_path(&directory);
        match self.remove_empty_child(&name, directory) {
            Ok(()) => cause,
            Err(error) => io::Error::new(
                cause.kind(),
                format!(
                    "{cause}; empty capture folder kept at {}: {error}",
                    kept.parent().unwrap_or(&kept).display()
                ),
            ),
        }
    }

    fn private_quarantine_directory(&self) -> io::Result<Self> {
        for attempt in 0..1000u32 {
            let mut hasher = RandomState::new().build_hasher();
            self.file.as_raw_fd().hash(&mut hasher);
            attempt.hash(&mut hasher);
            let name = format!(".held.se-recycle-{:016x}", hasher.finish());
            match self.create_private_child(OsStr::new(&name)) {
                Ok(directory) => return Ok(directory),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "quarantine names kept colliding",
        ))
    }
}

impl QuarantinedChild {
    pub fn file(&self) -> &File {
        &self.file
    }

    /// Diagnostics/recovery only: never pass this spelling to `trash::delete`.
    pub fn retained_location(&self) -> PathBuf {
        retained_path(&self.directory)
    }

    pub fn restore(&mut self) -> io::Result<()> {
        if !self.active {
            return Ok(());
        }
        self.verify()?;
        rename_no_replace(
            &self.directory,
            OsStr::new(HELD_NAME),
            &self.parent,
            &self.original,
        )?;
        self.active = false;
        Ok(())
    }

    /// Delete only the confirmed child inside our exclusive 0700 capture.
    /// The caller must already have confirmed its external publication.
    pub fn dispose(&mut self) -> io::Result<()> {
        if !self.active {
            return Ok(());
        }
        self.verify()?;
        self.directory.remove_child(OsStr::new(HELD_NAME))?;
        self.active = false;
        let name = self.directory.path.file_name().ok_or_else(changed)?;
        self.parent.remove_empty_child(name, self.directory.clone())
    }

    /// The target adapter owns trash metadata and placement policy. This
    /// move is confined to two already opened directories on the same device.
    pub fn move_to(&mut self, target: &DirectoryHandle, name: &OsStr) -> io::Result<()> {
        validate_name(name)?;
        if !self.active {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "quarantine is no longer active",
            ));
        }
        self.verify()?;
        rename_no_replace(&self.directory, OsStr::new(HELD_NAME), target, name)?;
        self.active = false;
        Ok(())
    }

    fn verify(&self) -> io::Result<()> {
        let actual = self.directory.metadata_at(OsStr::new(HELD_NAME))?;
        if !actual.is_file() || !same_object(&actual, &self.file.metadata()?) {
            return Err(changed());
        }
        Ok(())
    }
}

fn same_object(first: &std::fs::Metadata, second: &std::fs::Metadata) -> bool {
    first.dev() == second.dev() && first.ino() == second.ino()
}

fn changed() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "expected recycle child changed")
}

fn retained_path(directory: &DirectoryHandle) -> PathBuf {
    // The kernel updates this diagnostic spelling after ancestor renames.
    std::fs::read_link(format!("/proc/self/fd/{}", directory.file.as_raw_fd()))
        .unwrap_or_else(|_| directory.path.clone())
        .join(HELD_NAME)
}

fn rename_no_replace(
    source: &DirectoryHandle,
    old: &OsStr,
    target: &DirectoryHandle,
    new: &OsStr,
) -> io::Result<()> {
    let old = CString::new(old.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "source name contains NUL"))?;
    let new = CString::new(new.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "target name contains NUL"))?;
    // SAFETY: both descriptors and C strings stay live. No replacing or
    // cross-device fallback is allowed for capture, restore or trash handoff.
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            source.file.as_raw_fd(),
            old.as_ptr(),
            target.file.as_raw_fd(),
            new.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if matches!(
        error.raw_os_error(),
        Some(libc::EINVAL | libc::ENOSYS | libc::EOPNOTSUPP)
    ) {
        link_then_unlink(source, &old, target, &new, &error)
    } else {
        Err(error)
    }
}

/// The second step of the no-replace ladder (`android_fs::rename_no_replace`,
/// used by `publish_no_replace`) for a filesystem without the flag (FUSE
/// without rename2 such as ntfs-3g, NFS): the look at the destination, then a
/// create-only `linkat`, which still refuses a taken name atomically, then
/// removal of the old name. Both stay relative to the held descriptors. The
/// ladder's last step, a checked plain rename, is not taken here: a name
/// created between its check and the rename would be replaced, and capture
/// and restore never replace. A filesystem without hard links (exfat-fuse)
/// therefore stays unsupported, as before.
fn link_then_unlink(
    source: &DirectoryHandle,
    old: &CString,
    target: &DirectoryHandle,
    new: &CString,
    flag_error: &io::Error,
) -> io::Result<()> {
    let new_name = OsStr::from_bytes(new.as_bytes());
    match target.metadata_at(new_name) {
        Ok(_) => return Err(io::Error::from_raw_os_error(libc::EEXIST)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    // SAFETY: live descriptors and C strings; flags 0 never follows the old
    // name's final link, and linkat never replaces an existing new name.
    let linked = unsafe {
        libc::linkat(
            source.file.as_raw_fd(),
            old.as_ptr(),
            target.file.as_raw_fd(),
            new.as_ptr(),
            0,
        )
    };
    if linked != 0 {
        let link_error = io::Error::last_os_error();
        return Err(if link_error.raw_os_error() == Some(libc::EEXIST) {
            link_error
        } else {
            io::Error::new(
                io::ErrorKind::Unsupported,
                format!(
                    "filesystem cannot capture/restore without replacement \
                     (rename: {flag_error}; link: {link_error})"
                ),
            )
        });
    }
    // SAFETY: as above; unlinkat removes only the old final entry.
    if unsafe { libc::unlinkat(source.file.as_raw_fd(), old.as_ptr(), 0) } == 0 {
        return Ok(());
    }
    let unlink_error = io::Error::last_os_error();
    // Undo: the new name goes again only while both names are one object.
    let old_name = OsStr::from_bytes(old.as_bytes());
    let undo = match (source.metadata_at(old_name), target.metadata_at(new_name)) {
        (Ok(linked), Ok(created)) if same_object(&linked, &created) => {
            // SAFETY: as above.
            if unsafe { libc::unlinkat(target.file.as_raw_fd(), new.as_ptr(), 0) } == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        }
        _ => Err(io::Error::other(
            "the new name no longer is the linked entry",
        )),
    };
    match undo {
        Ok(()) => Err(unlink_error),
        // Two names of one object remain: never a refusal that callers may
        // read as "nothing changed".
        Err(undo) => Err(io::Error::other(format!(
            "{unlink_error}; both names were kept: {undo}"
        ))),
    }
}

#[cfg(test)]
#[path = "removable_task_tests.rs"]
mod removable_task_tests;
