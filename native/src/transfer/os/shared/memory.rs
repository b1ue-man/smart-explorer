//! Process-wide budget for bytes that transfers hold in memory: batches of
//! small files, Explorer prefetch and providers that cannot stream. However
//! high the concurrency climbs, buffered data never exceeds it.
//!
//! The capacity follows the memory available to programs: a quarter of it,
//! read again at most every two seconds while reserving. It may shrink below
//! what is reserved already; that revokes nothing, new reservations just wait
//! until enough is returned.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Without a memory reading: enough for several full batches in flight.
const FALLBACK_BUDGET: u64 = 256 * 1024 * 1024;
/// Never below what one maximal batch plus prefetch needs.
const MIN_BUDGET: u64 = 64 * 1024 * 1024;
/// Buffering more than bandwidth × latency cannot speed anything up; 2 GiB
/// covers 10 Gbit/s at a 1.6 s round trip.
const MAX_BUDGET: u64 = 2 * 1024 * 1024 * 1024;
const WAIT_SLICE: Duration = Duration::from_millis(100);
/// The available memory is read again at most this often.
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

struct Budget {
    capacity: AtomicU64,
    refreshed: Mutex<Instant>,
    used: Mutex<u64>,
    freed: Condvar,
}

/// A quarter of what is available leaves the rest to the system and the other
/// parts of the app; without a reading the fallback stands.
fn capacity_for(available: Option<u64>) -> u64 {
    available
        .map(|available| (available / 4).clamp(MIN_BUDGET, MAX_BUDGET))
        .unwrap_or(FALLBACK_BUDGET)
}

impl Budget {
    fn new(capacity: u64) -> Self {
        Self {
            capacity: AtomicU64::new(capacity),
            refreshed: Mutex::new(Instant::now()),
            used: Mutex::new(0),
            freed: Condvar::new(),
        }
    }

    fn capacity(&self) -> u64 {
        self.capacity.load(Ordering::Acquire)
    }

    /// Reads the available memory again when the last reading is two seconds
    /// old.
    fn refresh(&self) {
        {
            let mut refreshed = self
                .refreshed
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if refreshed.elapsed() < REFRESH_INTERVAL {
                return;
            }
            *refreshed = Instant::now();
        }
        self.set_available(super::platform::available_memory());
    }

    /// Follows a new reading. A request waiting for room is woken: the
    /// capacity may have grown.
    fn set_available(&self, available: Option<u64>) {
        let capacity = capacity_for(available);
        if self.capacity.swap(capacity, Ordering::AcqRel) < capacity {
            self.freed.notify_all();
        }
    }
}

fn budget() -> &'static Budget {
    static BUDGET: OnceLock<Budget> = OnceLock::new();
    BUDGET.get_or_init(|| Budget::new(capacity_for(super::platform::available_memory())))
}

/// Bytes the budget holds in total right now.
pub fn memory_budget() -> u64 {
    let budget = budget();
    budget.refresh();
    budget.capacity()
}

/// Held bytes; returned to the budget on drop.
pub struct MemoryReservation {
    budget: &'static Budget,
    bytes: u64,
}

impl MemoryReservation {
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

impl Drop for MemoryReservation {
    fn drop(&mut self) {
        let budget = self.budget;
        let mut used = budget
            .used
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *used = used.saturating_sub(self.bytes);
        drop(used);
        budget.freed.notify_all();
    }
}

/// Waits until `bytes` fit (a request above the whole budget waits for an
/// empty budget and then takes all of it); `None` once `cancel` is set.
pub fn reserve_memory(bytes: u64, cancel: &AtomicBool) -> Option<MemoryReservation> {
    reserve_in(budget(), bytes, cancel)
}

/// Reserves only if `bytes` fit right now.
pub fn try_reserve_memory(bytes: u64) -> Option<MemoryReservation> {
    try_reserve_in(budget(), bytes)
}

fn reserve_in(
    budget: &'static Budget,
    bytes: u64,
    cancel: &AtomicBool,
) -> Option<MemoryReservation> {
    budget.refresh();
    let mut used = budget
        .used
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    loop {
        if cancel.load(Ordering::Acquire) {
            return None;
        }
        // Read again on every look: the capacity moves with the memory.
        let bytes = bytes.min(budget.capacity());
        if used.saturating_add(bytes) <= budget.capacity() {
            *used += bytes;
            return Some(MemoryReservation { budget, bytes });
        }
        used = match budget.freed.wait_timeout(used, WAIT_SLICE) {
            Ok((guard, _)) => guard,
            Err(poisoned) => poisoned.into_inner().0,
        };
        // A waiting request is the one who notices a changed memory.
        budget.refresh();
    }
}

fn try_reserve_in(budget: &'static Budget, bytes: u64) -> Option<MemoryReservation> {
    budget.refresh();
    let bytes = bytes.min(budget.capacity());
    let mut used = budget
        .used
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if used.saturating_add(bytes) > budget.capacity() {
        return None;
    }
    *used += bytes;
    Some(MemoryReservation { budget, bytes })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1024 * 1024;

    fn budget_of(capacity: u64) -> &'static Budget {
        Box::leak(Box::new(Budget::new(capacity)))
    }

    #[test]
    fn transfer_engine_task_memory_budget_is_bounded_and_returned() {
        let capacity = memory_budget();
        assert!((MIN_BUDGET..=MAX_BUDGET).contains(&capacity));
        let budget = budget_of(1_000);
        let whole = try_reserve_in(budget, u64::MAX).expect("an idle budget admits one request");
        assert_eq!(whole.bytes(), 1_000);
        assert!(try_reserve_in(budget, 1).is_none());
        drop(whole);
        let small = try_reserve_in(budget, 100).expect("returned bytes are reusable");
        assert_eq!(small.bytes(), 100);
        let cancel = AtomicBool::new(true);
        assert!(reserve_in(budget, 950, &cancel).is_none());
    }

    #[test]
    fn speed_task_memory_budget_follows_available() {
        // a quarter of what is available, between 64 MiB and 2 GiB
        assert_eq!(capacity_for(Some(1024 * MIB)), 256 * MIB);
        assert_eq!(capacity_for(Some(100 * MIB)), MIN_BUDGET);
        assert_eq!(capacity_for(Some(64 * 1024 * MIB)), MAX_BUDGET);
        assert_eq!(capacity_for(None), FALLBACK_BUDGET);
        let budget = budget_of(256 * MIB);
        budget.set_available(Some(2048 * MIB));
        assert_eq!(budget.capacity(), 512 * MIB);
        budget.set_available(Some(512 * MIB));
        assert_eq!(budget.capacity(), 128 * MIB);
        // a reading that fails leaves the fallback, not zero
        budget.set_available(None);
        assert_eq!(budget.capacity(), FALLBACK_BUDGET);
        // the reading is not repeated within two seconds
        budget.set_available(Some(4096 * MIB));
        budget.refresh();
        assert_eq!(
            budget.capacity(),
            1024 * MIB,
            "refresh waits for the interval"
        );
        // the budget a request sees is the current one
        let grown = try_reserve_in(budget, 900 * MIB).expect("fits the grown capacity");
        assert_eq!(grown.bytes(), 900 * MIB);
    }

    #[test]
    fn speed_task_memory_budget_never_revokes() {
        let budget = budget_of(1_000);
        let held = try_reserve_in(budget, 900).expect("reserved");
        // the memory shrinks below what is held: nothing is taken back
        budget.set_available(Some(0));
        assert_eq!(budget.capacity(), MIN_BUDGET);
        assert_eq!(held.bytes(), 900);
        assert_eq!(*budget.used.lock().expect("used"), 900);
        // a smaller capacity only makes new requests wait
        budget.capacity.store(500, Ordering::Release);
        assert!(try_reserve_in(budget, 100).is_none());
        drop(held);
        assert_eq!(*budget.used.lock().expect("used"), 0);
        let again = try_reserve_in(budget, 100).expect("room again");
        assert_eq!(again.bytes(), 100);
        // a request above the whole capacity takes all of it, never more
        drop(again);
        let all = try_reserve_in(budget, u64::MAX).expect("an empty budget");
        assert_eq!(all.bytes(), 500);
    }
}
