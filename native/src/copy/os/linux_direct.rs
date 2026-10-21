//! Large local copies around the page cache (Linux and Android), the
//! counterpart of Windows' `COPY_FILE_NO_BUFFERING`. A file that is at least a
//! quarter of the available memory would push everything else out of the cache
//! and run into the dirty limits, which hold a buffered copy at the readahead
//! of one drive; with direct I/O and several requests in flight the drive
//! itself is the limit.
//!
//! The stage and the source are not opened again by path: the handles the copy
//! already holds and has checked are reopened with `O_DIRECT` through
//! `/proc/self/fd/<fd>`, which names the very same file (the identity is
//! compared), so no swap can happen in between. The part of the file that is a
//! multiple of the logical block size is copied by several threads, each with
//! one contiguous range and aligned 4 MiB buffers (`linux_range` runs them);
//! the short rest goes through the normal handles. Everything that cannot be
//! done directly (no block device, no `/proc`, a refusal of the very first
//! request) returns before the first byte moved, and the older path copies the
//! file; the progress counts every byte once.
use std::cell::RefCell;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt};
use std::os::unix::io::AsRawFd;
use std::sync::atomic::AtomicBool;

use super::linux_direct_plan::{
    block_backed, is_unsupported, split_main, unit_of, wanted, DioReport, BUFFER_ALIGN, CHUNK, MIB,
    THREADS,
};
use super::linux_range::{copy_span_with, Hooks, RangeCall};
use crate::copy::LocalFailure;

/// The O_DIRECT handles of one copy and how the file is cut.
pub(super) struct Direct {
    reader: File,
    writer: File,
    size: u64,
    /// The part copied directly: a multiple of the block unit.
    main: u64,
    call: RangeCall,
}

/// What the first request found out.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Start {
    /// The file system takes no direct I/O: nothing was copied.
    Unsupported,
    Canceled,
    /// The first chunk is in the stage and counted.
    Copied(u64),
}

/// How a whole file went.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Whole {
    /// Nothing was copied; use the buffered path.
    Unavailable,
    Canceled,
    Done(u64),
}

impl Direct {
    /// The direct handles for copying `size` bytes from `reader` into the
    /// stage `writer`, or `None` when the file is not to be copied directly
    /// (smaller than a quarter of the available memory, a side without block
    /// device, no `/proc`, a file system that refuses `O_DIRECT`).
    pub(super) fn open(reader: &File, writer: &File, size: u64) -> Option<Direct> {
        if !wanted(size, crate::transfer::available_memory()) {
            return None;
        }
        Direct::reopen(reader, writer, size, direct_call)
    }

    fn reopen(reader: &File, writer: &File, size: u64, call: RangeCall) -> Option<Direct> {
        if !block_backed(reader.metadata().ok()?.dev())
            || !block_backed(writer.metadata().ok()?.dev())
        {
            return None;
        }
        // A file system that clones extents (XFS with reflink, btrfs, bcachefs)
        // makes the kernel copy instant: that stays the way (`copy_file_range`).
        if clones(reader, writer) {
            return None;
        }
        let unit = unit_of([dio_report(reader), dio_report(writer)])?;
        let (main, _) = split_main(size, unit);
        if main == 0 {
            return None;
        }
        Some(Direct {
            reader: open_same(reader, false)?,
            writer: open_same(writer, true)?,
            size,
            main,
            call,
        })
    }

    /// The end of the part that is copied directly.
    pub(super) fn main(&self) -> u64 {
        self.main
    }

    /// The first request, on the calling thread: it finds out whether the
    /// file system takes direct I/O before any other thread starts.
    pub(super) fn start(
        &self,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64),
    ) -> io::Result<Start> {
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            return Ok(Start::Canceled);
        }
        let length = self.main.min(CHUNK as u64) as usize;
        match (self.call)(&self.reader, &self.writer, 0, 0, length) {
            Ok(0) => Err(tag(LocalFailure::Source(ended(0)))),
            Ok(moved) => {
                progress(moved as u64);
                Ok(Start::Copied(moved as u64))
            }
            Err(error) if refused(&error) => Ok(Start::Unsupported),
            Err(error) => Err(error),
        }
    }

    /// Copies `[from, to)` of the direct part with up to `threads` threads
    /// (never more than `THREADS`); `None` when canceled.
    pub(super) fn span(
        &self,
        from: u64,
        to: u64,
        threads: usize,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64),
    ) -> io::Result<Option<u64>> {
        if from >= to {
            return Ok(Some(0));
        }
        let hooks = Hooks {
            call: self.call,
            may_spawn: |_| true,
        };
        let threads = threads.clamp(1, THREADS);
        copy_span_with(
            &self.reader,
            &self.writer,
            from,
            to,
            threads,
            cancel,
            progress,
            &hooks,
        )
    }

    /// The rest after the direct part (shorter than the block unit) through
    /// the normal handles: a read error belongs to the source, a write error
    /// to the target. `None` when canceled.
    pub(super) fn tail(
        &self,
        reader: &File,
        writer: &File,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64),
    ) -> io::Result<Option<u64>> {
        let mut buffer = vec![0u8; (self.size - self.main) as usize];
        let mut offset = self.main;
        while offset < self.size {
            if cancel.load(std::sync::atomic::Ordering::Acquire) {
                return Ok(None);
            }
            let step = ((self.size - offset) as usize).min(buffer.len());
            let read = loop {
                match reader.read_at(&mut buffer[..step], offset) {
                    Ok(0) => return Err(tag(LocalFailure::Source(ended(offset)))),
                    Ok(read) => break read,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(tag(LocalFailure::Source(error))),
                }
            };
            writer
                .write_all_at(&buffer[..read], offset)
                .map_err(|error| tag(LocalFailure::Target(error)))?;
            offset += read as u64;
            progress(read as u64);
        }
        Ok(Some(self.size - self.main))
    }

    /// The whole file: first request, the ranges, the rest.
    pub(super) fn run(
        &self,
        reader: &File,
        writer: &File,
        threads: usize,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64),
    ) -> io::Result<Whole> {
        let first = match self.start(cancel, progress) {
            Ok(Start::Copied(first)) => first,
            Ok(Start::Canceled) => return Ok(Whole::Canceled),
            Ok(Start::Unsupported) => {
                release_buffer();
                return Ok(Whole::Unavailable);
            }
            Err(error) => {
                release_buffer();
                return Err(error);
            }
        };
        let body = self.span(first, self.main, threads, cancel, progress);
        release_buffer();
        let Some(body) = body? else {
            return Ok(Whole::Canceled);
        };
        let Some(rest) = self.tail(reader, writer, cancel, progress)? else {
            return Ok(Whole::Canceled);
        };
        Ok(Whole::Done(first + body + rest))
    }

    /// Test seam: handles opened by the test with a stand-in for the request.
    #[cfg(test)]
    pub(super) fn from_parts(
        reader: File,
        writer: File,
        size: u64,
        main: u64,
        call: RangeCall,
    ) -> Direct {
        Direct {
            reader,
            writer,
            size,
            main,
            call,
        }
    }
}

/// The whole file around the page cache when it qualifies (`Whole::Unavailable`
/// = nothing copied, use the older way).
pub(super) fn copy_whole(
    reader: &File,
    writer: &File,
    size: u64,
    threads: usize,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> io::Result<Whole> {
    match Direct::open(reader, writer, size) {
        Some(direct) => direct.run(reader, writer, threads, cancel, progress),
        None => Ok(Whole::Unavailable),
    }
}

/// Whether the file system clones extents from `reader` into `writer`: the
/// first mebibyte is cloned with `FICLONERANGE`. A refusal (other file system
/// or device, no reflink, a swapped-in link) answers no; on success the stage
/// holds shared extents of the same bytes that the copy would write anyway.
fn clones(reader: &File, writer: &File) -> bool {
    let mut range = libc::file_clone_range {
        src_fd: i64::from(reader.as_raw_fd()),
        src_offset: 0,
        src_length: MIB,
        dest_offset: 0,
    };
    // SAFETY: both descriptors belong to files that outlive the call, and the
    // argument points to a live `file_clone_range` of this frame.
    let result = unsafe {
        libc::ioctl(
            writer.as_raw_fd(),
            libc::FICLONERANGE,
            &mut range as *mut libc::file_clone_range,
        )
    };
    result == 0
}

/// Whether a failed request means "no direct I/O here", also when the request
/// tagged the side of the error.
fn refused(error: &io::Error) -> bool {
    match error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<LocalFailure>())
    {
        Some(LocalFailure::Source(inner) | LocalFailure::Target(inner)) => is_unsupported(inner),
        Some(LocalFailure::Publish(_)) => false,
        None => is_unsupported(error),
    }
}

/// An error that carries its side to the staging code (`stage_copy` sorts
/// plain errors by their kind alone) and is no refusal for the range code.
fn tag(failure: LocalFailure) -> io::Error {
    io::Error::other(failure)
}

fn ended(offset: u64) -> io::Error {
    io::Error::new(
        io::ErrorKind::UnexpectedEof,
        format!("source ended at byte {offset} (shortened during the copy)"),
    )
}

/// The same open file again with `O_DIRECT`, through its own descriptor:
/// `/proc/self/fd/<fd>` names the inode of the open file, never a path that
/// could be swapped. Plain `O_RDONLY`/`O_WRONLY`: no create, no truncate, no
/// append. `None` when `/proc` is missing or denied (Android may), the file
/// system refuses the flag, or the opened file is another one.
fn open_same(file: &File, write: bool) -> Option<File> {
    let path = format!("/proc/self/fd/{}", file.as_raw_fd());
    let mut options = OpenOptions::new();
    if write {
        options.write(true);
    } else {
        options.read(true);
    }
    let opened = options.custom_flags(libc::O_DIRECT).open(path).ok()?;
    let same = super::file_identity(&opened).ok()? == super::file_identity(file).ok()?;
    same.then_some(opened)
}

/// What `STATX_DIOALIGN` (Linux 6.1) says of `file`. glibc builds ask through
/// the system call number like `copy_file_range` (the `statx` symbol needs
/// glibc 2.28, the release baseline is 2.17); Android and musl do not ask.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn dio_report(file: &File) -> DioReport {
    // SAFETY: an all-zero `statx` is a valid value (integers only).
    let mut buffer: libc::statx = unsafe { std::mem::zeroed() };
    let empty = b"\0";
    // SAFETY: the descriptor belongs to `file`, the path is a NUL-terminated
    // empty string (with AT_EMPTY_PATH it names the descriptor itself), and
    // the buffer is a live `statx` of this frame.
    let result = unsafe {
        libc::syscall(
            libc::SYS_statx,
            file.as_raw_fd(),
            empty.as_ptr() as *const libc::c_char,
            libc::AT_EMPTY_PATH,
            libc::STATX_DIOALIGN,
            &mut buffer as *mut libc::statx,
        )
    };
    if result != 0 || buffer.stx_mask & libc::STATX_DIOALIGN == 0 {
        return None;
    }
    Some((buffer.stx_dio_mem_align, buffer.stx_dio_offset_align))
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn dio_report(_file: &File) -> DioReport {
    None
}

thread_local! {
    /// The request buffer of the thread: `CHUNK` bytes plus room to align.
    static BUFFER: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Gives the memory of the calling thread's request buffer back (a range
/// thread ends with its copy; the calling thread keeps its own otherwise).
pub(super) fn release_buffer() {
    BUFFER.with(|cell| *cell.borrow_mut() = Vec::new());
}

/// `len` bytes of `storage` that start at an address aligned to
/// `BUFFER_ALIGN`.
pub(super) fn aligned(storage: &mut Vec<u8>, len: usize) -> &mut [u8] {
    if storage.len() < len + BUFFER_ALIGN {
        storage.resize(len + BUFFER_ALIGN, 0);
    }
    let address = storage.as_ptr() as usize;
    let skip = (BUFFER_ALIGN - address % BUFFER_ALIGN) % BUFFER_ALIGN;
    &mut storage[skip..skip + len]
}

/// One direct request: reads up to `length` (at most `CHUNK`) bytes of `reader`
/// at `from` into an aligned buffer and writes them to `writer` at `to`;
/// returns the bytes moved. A read error is tagged as the source's, a write
/// error as the target's; a source that ends early is an error.
fn direct_call(
    reader: &File,
    writer: &File,
    from: u64,
    to: u64,
    length: usize,
) -> io::Result<usize> {
    let want = length.min(CHUNK);
    BUFFER.with(|cell| {
        let mut storage = cell.borrow_mut();
        let buffer = aligned(&mut storage, want);
        let read =
            read_full(reader, buffer, from).map_err(|error| tag(LocalFailure::Source(error)))?;
        if read < want {
            return Err(tag(LocalFailure::Source(ended(from + read as u64))));
        }
        write_full(writer, &buffer[..read], to)
            .map_err(|error| tag(LocalFailure::Target(error)))?;
        Ok(read)
    })
}

/// Reads until `buffer` is full or the file ends.
fn read_full(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
    let mut done = 0;
    while done < buffer.len() {
        match file.read_at(&mut buffer[done..], offset + done as u64) {
            Ok(0) => break,
            Ok(read) => done += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(done)
}

fn write_full(file: &File, buffer: &[u8], offset: u64) -> io::Result<()> {
    let mut done = 0;
    while done < buffer.len() {
        match file.write_at(&buffer[done..], offset + done as u64) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(written) => done += written,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "linux_direct_tests.rs"]
mod tests;
