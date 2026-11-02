//! A large file copied by several threads, each a contiguous range of it
//! with `copy_file_range` at explicit offsets (Linux and Android). Only with
//! permits the engine's flow handed out, on drives known to be flash or
//! measured to gain from it (`linux_probe`); the caller sizes the stage first
//! (`set_len`), so the ranges land in a sparse file that is complete once
//! every range is.
use std::fs::File;
use std::io;
use std::os::unix::fs::FileExt;
use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use super::KERNEL_CHUNK;

/// Range borders fall on whole mebibytes.
const BORDER: u64 = 1024 * 1024;
/// Bytes per `pread`/`pwrite` step of the fallback.
const FALLBACK_CHUNK: usize = 1024 * 1024;
/// How long the calling thread sleeps between two looks at the counters and
/// the cancel flag.
const MONITOR_SLICE: Duration = Duration::from_millis(40);

/// One kernel call: copies up to `len` bytes from `reader` at `from` to
/// `writer` at `to`; returns the bytes moved (0 = the source ended).
pub(super) type RangeCall = fn(&File, &File, u64, u64, usize) -> io::Result<usize>;

/// What tests replace: the system call and the thread start.
#[derive(Clone, Copy)]
pub(super) struct Hooks {
    pub call: RangeCall,
    /// Whether the thread of range `index` may be started.
    pub may_spawn: fn(usize) -> bool,
}

impl Hooks {
    pub(super) const SYSTEM: Hooks = Hooks {
        call: system_copy_file_range,
        may_spawn: always,
    };
}

fn always(_: usize) -> bool {
    true
}

/// Copies the first `size` bytes of `reader` into `writer` with `threads`
/// threads; `None` when canceled. `progress` is called by the calling
/// thread only.
pub(super) fn copy_ranges(
    reader: &File,
    writer: &File,
    size: u64,
    threads: usize,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> io::Result<Option<u64>> {
    let hooks = &Hooks::SYSTEM;
    copy_ranges_with(reader, writer, size, threads, cancel, progress, hooks)
}

pub(super) fn copy_ranges_with(
    reader: &File,
    writer: &File,
    size: u64,
    threads: usize,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
    hooks: &Hooks,
) -> io::Result<Option<u64>> {
    copy_span_with(reader, writer, 0, size, threads, cancel, progress, hooks)
}

/// Copies the bytes `[start, end)` of `reader` to the same offsets of
/// `writer` with `threads` threads; `None` when canceled. The measured probe
/// (`linux_probe`) copies a file in spans, each with its own thread count;
/// `progress` is called by the calling thread only and counts the span's
/// bytes once.
#[allow(clippy::too_many_arguments)]
pub(super) fn copy_span_with(
    reader: &File,
    writer: &File,
    start: u64,
    end: u64,
    threads: usize,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
    hooks: &Hooks,
) -> io::Result<Option<u64>> {
    let size = end.saturating_sub(start);
    let ranges = split_span(start, end, threads);
    let shared = Shared {
        reader,
        writer,
        cancel,
        stop: AtomicBool::new(false),
        failure: Mutex::new(None),
        call: hooks.call,
    };
    let counters: Vec<AtomicU64> = ranges.iter().map(|_| AtomicU64::new(0)).collect();
    let running = AtomicUsize::new(0);
    let caller = std::thread::current();
    let mut reported = 0u64;
    let mut report = |counters: &[AtomicU64]| {
        let total: u64 = counters.iter().map(|c| c.load(Ordering::Acquire)).sum();
        if total > reported {
            progress(total - reported);
            reported = total;
        }
    };
    std::thread::scope(|scope| {
        // A range whose thread could not be started is copied by the
        // calling thread: never a panic, at worst one thread.
        let mut own = Vec::new();
        for (index, &(start, end)) in ranges.iter().enumerate() {
            let counter = &counters[index];
            let mut started = false;
            if (hooks.may_spawn)(index) {
                running.fetch_add(1, Ordering::AcqRel);
                let (shared, running, caller) = (&shared, &running, &caller);
                let spawned = std::thread::Builder::new()
                    .name(format!("copy-range-{index}"))
                    .spawn_scoped(scope, move || {
                        // A range thread serves one engine copy a person
                        // waits for: normal priority like its worker (M20).
                        let _done = Done { running, caller };
                        shared.run(start, end, counter, &mut || {});
                    });
                started = spawned.is_ok();
                if !started {
                    running.fetch_sub(1, Ordering::AcqRel);
                }
            }
            if !started {
                own.push((index, start, end));
            }
        }
        for (index, start, end) in own {
            shared.run(start, end, &counters[index], &mut || report(&counters));
        }
        while running.load(Ordering::Acquire) > 0 {
            std::thread::park_timeout(MONITOR_SLICE);
            report(&counters);
            if cancel.load(Ordering::Acquire) {
                shared.stop.store(true, Ordering::Release);
            }
        }
    });
    report(&counters);
    if let Some(error) = shared.take_failure() {
        return Err(error);
    }
    if cancel.load(Ordering::Acquire) {
        return Ok(None);
    }
    let total: u64 = counters.iter().map(|c| c.load(Ordering::Acquire)).sum();
    if total != size {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "ranges copied fewer bytes than the file has",
        ));
    }
    Ok(Some(total))
}

/// `split` of the span `[start, end)`: the same borders, shifted by `start`.
fn split_span(start: u64, end: u64, threads: usize) -> Vec<(u64, u64)> {
    split(end.saturating_sub(start), threads)
        .into_iter()
        .map(|(from, to)| (start + from, start + to))
        .collect()
}

/// Border pairs `[start, end)`: `threads` contiguous ranges, borders on
/// whole mebibytes, the last takes the rest. Fewer ranges when the file is
/// too short to give each a mebibyte; at least one.
fn split(size: u64, threads: usize) -> Vec<(u64, u64)> {
    let threads = (threads.max(1) as u64).min((size / BORDER).max(1));
    let each = size / threads / BORDER * BORDER;
    (0..threads)
        .map(|index| {
            let start = index * each;
            let end = if index + 1 == threads {
                size
            } else {
                start + each
            };
            (start, end)
        })
        .collect()
}

struct Shared<'a> {
    reader: &'a File,
    writer: &'a File,
    cancel: &'a AtomicBool,
    /// Raised by the first failure and by the caller on cancel.
    stop: AtomicBool,
    failure: Mutex<Option<io::Error>>,
    call: RangeCall,
}

impl Shared<'_> {
    fn take_failure(&self) -> Option<io::Error> {
        self.failure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }

    /// Copies `[start, end)`; a failure is kept (the first one wins) and
    /// stops every other range.
    fn run(&self, start: u64, end: u64, counter: &AtomicU64, on_chunk: &mut dyn FnMut()) {
        if let Err(error) = self.copy_range(start, end, counter, on_chunk) {
            let mut failure = self
                .failure
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            failure.get_or_insert(error);
            self.stop.store(true, Ordering::Release);
        }
    }

    fn copy_range(
        &self,
        start: u64,
        end: u64,
        counter: &AtomicU64,
        on_chunk: &mut dyn FnMut(),
    ) -> io::Result<()> {
        let mut offset = start;
        // The kernel call is given up for this range when its very first
        // call is refused; `first` stays set across interrupted calls.
        let mut kernel = true;
        let mut first = true;
        let mut buffer = Vec::new();
        while offset < end {
            if self.stop.load(Ordering::Acquire) || self.cancel.load(Ordering::Acquire) {
                return Ok(());
            }
            let length = (end - offset).min(KERNEL_CHUNK) as usize;
            let moved = if kernel {
                match (self.call)(self.reader, self.writer, offset, offset, length) {
                    Ok(0) => return Err(source_ended(offset)),
                    Ok(moved) => moved as u64,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) if first && is_refusal(&error) => {
                        kernel = false;
                        first = false;
                        continue;
                    }
                    Err(error) => return Err(error),
                }
            } else {
                self.read_write(offset, length.min(FALLBACK_CHUNK), &mut buffer)?
            };
            first = false;
            offset += moved;
            counter.fetch_add(moved, Ordering::AcqRel);
            on_chunk();
        }
        Ok(())
    }

    /// The fallback step: `pread` then `pwrite` of the same range.
    fn read_write(&self, offset: u64, length: usize, buffer: &mut Vec<u8>) -> io::Result<u64> {
        buffer.resize(length, 0);
        let read = loop {
            match self.reader.read_at(buffer, offset) {
                Ok(0) => return Err(source_ended(offset)),
                Ok(read) => break read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        };
        self.writer.write_all_at(&buffer[..read], offset)?;
        Ok(read as u64)
    }
}

/// Signals the calling thread that one range thread has ended, however it
/// ended.
struct Done<'a> {
    running: &'a AtomicUsize,
    caller: &'a std::thread::Thread,
}

impl Drop for Done<'_> {
    fn drop(&mut self) {
        self.running.fetch_sub(1, Ordering::AcqRel);
        self.caller.unpark();
    }
}

fn source_ended(offset: u64) -> io::Error {
    io::Error::new(
        io::ErrorKind::UnexpectedEof,
        format!("source ended at byte {offset} (shortened during the copy)"),
    )
}

/// The refusals after which the same file pair is copied by `pread` and
/// `pwrite`: no such system call, another file system, an offset or flag
/// combination the file system rejects, and a descriptor not open for it.
fn is_refusal(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::ENOSYS | libc::EXDEV | libc::EINVAL | libc::EOPNOTSUPP | libc::EBADF)
    )
}

/// `copy_file_range(2)` through the system call number (the C library
/// function is missing on old glibc and on Android before API 34).
fn system_copy_file_range(
    reader: &File,
    writer: &File,
    from: u64,
    to: u64,
    length: usize,
) -> io::Result<usize> {
    let mut from = i64::try_from(from).map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))?;
    let mut to = i64::try_from(to).map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))?;
    // SAFETY: both descriptors belong to files that outlive the call, and
    // the offset pointers point to live locals of this frame.
    let moved = unsafe {
        libc::syscall(
            libc::SYS_copy_file_range,
            reader.as_raw_fd(),
            &mut from as *mut i64,
            writer.as_raw_fd(),
            &mut to as *mut i64,
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

#[cfg(test)]
#[path = "linux_range_tests.rs"]
mod tests;
