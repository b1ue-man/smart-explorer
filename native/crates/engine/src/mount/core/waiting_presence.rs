//! A request a program waits for that meets a fetch the engine started by
//! itself (plan 2026-11-14, E4c/E4d). The fetch is background work and sets
//! no metadata presence at the Share device, so transfers to the device
//! would go on at full speed while the program waits. While the program
//! waits, a helper asks the device for the waited path with the drive's own
//! origin (`User`, no background mark) and asks again before the hold time
//! of the last answer ends; the presence then lasts for the whole wait and
//! the hold time after it. Only a Share device knows presence: any other
//! source gets no request at all.
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use crate::vfs::{BackendHandle, Scheme};

/// Shorter than the shortest hold time of a presence (3 s,
/// `share/core/yield_engagement.rs`, `NAVIGATION_GAP`), so the next request
/// begins before the presence of the last one ends.
pub(super) const RENEW_AFTER: Duration = Duration::from_secs(2);

/// Holds the presence of a waiting request until it drops (the waiter has
/// the gate).
pub(super) struct WaitingPresence {
    _stop: mpsc::Sender<()>,
}

/// Starts the presence of a request that waits for `path`; `None` for a
/// source without a Share device, for background work (it waits for its own
/// kind) and when no helper thread starts.
pub(super) fn start(backend: &BackendHandle, path: &str) -> Option<WaitingPresence> {
    if crate::vfs::in_background() || backend.scheme() != Scheme::Peer {
        return None;
    }
    let (stop, stopped) = mpsc::channel::<()>();
    let backend = std::sync::Arc::clone(backend);
    let path = path.to_string();
    std::thread::Builder::new()
        .name("mount-presence".into())
        .spawn(move || loop {
            let _ = backend.stat(&path);
            match stopped.recv_timeout(RENEW_AFTER) {
                Err(RecvTimeoutError::Timeout) => continue,
                Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
            }
        })
        .ok()?;
    Some(WaitingPresence { _stop: stop })
}
