//! Android lock signals (C3.4): the system sends the app no signal the
//! worker could hear itself. `HostMonitor` reports the screen
//! (`ACTION_SCREEN_ON`/`OFF` → `sys.hostState.screenOn`) and the visibility
//! (`foreground`); `mobile/os/shared/sys.rs` hands both to the session
//! register, which locks on screen off and after the background minutes.
//! There is no sleep signal on Android (C7.1).
use std::sync::Weak;

use crate::vault::core::lock_policy::{SignalControl, SignalSink, SignalSupport};

/// Android needs no note next to its screen switch.
pub(super) const SCREEN_LOCK_NOTE: Option<&str> = None;

/// The host reports through `sys.hostState`; nothing to start.
struct HostReported;

impl SignalControl for HostReported {
    fn set_armed(&self, _armed: bool) {}

    fn support(&self) -> SignalSupport {
        SignalSupport {
            sleep: false,
            screen_lock: true,
            background: true,
        }
    }
}

pub(super) fn start_signal_watch(_sink: Weak<dyn SignalSink>) -> Box<dyn SignalControl> {
    Box::new(HostReported)
}

/// `MemAvailable` as a fallback; the app passes `memAvail − memThreshold`
/// of `ActivityManager.MemoryInfo` with `vault.unlock` (C1.3 (c)), which
/// takes precedence, because an allocation up to `MemAvailable` can wake the
/// low-memory killer.
pub(super) fn available_memory() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = text
        .lines()
        .find(|line| line.starts_with("MemAvailable:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kib.checked_mul(1024)
}
