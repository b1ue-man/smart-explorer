//! Strict P14 boundary; existing merge/copy primitives remain unchanged.
use super::core_impl::FtpBackend;
use crate::vfs::{Backend, VfsResult};

pub(super) fn create(backend: &FtpBackend, path: &str) -> VfsResult<()> {
    // One server-side MKDIR/MKD; post-error probes only classify a refusal.
    backend.create_dir_new(path)
}

pub(super) fn identity(_backend: &FtpBackend, _path: &str) -> VfsResult<Option<String>> {
    // MLST unique is scoped to the control connection (RFC 3659 7.5.2).
    // Unknown ownership permits the exclusive first-run MKD, never recovery.
    Ok(None)
}
