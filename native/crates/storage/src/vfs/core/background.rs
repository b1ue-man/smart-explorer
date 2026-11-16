//! Work a program started by itself (plan 2026-11-14, E4c): a cache refresh,
//! a preload, an upload of a closed file, a retry. Nobody waits for its
//! answer, so it must never count as interactive use of a device (metadata
//! presence, `share/core/metadata_presence.rs`), however its handle was
//! opened. A request that an operating-system call waits for stays outside
//! every scope.
//!
//! The mark belongs to the calling thread: the backends of this program
//! answer a call on the thread that makes it, so a scope held while a call
//! runs reaches every layer below it. Work that moves to another thread
//! enters its own scope there (`mount-metadata-load`, `mount-upload`).
use std::cell::Cell;
use std::marker::PhantomData;

thread_local! {
    static DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// Whether the calling thread runs work its program started by itself.
pub fn in_background() -> bool {
    DEPTH.with(|depth| depth.get() > 0)
}

/// The calling thread runs work its program started by itself until the
/// scope drops; scopes nest.
#[must_use = "the work counts as background only while the scope lives"]
pub struct BackgroundScope {
    /// Not `Send`: the mark belongs to the thread that entered it.
    _thread: PhantomData<*const ()>,
}

impl BackgroundScope {
    pub fn enter() -> Self {
        DEPTH.with(|depth| depth.set(depth.get().saturating_add(1)));
        Self {
            _thread: PhantomData,
        }
    }
}

impl Drop for BackgroundScope {
    fn drop(&mut self) {
        DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

#[cfg(test)]
#[path = "background_tests.rs"]
mod tests;
