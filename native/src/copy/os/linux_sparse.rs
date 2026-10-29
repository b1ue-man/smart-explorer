//! A source with holes, copied without them (Linux only): where the file
//! has more length than allocated blocks (`st_blocks * 512 < st_size`), only
//! its data extents are copied, found with `lseek(SEEK_DATA)` and
//! `lseek(SEEK_HOLE)`, and the stage is sized to the source's length at the
//! end, so the holes stay holes (coreutils `copy-file-data.c`, the
//! `lseek_copy` loop; xcp does the same). A copy that fills the holes writes
//! gigabytes of zeros for a virtual machine image and can fill the disk
//! (`copy_file_range` "may expand any holes", man copy_file_range). The stage
//! is new and empty, so what is left unwritten reads as zeros: the bytes are
//! the ones a plain copy produces, only not allocated.
//!
//! Large extents go to range threads when the engine handed out permits
//! (`linux_range`), small ones are copied by the calling thread. Anything
//! unexpected before the first byte moves (a file system without `SEEK_DATA`,
//! a source that is one extent after all, a stage that is not empty) leaves
//! the plain copy to its caller; an extent search that fails later counts the
//! rest as data.
use std::fs::{File, Metadata};
use std::io;
use std::os::unix::fs::MetadataExt;
use std::os::unix::io::AsRawFd;
use std::sync::atomic::AtomicBool;

use super::linux_plain::Sparse;
use super::linux_range::{copy_span_with, Hooks};
use crate::transfer::segments::SPLIT_FROM;

/// The calling thread copies a whole extent itself: no thread is started.
const INLINE: Hooks = Hooks {
    call: Hooks::SYSTEM.call,
    may_spawn: never,
};

fn never(_: usize) -> bool {
    false
}

/// Whether the file has holes, as coreutils tells it (`st_blocks` counts
/// 512-byte units).
fn is_sparse(facts: &Metadata) -> bool {
    facts.blocks().saturating_mul(512) < facts.len()
}

/// Copies the data extents of `reader` into the new, empty `writer` and gives
/// the stage the source's length (`facts` is the source's `fstat`); the bytes
/// counted are the file's length, as a plain copy counts them, the progress
/// only what moved. `parallel` > 1 lets extents of two transfer parts or more
/// use that many threads.
pub(super) fn copy(
    reader: &File,
    writer: &File,
    facts: &Metadata,
    parallel: usize,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> io::Result<Sparse> {
    let len = facts.len();
    if !is_sparse(facts) || !fresh_pair(reader, writer) {
        return Ok(Sparse::Dense);
    }
    let mut extent = match next_extent(reader, 0, len) {
        // One extent over the whole file: compression made `st_blocks` small,
        // there is no hole to keep.
        Ok(Some(whole)) if whole == (0, len) => return plain_again(reader),
        Ok(found) => found,
        Err(_) => return plain_again(reader),
    };
    let mut written_to = 0u64;
    while let Some((start, end)) = extent {
        if copy_extent(reader, writer, (start, end), parallel, cancel, progress)?.is_none() {
            return Ok(Sparse::Canceled);
        }
        written_to = end;
        extent = if end >= len {
            None
        } else {
            // A search that fails now counts the rest as data.
            next_extent(reader, end, len).unwrap_or(Some((end, len)))
        };
    }
    // A hole at the end is no write: the length makes it.
    if written_to < len {
        writer.set_len(len)?;
    }
    // Both files end where a plain copy leaves them.
    let _ = seek(reader, len, libc::SEEK_SET);
    let _ = seek(writer, len, libc::SEEK_SET);
    Ok(Sparse::Copied(len))
}

/// A new, empty stage and a source at its first byte: what the extents
/// below assume.
fn fresh_pair(reader: &File, writer: &File) -> bool {
    writer.metadata().is_ok_and(|facts| facts.len() == 0)
        && seek(reader, 0, libc::SEEK_CUR).is_ok_and(|at| at == 0)
}

/// Back to the first byte for the plain copy that follows.
fn plain_again(reader: &File) -> io::Result<Sparse> {
    seek(reader, 0, libc::SEEK_SET)?;
    Ok(Sparse::Dense)
}

/// The next data extent `[start, end)` at or after `from`, cut at `len`;
/// `None` when only a hole is left. Moves the source's file offset.
fn next_extent(file: &File, from: u64, len: u64) -> io::Result<Option<(u64, u64)>> {
    let start = match seek(file, from, libc::SEEK_DATA) {
        Ok(start) => start,
        // No data from here to the end.
        Err(error) if error.raw_os_error() == Some(libc::ENXIO) => return Ok(None),
        Err(error) => return Err(error),
    };
    if start >= len {
        return Ok(None);
    }
    let end = seek(file, start, libc::SEEK_HOLE)?.min(len);
    if end <= start {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "SEEK_HOLE ended an extent before it began",
        ));
    }
    Ok(Some((start, end)))
}

/// Copies the bytes `[start, end)` to the same offsets of the stage; `None`
/// when canceled.
fn copy_extent(
    reader: &File,
    writer: &File,
    (start, end): (u64, u64),
    parallel: usize,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> io::Result<Option<u64>> {
    if parallel > 1 && end - start >= SPLIT_FROM {
        let hooks = &Hooks::SYSTEM;
        copy_span_with(
            reader, writer, start, end, parallel, cancel, progress, hooks,
        )
    } else {
        copy_span_with(reader, writer, start, end, 1, cancel, progress, &INLINE)
    }
}

/// `lseek64(2)` on the file's own offset.
fn seek(file: &File, offset: u64, whence: libc::c_int) -> io::Result<u64> {
    let offset =
        libc::off64_t::try_from(offset).map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))?;
    // SAFETY: the descriptor belongs to `file`, which outlives the call; the
    // call only moves the file offset.
    let at = unsafe { libc::lseek64(file.as_raw_fd(), offset, whence) };
    if at < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(at as u64)
    }
}
