//! The relayed session's codec and rate changes (plan 2026-11-14
//! „Bildkette“ BK6, P1): what became of „Höchste Datenrate“ (moved here from
//! `ipc_protocol_screen.rs`, which stays below 500 lines), its relay event
//! for a change during the session, and the `SET_DECODERS` frame a client
//! builds from its request.
//!
//! `RELAY_RATE_CEILING` (worker to client, JSON `RateCeilingState`): the
//! worker answers every `RATE_CEILING` the client relays with what became of
//! it, `applied` for a host with `rate_ceiling_v1`, `unsupported_by_host`
//! otherwise (the frame then never leaves, `viewer_client_tasks.rs`),
//! `automatic` for 0. Before, only the opening said so (`RelayHello`), and a
//! change during the session showed „gesendet“ at best.
use serde::{Deserialize, Serialize};

use crate::screen::core::set_decoders::SetDecoders;

/// Worker to client: what became of a `RATE_CEILING` during the session.
pub const RELAY_RATE_CEILING: u8 = 7;
/// Worker to client: one sound packet of the host (plan 2026-11-17 P9,
/// `audio_wire.rs`); it arrives only after the client switched the sound
/// on, so a client without sound never sees it.
pub const RELAY_AUDIO: u8 = 8;

/// What became of „Höchste Datenrate“ in a relayed session (B-09, BK6):
/// the viewer says when the host cannot take it instead of doing nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateCeilingState {
    /// No ceiling asked for.
    #[default]
    Automatic,
    /// `RATE_CEILING` went to the host.
    Applied,
    /// The host lacks `rate_ceiling_v1` (v0.5.196 or older): its own
    /// regulation alone sets the rate.
    UnsupportedByHost,
}

impl RateCeilingState {
    pub fn of(requested: Option<u64>, host_takes_it: bool) -> Self {
        match (requested, host_takes_it) {
            (None, _) => Self::Automatic,
            (Some(_), true) => Self::Applied,
            (Some(_), false) => Self::UnsupportedByHost,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::Applied => "applied",
            Self::UnsupportedByHost => "unsupported_by_host",
        }
    }
}

impl super::StreamRequest {
    /// `SET_DECODERS` for this request: the offers under its „Codec“ and
    /// „Dekodierer“ (`chosen_decoders`, as the hello names them) and its
    /// continuation announcement.
    pub fn decoders_frame(&self) -> SetDecoders {
        SetDecoders {
            decoders: self.chosen_decoders(),
            gop_continue: self.gop_continue,
            gop_continue_decoders: self.gop_continue_decoders.clone(),
        }
    }
}
