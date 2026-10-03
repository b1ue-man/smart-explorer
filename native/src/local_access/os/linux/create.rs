//! Private directory and record creation relative to an opened parent.
use std::ffi::{CString, OsStr};
use std::fs::File;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::io::{AsRawFd, FromRawFd};

use super::{validate_name, DirectoryHandle};

impl DirectoryHandle {
    /// Exclusively create a 0700 directory; never adopt an existing child.
    pub(crate) fn create_private_child(&self, name: &OsStr) -> io::Result<Self> {
        validate_name(name)?;
        let c_name = child_name(name)?;
        // SAFETY: live directory descriptor and a validated C child name.
        if unsafe { libc::mkdirat(self.file.as_raw_fd(), c_name.as_ptr(), 0o700) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let child = self.open_child(name)?;
        verify_private(&child.metadata()?)?;
        Ok(child)
    }

    /// Exclusively create a 0600 record without following the final child.
    /// A failed private-mode check retains only this empty reservation.
    pub(crate) fn create_file_new(&self, name: &OsStr) -> io::Result<File> {
        validate_name(name)?;
        let c_name = child_name(name)?;
        // SAFETY: a live parent descriptor, validated C child name, and a
        // mode argument because O_CREAT is set. O_EXCL refuses existing links.
        let descriptor = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                c_name.as_ptr(),
                libc::O_WRONLY
                    | libc::O_CREAT
                    | libc::O_EXCL
                    | libc::O_NOFOLLOW
                    | libc::O_CLOEXEC
                    | libc::O_NONBLOCK
                    | libc::O_NOCTTY,
                0o600,
            )
        };
        if descriptor < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: this call owns the newly opened descriptor exactly once.
        let file = unsafe { File::from_raw_fd(descriptor) };
        let metadata = file.metadata()?;
        super::super::regular_or_refusal(metadata.file_type())?;
        verify_private(&metadata)?;
        Ok(file)
    }
}

fn child_name(name: &OsStr) -> io::Result<CString> {
    CString::new(name.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "entry name contains NUL"))
}

fn verify_private(metadata: &std::fs::Metadata) -> io::Result<()> {
    // SAFETY: plain getter of the process's effective uid.
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.permissions().mode() & 0o077 != 0 {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "created entry is not owner-private",
        ))
    } else {
        Ok(())
    }
}
