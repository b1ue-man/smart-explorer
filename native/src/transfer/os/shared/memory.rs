//! Process-wide budget for bytes that transfers hold in memory: batches of
//! small files, Explorer prefetch and providers that cannot stream. However
//! high the concurrency climbs, buffered data never exceeds it.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

/// Without a memory reading: enough for several full batches in flight.
const FALLBACK_BUDGET: u64 = 256 * 1024 * 1024;
/// Never below what one maximal batch plus prefetch needs.
const MIN_BUDGET: u64 = 64 * 1024 * 1024;
/// Buffering more than bandwidth × latency cannot speed anything up; 2 GiB
/// covers 10 Gbit/s at a 1.6 s round trip.
const MAX_BUDGET: u64 = 2 * 1024 * 1024 * 1024;
const WAIT_SLICE: Duration = Duration::from_millis(100);

struct Budget {
    capacity: u64,
    used: Mutex<u64>,
    freed: Condvar,
}

fn budget() -> &'static Budget {
    static BUDGET: OnceLock<Budget> = OnceLock::new();
    BUDGET.get_or_init(|| Budget {
        // A quarter of what is available leaves the rest to the system and
        // the other parts of the app.
        capacity: super::platform::available_memory()
            .map(|available| (available / 4).clamp(MIN_BUDGET, MAX_BUDGET))
            .unwrap_or(FALLBACK_BUDGET),
        used: Mutex::new(0),
        freed: Condvar::new(),
    })
}

/// Bytes the budget holds in total.
pub fn memory_budget() -> u64 {
    budget().capacity
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
    let bytes = bytes.min(budget.capacity);
    let mut used = budget
        .used
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    loop {
        if cancel.load(Ordering::Acquire) {
            return None;
        }
        if used.saturating_add(bytes) <= budget.capacity {
            *used += bytes;
            return Some(MemoryReservation { budget, bytes });
        }
        used = match budget.freed.wait_timeout(used, WAIT_SLICE) {
            Ok((guard, _)) => guard,
            Err(poisoned) => poisoned.into_inner().0,
        };
    }
}

fn try_reserve_in(budget: &'static Budget, bytes: u64) -> Option<MemoryReservation> {
    let bytes = bytes.min(budget.capacity);
    let mut used = budget
        .used
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if used.saturating_add(bytes) > budget.capacity {
        return None;
    }
    *used += bytes;
    Some(MemoryReservation { budget, bytes })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget_of(capacity: u64) -> &'static Budget {
        Box::leak(Box::new(Budget {
            capacity,
            used: Mutex::new(0),
            freed: Condvar::new(),
        }))
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
}
