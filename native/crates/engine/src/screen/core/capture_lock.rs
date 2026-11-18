//! The locked screen of a host (measurement of 2026-11-18 with 0.5.200, a
//! locked Windows PC: LogonUI showed, Desktop Duplication refused every
//! `DuplicateOutput` with `E_INVALIDARG`, the duplication was rebuilt and
//! logged about every 200 ms and the viewer got no picture and no reason).
//!
//! The decisions here are pure so the task suite checks them on every
//! platform; the Windows adapters ask the system (`secure_desktop.rs`,
//! `capture_source.rs`, `dxgi_rebuild.rs`):
//!
//! - `pause_state`: the state that pauses a capture, in this order: an
//!   inactive session, a locked session (`HostLocked`, the viewer shows
//!   „Das Gerät ist gesperrt“), the secure desktop (UAC, login). The
//!   session's lock flag is read as well as the input desktop: a locked
//!   session whose input desktop still reads as `Default` paused nothing
//!   before. A paused capture is opened anew once the pause ends
//!   (`WindowsSource::renew`), so the picture returns without a new
//!   connection.
//! - `session_lock_flag`: the lock flag of `WTSINFOEX_LEVEL1_W::SessionFlags`
//!   (`WTS_SESSIONSTATE_LOCK` 0, `WTS_SESSIONSTATE_UNLOCK` 1, otherwise
//!   unknown; Microsoft Learn, `WTSINFOEX_LEVEL1_W`). [Assumption
//!   A-ROLLOUT-LOCKFLAG: Windows 10 and later report the flag as documented;
//!   Windows 7 / Server 2008 R2 swapped both values, se does not support them.]
//! - `PauseLog`: a pause goes to the host log when it begins and ends, not
//!   per check.
//! - `RebuildBackoff`: a rebuild that keeps failing after its settling time
//!   is tried again after 1 s, 2 s, 4 s, then every 5 s, and logged once per
//!   streak; in between the last error stands (the fallback to the next
//!   capture path sees it as before).
use std::time::{Duration, Instant};

use super::session_state::SessionState;

/// What `pause_state` asks the system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PauseQuestion {
    /// The session is connected.
    SessionActive,
    /// The session's lock flag says locked.
    SessionLocked,
    /// The input desktop is not the user's.
    SecureDesktop,
}

/// The state that pauses a capture, asking `ask` in the order of
/// `PauseQuestion` and only as far as needed.
pub fn pause_state(mut ask: impl FnMut(PauseQuestion) -> bool) -> Option<SessionState> {
    if !ask(PauseQuestion::SessionActive) {
        return Some(SessionState::SessionInactive);
    }
    if ask(PauseQuestion::SessionLocked) {
        return Some(SessionState::HostLocked);
    }
    if ask(PauseQuestion::SecureDesktop) {
        return Some(SessionState::SecureDesktop);
    }
    None
}

/// Whether a pause in `state` makes the capture open anew when it ends.
pub fn renews_after(state: SessionState) -> bool {
    matches!(
        state,
        SessionState::SecureDesktop | SessionState::SessionInactive | SessionState::HostLocked
    )
}

/// `WTS_SESSIONSTATE_LOCK`.
const LOCK: u32 = 0;
/// `WTS_SESSIONSTATE_UNLOCK`.
const UNLOCK: u32 = 1;

/// The lock flag of a `WTSINFOEXW` of `level` with `flags`
/// (`SessionFlags`); `None` for another level or an unknown flag.
pub fn session_lock_flag(level: u32, flags: u32) -> Option<bool> {
    if level != 1 {
        return None;
    }
    match flags {
        LOCK => Some(true),
        UNLOCK => Some(false),
        _ => None,
    }
}

/// Logs a pause when it begins or ends (once per change).
#[derive(Debug, Default)]
pub struct PauseLog {
    last: Option<SessionState>,
}

impl PauseLog {
    pub const fn new() -> Self {
        Self { last: None }
    }

    /// The line for the host log when `state` differs from the last one.
    pub fn line(&mut self, state: Option<SessionState>) -> Option<String> {
        if state == self.last {
            return None;
        }
        let before = std::mem::replace(&mut self.last, state);
        Some(match (before, state) {
            (_, Some(state)) => format!("screen capture: paused ({})", state.key()),
            (Some(before), None) => format!("screen capture: pause ({}) ended", before.key()),
            (None, None) => return None,
        })
    }
}

/// The first wait of a rebuild that keeps failing.
pub const BACKOFF_FIRST: Duration = Duration::from_secs(1);
/// The longest wait between two rebuilds.
pub const BACKOFF_LONGEST: Duration = Duration::from_secs(5);

/// Rebuilds of a capture that keep failing after their settling time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RebuildBackoff {
    failures: u32,
    next: Option<Instant>,
}

impl RebuildBackoff {
    /// The wait after `failures` failed rebuilds: 1 s, 2 s, 4 s, then 5 s.
    pub fn wait_after(failures: u32) -> Duration {
        let doubled = BACKOFF_FIRST.saturating_mul(1u32 << failures.saturating_sub(1).min(8));
        doubled.min(BACKOFF_LONGEST)
    }

    /// Whether a rebuild may be tried at `now`.
    pub fn due(&self, now: Instant) -> bool {
        self.next.is_none_or(|next| now >= next)
    }

    /// A rebuild failed at `now`; `true` for the first failure of the streak
    /// (the one the host log gets).
    pub fn failed(&mut self, now: Instant) -> bool {
        self.failures = self.failures.saturating_add(1);
        self.next = Some(now + Self::wait_after(self.failures));
        self.failures == 1
    }

    /// A rebuild succeeded; the number of failures it ends (0: none).
    pub fn succeeded(&mut self) -> u32 {
        std::mem::take(self).failures
    }
}

#[cfg(test)]
#[path = "rollout_lock_task_tests.rs"]
mod rollout_lock_task_tests;
