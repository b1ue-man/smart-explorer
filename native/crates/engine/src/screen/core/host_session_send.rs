//! The session's control writes (review 2026-11-19, Codex 6/8, 7/6, round 8
//! Codex 8/5). Small, time-critical frames (`STATE`, `CONTROL_STATE`, the
//! pointer's position, `STATS`, errors, acknowledgements, family snapshots
//! and operations) go at once and count against „Höchste Datenrate“ as the
//! control stream's rate, which the pictures' share loses. Large frames
//! that nothing waits for at once (window lists, icons, family lists, the
//! pointer's shape up to 64 KiB) go, under a user ceiling, piece by piece:
//! each piece takes the stream's byte budget the pictures use
//! (`tiles_rate::SharedBudget`) before it is written, so pictures and these
//! frames together keep the ceiling also over short spans. The session loop
//! never waits for them: one piece goes per turn of its loop when the
//! budget has credit (`Outbox::ready`).
//!
//! Order: a frame is written whole or not begun, never interleaved. Small
//! frames overtake large ones not yet begun (none of them refers to one: a
//! viewer draws a position only with its own shape, `CursorView::visible`,
//! and the position of a held shape waits with it); while a large frame is
//! being written, small frames wait behind it, the newest position and
//! statistics replacing older ones. A newer window list, family list,
//! pointer shape or reply for the same icon replaces its held one. A full
//! queue holds the metadata answers back (`full`, the session's metadata
//! branch). Without a user ceiling everything goes at once, as before, a
//! large frame only after the ones still held.
use std::collections::VecDeque;

use super::*;
use crate::screen::core::cursor_overlay::CursorPosition;
use crate::screen::core::tiles_rate::SharedBudget;

/// Large frames held at most before the metadata answers wait (`full`).
const HELD_FRAMES: usize = 32;
/// A piece of a large frame: a tenth of a second of the budget's rate,
/// within these bounds.
const PIECE_MIN: usize = 1024;
const PIECE_MAX: usize = 16 * 1024;

/// One held-back frame; a metadata reply goes through its authorized writer.
enum Held {
    Frame(HostFrame),
    Metadata(HostFrame),
}

impl Held {
    fn frame(&self) -> &HostFrame {
        match self {
            Self::Frame(frame) | Self::Metadata(frame) => frame,
        }
    }
}

/// The large frame being written piece by piece.
struct Writing {
    bytes: Vec<u8>,
    offset: usize,
    metadata: bool,
    shape: Option<u32>,
}

fn large(frame: &HostFrame) -> bool {
    matches!(
        frame,
        HostFrame::Windows(_)
            | HostFrame::IconReply(_)
            | HostFrame::Families(_)
            | HostFrame::CursorShape(_)
    )
}

pub(super) struct Outbox {
    budget: SharedBudget,
    held: VecDeque<Held>,
    /// The pointer shape still held, and the newest position that needs it.
    shape: Option<u32>,
    position: Option<HostFrame>,
    /// The newest pointer shape the feed gave (held, being written or
    /// sent): a viewer keeps only its newest shape and draws a position only
    /// with it (`CursorView::visible`), so a position of any other shape
    /// would hide the pointer and is dropped (round 7 P8; Gemini 8/16: a
    /// list of 8 replaced shapes let positions of older ones through).
    latest_shape: Option<u32>,
    writing: Option<Writing>,
    /// Small frames that came while a large frame was being written.
    behind: VecDeque<HostFrame>,
    /// The sound under the viewer's ceiling (`SoundCeiling`).
    pub(super) sound: SoundCeiling,
}

/// Whether the viewer's sound fits under its „Höchste Datenrate“ (round 8
/// Gemini 8/10): the capture link sends none that does not
/// (`audio_codec::sound_fits`), and the viewer hears why once.
#[derive(Default)]
pub(super) struct SoundCeiling {
    ceiling: Option<u64>,
    on: bool,
    noticed: bool,
}

impl SoundCeiling {
    /// A new ceiling or sound wish; the notice once while the wanted sound
    /// does not fit, again after it fitted in between.
    pub(super) fn update(
        &mut self,
        ceiling: Option<Option<u64>>,
        on: Option<bool>,
    ) -> Option<String> {
        if let Some(ceiling) = ceiling {
            self.ceiling = ceiling;
        }
        if let Some(on) = on {
            self.on = on;
        }
        let unfit = self.on && !crate::screen::core::audio_codec::sound_fits(self.ceiling);
        let fresh = unfit && !self.noticed;
        self.noticed = unfit;
        let ceiling = self.ceiling?;
        fresh.then(|| {
            crate::screen::core::codec_reason_text::sound_over_ceiling(
                crate::screen::core::audio_codec::sound_min_bps(),
                ceiling,
            )
        })
    }
}

impl Outbox {
    pub(super) fn new(budget: SharedBudget, ceiling: Option<u64>) -> Self {
        Self {
            sound: SoundCeiling {
                ceiling,
                ..SoundCeiling::default()
            },
            budget,
            held: VecDeque::new(),
            shape: None,
            position: None,
            latest_shape: None,
            writing: None,
            behind: VecDeque::new(),
        }
    }

    /// A new user ceiling: held frames wait for at most its rate until the
    /// pipeline sets the exact rest for them (`RateController::stream_admit`);
    /// without a ceiling they go at once.
    pub(super) fn follow_ceiling(&self, ceiling: Option<u64>) {
        let rate =
            ceiling.map(|ceiling| self.budget.rate().map_or(ceiling, |rate| rate.min(ceiling)));
        self.budget.set_rate(rate);
    }

    pub(super) fn waiting(&self) -> bool {
        self.writing.is_some() || !self.held.is_empty()
    }

    /// Whether the metadata answers wait (no answer goes without credit).
    pub(super) fn full(&self) -> bool {
        self.held.len() >= HELD_FRAMES
    }

    /// Waits until the byte budget has credit for the next piece.
    pub(super) async fn ready(&self) {
        let Some(rate) = self.budget.rate() else {
            return;
        };
        let wait = self.budget.wait_us(now_us(), rate);
        if wait > 0 {
            tokio::time::sleep(Duration::from_micros(wait)).await;
        }
    }

    /// Whether a large `frame` waits: under a user ceiling always (it goes
    /// piece by piece), without one behind held ones (Codex 7/6).
    fn holds(&self, frame: &HostFrame) -> bool {
        large(frame) && (self.waiting() || self.budget.rate().is_some())
    }

    /// The held frame a newer `frame` takes the place of: the window list
    /// and the family list are complete states without a request id, a
    /// pointer shape is the newest one (`hold`), an icon reply answers the
    /// same icon.
    fn replaced_in_place(&self, frame: &HostFrame) -> Option<usize> {
        self.held
            .iter()
            .position(|held| match (held.frame(), frame) {
                (HostFrame::Windows(_), HostFrame::Windows(_))
                | (HostFrame::Families(_), HostFrame::Families(_))
                | (HostFrame::CursorShape(_), HostFrame::CursorShape(_)) => true,
                (HostFrame::IconReply(old), HostFrame::IconReply(new)) => old.icon == new.icon,
                _ => false,
            })
    }

    /// The viewer's pointer was hidden (source change, overlay off; Codex
    /// 7/14): a held shape and its position belong to the state before and
    /// never go; the feed of the new state sends shape and position again
    /// (`CursorFeed::reset`), with ids that may repeat the old ones. A shape
    /// being written ends its frame; its position no longer follows.
    fn forget_pointer(&mut self) {
        self.held
            .retain(|held| !matches!(held.frame(), HostFrame::CursorShape(_)));
        self.shape = None;
        self.position = None;
        self.latest_shape = None;
    }

    fn hold(&mut self, held: Held) {
        if let HostFrame::CursorShape(shape) = held.frame() {
            // A newer shape replaces one still held, and goes first: its
            // positions wait for it, a window list does not.
            self.held
                .retain(|other| !matches!(other.frame(), HostFrame::CursorShape(_)));
            if self.shape.replace(shape.id).is_some() {
                // Its position goes neither now nor with the new shape.
                self.position = None;
            }
            self.held.push_front(held);
            return;
        }
        if let Some(index) = self.replaced_in_place(held.frame()) {
            self.held[index] = held;
            return;
        }
        self.held.push_back(held);
    }

    /// A small frame behind the large one being written; a newer position
    /// or statistics frame replaces the one waiting.
    fn queue_behind(&mut self, frame: HostFrame) {
        let same = self.behind.iter().position(|old| {
            matches!(
                (old, &frame),
                (HostFrame::CursorPosition(_), HostFrame::CursorPosition(_))
                    | (HostFrame::Stats(_), HostFrame::Stats(_))
            )
        });
        match same {
            Some(index) => self.behind[index] = frame,
            None => self.behind.push_back(frame),
        }
    }

    /// The bytes of the next piece at the budget's rate (all without one).
    fn piece_len(&self, rest: usize) -> usize {
        self.budget.rate().map_or(rest, |rate| {
            usize::try_from(rate / 8 / 10)
                .unwrap_or(PIECE_MAX)
                .clamp(PIECE_MIN, PIECE_MAX)
                .min(rest)
        })
    }
}

#[path = "host_session_send_write.rs"]
mod write;
