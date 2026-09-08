//! Object-specific retirement work, independent of attached namespace paths.
use super::engine::Entry;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

#[derive(Default)]
pub(super) struct RetirementQueue {
    candidates: Mutex<HashMap<usize, Weak<Entry>>>,
}

impl RetirementQueue {
    pub fn enqueue(&self, entry: &Arc<Entry>) {
        // This is only an allocation identity, never a dereferenceable address.
        // The retained Weak keeps that allocation from being reused while its
        // candidate is queued. Renames and duplicate registry ownership cannot
        // turn a candidate into a different object or enqueue it twice.
        self.lock().entry(Arc::as_ptr(entry) as usize)
            .or_insert_with(|| Arc::downgrade(entry));
    }

    pub fn is_empty(&self) -> bool { self.lock().is_empty() }

    /// The caller owns namespace write protection. Release this mutex before
    /// inspecting entries; pin drops and successful commits may enqueue while
    /// an entry state lock is held. New/retried work belongs to the next batch.
    pub fn take(&self) -> impl Iterator<Item = Weak<Entry>> {
        // No individual removal occurs between batch swaps, so capacity tracks
        // distinct candidates inserted into this batch. Consuming it is
        // amortized linear; the next batch cannot inherit old empty buckets.
        let batch = std::mem::take(&mut *self.lock());
        batch.into_values()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<usize, Weak<Entry>>> {
        // Scheduling happens from Drop and cannot report an error. The queue
        // contains only conservative cleanup hints, never durability authority;
        // retaining its valid entries after poison is safer than forgetting them.
        match self.candidates.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}
