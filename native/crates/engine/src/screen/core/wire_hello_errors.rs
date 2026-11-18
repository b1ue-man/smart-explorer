//! Error codes of the screen hellos and `ERROR` (decision C7), moved out of
//! `wire_hello.rs` so that file stays below 500 lines; the paths
//! `wire_hello::ScreenErrorCode` and `wire_hello::ScreenHelloError` stay.
use serde::{Deserialize, Serialize};

/// Error codes of `HELLO_ERROR` and `ERROR` (C7). The wire carries the key,
/// so a code of a newer host survives as text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScreenErrorCode {
    /// Before the proof or without the right; strangers learn nothing.
    PermissionDenied,
    /// Control asked for, only viewing allowed.
    RightNotGranted,
    ApprovalDenied,
    ApprovalCancelled,
    NoDisplay,
    /// The host has no desktop app to show the indicator (C7).
    IndicatorUnavailable,
    ControlBusy,
    ControlUnavailable,
    HostShutdown,
    /// The device does not know the screen ALPN (viewer side, `0x178`), or a
    /// hello names a protocol version the other side does not speak.
    ScreenUnsupported,
    /// Nobody on the host can be asked (S1 C5).
    ApprovalUnavailable,
    /// The host's capture failed during the session (`ERROR`).
    CaptureFailed,
    /// The host's encoder failed or none fits (`ERROR`, `HELLO_ERROR`).
    EncoderFailed,
    /// Sent only by hosts before R15c, which refused past 8 sessions (4 of
    /// one device); current hosts have no session count. Kept so viewers
    /// still read that refusal.
    SessionLimit,
}

impl ScreenErrorCode {
    pub const ALL: [Self; 14] = [
        Self::PermissionDenied,
        Self::RightNotGranted,
        Self::ApprovalDenied,
        Self::ApprovalCancelled,
        Self::NoDisplay,
        Self::IndicatorUnavailable,
        Self::ControlBusy,
        Self::ControlUnavailable,
        Self::HostShutdown,
        Self::ScreenUnsupported,
        Self::ApprovalUnavailable,
        Self::CaptureFailed,
        Self::EncoderFailed,
        Self::SessionLimit,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::PermissionDenied => "permission_denied",
            Self::RightNotGranted => "right_not_granted",
            Self::ApprovalDenied => "approval_denied",
            Self::ApprovalCancelled => "approval_cancelled",
            Self::NoDisplay => "no_display",
            Self::IndicatorUnavailable => "indicator_unavailable",
            Self::ControlBusy => "control_busy",
            Self::ControlUnavailable => "control_unavailable",
            Self::HostShutdown => "host_shutdown",
            Self::ScreenUnsupported => "screen_unsupported",
            Self::ApprovalUnavailable => "approval_unavailable",
            Self::CaptureFailed => "capture_failed",
            Self::EncoderFailed => "encoder_failed",
            Self::SessionLimit => "session_limit",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|code| code.key() == key)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenHelloError {
    pub code: String,
    /// The reason in words (plan 2026-11-18 „Netzrate“, e.g.
    /// `chosen_codec_unavailable: av1: …`); absent before, and an older
    /// viewer ignores it (no hello has `deny_unknown_fields`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl ScreenHelloError {
    pub fn new(code: ScreenErrorCode) -> Self {
        Self {
            code: code.key().into(),
            message: None,
        }
    }

    /// `new` with the reason in words.
    pub fn with_message(code: ScreenErrorCode, message: String) -> Self {
        Self {
            message: Some(message),
            ..Self::new(code)
        }
    }

    #[allow(dead_code)] // Staged: the desktop viewer of SCR-M7 shows known codes.
    pub fn known_code(&self) -> Option<ScreenErrorCode> {
        ScreenErrorCode::from_key(&self.code)
    }
}
