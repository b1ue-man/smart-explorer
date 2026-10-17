//! Update stop for egui windows (DSK-M2, contract V8; fixes R70).
//!
//! Every window process of the GUI image (explorer windows, `--access-consent`,
//! `--screen-view`, `--screen-grid`, `--screen-indicator` and the windows of
//! `--open-location`/`--open-page`) must end on a new update-stop request, or
//! the updater helper gives up after 30 seconds. The window arms the shared
//! baseline at start; on a new request the watch thread raises a flag and
//! wakes the window, which then closes through `ViewportCommand::Close`: the
//! same path as a close by the user (sync exit gate, `on_exit`,
//! `prepare_for_exit(false)`). A marker present at start is ignored.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use eframe::egui;

use crate::daemon::{UpdateStopBaseline, UpdateStopWatch, UPDATE_STOP_POLL};

#[derive(Default)]
struct CloseSignal {
    requested: AtomicBool,
    /// The window's context, known from its first frame, to wake it up.
    ctx: Mutex<Option<egui::Context>>,
}

impl CloseSignal {
    fn raise(&self) {
        self.requested.store(true, Ordering::Release);
        if let Ok(slot) = self.ctx.lock() {
            if let Some(ctx) = slot.as_ref() {
                ctx.request_repaint();
            }
        }
    }
}

/// The update-stop state of one window process.
pub(crate) struct WindowUpdateStop {
    signal: Arc<CloseSignal>,
    _watch: UpdateStopWatch,
}

impl WindowUpdateStop {
    /// Baseline now, `daemon.stop` read once per second.
    pub(crate) fn arm() -> std::io::Result<Self> {
        Self::arm_with(UpdateStopBaseline::arm(), UPDATE_STOP_POLL)
    }

    pub(crate) fn arm_with(baseline: UpdateStopBaseline, poll: Duration) -> std::io::Result<Self> {
        let signal = Arc::new(CloseSignal::default());
        let raised = signal.clone();
        let watch = baseline.spawn_watch(poll, move || raised.raise())?;
        Ok(Self {
            signal,
            _watch: watch,
        })
    }

    /// Call once per frame: `true` exactly once after a new request, then the
    /// caller sends `ViewportCommand::Close`.
    pub(crate) fn take_close_request(&self, ctx: &egui::Context) -> bool {
        if let Ok(mut slot) = self.signal.ctx.lock() {
            if slot.is_none() {
                *slot = Some(ctx.clone());
            }
        }
        self.signal.requested.swap(false, Ordering::AcqRel)
    }
}

static WINDOW: OnceLock<WindowUpdateStop> = OnceLock::new();

/// Arms this process's window update stop; later calls keep the first
/// baseline. Without a watch thread the window behaves as before v0.5.183
/// (the update then fails with the helper's existing message).
pub(crate) fn arm_window() {
    if WINDOW.get().is_some() {
        return;
    }
    match WindowUpdateStop::arm() {
        Ok(stop) => {
            let _ = WINDOW.set(stop);
        }
        Err(error) => {
            eprintln!("Smart Explorer: Update-Stopp-Wache konnte nicht starten: {error}");
        }
    }
}

/// Per frame of an armed window: whether to close it for an update now.
pub(crate) fn close_requested(ctx: &egui::Context) -> bool {
    WINDOW
        .get()
        .is_some_and(|stop| stop.take_close_request(ctx))
}

#[cfg(test)]
#[path = "update_stop_ui_tests.rs"]
mod tests;
