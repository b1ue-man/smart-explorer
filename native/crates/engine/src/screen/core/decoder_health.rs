//! Codecs whose decoding gave no picture in a viewer session (fix of
//! 2026-11-17: a Windows viewer stayed black while the Windows host showed
//! its indicator).
//!
//! The negotiation prefers AV1, then HEVC, then H.264 when both sides offer
//! hardware (`negotiate.rs` `rank`), and an offered decoder is never checked
//! for turning a stream into a picture (on Windows a Media Foundation
//! transform that enumerates is offered). A decoder that opens but fails at
//! every key frame, or delivers nothing, left the window black with no
//! message and no way back to another codec. `DecoderGuard` watches one
//! session's codec; when that codec brings no picture, the session notes it
//! in its own `GivenUp` and negotiates again without it (desktop viewer:
//! `screen_decode_guard.rs`). The tiles are never given up: they are every
//! pairing's last fallback.
//!
//! Fix of 2026-11-18 („Kacheln immer“, v0.5.200): a give-up was a mark for
//! the whole process; the main window's viewer runs for days, so one bad
//! session (or a still screen, or one late picture) took a codec out of
//! every later session's offers until all three were gone and only the
//! tiles were offered. A give-up now lasts for its session only (`GivenUp`,
//! kept by the session), and the decoder stages a library gave up last
//! until the next viewer session opens (`begin_session`, `for_session`).
//!
//! A picture that comes out of a later decoder call than its own (`lag` >
//! 0) is a picture: the decoder decodes, so neither the failure count nor
//! the silence rule applies to it (fix of 2026-11-18; before, a late
//! picture counted as a failed key frame). `LAG_LIMIT` late pictures in a
//! row still give the codec up for the session: such a decoder holds the
//! newest picture back, on a still screen for good (plan 2026-11-17
//! „Windows-Bild“ P3). Decoders that failed their run-time check are left
//! out of the offers by the register (`CodecRegistry::decoder_offers`)
//! while another decoder of their codec remains.
//!
//! User rule of 2026-11-18 („jede Einstellung wird genau so verwendet“):
//! only „Codec: Automatisch“ gives a codec up and moves on. A named codec
//! (H.264, HEVC, AV1, „Scharf (GPU)“) is never given up; the viewer shows
//! the error with its reason instead (`DecoderGuard::reason`).
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use super::negotiate::{CodecKind, CodecOffer};

/// Failed decodes of one codec in a row, without a picture between them,
/// before it is given up.
pub const FAILURE_LIMIT: u32 = 3;
/// Late pictures in a row (`DecoderGuard::on_picture` with a lag) that
/// give a codec up even after it showed pictures.
pub const LAG_LIMIT: u32 = FAILURE_LIMIT;
/// Time from the first record handed to the codec's decoder without any
/// picture before it is given up: the host sends a key frame at once and
/// answers a key frame request within one capture interval, and a hardware
/// decoder that opens late (first use of the driver) is ready within a
/// second (assumption, not measured).
pub const SILENCE_LIMIT: Duration = Duration::from_secs(5);

/// Counts the viewer sessions this process opened (`begin_session`).
static SESSION_EPOCH: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// The viewer session whose decoders this thread opens (`enter_session`).
    static THREAD_SESSION: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

/// A viewer session opens: what a decoder library gave up for earlier
/// sessions (`for_session`) is tried again. Its number, for `enter_session`.
pub fn begin_session() -> u64 {
    SESSION_EPOCH.fetch_add(1, Ordering::AcqRel) + 1
}

/// The calling thread decodes for the session `epoch` (its decode thread):
/// what a decoder library gives up or learns there belongs to that session
/// alone (fix 2026-11-19: two viewers of one process, a full view and the
/// tiles, shared one epoch, so each opening wiped the other's state and a
/// failure of one counted for the other).
pub fn enter_session(epoch: u64) {
    THREAD_SESSION.with(|session| session.set(Some(epoch)));
}

/// The viewer session of this thread (`enter_session`), else the newest
/// one of this process (0 before the first, and in processes that open
/// none).
pub fn session_epoch() -> u64 {
    THREAD_SESSION
        .with(std::cell::Cell::get)
        .unwrap_or_else(|| SESSION_EPOCH.load(Ordering::Acquire))
}

/// What a decoder library keeps per viewer session, for the newest
/// `SESSION_SLOTS` sessions (several viewers run at once in one process).
pub struct SessionSlots<T>(Vec<(u64, T)>);

/// Sessions whose slots are kept (a full view and a grid of tiles).
const SESSION_SLOTS: usize = 32;

impl<T: Default> Default for SessionSlots<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Default> SessionSlots<T> {
    pub const fn new() -> Self {
        Self(Vec::new())
    }

    /// The slot of the session `epoch`, made empty when it is new; the
    /// oldest session's slot goes when there are too many.
    pub fn get(&mut self, epoch: u64) -> &mut T {
        let index = match self.0.iter().position(|(of, _)| *of == epoch) {
            Some(index) => index,
            None => {
                if self.0.len() >= SESSION_SLOTS {
                    if let Some(oldest) = (0..self.0.len()).min_by_key(|i| self.0[*i].0) {
                        self.0.swap_remove(oldest);
                    }
                }
                self.0.push((epoch, T::default()));
                self.0.len() - 1
            }
        };
        &mut self.0[index].1
    }
}

/// `slot` as kept for the session `epoch`: what was kept for an earlier
/// session is gone (a decoder library's given-up stages, fix of 2026-11-18).
pub fn for_session<T: Default>(slot: &mut (u64, T), epoch: u64) -> &mut T {
    if slot.0 != epoch {
        *slot = (epoch, T::default());
    }
    &mut slot.1
}

fn bit(codec: CodecKind) -> u8 {
    match codec {
        CodecKind::H264 => 1,
        CodecKind::Hevc => 2,
        CodecKind::Av1 => 4,
        CodecKind::Tiles => 0,
    }
}

/// The video codecs one viewer session gave up; never the tiles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GivenUp(u8);

impl GivenUp {
    /// Notes `codec`; `true` when it was not given up before.
    pub fn insert(&mut self, codec: CodecKind) -> bool {
        let new = bit(codec) != 0 && !self.contains(codec);
        self.0 |= bit(codec);
        new
    }

    pub fn contains(self, codec: CodecKind) -> bool {
        self.0 & bit(codec) != 0
    }

    /// `offers` without the codecs given up.
    pub fn filter(self, offers: &[CodecOffer]) -> Vec<CodecOffer> {
        offers
            .iter()
            .filter(|offer| !self.contains(offer.codec))
            .copied()
            .collect()
    }

    /// The codecs given up, for a log line.
    pub fn keys(self) -> Vec<&'static str> {
        [CodecKind::H264, CodecKind::Hevc, CodecKind::Av1]
            .into_iter()
            .filter(|codec| self.contains(*codec))
            .map(CodecKind::key)
            .collect()
    }
}

/// One session's watch over the codec the host chose.
#[derive(Debug, Default)]
pub struct DecoderGuard {
    codec: Option<CodecKind>,
    since: Option<Instant>,
    failures: u32,
    /// A picture of the codec came, in time or late: its decoder decodes.
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

    /// A record of `codec` went to its decoder; the codec to give up, once.
    /// Records skipped while the session waits for a key frame are not
    /// handed here: the silence counts from the first record decoded.
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
    /// are still in the decoder. Any picture shows that the decoder decodes
    /// (no silence) and ends a run of failures; `LAG_LIMIT` late ones in a
    /// row give the codec up. The codec to give up, once.
    pub fn on_picture(&mut self, codec: CodecKind, now: Instant, lag: usize) -> Option<CodecKind> {
        self.track(codec, now);
        self.shown = true;
        self.failures = 0;
        if lag == 0 {
            self.lagged = 0;
        } else {
            self.lagged = self.lagged.saturating_add(1);
        }
        self.verdict(now)
    }

    /// Any other event of the session (statistics come once a second).
    pub fn on_tick(&mut self, now: Instant) -> Option<CodecKind> {
        self.verdict(now)
    }

    /// Why the watched codec counts as without picture, in German for the
    /// viewer's message and log: late pictures, failed key frames or silence.
    pub fn reason(&self) -> String {
        if self.lagged >= LAG_LIMIT {
            format!(
                "{} Bilder in Folge kamen verspätet aus dem Dekodierer",
                self.lagged
            )
        } else if self.failures >= FAILURE_LIMIT {
            format!(
                "{} Bilder in Folge ließen sich nicht dekodieren",
                self.failures
            )
        } else {
            format!(
                "{} s lang kein Bild aus dem Dekodierer",
                SILENCE_LIMIT.as_secs()
            )
        }
    }

    fn verdict(&mut self, now: Instant) -> Option<CodecKind> {
        let codec = self.codec?;
        if self.given_up || codec == CodecKind::Tiles {
            return None;
        }
        let lagging = self.lagged >= LAG_LIMIT;
        let silent = self
            .since
            .is_some_and(|since| now.saturating_duration_since(since) >= SILENCE_LIMIT);
        // Failures count in a row since the last picture (fix 2026-11-19:
        // after the first picture a decoder that failed for good was never
        // given up); silence only before the first picture (a still screen).
        let failing = self.failures >= FAILURE_LIMIT;
        let without_picture = failing || (!self.shown && silent);
        if lagging || without_picture {
            self.given_up = true;
            return Some(codec);
        }
        None
    }
}

#[cfg(test)]
#[path = "bildkette_decoder_guard_task_tests.rs"]
mod bildkette_decoder_guard_task_tests;

#[cfg(test)]
#[path = "kachelfix_decoder_health_task_tests.rs"]
mod kachelfix_decoder_health_task_tests;
