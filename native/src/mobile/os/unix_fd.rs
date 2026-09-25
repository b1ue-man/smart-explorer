//! Ownership of file descriptors the host hands over (Android
//! `ParcelFileDescriptor.detachFd()`, `fs.import`).
use std::fs::File;
use std::os::fd::{FromRawFd, RawFd};

/// Takes ownership of `fd`; it is closed when the returned file is dropped.
pub(crate) fn adopt(fd: i64) -> Result<File, String> {
    let raw = RawFd::try_from(fd)
        .ok()
        .filter(|raw| *raw >= 0)
        .ok_or_else(|| format!("Ungültiger Dateideskriptor: {fd}"))?;
    // SAFETY: F_GETFD only queries the descriptor flags; an invalid
    // descriptor yields -1 (EBADF) without side effects.
    if unsafe { libc::fcntl(raw, libc::F_GETFD) } == -1 {
        return Err(format!("Dateideskriptor {fd} ist nicht geöffnet"));
    }
    // SAFETY: the host detached this open descriptor and passed its ownership
    // to the core; nothing else closes it, so the `File` is its only owner.
    Ok(unsafe { File::from_raw_fd(raw) })
}
