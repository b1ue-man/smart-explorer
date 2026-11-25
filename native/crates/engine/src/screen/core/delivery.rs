//! „Übertragung“ of the picture stream (user 2026-11-25, after Shadow's
//! „Prefer speed (UDP)“ / „Prefer reliability (TCP)“, support.shadow.tech
//! „Streaming Preferences Explained (TCP & UDP)“, read 2026-11-25):
//!
//! - `Realtime` („UDP“, the default as Shadow's): pictures go as QUIC
//!   DATAGRAMs where both sides announce `video_datagrams_v1`; a picture
//!   the line could not deliver in time is given up; the host resets its
//!   stream after the stale time of B-35 (`pacing::stale_after_us`), the
//!   viewer skips it and the host goes on from the last picture the viewer
//!   confirmed or from a key frame (B-36), as Moonlight and Sunshine drop an
//!   unrecoverable frame and invalidate its references.
//! - `Reliable` („TCP“): a reliable ordered stream (QUIC; over the relay
//!   TCP itself): every picture arrives; a slower
//!   line delays the picture instead of losing it and the stream catches
//!   up („tcp holt auf auf Kosten von Echtzeit“).
//!
//! Neither mode lowers the data rate or drops a picture before encoding for
//! the line's sake (user 2026-11-25: „ich WILL NICHT DASS DU DROSSELST“).
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    /// Late pictures are given up (UDP-like).
    #[default]
    Realtime,
    /// Every picture arrives (TCP-like).
    Reliable,
}

impl Delivery {
    pub const ALL: [Self; 2] = [Self::Realtime, Self::Reliable];

    /// The key on the wire, in settings and on the command line.
    pub fn key(self) -> &'static str {
        match self {
            Self::Realtime => "realtime",
            Self::Reliable => "reliable",
        }
    }

    /// The German name in the viewers.
    pub fn label(self) -> &'static str {
        match self {
            Self::Realtime => "UDP",
            Self::Reliable => "TCP",
        }
    }

    /// What the mode does, for hover texts and help.
    pub fn hint(self) -> &'static str {
        match self {
            Self::Realtime => {
                "Bilder als UDP-Datagramme ohne Wiederholung, mit Fehlerkorrektur; ein unvollständiges Bild wird verworfen, das Bild bleibt aktuell."
            }
            Self::Reliable => {
                "Zuverlässiger, geordneter Strom mit Wiederholung: jedes Bild kommt an; bei langsamer Leitung wird es später, nicht verworfen."
            }
        }
    }

    /// The key, or `udp`/`tcp` (user 2026-11-25: „NUTZ das“).
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "udp" => Some(Self::Realtime),
            "tcp" => Some(Self::Reliable),
            other => Self::ALL.into_iter().find(|mode| mode.key() == other),
        }
    }
}

#[cfg(test)]
#[path = "delivery_tests.rs"]
mod tests;
