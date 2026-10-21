//! Pieces of the stage table that do not depend on one table: the interval
//! lists of finished and accepted ranges, and the space all open stages of
//! the process still reserve on each volume.
use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, OnceLock};

use super::local_platform;

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Tests that reserve a large part of a disk run one at a time: the
/// reservations are per process and per volume.
#[cfg(test)]
pub(crate) fn space_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    lock(&LOCK)
}

/// Whether `[start, end)` touches a range of the list.
pub(crate) fn overlaps(ranges: &[(u64, u64)], start: u64, end: u64) -> bool {
    ranges.iter().any(|&(from, to)| start < to && from < end)
}

/// Adds `[start, end)` and merges ranges that touch (sorted, disjoint).
pub(crate) fn merge_in(ranges: &mut Vec<(u64, u64)>, start: u64, end: u64) {
    ranges.push((start, end));
    ranges.sort_unstable();
    let mut merged: Vec<(u64, u64)> = Vec::with_capacity(ranges.len());
    for &(from, to) in ranges.iter() {
        match merged.last_mut() {
            Some(last) if from <= last.1 => last.1 = last.1.max(to),
            _ => merged.push((from, to)),
        }
    }
    *ranges = merged;
}

/// Bytes all open stages of the process still have to fill, per volume. The
/// lock is never held together with a table's lock.
fn reserved() -> &'static Mutex<HashMap<String, u64>> {
    static RESERVED: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    RESERVED.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Reserves `size` bytes on the volume `key` of `dir`, or `StorageFull`
/// when the free space is less than everything reserved plus `size`.
pub(super) fn reserve_global(key: &str, dir: &Path, size: u64) -> io::Result<()> {
    let mut reserved = lock(reserved());
    let available = local_platform::available_space(dir)?;
    let held = reserved.get(key).copied().unwrap_or(0);
    if available < held.saturating_add(size) {
        return Err(io::Error::new(
            io::ErrorKind::StorageFull,
            "No space left on device for the stage (the free space is reserved by other open stages)",
        ));
    }
    let entry = reserved.entry(key.to_string()).or_default();
    *entry = entry.saturating_add(size);
    Ok(())
}

pub(super) fn release_global(key: &str, bytes: u64) {
    if bytes == 0 {
        return;
    }
    let mut reserved = lock(reserved());
    if let Some(entry) = reserved.get_mut(key) {
        *entry = entry.saturating_sub(bytes);
        if *entry == 0 {
            reserved.remove(key);
        }
    }
}
