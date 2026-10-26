//! One cancellable metadata worker per engine; callbacks only read its snapshot.
use crate::vfs::{BackendHandle, CapacitySnapshot};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
#[derive(Default)]
pub(crate) struct CapacityCache {
    pub(crate) value: Mutex<Option<CapacitySnapshot>>,
    pub(crate) observer: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    stopped: Mutex<bool>,
    wake: Condvar,
}
impl CapacityCache {
    pub(crate) fn stop(&self) {
        if let Ok(mut stopped) = self.stopped.lock() {
            *stopped = true;
            self.wake.notify_all();
        }
    }
}
pub(crate) fn start(cache: Arc<CapacityCache>, backend: BackendHandle, root: String) {
    let _ = std::thread::Builder::new()
        .name("mount-capacity".into())
        .spawn(move || loop {
            if cache.stopped.lock().map_or(true, |stopped| *stopped) {
                break;
            }
            let snapshot = crate::vfs::capacity(&*backend, &root).unwrap_or_else(|error| {
                crate::vfs::capacity_failure(
                    backend.state_identity(),
                    &root,
                    crate::vfs::CapacitySource::Unknown,
                    &error,
                )
            });
            if cache.stopped.lock().map_or(true, |stopped| *stopped) {
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
            let Ok(stopped) = cache.stopped.lock() else {
                break;
            };
            let Ok((stopped, _)) =
                cache
                    .wake
                    .wait_timeout_while(stopped, Duration::from_secs(30), |stop| !*stop)
            else {
                break;
            };
            if *stopped {
                break;
            }
        });
}
