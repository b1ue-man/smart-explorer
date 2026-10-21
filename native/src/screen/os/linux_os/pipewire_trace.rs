//! What a PipeWire screen stream did (SCR-M5): the state it reached, the
//! format it negotiated, how many buffers the process callback saw and why
//! a buffer gave no picture. Nothing in it decides anything; it is the text
//! of the failure of a stream that stays silent ("no picture": no
//! connection, no format, no buffers, or buffers without content), so the
//! evidence of a run names the stage.
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

/// `enum pw_stream_state` as the callbacks report it.
#[cfg(test)]
fn state_name(state: i32) -> &'static str {
    match state {
        -1 => "error",
        0 => "unconnected",
        1 => "connecting",
        2 => "paused",
        3 => "streaming",
        _ => "unknown",
    }
}

#[derive(Default)]
pub(crate) struct Trace {
    state: AtomicI32,
    state_changes: AtomicU64,
    format_events: AtomicU64,
    process_calls: AtomicU64,
    skipped: AtomicU64,
    empty_chunks: AtomicU64,
    format: Mutex<String>,
    last_skip: Mutex<String>,
}

fn locked(text: &Mutex<String>) -> std::sync::MutexGuard<'_, String> {
    text.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Trace {
    pub(crate) fn state(&self, state: i32) {
        self.state.store(state, Ordering::Relaxed);
        self.state_changes.fetch_add(1, Ordering::Relaxed);
    }

    /// A `Format` param arrived: `text` names it (`Bgrx 1280x800`, or
    /// `none` when the format was cleared).
    pub(crate) fn format(&self, text: &str) {
        self.format_events.fetch_add(1, Ordering::Relaxed);
        *locked(&self.format) = text.to_string();
    }

    /// The process callback ran.
    pub(crate) fn process(&self) {
        self.process_calls.fetch_add(1, Ordering::Relaxed);
    }

    /// A buffer gave no picture, for `reason`.
    pub(crate) fn skip(&self, reason: &str) {
        self.skipped.fetch_add(1, Ordering::Relaxed);
        *locked(&self.last_skip) = reason.to_string();
    }

    /// A buffer was an empty or corrupted chunk (a cursor-only buffer).
    pub(crate) fn empty_chunk(&self) {
        self.empty_chunks.fetch_add(1, Ordering::Relaxed);
    }

    /// The stream streams with a negotiated format, and all of its buffers
    /// without picture (at least one) were empty or corrupted chunks.
    #[cfg(test)]
    pub(crate) fn only_empty_chunks(&self) -> bool {
        let skipped = self.skipped.load(Ordering::Relaxed);
        self.state.load(Ordering::Relaxed) == 3
            && self.format_events.load(Ordering::Relaxed) > 0
            && skipped > 0
            && skipped == self.empty_chunks.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn describe(&self) -> String {
        format!(
            "stream {} after {} state changes; format {:?} after {} format events; \
             process callback {} times, {} buffers without picture, {} of them empty or \
             corrupted chunks (last: {:?})",
            state_name(self.state.load(Ordering::Relaxed)),
            self.state_changes.load(Ordering::Relaxed),
            locked(&self.format).as_str(),
            self.format_events.load(Ordering::Relaxed),
            self.process_calls.load(Ordering::Relaxed),
            self.skipped.load(Ordering::Relaxed),
            self.empty_chunks.load(Ordering::Relaxed),
            locked(&self.last_skip).as_str(),
        )
    }
}
