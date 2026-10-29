//! The single stream of a copy (Linux and Android): the bytes of one regular
//! file into the exclusively created stage on one thread.
//!
//! Linux asks the kernel for the whole known length, the way coreutils `cp`
//! does (`copy-file-data.c`: `copy_file_range` in a loop without a buffer):
//! one `fstat` and one call per 8 MiB. `std::io::copy` spends two `statx` and
//! a final `copy_file_range` that returns 0 on every call (Rust std
//! `kernel_copy/linux.rs`). A source with holes goes through `linux_sparse`
//! first. Whatever the kernel refuses, or a 0 before the known length (a file
//! whose size says nothing, such as `/proc`), the rest is copied by
//! `io::copy` as before: the stage holds the same bytes either way. The
//! direct call goes through the system call number, like `linux_range`, and
//! Android keeps `io::copy` altogether.
use std::fs::File;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use super::KERNEL_CHUNK;

/// What the sparse copy (`linux_sparse`) came to.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(super) enum Sparse {
    /// The data extents are in and the stage has the source's length: the
    /// number of bytes is the file's length, as a plain copy counts them.
    Copied(u64),
    Canceled,
    /// Nothing moved: no sparse file after all, or a file system that cannot
    /// tell its extents. The plain copy takes over.
    Dense,
}

#[cfg(target_os = "linux")]
pub(super) use super::linux_sparse::copy as copy_sparse;

/// Sparse copies exist on Linux only.
#[cfg(not(target_os = "linux"))]
pub(super) fn copy_sparse(
    _reader: &File,
    _writer: &File,
    _facts: &std::fs::Metadata,
    _parallel: usize,
    _cancel: &AtomicBool,
    _progress: &mut dyn FnMut(u64),
) -> io::Result<Sparse> {
    Ok(Sparse::Dense)
}

/// Copies `reader` to the end into `writer`; `None` when canceled.
#[cfg(target_os = "linux")]
pub(super) fn copy_stream(
    reader: &File,
    writer: &mut File,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> io::Result<Option<u64>> {
    let Ok(facts) = reader.metadata() else {
        return copy_std(reader, writer, cancel, progress, 0);
    };
    match copy_sparse(reader, writer, &facts, 1, cancel, progress)? {
        Sparse::Copied(copied) => return Ok(Some(copied)),
        Sparse::Canceled => return Ok(None),
        Sparse::Dense => {}
    }
    let (copied, stop) = copy_known(reader, writer, facts.len(), cancel, progress)?;
    match stop {
        Stop::Complete => Ok(Some(copied)),
        Stop::Canceled => Ok(None),
        Stop::Rest => copy_std(reader, writer, cancel, progress, copied),
    }
}

/// Copies `reader` to the end into `writer`; `None` when canceled.
#[cfg(not(target_os = "linux"))]
pub(super) fn copy_stream(
    reader: &File,
    writer: &mut File,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> io::Result<Option<u64>> {
    copy_std(reader, writer, cancel, progress, 0)
}

/// Why the kernel loop ended.
#[cfg(target_os = "linux")]
enum Stop {
    /// The known length is in.
    Complete,
    Canceled,
    /// The kernel declined or the source ended early: `io::copy` takes the
    /// rest from the files' current offsets.
    Rest,
}

/// The kernel loop: `copy_file_range` at the files' own offsets until `len`
/// bytes moved. Returns the bytes moved with the reason it ended; progress is
/// reported per call.
#[cfg(target_os = "linux")]
fn copy_known(
    reader: &File,
    writer: &File,
    len: u64,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> io::Result<(u64, Stop)> {
    // A size of 0 says nothing about files such as those of `/proc`.
    if len == 0 {
        return Ok((0, Stop::Rest));
    }
    let mut copied = 0u64;
    while copied < len {
        if cancel.load(Ordering::Acquire) {
            return Ok((copied, Stop::Canceled));
        }
        let wanted = (len - copied).min(KERNEL_CHUNK) as usize;
        match kernel_copy(reader, writer, wanted) {
            Ok(0) => return Ok((copied, Stop::Rest)),
            Ok(moved) => {
                copied += moved as u64;
                progress(moved as u64);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if refused(&error) => return Ok((copied, Stop::Rest)),
            Err(error) => return Err(error),
        }
    }
    Ok((copied, Stop::Complete))
}

/// The errors after which `io::copy` still has a way: a file system or
/// kernel without the call, another file system, a seccomp filter, a file
/// that cannot take it (the list of Rust std and coreutils), and a busy text
/// file.
#[cfg(target_os = "linux")]
fn refused(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(
            libc::ENOSYS
                | libc::EXDEV
                | libc::EINVAL
                | libc::EOPNOTSUPP
                | libc::EPERM
                | libc::EBADF
                | libc::ETXTBSY
                | libc::EOVERFLOW
        )
    )
}

/// `copy_file_range(2)` at the files' offsets through the system call number
/// (the C library function is missing on old glibc).
#[cfg(target_os = "linux")]
fn kernel_copy(reader: &File, writer: &File, length: usize) -> io::Result<usize> {
    use std::os::unix::io::AsRawFd;
    // SAFETY: both descriptors belong to files that outlive the call; null
    // offsets make the kernel use and advance the files' own offsets, as
    // `io::copy` does.
    let moved = unsafe {
        libc::syscall(
            libc::SYS_copy_file_range,
            reader.as_raw_fd(),
            std::ptr::null_mut::<i64>(),
            writer.as_raw_fd(),
            std::ptr::null_mut::<i64>(),
            length,
            0u32,
        )
    };
    if moved < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(moved as usize)
    }
}

/// `std::io::copy` between two files (copy_file_range, which reflinks on
/// btrfs/xfs and copies server-side on NFS, then sendfile, splice and
/// read/write as std decides), 8 MiB at a time so cancel and progress are
/// seen; `already` bytes are counted as copied before.
fn copy_std(
    reader: &File,
    writer: &mut File,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
    already: u64,
) -> io::Result<Option<u64>> {
    use std::io::Read;
    let mut copied = already;
    loop {
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        let mut chunk = Read::take(reader, KERNEL_CHUNK);
        let moved = std::io::copy(&mut chunk, writer)?;
        if moved == 0 {
            return Ok(Some(copied));
        }
        copied = copied.saturating_add(moved);
        progress(moved);
        // `io::copy` only returns early at the end of the source: a short
        // chunk is the last one, so the file needs no second round of
        // `fstat`s and a zero-length probe call.
        if moved < KERNEL_CHUNK {
            return Ok(Some(copied));
        }
    }
}
