//! The session's side of the outbox (`host_session_send.rs`): frames in,
//! pieces out.
use super::super::*;
use super::{large, Held, Writing};
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
                Writing {
                    bytes: control_sender::encode_wire(&frame)?,
                    offset: 0,
                    metadata,
                    shape: match &frame {
                        HostFrame::CursorShape(shape) => Some(shape.id),
                        _ => None,
                    },
                }
            }
        };
        let len = self.outbox.piece_len(writing.bytes.len() - writing.offset);
        let end = writing.offset + len;
        // The budget is taken before the piece goes: under a ceiling the
        // line never carries more than its credit plus one piece.
        match self.outbox.budget.rate() {
            Some(rate) => self.outbox.budget.on_held_sent(now_us(), len as u64, rate),
            // The control stream counts against „Höchste Datenrate“.
            None => self.pipeline.book_control_bytes(len as u64),
        }
        let piece = &writing.bytes[writing.offset..end];
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

    /// The session ends (round 9 F3): the large frame being written goes out
    /// whole without the budget, then the small frames behind it, so the
    /// `ERROR` and the last `STATE` that follow reach the viewer; held frames
    /// not yet begun are dropped. A metadata reply whose rights are gone is
    /// not finished (its error ends the farewell, as before).
    pub(in super::super) async fn end_outbox(&mut self) -> io::Result<()> {
        self.outbox.held.clear();
        self.outbox.shape = None;
        self.outbox.position = None;
        if let Some(writing) = self.outbox.writing.take() {
            let rest = &writing.bytes[writing.offset..];
            if writing.metadata {
                host_metadata::send_piece(
                    self.context,
                    self.principal,
                    self.mode,
                    self.connection,
                    &mut self.send,
                    rest,
                    self.handle.stop_signal(),
                )
                .await?;
            } else {
                let peer = self.connection.remote_id().to_string();
                control_sender::write_piece(&peer, &mut self.send, rest).await?;
            }
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
