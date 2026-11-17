//! Codecs whose decoding gave no picture on this device (fix of 2026-11-17:
//! a Windows viewer stayed black while the Windows host showed its
//! indicator).
//!
//! The negotiation prefers AV1, then HEVC, then H.264 when both sides offer
//! hardware (`negotiate.rs` `rank`), and an offered decoder is never checked
//! for turning a stream into a picture (on Windows a Media Foundation
//! transform that enumerates is offered). A decoder that opens but fails at
//! every key frame, or delivers nothing, left the window black with no
//! message and no way back to another codec. `DecoderGuard` watches one
//! session's codec; when that codec brings no picture, the viewer marks it
//! here, its offers leave every later hello and `SET_DECODERS`
//! (`usable_offers`), and the session negotiates again (desktop viewer:
//! `screen_decode_guard.rs`). The tiles are never given up: they are every
//! pairing's last fallback. The mark lasts for the process; the next viewer
//! start tries the codec again.
//!
//! Plan 2026-11-17 „Windows-Bild“ P3: a picture that comes out of a later
//! decoder call than its own (`lag` > 0) counts as a failure, not as shown:
//! such a decoder holds the newest picture back, on a still screen for
//! good. `LAG_LIMIT` late pictures in a row give the codec up even after it
//! showed pictures. Decoders that failed their run-time check are left out
//! of the offers by the register (`CodecRegistry::decoder_offers`) before
//! `usable_offers` sees them.
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use super::negotiate::{CodecKind, CodecOffer};

/// Failed key frames of one codec without any picture before it is given up.
pub const FAILURE_LIMIT: u32 = 3;
/// Late pictures in a row (`DecoderGuard::on_picture` with a lag) that
/// give a codec up even after it showed pictures.
pub const LAG_LIMIT: u32 = FAILURE_LIMIT;
/// Time from the codec's first record without any picture before it is
/// given up: the host sends a key frame at once and answers a key frame
/// request within one capture interval, and a hardware decoder that opens
/// late (first use of the driver) is ready within a second (assumption, not
/// measured).
pub const SILENCE_LIMIT: Duration = Duration::from_secs(5);

static UNUSABLE: AtomicU8 = AtomicU8::new(0);

fn bit(codec: CodecKind) -> u8 {
    match codec {
        CodecKind::H264 => 1,
        CodecKind::Hevc => 2,
        CodecKind::Av1 => 4,
        CodecKind::Tiles => 0,
    }
}

/// This process does not offer `codec` again.
pub fn mark_unusable(codec: CodecKind) {
    UNUSABLE.fetch_or(bit(codec), Ordering::AcqRel);
}

/// Whether this process still offers `codec`.
pub fn usable(codec: CodecKind) -> bool {
    UNUSABLE.load(Ordering::Acquire) & bit(codec) == 0
}

/// `offers` without the codecs this process gave up.
pub fn usable_offers(offers: &[CodecOffer]) -> Vec<CodecOffer> {
    offers
        .iter()
        .filter(|offer| usable(offer.codec))
        .copied()
        .collect()
}

/// One session's watch over the codec the host chose.
#[derive(Debug, Default)]
pub struct DecoderGuard {
    codec: Option<CodecKind>,
    since: Option<Instant>,
    failures: u32,
    shown: bool,
    given_up: bool,
    /// Late pictures in a row.
    lagged: u32,
}

impl DecoderGuard {
    fn track(&mut self, codec: CodecKind, now: Instant) {
        if self.codec != Some(codec) {
            *self = Self {
                codec: Some(codec),
                since: Some(now),
                ..Self::default()
            };
        }
    }

    /// A record of `codec` arrived; the codec to give up, once.
    pub fn on_record(&mut self, codec: CodecKind, now: Instant) -> Option<CodecKind> {
        self.track(codec, now);
        self.verdict(now)
    }

    /// Opening or decoding a picture of `codec` failed.
    pub fn on_failure(&mut self, codec: CodecKind, now: Instant) -> Option<CodecKind> {
        self.track(codec, now);
        self.failures = self.failures.saturating_add(1);
        self.verdict(now)
    }

    /// A picture of `codec` was shown; `lag` pictures submitted after it
    /// are still in the decoder. Without lag the codec works in this
    /// session; a late picture counts as a failure. The codec to give up,
    /// once.
    pub fn on_picture(&mut self, codec: CodecKind, now: Instant, lag: usize) -> Option<CodecKind> {
        self.track(codec, now);
        if lag == 0 {
            self.shown = true;
            self.lagged = 0;
        } else {
            self.failures = self.failures.saturating_add(1);
            self.lagged = self.lagged.saturating_add(1);
        }
        self.verdict(now)
    }

    /// Any other event of the session (statistics come once a second).
    pub fn on_tick(&mut self, now: Instant) -> Option<CodecKind> {
        self.verdict(now)
    }

    fn verdict(&mut self, now: Instant) -> Option<CodecKind> {
        let codec = self.codec?;
        let lagging = self.lagged >= LAG_LIMIT;
        if (self.shown && !lagging) || self.given_up || codec == CodecKind::Tiles {
            return None;
        }
        let silent = self
            .since
            .is_some_and(|since| now.saturating_duration_since(since) >= SILENCE_LIMIT);
        if lagging || self.failures >= FAILURE_LIMIT || silent {
            self.given_up = true;
            return Some(codec);
        }
        None
    }
}

#[cfg(test)]
#[path = "bildkette_decoder_guard_task_tests.rs"]
mod bildkette_decoder_guard_task_tests;
