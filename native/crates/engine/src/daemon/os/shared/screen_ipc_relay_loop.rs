//! The relay loop of a viewer session (moved out of `screen_ipc.rs`): host
//! frames and pictures go to the client, the client's frames to the host.
//!
//! Fix 2026-11-19: the loop awaited `out.send` of a picture inside its
//! select, so while the client's socket was full it neither forwarded the
//! client's input to the host nor read the host's frames: steering waited
//! behind pictures. Now each item for the client waits in a small pending
//! queue and goes out through `out.reserve()` beside the other branches, so
//! the client's frames reach the host at once whatever the client reads.
//!
//! Order: frames of one kind keep their order (host frames among
//! themselves, pictures among themselves: the records of a group and its
//! `BROKEN` after them; one pending picture at a time). A host frame may go
//! out before a picture that waits; the host sends both on separate QUIC
//! streams, so the viewer never relied on their order (a picture that
//! overtakes a visibility acknowledgement waits for it,
//! `ViewerSession::keep_if_awaiting_floor`). Memory stays bounded: one
//! picture and `CONTROL_PENDING` frames; beyond that the host's frames wait
//! in their channel as before.
use std::collections::VecDeque;

use tokio::sync::mpsc;

use crate::daemon::ipc_protocol::screen::{
    encode_relay_record, RELAY_BROKEN, RELAY_CLOCK, RELAY_HOST, RELAY_RATE_CEILING, RELAY_RECORD,
};
use crate::screen::core::gop_stream::{now_us, PictureEvent};
use crate::screen::core::metrics::ClockSync;
use crate::screen::core::session_state::EndReason;
use crate::screen::core::viewer_client::ViewerSession;
use crate::screen::core::wire_control::{encode_host_frame, HostFrame, ViewerFrame};

use super::reconnect;

/// Frames for the client (host frames, clock, rate state) held while the
/// client reads slower than they come; the host's next frame waits beyond.
const CONTROL_PENDING: usize = 8;

type Item = (u8, Vec<u8>);

/// What waits for the client.
#[derive(Default)]
struct Pending {
    control: VecDeque<Item>,
    picture: Option<Item>,
}

impl Pending {
    fn next(&mut self) -> Option<Item> {
        self.control.pop_front().or_else(|| self.picture.take())
    }

    fn is_empty(&self) -> bool {
        self.control.is_empty() && self.picture.is_none()
    }

    /// Sends what still waits (the session ends; nothing more comes in).
    async fn flush(&mut self, out: &mpsc::Sender<Item>) {
        while let Some(item) = self.next() {
            if out.send(item).await.is_err() {
                return;
            }
        }
    }
}

/// Forwards host frames and pictures to the client and its control frames
/// to the host; returns the end code and detail.
pub(super) async fn relay_loop(
    session: &mut ViewerSession,
    out: &mpsc::Sender<Item>,
    input: &mut mpsc::Receiver<Option<ViewerFrame>>,
    modes: &mut reconnect::ModeMemory,
) -> (String, Option<String>) {
    let mut clock = ClockSync::default();
    let mut offset = None;
    let mut pending = Pending::default();
    loop {
        let host_room = pending.control.len() < CONTROL_PENDING;
        let picture_room = pending.picture.is_none();
        let waiting = !pending.is_empty();
        tokio::select! {
            permit = out.reserve(), if waiting => match permit {
                Ok(permit) => {
                    if let Some(item) = pending.next() {
                        permit.send(item);
                    }
                }
                Err(_) => return ("viewer_closed".into(), None),
            },
            frame = session.host.recv(), if host_room => match frame {
                Some(Ok(frame)) => {
                    if reconnect::changed_session(&frame) {
                        return (EndReason::SessionChanged.key().into(), None);
                    }
                    if let HostFrame::Pong(pong) = &frame {
                        clock.on_pong(pong, now_us());
                        if clock.offset_us() != offset {
                            offset = clock.offset_us();
                            let value = offset.unwrap_or(0).to_be_bytes().to_vec();
                            pending.control.push_back((RELAY_CLOCK, value));
                        }
                    }
                    if let Ok((tag, payload)) = encode_host_frame(&frame) {
                        let mut body = Vec::with_capacity(payload.len() + 1);
                        body.push(tag);
                        body.extend_from_slice(&payload);
                        pending.control.push_back((RELAY_HOST, body));
                    }
                }
                Some(Err(error)) => {
                    pending.flush(out).await;
                    return ("connection_lost".into(), Some(error.to_string()));
                }
                None => {
                    pending.flush(out).await;
                    return ("ended".into(), None);
                }
            },
            event = session.pictures.recv(), if picture_room => match event {
                Some(PictureEvent::Record { header, record, .. }) => {
                    if let Ok(body) = encode_relay_record(&header, &record) {
                        pending.picture = Some((RELAY_RECORD, body));
                    }
                }
                Some(PictureEvent::Broken { gop_id }) => {
                    pending.picture = Some((RELAY_BROKEN, gop_id.to_be_bytes().to_vec()));
                }
                None => {
                    pending.flush(out).await;
                    return ("ended".into(), None);
                }
            },
            frame = input.recv() => match frame {
                Some(Some(frame)) => {
                    modes.note(&frame);
                    // What became of „Höchste Datenrate“ changed during the
                    // session (P1): applied, or dropped for an old host.
                    let state = reconnect::rate_state(&frame, session.rate_ceiling);
                    let _ = session.control.send(frame).await;
                    if let Some(json) = state {
                        // Rare (a user's choice); it never blocks the input.
                        pending.control.push_back((RELAY_RATE_CEILING, json));
                    }
                }
                Some(None) | None => return ("viewer_closed".into(), None),
            },
        }
    }
}
