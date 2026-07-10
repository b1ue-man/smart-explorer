/// Effect reported by the drop target after a completed OS drag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragOutEffect {
    None,
    Copy,
    Move,
}

/// Final state of an OS drag. Cancellation is deliberately not an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragOutOutcome {
    Cancelled,
    Dropped(DragOutEffect),
}

#[cfg(windows)]
#[path = "os/windows.rs"]
mod imp;

#[cfg(windows)]
pub use imp::*;
