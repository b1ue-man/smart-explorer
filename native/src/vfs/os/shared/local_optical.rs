//! Optical arbitration at the native VFS boundary. Ordinary local and remote
//! mounts keep their existing handle, range and metadata behavior.
use std::{io, path::Path};

pub(super) fn guard(path: &Path) -> io::Result<Option<crate::source_media::Guard>> {
    crate::source_media::acquire_current(path, crate::source_media::Priority::Foreground)
}

pub(super) fn ranges(path: &Path) -> bool {
    // An unproved device does not grant parallel reads. The actual open
    // reports the probe error instead of converting it to a missing path.
    matches!(crate::source_media::probe(path), Ok(None))
}

/// Stat observes the link itself, including a dangling link. Resolving its
/// target for device discovery would incorrectly turn it into NotFound.
pub(super) fn metadata_guard(path: &Path) -> io::Result<Option<crate::source_media::Guard>> {
    let metadata = crate::local_access::symlink_metadata(path)?;
    let location = if crate::local_access::metadata_is_link_like(path, &metadata) {
        path.parent().unwrap_or(path)
    } else { path };
    guard(location)
}
