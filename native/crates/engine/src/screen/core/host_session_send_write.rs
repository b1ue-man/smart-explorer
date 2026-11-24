//! The session's side of the outbox (`host_session_send.rs`): frames in,
//! pieces out.
use super::super::*;
use super::{budget_takes_excess, family, large, Held, Writing};
use crate::screen::core::cursor_overlay::CursorPosition;

impl Session<'_> {
    pub(in super::super) async fn frame(&mut self, frame: HostFrame) -> io::Result<()> {
        if let HostFrame::CursorShape(shape) = &frame {
            self.outbox.latest_shape = Some(shape.id);
        }
        if let HostFrame::CursorPosition(position) = &frame {
            if *position == CursorPosition::hidden() {
                self.outbox.forget_pointer();
            } else {
                if self
                    .outbox
                    .latest_shape
                    .is_some_and(|latest| latest != position.shape_id)
                {
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

    /// Holds a large frame (`Outbox::holds`), queues a small one behind a
    /// large frame being written, writes every other at once.
    async fn hold_or_write(&mut self, held: Held) -> io::Result<()> {
        if self.outbox.holds(held.frame()) {
            self.outbox.hold(held);
            return Ok(());
        }
        // Round 12: small frames waiting for the budget keep their order.
        if !self.outbox.behind.is_empty() && self.outbox.writing.is_none() {
            if let Held::Frame(frame) = held {
                if large(&frame) {
                    self.outbox.hold(Held::Frame(frame));
                } else {
                    self.outbox.queue_behind(frame);
                }
                return Ok(());
            }
        }
        if self.outbox.writing.is_some() {
            match held {
                // A viewer that takes parts gets small frames between them
                // (round 10, Codex 10/10); a family's frames keep their order,
                // and so do frames behind one that waits for the budget.
                Held::Frame(frame)
                    if !large(&frame)
                        && self.outbox.parts
                        && !family(&frame)
                        && self.outbox.behind.is_empty() =>
                {
                    return self.write(Held::Frame(frame)).await;
                }
                Held::Frame(frame) if !large(&frame) => {
                    self.outbox.queue_behind(frame);
                    return Ok(());
                }
                held => {
                    self.outbox.hold(held);
                    return Ok(());
                }
            }
        }
        self.write(held).await
    }

    /// The answer of a finished metadata job, as the synchronous path gave
    /// it: a failed window list is an `ERROR` and the session goes on, a
    /// failed icon lookup omits the icon, a lost right or a failed family
    /// list ends the session.
    pub(in super::super) async fn metadata_reply(
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

    /// The next piece of the large frame being written, or of the next held
    /// one, now that the budget has credit; the piece's bytes are taken
    /// from the budget before it goes (round 8, Codex 8/5). A frame done,
    /// the position that waited for its shape and the small frames behind
    /// it follow.
    pub(in super::super) async fn flush_outbox(&mut self) -> io::Result<()> {
        // Small frames that wait for the budget go before held large ones,
        // and (round 13, H1) before the next part of the one being written.
        if self.outbox.small_due() {
            self.send_behind().await?;
            if self.outbox.writing.is_none() {
                return Ok(());
            }
            // The part goes on the credit the small frames left, else it
            // waits for the budget again (`Outbox::ready`).
            if let Some(rate) = self.outbox.budget.rate() {
                if !self.outbox.budget.admits(now_us(), rate) {
                    return Ok(());
                }
            }
        }
        let mut writing = match self.outbox.writing.take() {
            Some(writing) => writing,
            None => {
                let Some(held) = self.outbox.held.pop_front() else {
                    return Ok(());
                };
                let (frame, metadata) = match held {
                    Held::Frame(frame) => (frame, false),
                    Held::Metadata(frame) => (frame, true),
                };
                let bytes = control_sender::encode_wire(&frame)?;
                // In parts: the inner payload, each piece framed on its own.
                let (bytes, part_tag) = if self.outbox.parts {
                    (bytes[5..].to_vec(), Some(bytes[0]))
                } else {
                    (bytes, None)
                };
                Writing {
                    bytes,
                    offset: 0,
                    part_tag,
                    metadata,
                    shape: match &frame {
                        HostFrame::CursorShape(shape) => Some(shape.id),
                        _ => None,
                    },
                }
            }
        };
        let len = self.outbox.piece_len(
            writing.bytes.len() - writing.offset,
            writing.part_tag.is_some(),
        );
        let end = writing.offset + len;
        let framed = writing.part_tag.map(|tag| {
            let part = crate::screen::core::control_parts::encode_part(
                tag,
                &writing.bytes,
                writing.offset,
                len,
            );
            let mut frame = Vec::with_capacity(5 + part.len());
            frame.push(crate::screen::core::control_parts::CONTROL_PART);
            frame.extend_from_slice(&u32::try_from(part.len()).unwrap_or(u32::MAX).to_be_bytes());
            frame.extend_from_slice(&part);
            frame
        });
        let piece = framed
            .as_deref()
            .unwrap_or(&writing.bytes[writing.offset..end]);
        // The budget is taken before the piece goes: under a ceiling the
        // line never carries more than its credit plus one piece.
        let booked = piece.len() as u64;
        match self.outbox.budget.rate() {
            // Codex 10/9: no piece while a picture holds the line; it waits
            // again (`Outbox::ready`).
            Some(rate) if !self.outbox.budget.take_held_piece(now_us(), booked, rate) => {
                self.outbox.writing = Some(writing);
                return Ok(());
            }
            Some(_) => {}
            // The control stream counts against „Höchste Datenrate“.
            None => self.pipeline.book_control_bytes(booked),
        }
        if writing.metadata {
            host_metadata::send_piece(
                self.context,
                self.principal,
                self.mode,
                self.connection,
                &mut self.send,
                piece,
                self.handle.stop_signal(),
            )
            .await?;
        } else {
            let peer = self.connection.remote_id().to_string();
            control_sender::write_piece(&peer, &mut self.send, piece).await?;
        }
        // The picture that waited for this piece may go (`control_turn`).
        self.outbox.budget.control_went();
        writing.offset = end;
        if writing.offset < writing.bytes.len() {
            self.outbox.writing = Some(writing);
            return Ok(());
        }
        if writing.shape.is_some() && writing.shape == self.outbox.shape {
            self.outbox.shape = None;
            // The position that waited for its shape goes first.
            if let Some(position) = self.outbox.position.take() {
                self.outbox.behind.push_front(position);
            }
        }
        self.send_behind().await
    }

    /// The small frames in `behind`, in their order, while the budget takes
    /// them (`take_small_share`); the first it does not take waits there,
    /// and so does a family frame while a large frame is being written
    /// (round 13, H1: they go between its parts, a family frame after it).
    async fn send_behind(&mut self) -> io::Result<()> {
        while let Some(frame) = self.outbox.behind.pop_front() {
            if self.outbox.writing.is_some() && (!self.outbox.parts || family(&frame)) {
                self.outbox.behind.push_front(frame);
                return Ok(());
            }
            if let Some(rate) = self.outbox.budget.rate() {
                if !self.take_small_share(rate, &frame) {
                    self.outbox.behind.push_front(frame);
                    return Ok(());
                }
            }
            self.send_small(&frame).await?;
        }
        Ok(())
    }

    /// The session ends (round 9 F3, round 10): the large frame being
    /// written goes out piece by piece within the byte budget (the ceiling
    /// stays hard), then the small frames behind it, so the `ERROR` and the
    /// last `STATE` that follow reach the viewer; held frames not yet begun
    /// are dropped. What does not fit into `CLOSE_GRACE`, and a metadata
    /// reply whose rights are gone, ends the farewell: the connection then
    /// closes with the reason as its close reason (`finish`).
    pub(in super::super) async fn end_outbox(&mut self) -> io::Result<()> {
        self.outbox.held.clear();
        self.outbox.shape = None;
        self.outbox.position = None;
        // Round 12: the farewell's own frames wait for the budget in place.
        self.outbox.ending = true;
        while self.outbox.writing.is_some() {
            self.outbox.ready().await;
            self.flush_outbox().await?;
        }
        self.drain_behind().await
    }

    /// Waits for the budget until every small frame in `behind` went (the
    /// farewell, within `CLOSE_GRACE`).
    async fn drain_behind(&mut self) -> io::Result<()> {
        while !self.outbox.behind.is_empty() {
            self.outbox.ready().await;
            // A large frame still being written goes on, so a family frame
            // behind it is reached (`send_behind`).
            if self.outbox.writing.is_some() {
                self.flush_outbox().await?;
            } else {
                self.send_behind().await?;
            }
        }
        Ok(())
    }

    /// A small frame whole, booked as the control stream's rate.
    async fn send_small(&mut self, frame: &HostFrame) -> io::Result<()> {
        let peer = self.connection.remote_id().to_string();
        let sent = control_sender::send_counted(peer, &mut self.send, frame).await?;
        self.pipeline.book_control_bytes(sent);
        Ok(())
    }

    /// Writes `held` whole and books it as the control stream's rate (a
    /// small frame, or a large one without a user ceiling). A small frame
    /// the budget does not take now waits in `behind` (`SmallLane`).
    async fn write(&mut self, held: Held) -> io::Result<()> {
        let waits = match (&held, self.outbox.budget.rate()) {
            (Held::Frame(frame), Some(rate)) => !self.take_small_share(rate, frame),
            _ => false,
        };
        if waits {
            if let Held::Frame(frame) = held {
                self.outbox.queue_behind(frame);
            }
            if self.outbox.ending {
                return self.drain_behind().await;
            }
            return Ok(());
        }
        let sent = match &held {
            Held::Frame(frame) => return self.send_small(frame).await,
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
        self.pipeline.book_control_bytes(sent);
        Ok(())
    }

    /// A small frame under a ceiling (Codex 11/7): what goes beyond the
    /// share the pictures leave the control stream is booked in their budget
    /// before it is sent; a picture being written takes it into its pace
    /// (round 12: booked with the picture, `SharedBudget::picture_sent`).
    /// `false`: the budget is in debt, nothing is taken, the frame waits
    /// (round 12, Codex 12/5, H5: no frame goes on top of the debt, and a
    /// position or statistics frame is no longer dropped). Round 13 (H2):
    /// while the pictures' share is short the budget takes no excess the
    /// share will cover in time (`budget_takes_excess`).
    fn take_small_share(&mut self, rate: u64, frame: &HostFrame) -> bool {
        let bytes = control_sender::wire_bytes(frame);
        let reserved = self.outbox.budget.reserved();
        let now = now_us();
        let excess = self.outbox.lane.peek(now, reserved, bytes);
        let shared = self.outbox.lane.wait_us(now, reserved, bytes).is_some();
        let taken = if excess == 0 || self.outbox.budget.absorb_while_writing(excess) {
            true
        } else if budget_takes_excess(&self.outbox.budget, shared)
            && self.outbox.budget.admits(now, rate)
        {
            self.outbox.budget.on_sent(now, excess, rate);
            true
        } else {
            false
        };
        if taken {
            let _ = self.outbox.lane.excess(now, reserved, bytes);
        }
        taken
    }
}
