//! `WalkHashed2` with SHA-256 on one regular file (`query::FILE_DIGEST`):
//! the served backend computes the digest where the file is stored
//! (`vfs::content_digest`: a Share host or room member with `hash_file_v1`,
//! the SSH agent, a local disk) and only the digest crosses the connection,
//! as one `HashEntry` with an empty `rel`, then `End`. A backend that cannot
//! do that answers "unsupported", which the client reads as `None`; this
//! side never reads the file to obtain a digest.
use std::io;
use std::sync::atomic::AtomicBool;

use crate::agent_proto::{Frame, UNSUPPORTED_EXTENSION};
use crate::vfs::{self as vfs, BackendHandle};

use super::super::backend_server::{emit, Sink};

/// Serves `root` when it is one regular file. `Ok(false)`: a folder, a link,
/// a special entry or a root whose metadata cannot be read; the caller keeps
/// its existing folder walk (and its existing errors) for those.
pub(super) fn serve(
    sink: &Sink,
    id: u64,
    backend: &BackendHandle,
    root: &str,
    cancel: &AtomicBool,
) -> io::Result<bool> {
    let Ok(meta) = backend.stat(root) else {
        return Ok(false);
    };
    if meta.is_dir || meta.is_symlink || meta.special {
        return Ok(false);
    }
    match vfs::content_digest(&**backend, root, cancel)? {
        Some(digest) => {
            let entry = Frame::HashEntry {
                rel: String::new(),
                is_dir: false,
                size: digest.size,
                mtime_ms: meta.mtime_ms,
                md5: Some(digest.sha256),
            };
            emit(sink, id, &entry)?;
            emit(sink, id, &Frame::End)?;
        }
        None => emit(sink, id, &Frame::Err(UNSUPPORTED_EXTENSION.to_string()))?,
    }
    Ok(true)
}
