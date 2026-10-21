//! The measured probe of a large local copy onto a drive that is not known to
//! be flash (it reports "rotating" or nothing at all). Whether side-by-side
//! ranges help such a drive cannot be read from `queue/rotational` (virtual
//! disks and loop images say "rotating" while being flash), so the copy
//! measures it with its own bytes: the first section of the file is copied by
//! one thread, the next section of the same length by two, both timed, and the
//! rest by the full thread count only when two threads were at least 10 % faster
//! (the 10 % rule of U50). Every byte lands
//! once at its own offset of the same stage, probe sections included; the
//! verdict goes to the caller, which keeps it for the device.
//!
//! Size of a section: the time it takes must dwarf the noise of the clock, the
//! thread start and the disk sync that ends it (about 5 ms together), so that
//! two sections differing by 10 % are told apart. At 150 MB/s a section of
//! 32 MiB lasts 0.22 s (noise 2 %), one of 64 MiB 0.45 s (1 %); at 400 MB/s
//! 0.08 s (6 %) and 0.17 s (3 %), which is why a section never shrinks below
//! 32 MiB and grows to 64 MiB for larger files (one eighth of the file).
//! Two sections of at most half the file leave a rest that is worth the
//! decision, and a file below 128 MiB is not probed at all.
//!
//! The page cache would hide the drive: a section is written into memory first
//! and a probe that stopped there would time memory. Each section therefore
//! ends with `fdatasync` of the stage, inside its measured time, so the
//! drive's own writing is part of the number; the sync of the first section
//! also leaves no dirty pages behind that would slow the second one. Reading
//! ahead of the source helps both sections alike (both start cold at their
//! offset), so it does not tilt the comparison.
use std::fs::File;
use std::io;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

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

/// What tests replace: the range copy, the sync that ends a section and the
/// time that is measured.
#[derive(Clone, Copy)]
pub(super) struct ProbeHooks {
    pub ranges: Hooks,
    pub settle: fn(&File),
    pub elapsed: fn(Phase, Duration) -> Duration,
}

impl ProbeHooks {
    pub(super) const SYSTEM: ProbeHooks = ProbeHooks {
        ranges: Hooks::SYSTEM,
        settle: settle_system,
        elapsed: real_time,
    };
}

fn settle_system(stage: &File) {
    // Only the timing needs the data on its way to the drive; a file system
    // that refuses is measured without it.
    let _ = stage.sync_data();
}

fn real_time(_: Phase, took: Duration) -> Duration {
    took
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

/// Copies the whole `size` bytes of `reader` into the stage `writer` (already
/// sized to `size`): a section with one thread, a section with two, then the
/// rest by up to `threads` threads when two were faster. `report(helps)` is
/// called once, from the calling thread, as soon as both are measured. Files
/// too short for the probe and a single permitted thread copy plainly.
pub(in crate::copy) fn copy_probed(
    reader: &File,
    writer: &mut File,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
    threads: usize,
    report: &dyn Fn(bool),
) -> io::Result<Option<u64>> {
    let size = reader.metadata()?.len();
    let Some(section) = section_len(size).filter(|_| threads >= PROBE_THREADS) else {
        return super::copy_handles(reader, writer, cancel, progress, 1);
    };
    // Sparse until the sections are in; a short source is an error.
    writer.set_len(size)?;
    let sections = Sections {
        size,
        section,
        threads,
    };
    let hooks = &ProbeHooks::SYSTEM;
    copy_probed_with(reader, writer, &sections, cancel, progress, report, hooks)
}

/// How a file is cut: `size` bytes, probe sections of `section` bytes, at most
/// `threads` threads for the rest.
pub(super) struct Sections {
    pub size: u64,
    pub section: u64,
    pub threads: usize,
}

pub(super) fn copy_probed_with(
    reader: &File,
    writer: &File,
    sections: &Sections,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
    report: &dyn Fn(bool),
    hooks: &ProbeHooks,
) -> io::Result<Option<u64>> {
    let Sections {
        size,
        section,
        threads,
    } = *sections;
    // One span of the file with `count` threads: the bytes copied and, for a
    // measured phase, the time they took including the sync; `None` = canceled.
    let mut span = |phase: Option<Phase>,
                    start: u64,
                    end: u64,
                    count: usize|
     -> io::Result<Option<(u64, Duration)>> {
        let began = Instant::now();
        let copied = copy_span_with(
            reader,
            writer,
            start,
            end,
            count,
            cancel,
            progress,
            &hooks.ranges,
        )?;
        let Some(copied) = copied else {
            return Ok(None);
        };
        let Some(phase) = phase else {
            return Ok(Some((copied, Duration::ZERO)));
        };
        (hooks.settle)(writer);
        Ok(Some((copied, (hooks.elapsed)(phase, began.elapsed()))))
    };
    let Some(sequential) = span(Some(Phase::Sequential), 0, section, 1)? else {
        return Ok(None);
    };
    let Some(parallel) = span(Some(Phase::Parallel), section, 2 * section, PROBE_THREADS)? else {
        return Ok(None);
    };
    let helps = parallel_helps(sequential, parallel);
    // The verdict is about the drive: it stands even when the rest fails.
    report(helps);
    let rest_from = 2 * section;
    let rest_parts =
        usize::try_from(size.saturating_sub(rest_from) / MIN_SEGMENT).unwrap_or(usize::MAX);
    let rest_threads = if helps {
        threads.min(rest_parts).max(1)
    } else {
        1
    };
    let Some((rest, _)) = span(None, rest_from, size, rest_threads)? else {
        return Ok(None);
    };
    Ok(Some(sequential.0 + parallel.0 + rest))
}

#[cfg(test)]
#[path = "linux_probe_tests.rs"]
mod tests;
