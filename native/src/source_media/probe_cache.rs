//! Remembers for a moment that a location is on no optical medium (M6,
//! 2026-10-28). A copy probes its source for every file; the answer for a
//! fixed disk never changes while files are copied, but each probe walks
//! sysfs (Linux) or the volume path (Windows). Only "not optical" is kept,
//! and only briefly: an optical medium is probed in full every time (its
//! media sequence and mount are part of the answer), and a device number or
//! folder that turns into an optical mount is seen again within `FRESH`.
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// How long a "not optical" answer stands.
const FRESH: Duration = Duration::from_secs(2);
/// Entries kept at most; stale ones go first, then all.
const MAX_ENTRIES: usize = 4096;

fn entries() -> MutexGuard<'static, HashMap<String, Instant>> {
    static ENTRIES: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    ENTRIES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Whether `key` was found on no optical medium a moment ago.
pub(super) fn known_plain(key: &str) -> bool {
    let mut entries = entries();
    match entries.get(key) {
        Some(at) if at.elapsed() < FRESH => true,
        Some(_) => {
            entries.remove(key);
            false
        }
        None => false,
    }
}

/// Keeps "not optical" for `key`.
pub(super) fn remember_plain(key: String) {
    let mut entries = entries();
    if entries.len() >= MAX_ENTRIES {
        entries.retain(|_, at| at.elapsed() < FRESH);
        if entries.len() >= MAX_ENTRIES {
            entries.clear();
        }
    }
    entries.insert(key, Instant::now());
}

#[cfg(test)]
mod tests {
    #[test]
    fn local_speed_task_probe_cache_keeps_only_fresh_plain_answers() {
        let key = format!("probe-cache-test-{}", std::process::id());
        assert!(!super::known_plain(&key));
        super::remember_plain(key.clone());
        assert!(super::known_plain(&key));
        let stale = std::time::Instant::now()
            .checked_sub(super::FRESH)
            .expect("the clock runs longer than the freshness span");
        super::entries().insert(key.clone(), stale);
        assert!(!super::known_plain(&key));
        assert!(!super::entries().contains_key(&key));
    }
}
