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
pub(crate) struct QuarantinedChild {
    parent: DirectoryHandle,
    original: OsString,
    directory: DirectoryHandle,
    file: File,
    active: bool,
}

impl DirectoryHandle {
    pub(crate) fn quarantine_regular_child(
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
        rename_no_replace(self, name, &directory, OsStr::new(HELD_NAME))?;
        let captured = directory.open_regular_child(OsStr::new(HELD_NAME)).and_then(|file| {
            if same_object(&file.metadata()?, &expected_metadata) {
                Ok(file)
            } else {
                Err(changed())
            }
        });
        match captured {
            Ok(file) => {
                Ok(QuarantinedChild {
                    parent: self.clone(),
                    original: name.to_os_string(),
                    directory,
                    file,
                    active: true,
                })
            }
            Err(cause) => {
                match rename_no_replace(&directory, OsStr::new(HELD_NAME), self, name) {
                    Ok(()) => Err(cause),
                    Err(restore) => Err(io::Error::new(
                        cause.kind(),
                        format!(
                            "{cause}; captured entry retained at {}: {restore}",
                            retained_path(&directory).display(),
                        ),
                    )),
                }
            }
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
        Err(io::Error::new(io::ErrorKind::AlreadyExists, "quarantine names kept colliding"))
    }
}

impl QuarantinedChild {
    pub(crate) fn file(&self) -> &File {
        &self.file
    }

    /// Diagnostics/recovery only: never pass this spelling to `trash::delete`.
    pub(crate) fn retained_location(&self) -> PathBuf {
        retained_path(&self.directory)
    }

    pub(crate) fn restore(&mut self) -> io::Result<()> {
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

    /// The target adapter owns trash metadata and placement policy. This
    /// move is confined to two already opened directories on the same device.
    pub(crate) fn move_to(&mut self, target: &DirectoryHandle, name: &OsStr) -> io::Result<()> {
        validate_name(name)?;
        if !self.active {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "quarantine is no longer active"));
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
    if matches!(error.raw_os_error(), Some(libc::EINVAL | libc::ENOSYS | libc::EOPNOTSUPP)) {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "filesystem cannot capture/restore without replacement",
        ))
    } else {
        Err(error)
    }
}
