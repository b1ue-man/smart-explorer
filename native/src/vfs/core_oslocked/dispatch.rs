use std::io;
use std::sync::Arc;

use super::{BackendHandle, LocalBackend};

/// Build the backend for a root string. Remote schemes are recognized by URL
/// prefix; everything else (drive paths, `\\server\share` UNC, mapped drives)
/// is local. The SFTP/FTP arms are filled in by their respective steps so that
/// adding a protocol never touches the callers.
pub fn backend_for(root: &str) -> io::Result<BackendHandle> {
    let r = root.trim();
    let lower = r.to_ascii_lowercase();
    if lower.starts_with("sftp://") {
        Ok(Arc::new(crate::sftp::backend_from_url(r)?))
    } else if lower.starts_with("ftp://") || lower.starts_with("ftps://") {
        Ok(Arc::new(crate::ftp::backend_from_url(r)?))
    } else {
        Ok(Arc::new(LocalBackend::new(r)))
    }
}

/// Whether a root string is served by a remote (non-local) backend. Lets the
/// app pick the remote scan path and disable the inotify watcher for remote
/// roots without constructing a backend.
pub fn is_remote_root(root: &str) -> bool {
    let lower = root.trim().to_ascii_lowercase();
    lower.starts_with("sftp://") || lower.starts_with("ftp://") || lower.starts_with("ftps://")
}
