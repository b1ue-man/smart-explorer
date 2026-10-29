//! What one local copy of a large file asks for beyond its own thread: more
//! threads at once when the drive is flash, a measurement first when it is
//! not known to be (only for a file that is copied around the page cache),
//! and what the process remembers of that measurement. Pure decisions;
//! `local.rs` takes the permits and hands the result to the copy.
use crate::transfer::segments::{MIN_SEGMENT, SPLIT_FROM};
use std::collections::BTreeMap;
use std::sync::Mutex;

/// Smallest file that is measured: the copy cuts two probe sections of
/// 32 MiB each out of it (`copy::linux_probe`), at most half of the file.
pub(super) const PROBE_FROM: u64 = 16 * MIN_SEGMENT;

/// A file at least this part of the available memory is copied around the
/// page cache on Linux (`copy::linux_direct`, the rule of Windows'
/// `COPY_FILE_NO_BUFFERING`, `copy_flags::NO_BUFFERING_DIVISOR`; both sides
/// decide with the same divisor). Windows' `COPY_FILE_NO_BUFFERING` has one
/// more, fixed threshold of 256 MiB (`copy_flags::NO_BUFFERING_FROM`); it is
/// not repeated here: the Windows copy has no ranges, so the engine's answer
/// changes nothing there (`local::unbuffered_of`), and the Linux rule stays.
pub(super) const UNBUFFERED_DIVISOR: u64 = 4;

/// Extra threads of a file that is copied directly: the copy runs 4 threads
/// in all (`copy::linux_direct_plan::THREADS`: 4 requests of 4 MiB in flight,
/// the depth of the 16 × 1 MiB that `fio` needed to reach the drive's
/// capacity), so it asks for three permits beyond its own, never for one per
/// 8 MiB part that it could not use. The flow hands out only what is free.
pub(super) const UNBUFFERED_EXTRAS: usize = 3;

/// Whether a file of `size` bytes is copied around the page cache: at least
/// a quarter of the available memory (unknown memory: never).
pub(super) fn unbuffered(size: u64, available_memory: Option<u64>) -> bool {
    size > 0 && available_memory.is_some_and(|memory| size >= memory / UNBUFFERED_DIVISOR)
}

/// The two devices a copy runs on (`st_dev` of source and target; the same
/// number twice on one device). Copying A to B and B to A are two keys.
pub(super) type DeviceKey = (u64, u64);

/// What the system says of the drive under the target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Drive {
    /// `queue/rotational` is 0.
    Flash,
    /// `queue/rotational` is 1.
    Spinning,
    /// Nothing could be read, or the platform has no such file.
    Unknown,
}

impl Drive {
    pub(super) fn of(rotational: Option<bool>) -> Drive {
        match rotational {
            Some(false) => Drive::Flash,
            Some(true) => Drive::Spinning,
            None => Drive::Unknown,
        }
    }
}

/// What this process knows of whether side-by-side ranges help a device pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Memory {
    /// No key (no block device on a side, or a platform without ranges): the
    /// pair cannot be measured, so it is not copied in parallel.
    Unavailable,
    /// Nothing measured and nobody measuring.
    Untested,
    /// A copy is measuring it right now.
    Probing,
    Helps,
    Hurts,
}

/// How the copy of one file runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Plan {
    /// One thread.
    Single,
    /// This many extra threads from the start.
    Ranges(usize),
    /// A measured start; afterwards this many extra threads or none.
    Probe(usize),
}

/// How many threads beyond the one that holds the operation's permit may
/// copy ranges of a file: none for a move that stays one rename on its
/// volume (`renames`), none below two shortest transfer parts and none while
/// anything else waits in the queue (another file would use the permit
/// better); at most one per part beyond the first, because every thread
/// needs a part of at least `MIN_SEGMENT`. A move to another volume is a
/// durable copy and asks like any copy (M15, 2026-10-28): its stage is
/// synced whole before publication however many threads filled it.
pub(super) fn extras_wanted(size: u64, renames: bool, queued: usize) -> usize {
    if renames || queued > 0 || size < SPLIT_FROM {
        return 0;
    }
    usize::try_from(size / MIN_SEGMENT - 1).unwrap_or(usize::MAX)
}

/// The plan of one file. A drive that is flash copies in ranges at once.
/// A file that is copied around the page cache (`unbuffered`) on any other
/// drive is not trusted either way: a rotating one loses to the head jumps of
/// side-by-side requests, but the flag also says "rotating" for virtual and
/// network-attached flash, so a file of `PROBE_FROM` or more measures it once
/// per device pair with direct requests (no cache hides the drive, no sync
/// is needed) and the process keeps the answer. A buffered copy on such a
/// drive stays on one thread: its writes are issued by the kernel's writeback
/// with a deep queue whatever the number of copying threads, so ranges do not
/// help it and the probe's syncs cost seconds (suite run 4: 3 s of 32 s for no
/// gain). The drive and the memory are asked only when the file qualifies.
pub(super) fn plan(
    size: u64,
    renames: bool,
    queued: usize,
    unbuffered: bool,
    drive: impl FnOnce() -> Drive,
    memory: impl FnOnce() -> Memory,
) -> Plan {
    let mut extras = extras_wanted(size, renames, queued);
    if unbuffered {
        extras = extras.min(UNBUFFERED_EXTRAS);
    }
    if extras == 0 {
        return Plan::Single;
    }
    if drive() == Drive::Flash {
        return Plan::Ranges(extras);
    }
    if !unbuffered {
        return Plan::Single;
    }
    match memory() {
        Memory::Helps => Plan::Ranges(extras),
        Memory::Untested if size >= PROBE_FROM => Plan::Probe(extras),
        Memory::Untested | Memory::Unavailable | Memory::Probing | Memory::Hurts => Plan::Single,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Entry {
    Probing,
    Helps,
    Hurts,
}

/// The verdicts of a process, one per device pair. A verdict is fixed once
/// given: later copies of the pair neither measure again nor change it.
pub(super) struct Verdicts {
    entries: Mutex<BTreeMap<DeviceKey, Entry>>,
}

impl Verdicts {
    pub(super) const fn new() -> Verdicts {
        Verdicts {
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<DeviceKey, Entry>> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn memory(&self, key: DeviceKey) -> Memory {
        match self.lock().get(&key) {
            None => Memory::Untested,
            Some(Entry::Probing) => Memory::Probing,
            Some(Entry::Helps) => Memory::Helps,
            Some(Entry::Hurts) => Memory::Hurts,
        }
    }

    /// The right to measure `key`: only one copy at a time, and only while
    /// the pair has no verdict.
    pub(super) fn claim(&self, key: DeviceKey) -> Option<Ticket<'_>> {
        let mut entries = self.lock();
        if entries.contains_key(&key) {
            return None;
        }
        entries.insert(key, Entry::Probing);
        Some(Ticket { store: self, key })
    }
}

/// A claim on measuring one device pair. Without a verdict (the copy was
/// canceled, failed, or never reached the measurement) the claim is given
/// back when the ticket ends, so a later copy measures instead.
pub(super) struct Ticket<'a> {
    store: &'a Verdicts,
    key: DeviceKey,
}

impl Ticket<'_> {
    /// Keeps the verdict for the process; the first one stands.
    pub(super) fn record(&self, helps: bool) {
        let mut entries = self.store.lock();
        if entries.get(&self.key) == Some(&Entry::Probing) {
            let verdict = if helps { Entry::Helps } else { Entry::Hurts };
            entries.insert(self.key, verdict);
        }
    }
}

impl Drop for Ticket<'_> {
    fn drop(&mut self) {
        let mut entries = self.store.lock();
        if entries.get(&self.key) == Some(&Entry::Probing) {
            entries.remove(&self.key);
        }
    }
}
