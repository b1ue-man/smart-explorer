//! The session's side of the outbox (`host_session_send.rs`): frames in,
//! pieces out.
use super::super::*;
use super::{family, large, Held, Writing};
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
        if self.outbox.writing.is_some() {
            match held {
                // A viewer that takes parts gets small frames between them
                // (round 10, Codex 10/10); a family's frames keep their order.
                Held::Frame(frame) if !large(&frame) && self.outbox.parts && !family(&frame) => {
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
            if let Some(position) = self.outbox.position.take() {
                self.write(Held::Frame(position)).await?;
            }
        }
        while let Some(frame) = self.outbox.behind.pop_front() {
            self.write(Held::Frame(frame)).await?;
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
        while self.outbox.writing.is_some() {
            self.outbox.ready().await;
            self.flush_outbox().await?;
        }
        while let Some(frame) = self.outbox.behind.pop_front() {
            self.write(Held::Frame(frame)).await?;
        }
        Ok(())
    }

    /// Writes `held` whole and books it as the control stream's rate (a
    /// small frame, or a large one without a user ceiling).
    async fn write(&mut self, held: Held) -> io::Result<()> {
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
        self.pipeline.book_control_bytes(sent);
        Ok(())
    }
}
