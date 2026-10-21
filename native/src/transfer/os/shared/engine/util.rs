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

/// Sixteen random hex digits, only when the system's random source gave
/// them: the 64 bits of a private name that nobody can know before it is
/// created (FTP skips its probe of such a name, `ftp/core/staging.rs`).
pub(crate) fn random_hex_checked() -> Option<String> {
    let mut bytes = [0u8; 8];
    getrandom::getrandom(&mut bytes).ok()?;
    Some(format!("{:016x}", u64::from_le_bytes(bytes)))
}

/// Sixteen digits for private names. With the system's random source they
/// are random hex digits (`random_hex_checked`). Without it, a predictable
/// value from the clock, a counter and the process id stands in; it starts
/// with `w` (not a hex digit), so a name made from it is recognizable as one
/// that has to be probed before it is used.
pub(crate) fn random_hex() -> String {
    random_hex_checked().unwrap_or_else(stand_in)
}

/// The predictable stand-in, marked by its first character.
fn stand_in() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as u64)
        .unwrap_or_default();
    static FALLBACK: AtomicU64 = AtomicU64::new(1);
    let counter = FALLBACK.fetch_add(1, Ordering::AcqRel);
    let mixed = nanos ^ counter.rotate_left(32) ^ u64::from(std::process::id());
    format!("w{:015x}", mixed & 0x0fff_ffff_ffff_ffff)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn hex16(text: &str) -> bool {
        text.len() == 16 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    }

    #[test]
    fn speed_task_random_hex_checked_is_system_random_and_the_stand_in_is_marked() {
        let first = random_hex_checked().expect("the system random source answers");
        let second = random_hex_checked().expect("the system random source answers");
        assert!(hex16(&first) && hex16(&second));
        assert_ne!(first, second, "64 random bits do not repeat");
        // `random_hex` gives the checked value when there is one.
        assert!(hex16(&random_hex()));
        // The stand-in has the same length but is not hex: it cannot pass for
        // a name that the system's random source made.
        let stand_in = stand_in();
        assert_eq!(stand_in.len(), 16);
        assert!(!hex16(&stand_in), "{stand_in}");
        assert!(stand_in.starts_with('w'));
        assert_ne!(
            super::stand_in(),
            super::stand_in(),
            "a counter separates calls"
        );
    }
}
