//! The source time a one-step writer is given while it is opened.
//!
//! `Backend::open_write_fresh` and `vfs::open_write_absent` create the file
//! under its final name in one request, so the object exists only after the
//! upload and no stage is left to be timed afterwards. Providers that take
//! the time in the create request (Drive `modifiedTime`) read it from here
//! while the engine opens the writer on this thread. The engine offers it
//! around exactly that call (`CreationTime::offer`); the guard restores what
//! was offered before, so nothing outlives the open and no other writer
//! (sync, mounts, other threads) ever sees it.
use std::cell::Cell;

thread_local! {
    static OFFERED: Cell<Option<i64>> = const { Cell::new(None) };
}

/// The offer is withdrawn when this is dropped.
#[must_use = "the offer ends when the guard is dropped"]
pub(crate) struct CreationTime {
    previous: Option<i64>,
}

impl CreationTime {
    /// Offers `mtime_ms` (Unix milliseconds) to the writer opened next on
    /// this thread; `0` or less is "unknown" and offers nothing.
    pub(crate) fn offer(mtime_ms: i64) -> Self {
        let previous = OFFERED.with(|cell| cell.replace((mtime_ms > 0).then_some(mtime_ms)));
        Self { previous }
    }
}

impl Drop for CreationTime {
    fn drop(&mut self) {
        OFFERED.with(|cell| cell.set(self.previous));
    }
}

/// The time offered for the writer being opened on this thread right now.
pub(crate) fn offered() -> Option<i64> {
    OFFERED.with(Cell::get)
}

#[cfg(test)]
#[path = "creation_time_tests.rs"]
mod tests;
