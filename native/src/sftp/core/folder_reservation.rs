//! Strict P14 boundary; existing merge/copy primitives remain unchanged.
use super::backend::SftpBackend;
use crate::vfs::{Backend, VfsResult};

pub(super) fn create(backend: &SftpBackend, path: &str) -> VfsResult<()> {
    // One server-side MKDIR/MKD; post-error probes only classify a refusal.
    backend.create_dir_new(path)
}

pub(super) fn identity(_backend: &SftpBackend, _path: &str) -> VfsResult<Option<String>> {
    // SFTP v3 ATTRS has no persistent object ID. MKDIR still arbitrates a
    // first-run reservation; a negotiated se-agent supplies stronger proofs.
    Ok(None)
}
