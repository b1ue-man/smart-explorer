//! States of one screen session (decisions C1, C5, C7; `se screen sessions`):
//! the keys that `STATE` frames, the CLI and the IPC show, which moves between
//! them are allowed, why a session ended, and the viewer rule of at most one
//! open key-frame request (C3 „Schlüsselbild“).
use std::io;

use serde::{Deserialize, Serialize};

use super::wire::invalid;

/// After this time without a key frame the viewer may ask again; it equals
/// the time after which the host resets a stale picture stream (C3).
pub(crate) const KEYFRAME_RETRY_US: u64 = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SessionState {
    Connecting,
    /// The host user is asked (S1 `access_approval`).
    WaitingForApproval,
    /// The Wayland portal dialog is open on the host (SCR-M5).
    WaitingForPortal,
    Starting,
    Streaming,
    /// Windows shows the secure desktop (UAC prompt, lock screen); capture
    /// and input pause.
    SecureDesktop,
    /// The shown window is minimized (Windows `IsIconic`).
    WindowMinimized,
    /// The Android host locked; media projection ended.
    HostLocked,
    /// The host's user session is disconnected (Windows).
    SessionInactive,
    /// No graphical session or capture path; the report names the reason,
    /// e.g. `pipewire_missing`.
    NoDisplay,
    Ended,
    /// A state of a newer host; the viewer keeps the session.
    #[serde(other)]
    Unknown,
}

impl SessionState {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::WaitingForApproval => "waiting_for_approval",
            Self::WaitingForPortal => "waiting_for_portal",
            Self::Starting => "starting",
            Self::Streaming => "streaming",
            Self::SecureDesktop => "secure_desktop",
            Self::WindowMinimized => "window_minimized",
            Self::HostLocked => "host_locked",
            Self::SessionInactive => "session_inactive",
            Self::NoDisplay => "no_display",
            Self::Ended => "ended",
            Self::Unknown => "unknown",
        }
    }

    /// The source exists but delivers no usable pictures for now.
    pub(crate) fn is_paused(self) -> bool {
        matches!(
            self,
            Self::SecureDesktop | Self::WindowMinimized | Self::HostLocked | Self::SessionInactive
        )
    }

    pub(crate) fn is_terminal(self) -> bool {
        self == Self::Ended
    }

    /// Whether a session may move from `self` to `next`. Every state may end;
    /// nothing leaves `Ended`; after an unknown state anything may follow. A
    /// running session may wait again: a preview raised to the full view
    /// under `Ask` waits for approval, control on Wayland for a new portal
    /// dialog (C5).
    pub(crate) fn can_move_to(self, next: Self) -> bool {
        use SessionState::*;
        if self == Ended || next == Unknown || self == next {
            return false;
        }
        if next == Ended || self == Unknown {
            return true;
        }
        let running = matches!(self, Starting | Streaming) || self.is_paused();
        let waiting = matches!(self, WaitingForApproval | WaitingForPortal);
        match next {
            WaitingForApproval => self == Connecting || running,
            WaitingForPortal => matches!(self, Connecting | WaitingForApproval) || running,
            Starting => matches!(self, Connecting | NoDisplay) || waiting || running,
            Streaming => waiting || running,
            SecureDesktop | WindowMinimized | HostLocked | SessionInactive => running,
            NoDisplay => matches!(self, Connecting | WaitingForPortal) || running,
            Connecting | Ended | Unknown => false,
        }
    }
}

/// The state of one session with the time it was entered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SessionTracker {
    state: SessionState,
    since_us: u64,
}

impl SessionTracker {
    pub(crate) fn new(now_us: u64) -> Self {
        Self {
            state: SessionState::Connecting,
            since_us: now_us,
        }
    }

    pub(crate) fn state(&self) -> SessionState {
        self.state
    }

    pub(crate) fn since_us(&self) -> u64 {
        self.since_us
    }

    /// Moves to `next`; a repeated state is a no-op, a move the table does
    /// not allow is refused and leaves the state unchanged.
    pub(crate) fn move_to(&mut self, next: SessionState, now_us: u64) -> io::Result<()> {
        if next == self.state {
            return Ok(());
        }
        if !self.state.can_move_to(next) {
            return Err(invalid("screen session state change is not allowed"));
        }
        self.state = next;
        self.since_us = now_us;
        Ok(())
    }
}

/// Why a session ended (audit event `session_ended`, `STATE` reason).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum EndReason {
    /// The viewer closed or withdrew its request.
    ViewerClosed,
    /// Ended on the host: indicator window, tab „Bildschirm“, `se screen
    /// stop`, Android notification.
    HostStopped,
    /// `RestrictionReason::CapabilityNarrowed(ScreenView | ScreenControl)`.
    CapabilityNarrowed,
    /// `RestrictionReason::GrantExpired`.
    GrantExpired,
    /// The daemon stopped (`host_shutdown`).
    HostShutdown,
    /// The host process saw the update stop marker (V8, exit 75).
    UpdateStop,
    ConnectionLost,
    /// Capture, encoder or protocol failure; the detail travels in `ERROR`.
    Failed,
}

impl EndReason {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::ViewerClosed => "viewer_closed",
            Self::HostStopped => "host_stopped",
            Self::CapabilityNarrowed => "capability_narrowed",
            Self::GrantExpired => "grant_expired",
            Self::HostShutdown => "host_shutdown",
            Self::UpdateStop => "update_stop",
            Self::ConnectionLost => "connection_lost",
            Self::Failed => "failed",
        }
    }
}

/// Viewer side of `KEYFRAME_REQUEST`: at most one open request, because a
/// key frame costs a multiple of a delta frame; a request older than
/// `KEYFRAME_RETRY_US` may be repeated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct KeyframeRequester {
    open_since_us: Option<u64>,
}

impl KeyframeRequester {
    /// Whether to send a request now; if so, it counts as open from `now_us`.
    pub(crate) fn should_request(&mut self, now_us: u64) -> bool {
        match self.open_since_us {
            Some(since) if now_us.saturating_sub(since) < KEYFRAME_RETRY_US => false,
            _ => {
                self.open_since_us = Some(now_us);
                true
            }
        }
    }

    /// A key frame arrived; the open request is answered.
    pub(crate) fn on_keyframe(&mut self) {
        self.open_since_us = None;
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open_since_us.is_some()
    }
}

#[cfg(test)]
#[path = "session_state_tests.rs"]
mod tests;
