//! The session's control writes (review 2026-11-19, Codex 6/8, 7/6, round 8
//! Codex 8/5). Small, time-critical frames (`STATE`, `CONTROL_STATE`, the
//! pointer's position, `STATS`, errors, acknowledgements, family operation
//! results without a state) go at once and count against „Höchste
//! Datenrate“ as the control stream's rate, which the pictures' share loses.
//! Large frames (window lists, icons, family lists, the pointer's shape up
//! to 64 KiB, a family's state up to 1 MiB and an operation result carrying
//! one, Codex 9/9) go, under a user ceiling, piece by piece:
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
use crate::screen::core::tiles_rate::SharedBudget;

/// Large frames held at most before the metadata answers wait (`full`).
const HELD_FRAMES: usize = 32;
/// A piece of a large frame: a tenth of a second of the budget's rate,
/// within these bounds.
const PIECE_MIN: usize = 1024;
const PIECE_MAX: usize = crate::screen::core::control_parts::MAX_PART_BYTES;

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
    /// The inner frame's tag when it goes in `CONTROL_PART`s (`bytes` is
    /// then its payload); `None`: `bytes` is the whole frame.
    part_tag: Option<u8>,
    metadata: bool,
    shape: Option<u32>,
}

/// A frame that goes piece by piece under a user ceiling: the lists, icons
/// and pointer shapes, a family's state (up to 1 MiB, Codex 9/9) and an
/// operation result that carries one.
fn large(frame: &HostFrame) -> bool {
    match frame {
        HostFrame::Windows(_)
        | HostFrame::IconReply(_)
        | HostFrame::Families(_)
        | HostFrame::CursorShape(_)
        | HostFrame::FamilySnapshot(_) => true,
        HostFrame::FamilyOperation(result) => result.snapshot.is_some(),
        _ => false,
    }
}

/// A family's state or an operation result about it: they reach the viewer
/// in their order (Codex 9/9: a focus result checks the focus in the newest
/// state the viewer holds, `FamilyViewer::result`).
fn family(frame: &HostFrame) -> bool {
    matches!(
        frame,
        HostFrame::FamilySnapshot(_) | HostFrame::FamilyOperation(_)
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
    /// Small frames that came while a large frame was being written, or
    /// that wait for the budget's credit (round 12, `SmallLane`).
    behind: VecDeque<HostFrame>,
    /// The sound under the viewer's ceiling (`SoundCeiling`).
    pub(super) sound: SoundCeiling,
    /// The small frames' share under a ceiling (`SmallLane`).
    lane: SmallLane,
    /// The viewer takes large frames in parts (`control_parts.rs`).
    parts: bool,
    /// The session ends (`end_outbox`): a small frame waits for the budget
    /// in place instead of in `behind`.
    ending: bool,
}

/// The burst of the small frames' share: the window their rate is measured
/// over (`RateController::update_control`, one second). Round 12 (H2): with
/// 200 ms, a frame sent once a second (`STATS`, `PONG`) found a fifth of its
/// bytes in the share and took the rest from the pictures' budget for good.
const LANE_BURST_US: u64 = 1_000_000;

/// The small control frames under a user ceiling (Codex 11/7). The pictures
/// leave the control stream's measured rate free (`SharedBudget::reserved`,
/// at most the ceiling); small frames take that share as a token bucket, and
/// only what goes beyond it is booked in the pictures' budget before it is
/// sent. Every small frame is still measured as control bytes, so a lasting
/// rise moves into the reserved share with the next reading: the excess is
/// counted twice only while the share catches up, never in a steady state.
/// Round 12 (Codex 12/5, H5): a frame whose excess finds the budget in debt
/// waits in `Outbox::behind` until it has credit again (the session loop
/// goes on meanwhile, `Outbox::ready`), the newest position and statistics
/// replacing older ones; before, a position or statistics frame was dropped
/// (a pointer at rest kept its old place on the viewer) and every other
/// frame went after 20 ms on top of the debt, which let steady control
/// traffic exceed the ceiling.
#[derive(Default)]
pub(super) struct SmallLane {
    /// The last booking: its time and the share's balance in bits.
    booked: Option<(u64, i128)>,
}

impl SmallLane {
    /// The bytes of `bytes` at `now_us` beyond the share at `reserved_bps`,
    /// and the share's booking that takes them (`take`).
    fn excess_of(&self, now_us: u64, reserved_bps: u64, bytes: u64) -> (u64, (u64, i128)) {
        let rate = i128::from(reserved_bps);
        let cap = rate * i128::from(LANE_BURST_US) / 1_000_000;
        let balance = match self.booked {
            None => cap,
            Some((at, balance)) => {
                let earned = rate * i128::from(now_us.saturating_sub(at)) / 1_000_000;
                balance.saturating_add(earned).min(cap)
            }
        };
        let need = i128::from(bytes) * 8;
        let covered = need.min(balance.max(0));
        let excess = u64::try_from((need - covered + 7) / 8).unwrap_or(u64::MAX);
        (excess, (now_us, balance - covered))
    }

    /// The bytes beyond the share, without taking it (the frame may wait).
    pub(super) fn peek(&self, now_us: u64, reserved_bps: u64, bytes: u64) -> u64 {
        self.excess_of(now_us, reserved_bps, bytes).0
    }

    /// Takes `bytes` at `now_us` from the share at `reserved_bps`; the bytes
    /// beyond it.
    pub(super) fn excess(&mut self, now_us: u64, reserved_bps: u64, bytes: u64) -> u64 {
        let (excess, booked) = self.excess_of(now_us, reserved_bps, bytes);
        self.booked = Some(booked);
        excess
    }
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
                crate::screen::core::audio_codec::sound_needs_bps(ceiling),
                ceiling,
            )
        })
    }
}

impl Outbox {
    pub(super) fn new(budget: SharedBudget, ceiling: Option<u64>) -> Self {
        Self {
            parts: false,
            ending: false,
            lane: SmallLane::default(),
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
        self.writing.is_some() || !self.held.is_empty() || !self.behind.is_empty()
    }

    /// Small frames wait for the budget, not behind a large frame (`SmallLane`).
    fn small_due(&self) -> bool {
        self.writing.is_none() && !self.behind.is_empty()
    }

    /// Whether the metadata answers wait (no answer goes without credit).
    pub(super) fn full(&self) -> bool {
        self.held.len() >= HELD_FRAMES
    }

    /// Waits until the byte budget has credit for the next piece and no
    /// picture is being written (round 9 F4: a picture is booked when its
    /// write ends).
    pub(super) async fn ready(&self) {
        loop {
            let Some(rate) = self.budget.rate() else {
                return;
            };
            // A picture being written takes the small frames' excess into
            // its pace (`take_small_share`): they need not wait for it.
            if self.small_due() && self.budget.picture_writing() {
                return;
            }
            if self.budget.picture_writing() {
                // Woken when the picture ends; the next picture leaves this
                // frame one piece first (round 10: no polling, no starving).
                self.budget.wait_picture().await;
                continue;
            }
            let wait = self.budget.wait_us(now_us(), rate);
            if wait == 0 {
                return;
            }
            tokio::time::sleep(Duration::from_micros(wait)).await;
        }
    }

    /// Whether a large `frame` waits: under a user ceiling always (it goes
    /// piece by piece), without one behind held ones (Codex 7/6). A family
    /// frame waits behind a family state still held (Codex 9/9).
    fn holds(&self, frame: &HostFrame) -> bool {
        (large(frame) && (self.waiting() || self.budget.rate().is_some()))
            // Codex 10/15: behind any held family frame, a state or an
            // operation result, so results reach the viewer in their order.
            || (family(frame) && self.held.iter().any(|held| family(held.frame())))
    }

    /// The held frame a newer `frame` takes the place of: the window list
    /// and the family list are complete states without a request id, a
    /// pointer shape is the newest one (`hold`), an icon reply answers the
    /// same icon.
    /// A family state replaces only the newest held family frame when that
    /// is a state: an operation result after it is an order boundary (Codex
    /// 10/15).
    fn replaced_in_place(&self, frame: &HostFrame) -> Option<usize> {
        if matches!(frame, HostFrame::FamilySnapshot(_)) {
            return self
                .held
                .iter()
                .rposition(|held| family(held.frame()))
                .filter(|&index| matches!(self.held[index].frame(), HostFrame::FamilySnapshot(_)));
        }
        self.held
            .iter()
            .position(|held| match (held.frame(), frame) {
                (HostFrame::Windows(_), HostFrame::Windows(_))
                | (HostFrame::Families(_), HostFrame::Families(_))
                | (HostFrame::FamilySnapshot(_), HostFrame::FamilySnapshot(_))
                | (HostFrame::CursorShape(_), HostFrame::CursorShape(_)) => true,
                (HostFrame::IconReply(old), HostFrame::IconReply(new)) => {
                    old.request == new.request
                }
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
    /// In parts never above `PIECE_MAX` (round 11: after a switch to
    /// „Automatisch“ the whole rest was one part above its tag's limit).
    fn piece_len(&self, rest: usize, parts: bool) -> usize {
        let unbounded = if parts { rest.min(PIECE_MAX) } else { rest };
        self.budget.rate().map_or(unbounded, |rate| {
            usize::try_from(rate / 8 / 10)
                .unwrap_or(PIECE_MAX)
                .clamp(PIECE_MIN, PIECE_MAX)
                .min(rest)
        })
    }
}

impl Session<'_> {
    /// The viewer offered `control_parts` (`ScreenClientHello`).
    pub(in super::super) fn set_control_parts(&mut self, parts: bool) {
        self.outbox.parts = parts;
    }

    pub(in super::super) fn control_parts(&self) -> bool {
        self.outbox.parts
    }
}

#[path = "host_session_send_write.rs"]
mod write;

#[cfg(test)]
#[path = "host_session_send_tests.rs"]
mod tests;
