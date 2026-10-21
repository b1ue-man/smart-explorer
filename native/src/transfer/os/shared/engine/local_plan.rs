//! What one local copy of a large file asks for beyond its own thread: more
//! threads at once when the drive is flash, a measurement first when it is
//! not known to be, and what the process remembers of that measurement. Pure
//! decisions; `local.rs` takes the permits and hands the result to the copy.
use crate::transfer::segments::{MIN_SEGMENT, SPLIT_FROM};
use std::collections::BTreeMap;
use std::sync::Mutex;

/// Smallest file that is measured: the copy cuts two probe sections of
/// 32 MiB each out of it (`copy::linux_probe`), at most half of the file.
pub(super) const PROBE_FROM: u64 = 16 * MIN_SEGMENT;

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
/// copy ranges of a file: none unless it is a copy (a move stays one
/// rename or one durable copy) of at least two shortest transfer parts and
/// nothing else waits in the queue (another file would use the permit
/// better); at most one per part beyond the first, because every thread
/// needs a part of at least `MIN_SEGMENT`.
pub(super) fn extras_wanted(size: u64, is_move: bool, queued: usize) -> usize {
    if is_move || queued > 0 || size < SPLIT_FROM {
        return 0;
    }
    usize::try_from(size / MIN_SEGMENT - 1).unwrap_or(usize::MAX)
}

/// The plan of one file. A drive that is flash copies in ranges at once.
/// Any other drive is not trusted either way: a rotating one loses to the
/// head jumps of side-by-side ranges, but the flag also says "rotating" for
/// virtual and network-attached flash, so a file of `PROBE_FROM` or more
/// measures it once per device pair and the process keeps the answer. The
/// drive and the memory are asked only when the file qualifies.
pub(super) fn plan(
    size: u64,
    is_move: bool,
    queued: usize,
    drive: impl FnOnce() -> Drive,
    memory: impl FnOnce() -> Memory,
) -> Plan {
    let extras = extras_wanted(size, is_move, queued);
    if extras == 0 {
        return Plan::Single;
    }
    if drive() == Drive::Flash {
        return Plan::Ranges(extras);
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
