//! Small helpers of the engine: poison-tolerant locks, randomness for
//! backoff jitter and private names, and waits that end on stop.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Waits are split into slices of this length so a stop is seen quickly.
const SLEEP_SLICE: Duration = Duration::from_millis(50);

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A random fraction 0.0‥1.0 for backoff jitter.
pub(crate) fn jitter() -> f64 {
    let mut bytes = [0u8; 8];
    if getrandom::getrandom(&mut bytes).is_err() {
        return 0.5;
    }
    (u64::from_le_bytes(bytes) >> 11) as f64 / (1u64 << 53) as f64
}

/// Sixteen random hex digits for private names nobody else predicts.
pub(crate) fn random_hex() -> String {
    let mut bytes = [0u8; 8];
    if getrandom::getrandom(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos() as u64)
            .unwrap_or_default();
        static FALLBACK: AtomicU64 = AtomicU64::new(1);
        let counter = FALLBACK.fetch_add(1, Ordering::AcqRel);
        bytes = (nanos ^ counter.rotate_left(32) ^ u64::from(std::process::id())).to_le_bytes();
    }
    format!("{:016x}", u64::from_le_bytes(bytes))
}

/// Sleeps `duration` unless `stop` is set first; false when stopped.
pub(crate) fn sleep_unless(stop: &AtomicBool, duration: Duration) -> bool {
    let deadline = Instant::now() + duration;
    loop {
        if stop.load(Ordering::Acquire) {
            return false;
        }
        let now = Instant::now();
        if now >= deadline {
            return true;
        }
        std::thread::sleep((deadline - now).min(SLEEP_SLICE));
    }
}
