//! The session's control writes (review 2026-11-19, Codex 6/8). Small,
//! time-critical frames (`STATE`, `CONTROL_STATE`, the pointer's position,
//! `STATS`, errors, acknowledgements, family snapshots and operations) go
//! at once and count against „Höchste Datenrate“ as the control stream's
//! rate, which the pictures' share loses. Large frames that nothing waits
//! for at once (window lists, icons, family lists, the pointer's shape up
//! to 64 KiB) wait, under a user ceiling, for the stream's byte budget the
//! pictures use (`tiles_rate::SharedBudget`): one goes when the budget has
//! credit and books its bytes there, so pictures and these frames together
//! keep the ceiling over time. The session loop never waits for them; they
//! leave in their order (a pointer shape first), a position of a shape still held waits
//! with it (the newest one), and a session that ends drops them. Without a
//! user ceiling everything goes at once, as before, a large frame only after
//! the ones still held.
use std::collections::VecDeque;

use super::*;
use crate::screen::core::cursor_overlay::CursorPosition;
use crate::screen::core::tiles_rate::SharedBudget;

/// Frames held back at most. A newer window list, family list, pointer
/// shape or reply for the same icon replaces its held one in place; any
/// other frame beyond this first sends the oldest held one, so the order
/// stays and its bytes are the budget's debt (Codex 7/6: before, the next
/// frame went at once and overtook older ones).
const HELD_FRAMES: usize = 32;
/// Replaced shapes remembered (their ids only rise; a few cover a burst).
const REPLACED_SHAPES: usize = 8;

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

pub(super) struct Outbox {
    budget: SharedBudget,
    held: VecDeque<Held>,
    /// The pointer shape still held, and the newest position that needs it.
    shape: Option<u32>,
    position: Option<HostFrame>,
    /// Shapes replaced before they went (round 7 P8): their positions are
    /// dropped, since a viewer draws a position only with its own shape
    /// (`CursorView::visible`) and would hide the pointer.
    replaced: Vec<u32>,
}

impl Outbox {
    pub(super) fn new(budget: SharedBudget) -> Self {
        Self {
            budget,
            held: VecDeque::new(),
            shape: None,
            position: None,
            replaced: Vec::new(),
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
        !self.held.is_empty()
    }

    /// Waits until the byte budget has credit for the next held frame.
    pub(super) async fn ready(&self) {
        let Some(rate) = self.budget.rate() else {
            return;
        };
        let wait = self.budget.wait_us(now_us(), rate);
        if wait > 0 {
            tokio::time::sleep(Duration::from_micros(wait)).await;
        }
    }

    /// Whether `frame` waits: a large frame under a user ceiling while one
    /// waits already or the budget has no credit.
    fn holds(&self, frame: &HostFrame) -> bool {
        let large = matches!(
            frame,
            HostFrame::Windows(_)
                | HostFrame::IconReply(_)
                | HostFrame::Families(_)
                | HostFrame::CursorShape(_)
        );
        if !large {
            return false;
        }
        // Behind held ones also when the ceiling was just lifted: they leave
        // first, in their order (Codex 7/6).
        if self.waiting() {
            return true;
        }
        self.budget
            .rate()
            .is_some_and(|rate| !self.budget.admits(now_us(), rate))
    }

    /// Whether holding `frame` needs room first: the queue is full and
    /// `frame` replaces none of the held frames.
    fn full_for(&self, frame: &HostFrame) -> bool {
        self.held.len() >= HELD_FRAMES && self.replaced_in_place(frame).is_none()
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
    /// (`CursorFeed::reset`), with ids that may repeat the old ones.
    fn forget_pointer(&mut self) {
        self.held
            .retain(|held| !matches!(held.frame(), HostFrame::CursorShape(_)));
        self.shape = None;
        self.position = None;
        self.replaced.clear();
    }

    fn hold(&mut self, held: Held) {
        if let HostFrame::CursorShape(shape) = held.frame() {
            // A newer shape replaces one still held, and goes first: its
            // positions wait for it, a window list does not.
            self.held
                .retain(|other| !matches!(other.frame(), HostFrame::CursorShape(_)));
            if let Some(old) = self.shape.replace(shape.id) {
                if self.replaced.len() == REPLACED_SHAPES {
                    self.replaced.remove(0);
                }
                self.replaced.push(old);
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
}

impl Session<'_> {
    pub(super) async fn frame(&mut self, frame: HostFrame) -> io::Result<()> {
        if let HostFrame::CursorPosition(position) = &frame {
            if *position == CursorPosition::hidden() {
                self.outbox.forget_pointer();
            } else {
                if self.outbox.replaced.contains(&position.shape_id) {
                    return Ok(());
                }
                if self.outbox.shape == Some(position.shape_id) {
                    self.outbox.position = Some(frame);
                    return Ok(());
                }
                // A position of another shape is newer than a held one.
                self.outbox.position = None;
            }
        }
        self.hold_or_write(Held::Frame(frame)).await
    }

    /// Holds `held` while the budget has no credit for it, else writes it.
    async fn hold_or_write(&mut self, held: Held) -> io::Result<()> {
        if !self.outbox.holds(held.frame()) {
            return self.write(held).await;
        }
        if self.outbox.full_for(held.frame()) {
            self.flush_outbox().await?;
        }
        self.outbox.hold(held);
        Ok(())
    }

    /// The answer of a finished metadata job, as the synchronous path gave
    /// it: a failed window list is an `ERROR` and the session goes on, a
    /// failed icon lookup omits the icon, a lost right or a failed family
    /// list ends the session.
    pub(super) async fn metadata_reply(
        &mut self,
        fallback: Fallback,
        result: io::Result<HostFrame>,
    ) -> io::Result<()> {
        let response = match (fallback, result) {
            (Fallback::Windows, Err(error)) => {
                return self
                    .error(ScreenErrorCode::CaptureFailed, error.to_string())
                    .await
            }
            (fallback, result) => fallback.settle(result)?,
        };
        self.hold_or_write(Held::Metadata(response)).await
    }

    /// The next held frame, now that the budget has credit; a position that
    /// waited for its shape follows it.
    pub(super) async fn flush_outbox(&mut self) -> io::Result<()> {
        let Some(held) = self.outbox.held.pop_front() else {
            return Ok(());
        };
        let shape = match held.frame() {
            HostFrame::CursorShape(shape) => Some(shape.id),
            _ => None,
        };
        self.write(held).await?;
        if shape.is_some() && shape == self.outbox.shape {
            self.outbox.shape = None;
            if let Some(position) = self.outbox.position.take() {
                self.write(Held::Frame(position)).await?;
            }
        }
        Ok(())
    }

    /// Writes `held` and books it: a large frame under a user ceiling in the
    /// pictures' byte budget, every other one as the control stream's rate.
    async fn write(&mut self, held: Held) -> io::Result<()> {
        let large = matches!(
            held.frame(),
            HostFrame::Windows(_)
                | HostFrame::IconReply(_)
                | HostFrame::Families(_)
                | HostFrame::CursorShape(_)
        );
        let sent = match &held {
            Held::Frame(frame) => {
                let peer = self.connection.remote_id().to_string();
                control_sender::send_counted(peer, &mut self.send, frame).await?
            }
            Held::Metadata(frame) => {
                host_metadata::send(
                    self.context,
                    self.principal,
                    self.mode,
                    self.connection,
                    &mut self.send,
                    frame,
                    self.handle.stop_signal(),
                )
                .await?;
                control_sender::wire_bytes(frame)
            }
        };
        match self.outbox.budget.rate().filter(|_| large) {
            Some(rate) => self.outbox.budget.on_held_sent(now_us(), sent, rate),
            // The control stream counts against „Höchste Datenrate“.
            None => self.pipeline.book_control_bytes(sent),
        }
        Ok(())
    }
}
