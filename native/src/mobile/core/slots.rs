//! A counting gate: at most `capacity` holders at once, waiting in order of
//! arrival is not guaranteed. Waiters give up when their task is canceled.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

const CANCEL_CHECK: Duration = Duration::from_millis(200);

pub(crate) struct Slots {
    used: Mutex<usize>,
    freed: Condvar,
    capacity: usize,
}

/// Holds one slot until dropped.
pub(crate) struct SlotGuard<'a> {
    slots: &'a Slots,
}

impl Drop for SlotGuard<'_> {
    fn drop(&mut self) {
        let mut used = self
            .slots
            .used
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *used = used.saturating_sub(1);
        self.slots.freed.notify_one();
    }
}

impl Slots {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            used: Mutex::new(0),
            freed: Condvar::new(),
            capacity: capacity.max(1),
        }
    }

    /// Waits for a free slot; `None` once `cancel` is set.
    pub(crate) fn acquire(&self, cancel: &AtomicBool) -> Option<SlotGuard<'_>> {
        let mut used = self
            .used
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            if cancel.load(Ordering::Acquire) {
                return None;
            }
            if *used < self.capacity {
                *used += 1;
                return Some(SlotGuard { slots: self });
            }
            used = match self.freed.wait_timeout(used, CANCEL_CHECK) {
                Ok((guard, _)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
    }
}
