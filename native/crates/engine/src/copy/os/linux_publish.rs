//! Publishing a stage, or moving a file, onto a name that must stay free
//! (Linux and Android).
//!
//! `renameat2(RENAME_NOREPLACE)` refuses a taken name itself (`EEXIST`,
//! decided by the kernel before the file system is asked), so on Linux it is
//! tried first and an `lstat` of the destination before it would only repeat
//! the answer. When it fails with an error that changed nothing (a taken name,
//! a file system without the flag, a vanished source) the publication goes the
//! way it always went: the look at the destination (`preflight_no_replace`),
//! which labels a taken name as certainly not published, then the fallback
//! ladder for file systems without the flag (`android_fs::rename_no_replace`).
//! Callers therefore see the same errors as before; only the successful
//! rename on a file system that knows the flag saves the extra call. Any other
//! error may have happened on a network file system and is returned as the
//! rename reported it, which is what the second call after a free name
//! returned before. Android keeps the look first: its storage often does not
//! know the flag, and the ladder remembers which devices do not.
use std::io;
use std::path::Path;

/// Renames `source` to `destination` without replacing an existing name.
pub(super) fn publish_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        match renameat2_no_replace(source, destination) {
            Ok(()) => return Ok(()),
            Err(error) if nothing_changed(&error) => {}
            Err(error) => return Err(error),
        }
    }
    crate::copy::publication_proof::preflight_no_replace(destination)?;
    super::rename_no_replace(source, destination)
}

/// Errors of the first attempt after which the old order of the look and
/// the ladder still applies: the kernel refused before it touched anything.
#[cfg(target_os = "linux")]
fn nothing_changed(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::EEXIST | libc::EINVAL | libc::ENOSYS | libc::EOPNOTSUPP | libc::ENOENT)
    )
}

/// `renameat2(AT_FDCWD, source, AT_FDCWD, destination, RENAME_NOREPLACE)`
/// through the system call number (the libc crate has no wrapper on musl),
/// as `android_fs::rename_no_replace` makes it.
#[cfg(target_os = "linux")]
fn renameat2_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "source path contains NUL"))?;
    let destination = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "destination path contains NUL")
    })?;
    // SAFETY: both paths are NUL-terminated and live for the call.
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
