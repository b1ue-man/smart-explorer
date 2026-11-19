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
//! user ceiling everything goes at once, as before.
use std::collections::VecDeque;

use super::*;
use crate::screen::core::tiles_rate::SharedBudget;

/// Frames held back at most; one more goes at once (a viewer that asks
/// faster than the ceiling carries is not answered never).
const HELD_FRAMES: usize = 32;

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
}

impl Outbox {
    pub(super) fn new(budget: SharedBudget) -> Self {
        Self {
            budget,
            held: VecDeque::new(),
            shape: None,
            position: None,
        }
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
        let Some(rate) = self.budget.rate().filter(|_| large) else {
            return false;
        };
        self.held.len() < HELD_FRAMES && (self.waiting() || !self.budget.admits(now_us(), rate))
    }

    fn hold(&mut self, held: Held) {
        if let HostFrame::CursorShape(shape) = held.frame() {
            // A newer shape replaces one still held, and goes first: its
            // positions wait for it, a window list does not.
            self.held
                .retain(|other| !matches!(other.frame(), HostFrame::CursorShape(_)));
            self.shape = Some(shape.id);
            self.held.push_front(held);
            return;
        }
        self.held.push_back(held);
    }
}

impl Session<'_> {
    pub(super) async fn frame(&mut self, frame: HostFrame) -> io::Result<()> {
        if let HostFrame::CursorPosition(position) = &frame {
            if self.outbox.shape == Some(position.shape_id) {
                self.outbox.position = Some(frame);
                return Ok(());
            }
        }
        if self.outbox.holds(&frame) {
            self.outbox.hold(Held::Frame(frame));
            return Ok(());
        }
        self.write(Held::Frame(frame)).await
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
        if self.outbox.holds(&response) {
            self.outbox.hold(Held::Metadata(response));
            return Ok(());
        }
        self.write(Held::Metadata(response)).await
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
