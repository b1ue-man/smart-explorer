//! Device conditions reported by an embedding host (Android: battery saver,
//! metered network and whether scheduled jobs wait for the host's own worker
//! run). The Android platform adapter reads the first two for auto-pause and
//! the scheduling loop reads the third; desktop hosts never set any of them,
//! so they stay false there. The host also reports whether the app may use
//! all of shared storage (`set_storage_access`).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

const POWER_SAVE: u8 = 0b001;
const METERED: u8 = 0b010;
const DEFER_SCHEDULING: u8 = 0b100;

const ACCESS_UNREPORTED: u8 = 0;
const ACCESS_GRANTED: u8 = 1;
const ACCESS_MISSING: u8 = 2;

/// One word so a reader never observes half of an update.
static HOST_STATE: AtomicU8 = AtomicU8::new(0);
/// Shared-storage access as last reported by the host.
static STORAGE_ACCESS: AtomicU8 = AtomicU8::new(ACCESS_UNREPORTED);
static STORAGE_RUNS: Mutex<BTreeMap<u64, Weak<AtomicBool>>> = Mutex::new(BTreeMap::new());
static NEXT_STORAGE_RUN: AtomicU64 = AtomicU64::new(1);

/// Registration lives with the actual worker, including a worker whose
/// blocking I/O has not returned after the supervisor stopped waiting.
pub(crate) struct StorageRunGuard {
    id: Option<u64>,
}

impl StorageRunGuard {
    pub(crate) fn access_missing(&self) -> bool {
        self.id.is_some() && storage_access() != Some(true)
    }
}

impl Drop for StorageRunGuard {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            STORAGE_RUNS
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&id);
        }
    }
}

/// The central locator/platform boundary selects shared-storage runs. No
/// endpoint is opened here; remote and app-private runs need no registration.
pub(crate) fn register_storage_run(
    source: &str,
    target: &str,
    cancel: &Arc<AtomicBool>,
) -> StorageRunGuard {
    if !needs_shared_access(
        super::platform::requires_storage_access(source),
        super::platform::requires_storage_access(target),
    ) {
        return StorageRunGuard { id: None };
    }
    let id = NEXT_STORAGE_RUN.fetch_add(1, Ordering::Relaxed);
    let mut runs = STORAGE_RUNS.lock().unwrap_or_else(PoisonError::into_inner);
    register_storage_marker(&mut runs, id, cancel, &STORAGE_ACCESS);
    StorageRunGuard { id: Some(id) }
}

/// Caller holds the registry mutex used for both registration and revoke.
fn register_storage_marker(
    runs: &mut BTreeMap<u64, Weak<AtomicBool>>,
    id: u64,
    cancel: &Arc<AtomicBool>,
    access: &AtomicU8,
) {
    runs.retain(|_, weak| weak.strong_count() > 0);
    runs.insert(id, Arc::downgrade(cancel));
    // Same mutex as revocation: a registration can neither miss a revoke
    // nor open a root using the earlier granted report.
    if access.load(Ordering::Acquire) != ACCESS_GRANTED {
        cancel.store(true, Ordering::Release);
    }
}

fn needs_shared_access(source: bool, target: bool) -> bool {
    source || target
}

fn cancel_storage_runs(runs: &mut BTreeMap<u64, Weak<AtomicBool>>) {
    runs.retain(|_, weak| match weak.upgrade() {
        Some(cancel) => {
            cancel.store(true, Ordering::Release);
            true
        }
        None => false,
    });
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostState {
    pub power_save: bool,
    pub metered: bool,
    /// Scheduled jobs (startup, timer, real-time, on-connect) are not
    /// enqueued; running jobs, catch-up runs and auto-pause are unaffected.
    pub defer_scheduling: bool,
}

/// Replace the host-reported conditions; the next pause check sees them.
pub fn set_host_state(state: HostState) {
    HOST_STATE.store(encode(state), Ordering::Release);
}

/// Hold scheduled jobs until the host reports its conditions for the first
/// time (`set_host_state` replaces this bit). Only this bit changes, so a
/// concurrent report of the other conditions is never lost.
pub fn defer_scheduling_until_reported() {
    mark_deferred(&HOST_STATE);
}

/// The last host-reported conditions (all false until a host reports).
pub fn host_state() -> HostState {
    decode(HOST_STATE.load(Ordering::Acquire))
}

/// Android: whether the app may read and write all of shared storage
/// ("Zugriff auf alle Dateien"). Without it, jobs with a local shared-storage
/// side do not run (the filtered view would turn other apps' files into
/// deletions) and fail with "Dateizugriff fehlt".
pub fn set_storage_access(granted: bool) {
    let mut runs = STORAGE_RUNS.lock().unwrap_or_else(PoisonError::into_inner);
    let value = if granted {
        ACCESS_GRANTED
    } else {
        ACCESS_MISSING
    };
    STORAGE_ACCESS.store(value, Ordering::Release);
    if !granted {
        cancel_storage_runs(&mut runs);
    }
}

/// The last reported shared-storage access; `None` until the host reports
/// (desktop hosts never do: local paths are accessed as the user).
pub fn storage_access() -> Option<bool> {
    match STORAGE_ACCESS.load(Ordering::Acquire) {
        ACCESS_GRANTED => Some(true),
        ACCESS_MISSING => Some(false),
        _ => None,
    }
}

fn mark_deferred(word: &AtomicU8) {
    word.fetch_or(DEFER_SCHEDULING, Ordering::AcqRel);
}

fn encode(state: HostState) -> u8 {
    (if state.power_save { POWER_SAVE } else { 0 })
        | (if state.metered { METERED } else { 0 })
        | (if state.defer_scheduling {
            DEFER_SCHEDULING
        } else {
            0
        })
}

fn decode(bits: u8) -> HostState {
    HostState {
        power_save: bits & POWER_SAVE != 0,
        metered: bits & METERED != 0,
        defer_scheduling: bits & DEFER_SCHEDULING != 0,
    }
}

#[cfg(test)]
mod tests {
    use super::{decode, encode, mark_deferred, HostState};
    use std::sync::atomic::{AtomicU8, Ordering};

    #[test]
    fn android_shared_storage_revoke_selection_and_weak_lifetime() {
        use super::{cancel_storage_runs, needs_shared_access};
        use std::collections::BTreeMap;
        use std::sync::{atomic::AtomicBool, Arc};
        assert!(!needs_shared_access(false, false)); // remote/remote or private
        assert!(needs_shared_access(true, false));
        assert!(needs_shared_access(false, true));
        let active = Arc::new(AtomicBool::new(false));
        let ended = Arc::new(AtomicBool::new(false));
        let mut runs = BTreeMap::from([(1, Arc::downgrade(&active)), (2, Arc::downgrade(&ended))]);
        drop(ended);
        cancel_storage_runs(&mut runs);
        assert!(active.load(Ordering::Acquire));
        assert_eq!(runs.len(), 1);
    }

    #[test]
    fn android_shared_storage_registration_covers_both_revoke_orderings() {
        use super::{cancel_storage_runs, register_storage_marker, ACCESS_GRANTED, ACCESS_MISSING};
        use std::collections::BTreeMap;
        use std::sync::{atomic::AtomicBool, Arc, Mutex};
        for revoke_first in [false, true] {
            let access = AtomicU8::new(ACCESS_GRANTED);
            let registry = Mutex::new(BTreeMap::new());
            let cancel = Arc::new(AtomicBool::new(false));
            if revoke_first {
                let mut runs = registry.lock().unwrap();
                access.store(ACCESS_MISSING, Ordering::Release);
                cancel_storage_runs(&mut runs);
            }
            register_storage_marker(&mut registry.lock().unwrap(), 1, &cancel, &access);
            if !revoke_first {
                let mut runs = registry.lock().unwrap();
                access.store(ACCESS_MISSING, Ordering::Release);
                cancel_storage_runs(&mut runs);
            }
            assert!(cancel.load(Ordering::Acquire));
        }
    }

    #[test]
    fn android_task_host_state_encoding_keeps_both_conditions() {
        for power_save in [false, true] {
            for metered in [false, true] {
                let state = HostState {
                    power_save,
                    metered,
                    ..HostState::default()
                };
                assert_eq!(decode(encode(state)), state);
            }
        }
        assert_eq!(decode(0), HostState::default());
    }

    #[test]
    fn android_background_task_defer_bit_is_independent_of_conditions() {
        for bits in 0..8u8 {
            let state = decode(bits);
            assert_eq!(encode(state), bits);
        }
        let reported = HostState {
            power_save: true,
            metered: true,
            defer_scheduling: false,
        };
        let word = AtomicU8::new(encode(reported));
        mark_deferred(&word);
        let deferred = decode(word.load(Ordering::Acquire));
        assert!(deferred.defer_scheduling);
        assert!(deferred.power_save && deferred.metered);
        // A host report replaces the start-up deferral.
        word.store(encode(reported), Ordering::Release);
        assert!(!decode(word.load(Ordering::Acquire)).defer_scheduling);
    }
}
