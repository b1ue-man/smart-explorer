//! One cancellable metadata worker per engine; callbacks only read its snapshot.
//!
//! The worker asks the provider only on demand: a reader that finds no snapshot,
//! or one older than `FRESH`, requests a refresh and gets the cached value at once.
//! A mount nobody asks for its capacity (an idle drive, an Android documents root
//! nobody browses, the status report of the host) causes no provider request and
//! no wake-up; the parked worker waits on its condition variable without a timer.
use crate::vfs::{BackendHandle, CapacitySnapshot};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// How long a snapshot answers readers before the next one is taken: the
/// refresh period of the former polling worker.
const FRESH: Duration = Duration::from_secs(30);

#[derive(Default)]
pub struct CapacityCache {
    pub value: Mutex<Option<CapacitySnapshot>>,
    pub observer: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    control: Mutex<Control>,
    wake: Condvar,
}

#[derive(Default)]
struct Control {
    stopped: bool,
    requested: bool,
    refreshed: Option<Instant>,
}

impl CapacityCache {
    pub fn stop(&self) {
        if let Ok(mut control) = self.control.lock() {
            control.stopped = true;
            self.wake.notify_all();
        }
    }

    /// A reader looked at the snapshot: refresh it in the background when none
    /// was taken yet or the last one is older than `FRESH`. Never waits.
    pub fn demand(&self) {
        let Ok(mut control) = self.control.lock() else {
            return;
        };
        if control.stopped
            || control.requested
            || control.refreshed.is_some_and(|at| at.elapsed() < FRESH)
        {
            return;
        }
        control.requested = true;
        self.wake.notify_all();
    }

    /// Waits for the next request; `false` once the engine stopped the worker.
    fn next_request(&self) -> bool {
        let Ok(control) = self.control.lock() else {
            return false;
        };
        self.wake
            .wait_while(control, |control| !control.stopped && !control.requested)
            .is_ok_and(|control| !control.stopped)
    }

    /// Ends the request; `false` once the engine stopped the worker.
    fn finished(&self) -> bool {
        let Ok(mut control) = self.control.lock() else {
            return false;
        };
        control.requested = false;
        control.refreshed = Some(Instant::now());
        !control.stopped
    }
}

pub fn start(cache: Arc<CapacityCache>, backend: BackendHandle, root: String) {
    let _ = std::thread::Builder::new()
        .name("mount-capacity".into())
        .spawn(move || {
            while cache.next_request() {
                let snapshot = crate::vfs::capacity(&*backend, &root).unwrap_or_else(|error| {
                    crate::vfs::capacity_failure(
                        backend.state_identity(),
                        &root,
                        crate::vfs::CapacitySource::Unknown,
                        &error,
                    )
                });
                if !cache.finished() {
                    break;
                }
                let changed = if let Ok(mut slot) = cache.value.lock() {
                    let changed = slot.as_ref().is_none_or(|previous| {
                        let mut previous = previous.clone();
                        previous.observed_at_ms = snapshot.observed_at_ms;
                        previous != snapshot
                    });
                    *slot = Some(snapshot);
                    changed
                } else {
                    false
                };
                if changed {
                    let observer = cache
                        .observer
                        .lock()
                        .ok()
                        .and_then(|observer| observer.clone());
                    if let Some(observer) = observer {
                        observer();
                    }
                }
            }
        });
}
