//! Strict P14 boundary; existing merge/copy primitives remain unchanged.
use super::core_impl::FtpBackend;
use crate::vfs::{Backend, VfsResult};
use std::io;

pub(super) fn create(backend: &FtpBackend, path: &str) -> VfsResult<()> {
    // One server-side MKDIR/MKD; post-error probes only classify a refusal.
    backend.create_dir_new(path)
}

pub(super) fn identity(_backend: &FtpBackend, _path: &str) -> VfsResult<Option<String>> {
    Err(io::Error::new(io::ErrorKind::Unsupported,
        "FTP MLST unique is guaranteed only for the control connection (RFC 3659 7.5.2); FTP has no atomic no-replace directory rename"))
}
