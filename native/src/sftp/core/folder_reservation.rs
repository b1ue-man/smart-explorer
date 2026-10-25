//! Strict P14 boundary; existing merge/copy primitives remain unchanged.
use super::backend::SftpBackend;
use crate::vfs::{Backend, VfsResult};
use std::io;

pub(super) fn create(backend: &SftpBackend, path: &str) -> VfsResult<()> {
    // One server-side MKDIR/MKD; post-error probes only classify a refusal.
    backend.create_dir_new(path)
}

pub(super) fn identity(_backend: &SftpBackend, _path: &str) -> VfsResult<Option<String>> {
    Err(io::Error::new(io::ErrorKind::Unsupported,
        "SFTP v3 ATTRS contains no persistent object ID; use a negotiated SSH agent for recoverable folder reservation"))
}
