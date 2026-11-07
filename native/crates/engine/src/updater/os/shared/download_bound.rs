//! Upper bound of every update download (r-lokal-4, review 2026-11-07).
//!
//! A feed that never ends its answer must not fill the drive of the app
//! data (sync databases, secret store, logs). The bound is the announced
//! `Content-Length` when the server sends one (more or fewer bytes fail);
//! without it, the free space of the target folder minus [`SPACE_RESERVE`]
//! when the platform reports it. No fixed size: large releases stay
//! possible. A signed release manifest (E12) sets the exact payload size
//! instead ([`DownloadBound::exact`]).
use std::io::{self, Read, Write};
use std::path::Path;

/// Space left free for the rest of the app data while a download without
/// `Content-Length` runs.
pub(super) const SPACE_RESERVE: u64 = 1 << 30;
const CHUNK: usize = 64 * 1024;

/// The limit of one download into `folder`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DownloadBound {
    /// Exactly this many bytes (an announced length).
    pub(super) exact: Option<u64>,
    /// At most this many bytes (free space minus reserve).
    pub(super) ceiling: Option<u64>,
}

impl DownloadBound {
    pub(super) fn new(announced: Option<u64>, folder: &Path) -> Self {
        let ceiling = match announced {
            Some(_) => None,
            None => {
                super::os::available_space(folder).map(|free| free.saturating_sub(SPACE_RESERVE))
            }
        };
        Self {
            exact: announced,
            ceiling,
        }
    }

    /// Exactly `size` bytes: the size the signed release manifest names.
    pub(super) fn exact(size: u64) -> Self {
        Self {
            exact: Some(size),
            ceiling: None,
        }
    }
}

/// Copies `reader` to `writer` within `bound`; returns the bytes written.
/// `progress` sees the running count; `cancelled` is asked before each read
/// (an `Interrupted` error then). Exceeding the bound or ending short of an
/// announced length is an error; the caller removes the partial file.
pub(super) fn copy_bounded(
    reader: &mut dyn Read,
    writer: &mut dyn Write,
    bound: DownloadBound,
    progress: &mut dyn FnMut(u64),
    cancelled: &dyn Fn() -> bool,
) -> io::Result<u64> {
    let mut buffer = vec![0u8; CHUNK];
    let mut done = 0u64;
    loop {
        if cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Download abgebrochen",
            ));
        }
        let read = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        done = done.saturating_add(read as u64);
        if bound.exact.is_some_and(|exact| done > exact) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the feed sends more bytes than its announced or signed length",
            ));
        }
        if bound.ceiling.is_some_and(|ceiling| done > ceiling) {
            return Err(io::Error::other(
                "the download would leave less than 1 GiB free for the app data",
            ));
        }
        let chunk = buffer.get(..read).unwrap_or(&[]);
        writer.write_all(chunk)?;
        progress(done);
    }
    if bound.exact.is_some_and(|exact| done != exact) {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the feed ended before its announced or signed length",
        ));
    }
    Ok(done)
}

#[cfg(test)]
#[path = "download_bound_tests.rs"]
pub(in crate::updater) mod tests;
