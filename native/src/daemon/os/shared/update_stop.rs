//! Shared update-stop helper (DSK-M2, contract V8) for every long-running
//! process of the GUI or CLI image.
//!
//! The updater helper never kills processes: it writes `daemon.stop` = `stop`
//! and waits at most 30 seconds until no process of the target image runs
//! (`bin/smart_explorer_updater/process.rs`), then it gives up with "Smart
//! Explorer laeuft noch und blockiert das Update". Each such process therefore
//! ends itself on a *new* stop request. The marker time present when the
//! process starts is its baseline (a crashed update can leave the marker
//! behind); a new or rewritten marker is a request. The marker is read once
//! per second, far inside the 30 seconds of the helper. Version handoffs
//! (`handoff:<generation>`) never count, because `update_stop_marker()` only
//! reports the value `stop`.
//!
//! Users (V8): `se mcp`, every GUI window (through
//! `app/core/update_stop_ui.rs`), the se Desktop shell and surfaces, the screen
//! host worker and indicator, and the drive hosts. A background process ends
//! with [`EXIT_UPDATE_STOP`] so that no supervisor restarts it.
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

/// Exit code of a background process that ended on an update-stop request
/// (`EX_TEMPFAIL`). Supervisors never count it as a crash and never restart
/// the process.
pub const EXIT_UPDATE_STOP: i32 = 75;

/// Interval between two reads of the stop marker (V8: once per second).
pub const UPDATE_STOP_POLL: Duration = Duration::from_secs(1);

/// Shortest accepted interval; a zero interval would spin.
const MIN_POLL: Duration = Duration::from_millis(10);

/// Reads the modification time of a pending update-stop request. Production
/// reads `daemon.stop` through `update_stop_marker()`; tests and fakes pass
/// their own source to [`UpdateStopBaseline::with_source`].
pub type UpdateStopSource = Arc<dyn Fn() -> Option<SystemTime> + Send + Sync>;

/// Whether `current` is an update-stop request that did not exist at the
/// baseline: no marker is no request; a marker without baseline is a request;
/// otherwise every change of the marker time is a request. This is the rule
/// `se mcp` applied before this helper existed (test
/// `mcp_task_only_new_update_stop_requests_end_the_server`).
pub fn is_new_request(baseline: Option<SystemTime>, current: Option<SystemTime>) -> bool {
    match (baseline, current) {
        (_, None) => false,
        (None, Some(_)) => true,
        (Some(before), Some(now)) => before != now,
    }
}

/// The marker time seen when the process started; later markers are
/// compared against it.
#[derive(Clone)]
pub struct UpdateStopBaseline {
    baseline: Option<SystemTime>,
    source: UpdateStopSource,
    installation_stop: Option<(
        std::path::PathBuf,
        Option<crate::mcp_launcher::core::lifecycle::StopIdentity>,
    )>,
}

impl UpdateStopBaseline {
    /// Records the current `daemon.stop` request as baseline. Call it as
    /// early as possible: a request written before this call is ignored.
    pub fn arm() -> Self {
        let mut baseline = Self::with_source(Arc::new(super::handoff::update_stop_marker));
        baseline.installation_stop = crate::mcp_launcher::installation_stop_marker().map(|path| {
            let identity = crate::mcp_launcher::stop_marker_identity(&path);
            (path, identity)
        });
        baseline
    }

    /// The same rule against another marker source.
    pub fn with_source(source: UpdateStopSource) -> Self {
        let baseline = source();
        Self {
            baseline,
            source,
            installation_stop: None,
        }
    }

    /// Whether a new update-stop request exists since the baseline.
    pub fn requested(&self) -> bool {
        is_new_request(self.baseline, (self.source)())
            || self
                .installation_stop
                .as_ref()
                .is_some_and(|(path, before)| {
                    let current = crate::mcp_launcher::stop_marker_identity(path);
                    current.is_some() && current != *before
                })
    }

    /// Reads the marker every `poll` (at least 10 ms) in a background thread
    /// and calls `on_request` once on the first new request; the thread then
    /// ends. The watch lives as long as the returned handle.
    pub fn spawn_watch<F>(self, poll: Duration, on_request: F) -> io::Result<UpdateStopWatch>
    where
        F: FnOnce() + Send + 'static,
    {
        let poll = poll.max(MIN_POLL);
        let fired = Arc::new(AtomicBool::new(false));
        let thread_fired = fired.clone();
        let (cancel, cancelled) = mpsc::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("se-update-stop".into())
            .spawn(move || loop {
                match cancelled.recv_timeout(poll) {
                    Err(RecvTimeoutError::Timeout) => {
                        if self.requested() {
                            thread_fired.store(true, Ordering::Release);
                            on_request();
                            return;
                        }
                    }
                    Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
                }
            })?;
        Ok(UpdateStopWatch {
            fired,
            cancel: Some(cancel),
            thread: Some(thread),
        })
    }
}

impl std::fmt::Debug for UpdateStopBaseline {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UpdateStopBaseline")
            .field("baseline", &self.baseline)
            .finish_non_exhaustive()
    }
}

/// Starts [`UpdateStopBaseline::spawn_watch`] with a baseline taken now.
pub fn spawn_update_stop_watch<F>(poll: Duration, on_request: F) -> io::Result<UpdateStopWatch>
where
    F: FnOnce() + Send + 'static,
{
    UpdateStopBaseline::arm().spawn_watch(poll, on_request)
}

/// Handle of a running watch. Dropping it ends the watch thread at once
/// without waiting for it, so `on_request` may drop the owner of the handle.
#[must_use = "dropping the watch stops it"]
pub struct UpdateStopWatch {
    fired: Arc<AtomicBool>,
    cancel: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl UpdateStopWatch {
    /// Whether the watch saw a new request; set before `on_request` runs.
    pub fn fired(&self) -> bool {
        self.fired.load(Ordering::Acquire)
    }

    /// Ends the watch and waits for its thread, except when called from
    /// `on_request` itself (that thread ends on its own).
    pub fn stop(mut self) {
        self.cancel.take();
        if let Some(thread) = self.thread.take() {
            if thread.thread().id() != std::thread::current().id() {
                let _ = thread.join();
            }
        }
    }
}

impl Drop for UpdateStopWatch {
    fn drop(&mut self) {
        // Closing the channel wakes the thread immediately; never join here.
        self.cancel.take();
    }
}

impl std::fmt::Debug for UpdateStopWatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UpdateStopWatch")
            .field("fired", &self.fired())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "update_stop_tests.rs"]
mod tests;
