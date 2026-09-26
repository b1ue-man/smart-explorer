//! Staged writes on FTP. The protocol has no exclusive create and no
//! no-replace rename, so opening the stage and publishing it are each an
//! absence check right before the one mutating command; the stage itself is a
//! fresh random sibling name (`vfs::unique_staging_path`). The general
//! `rename_no_replace` stays unsupported, so user renames and conflict copies
//! keep refusing. Replacing an existing file is a single RNFR/RNTO pair, which
//! POSIX servers carry out as one `rename(2)`.
use std::io;

use crate::vfs::Backend;

/// Fails with `AlreadyExists` when `path` exists; a failed probe is an error,
/// never "absent".
pub(super) fn require_absent<B: Backend + ?Sized>(backend: &B, path: &str) -> io::Result<()> {
    if backend.try_exists(path)? {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{path} existiert bereits"),
        ));
    }
    Ok(())
}
