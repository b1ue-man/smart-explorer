//! Remembers for a moment that a location is on no optical medium (M6,
//! 2026-10-28). A copy probes its source for every file; the answer for a
//! fixed disk never changes while files are copied, but each probe walks
//! sysfs (Linux) or the volume path (Windows). Only "not optical" is kept,
//! and only briefly: an optical medium is probed in full every time (its
//! media sequence and mount are part of the answer), and a device number or
//! folder that turns into an optical mount is seen again within `FRESH`.
//! An answer one probe takes from another (Windows: a folder's answer for
//! the paths in it, `os/windows_plain.rs`) keeps the time of that answer, so
//! it lapses with it.
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
    plain_since(key).is_some()
}

/// When the "not optical" answer kept for `key` was found, while it stands.
pub(super) fn plain_since(key: &str) -> Option<Instant> {
    let mut entries = entries();
    match entries.get(key) {
        Some(at) if at.elapsed() < FRESH => Some(*at),
        Some(_) => {
            entries.remove(key);
            None
        }
        None => None,
    }
}

/// Keeps "not optical" for `key`.
pub(super) fn remember_plain(key: String) {
    remember_plain_at(key, Instant::now());
}

/// Keeps "not optical" for `key` as found at `at`. An answer taken from an
/// earlier one passes that one's time, so it never stands longer than the
/// answer it came from; a later time kept for `key` already stays.
pub(super) fn remember_plain_at(key: String, at: Instant) {
    if at.elapsed() >= FRESH {
        return;
    }
    let mut entries = entries();
    if entries.len() >= MAX_ENTRIES {
        entries.retain(|_, at| at.elapsed() < FRESH);
        if entries.len() >= MAX_ENTRIES {
            entries.clear();
        }
    }
    let kept = entries.entry(key).or_insert(at);
    if *kept < at {
        *kept = at;
    }
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

    /// An answer taken from another keeps that one's time: it lapses with it,
    /// never replaces a later answer and is not kept once that time lapsed.
    #[test]
    fn abgleich_task_probe_cache_derived_answers_keep_the_time_they_came_from() {
        let key = format!("probe-cache-derived-{}", std::process::id());
        let earlier = std::time::Instant::now()
            .checked_sub(super::FRESH / 2)
            .expect("the clock runs longer than half the freshness span");
        super::remember_plain_at(key.clone(), earlier);
        assert_eq!(super::plain_since(&key), Some(earlier));
        let later = std::time::Instant::now();
        super::remember_plain_at(key.clone(), later);
        assert_eq!(super::plain_since(&key), Some(later));
        super::remember_plain_at(key.clone(), earlier);
        assert_eq!(
            super::plain_since(&key),
            Some(later),
            "an earlier answer does not replace a later one"
        );
        let lapsed_key = format!("probe-cache-lapsed-{}", std::process::id());
        let lapsed = std::time::Instant::now()
            .checked_sub(super::FRESH)
            .expect("the clock runs longer than the freshness span");
        super::remember_plain_at(lapsed_key.clone(), lapsed);
        assert_eq!(super::plain_since(&lapsed_key), None);
        assert!(!super::entries().contains_key(&lapsed_key));
        super::entries().remove(&key);
    }
}
