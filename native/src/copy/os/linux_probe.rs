//! The measured probe of an unbuffered local copy (`linux_direct`) onto a
//! drive that is not known to be flash (it reports "rotating" or nothing at
//! all). Whether side-by-side requests help such a drive cannot be read from
//! `queue/rotational` (virtual disks and loop images say "rotating" while
//! being flash), so the copy measures it with its own bytes: after the first
//! chunk, the first section of the file is copied by one thread, the next
//! section of the same length by two, both timed, and the rest by the full
//! thread count only when two threads were at least 10 % faster (the 10 % rule
//! of U50). A drive that does not gain (a spinning disk would only seek
//! between the ranges) gets the rest in one buffered stream, the way it was
//! copied before. Every byte lands once at its own offset of the same stage,
//! probe sections included; the verdict goes to the caller, which keeps it for
//! the device.
//!
//! Direct I/O is on the drive when the call returns, so the sections need no
//! `fdatasync` and no cache hides the drive. The earlier probe of buffered
//! copies ended each section with one: it cost 3 s of a 32 s copy (suite run
//! 4: two `fdatasync` on a loop device that flushes its backing file) and found
//! no gain, for a reason that is no accident: a buffered copy's writes are
//! issued by the kernel's writeback with a deep queue whatever the number of
//! copying threads, so a second thread does not deepen the drive's queue;
//! what holds a buffered copy back are the dirty limits and the readahead,
//! which a section of 64 MiB (far below them) cannot show. Buffered copies are
//! therefore no longer probed: only a drive known to be flash gets ranges.
//!
//! Size of a section: the time it takes must dwarf the noise of the clock and
//! the thread start (about 5 ms), so that two sections differing by 10 % are
//! told apart. At 150 MB/s a section of 32 MiB lasts 0.22 s (noise 2 %), one
//! of 64 MiB 0.45 s (1 %); at 400 MB/s 0.08 s (6 %) and 0.17 s (3 %), which is
//! why a section never shrinks below 32 MiB and grows to 64 MiB for larger
//! files (one eighth of the file). Two sections of at most half the file
//! leave a rest that is worth the decision, and a file below 128 MiB is not
//! probed at all.
use std::fs::File;
use std::io;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use super::linux_direct::{release_buffer, Direct, Start};
use super::linux_plain::{copy_sparse, Sparse};
use super::linux_range::{copy_span_with, Hooks};
use crate::transfer::segments::MIN_SEGMENT;

const MIB: u64 = 1024 * 1024;
/// Shortest section: four transfer parts (32 MiB).
const SECTION_MIN: u64 = 4 * MIN_SEGMENT;
/// Longest section: eight transfer parts (64 MiB).
const SECTION_MAX: u64 = 8 * MIN_SEGMENT;
/// A section is this share of the file, within the two limits above.
const SECTION_SHARE: u64 = 8;
/// Threads of the second section.
const PROBE_THREADS: usize = 2;
/// Parallel must be at least this many percent faster (U50: 10 %).
const GAIN_PERCENT: u128 = 10;

/// Which of the two measured sections.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Sequential,
    Parallel,
}

/// The two ways the probe copies a span `[from, to)` of the file with up to
/// `threads` threads (`None` = canceled). Tests replace both.
pub(super) trait Spans {
    /// With direct I/O: the sections and, when two threads helped, the rest.
    fn direct(&mut self, from: u64, to: u64, threads: usize) -> io::Result<Option<u64>>;
    /// Through the page cache with one stream: the rest when they did not.
    fn buffered(&mut self, from: u64, to: u64, threads: usize) -> io::Result<Option<u64>>;
}

/// Length of one probe section of a file of `size` bytes; `None` when the
/// file is too short to be probed (two sections must fit twice into it).
pub(super) fn section_len(size: u64) -> Option<u64> {
    let section = (size / SECTION_SHARE / MIB * MIB).clamp(SECTION_MIN, SECTION_MAX);
    (size / 4 >= section).then_some(section)
}

/// Whether `parallel` (bytes, time) is at least 10 % faster than `sequential`:
/// `par_bytes / par_time >= 1.10 * seq_bytes / seq_time`, in integers.
pub(super) fn parallel_helps(sequential: (u64, Duration), parallel: (u64, Duration)) -> bool {
    let (seq_bytes, seq_time) = sequential;
    let (par_bytes, par_time) = parallel;
    let seq_ns = seq_time.as_nanos().max(1);
    let par_ns = par_time.as_nanos().max(1);
    u128::from(par_bytes) * seq_ns * 100 >= (100 + GAIN_PERCENT) * u128::from(seq_bytes) * par_ns
}

/// Copies the whole `size` bytes of `reader` into the stage `writer`: with
/// O_DIRECT, a first chunk, a section with one thread, a section with two,
/// then the rest by up to `threads` threads when two were faster, else in one
/// buffered stream; the short unaligned end through the normal handles.
/// `report(helps)` is called once, from the calling thread, as soon as both
/// sections are measured. A file too short for the probe, a single permitted
/// thread, and a file that cannot be copied directly are copied plainly
/// (`copy_handles`, one thread, no verdict).
pub(in crate::copy) fn copy_probed(
    reader: &File,
    writer: &mut File,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
    threads: usize,
    report: &dyn Fn(bool),
) -> io::Result<Option<u64>> {
    let facts = reader.metadata()?;
    let size = facts.len();
    // A source with holes copies its data extents only, on one thread
    // (`linux_sparse`); a file that has none is measured as before.
    match copy_sparse(reader, writer, &facts, 1, cancel, progress)? {
        Sparse::Copied(copied) => return Ok(Some(copied)),
        Sparse::Canceled => return Ok(None),
        Sparse::Dense => {}
    }
    let Some(section) = section_len(size).filter(|_| threads >= PROBE_THREADS) else {
        return super::copy_handles(reader, writer, cancel, progress, 1);
    };
    let Some(direct) = Direct::open(reader, writer, size) else {
        return super::copy_handles(reader, writer, cancel, progress, 1);
    };
    // Sparse until the sections are in; a short source is an error.
    writer.set_len(size)?;
    let first = match direct.start(cancel, progress) {
        Ok(Start::Copied(first)) => first,
        Ok(Start::Canceled) => return Ok(None),
        Ok(Start::Unsupported) => {
            release_buffer();
            writer.set_len(0)?;
            return super::copy_handles(reader, writer, cancel, progress, 1);
        }
        Err(error) => {
            release_buffer();
            return Err(error);
        }
    };
    let sections = Sections {
        from: first,
        end: direct.main(),
        section,
        threads,
    };
    let (reader, writer) = (&*reader, &*writer);
    let body = {
        let mut spans = System {
            direct: &direct,
            reader,
            writer,
            cancel,
            progress: &mut *progress,
        };
        probe_sections(&sections, &mut spans, report, real_time)
    };
    release_buffer();
    let Some(body) = body? else {
        return Ok(None);
    };
    let Some(rest) = direct.tail(reader, writer, cancel, progress)? else {
        return Ok(None);
    };
    Ok(Some(first + body + rest))
}

fn real_time(_: Phase, took: Duration) -> Duration {
    took
}

/// The spans of a real copy.
struct System<'a> {
    direct: &'a Direct,
    reader: &'a File,
    writer: &'a File,
    cancel: &'a AtomicBool,
    progress: &'a mut dyn FnMut(u64),
}

impl Spans for System<'_> {
    fn direct(&mut self, from: u64, to: u64, threads: usize) -> io::Result<Option<u64>> {
        self.direct
            .span(from, to, threads, self.cancel, &mut *self.progress)
    }

    fn buffered(&mut self, from: u64, to: u64, threads: usize) -> io::Result<Option<u64>> {
        copy_span_with(
            self.reader,
            self.writer,
            from,
            to,
            threads,
            self.cancel,
            &mut *self.progress,
            &Hooks::SYSTEM,
        )
    }
}

/// How a file is cut: the direct part is `[from, end)` (`from` is where the
/// first chunk ended), probe sections of `section` bytes, at most `threads`
/// threads for the rest.
pub(super) struct Sections {
    pub from: u64,
    pub end: u64,
    pub section: u64,
    pub threads: usize,
}

/// The two measured sections and the rest of `[from, end)`; the bytes copied,
/// `None` when canceled. `elapsed` turns the measured time into the compared
/// one (tests make it up).
pub(super) fn probe_sections(
    sections: &Sections,
    spans: &mut dyn Spans,
    report: &dyn Fn(bool),
    elapsed: fn(Phase, Duration) -> Duration,
) -> io::Result<Option<u64>> {
    let Sections {
        from,
        end,
        section,
        threads,
    } = *sections;
    let mut timed = |phase: Phase, start: u64, stop: u64, count: usize| {
        let began = Instant::now();
        let copied = spans.direct(start, stop, count)?;
        Ok::<_, io::Error>(copied.map(|bytes| (bytes, elapsed(phase, began.elapsed()))))
    };
    let Some(sequential) = timed(Phase::Sequential, from, section, 1)? else {
        return Ok(None);
    };
    let Some(parallel) = timed(Phase::Parallel, section, 2 * section, PROBE_THREADS)? else {
        return Ok(None);
    };
    let helps = parallel_helps(sequential, parallel);
    // The verdict is about the drive: it stands even when the rest fails.
    report(helps);
    let rest_from = 2 * section;
    let rest = if helps {
        let parts =
            usize::try_from(end.saturating_sub(rest_from) / MIN_SEGMENT).unwrap_or(usize::MAX);
        spans.direct(rest_from, end, threads.min(parts).max(1))?
    } else {
        spans.buffered(rest_from, end, 1)?
    };
    let Some(rest) = rest else {
        return Ok(None);
    };
    Ok(Some(sequential.0 + parallel.0 + rest))
}

#[cfg(test)]
#[path = "linux_probe_tests.rs"]
mod tests;
