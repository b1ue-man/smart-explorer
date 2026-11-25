//! The viewer's seam of the carriers (`video_transport.rs`): whatever brings
//! a whole picture message (the packets of the datagram carriers, a record
//! of the direct TCP link) calls `MessageDelivery::deliver`; it checks the
//! message as a stream's reader checks its records (`video_message::decode`),
//! keeps the chain (`video_chain.rs`), sends the records as
//! `PictureEvent::Record` and `FRAME_ACK` after the last one, and on a break
//! `FRAME_LOSS` to the host and `PictureEvent::Broken` to the decoder, which
//! waits for the continuation or asks for a key frame as after a reset
//! stream. Moved out of `video_viewer.rs`.
use tokio::sync::mpsc;

use super::super::gop_stream::PictureEvent;
use super::super::wire_control::{FrameAck, ViewerFrame};
use super::video_chain::{ChainGate, Verdict};
use super::video_message::{decode, Admission};
use super::FrameLoss;

/// Hands whole picture messages to the decode path.
pub struct MessageDelivery {
    events: mpsc::Sender<PictureEvent>,
    acks: mpsc::Sender<ViewerFrame>,
    admission: Admission,
    gate: ChainGate,
    pub(super) pictures: u64,
    pub(super) skipped: u64,
    pub(super) unreadable: u64,
}

impl MessageDelivery {
    pub fn new(
        events: mpsc::Sender<PictureEvent>,
        acks: mpsc::Sender<ViewerFrame>,
        admission: Admission,
    ) -> Self {
        Self {
            events,
            acks,
            admission,
            gate: ChainGate::default(),
            pictures: 0,
            skipped: 0,
            unreadable: 0,
        }
    }

    /// One whole message that arrived at `received_us`; `ids`: group and
    /// frame its packets named (an unreadable message is a lost picture of
    /// them). `false`: the session's channels are closed.
    pub async fn deliver(
        &mut self,
        message: &[u8],
        ids: Option<(u64, u64)>,
        received_us: u64,
    ) -> bool {
        let picture = match decode(message, &self.admission) {
            Ok(picture) => picture,
            Err(_) => {
                self.unreadable += 1;
                return match ids {
                    Some((gop_id, frame_id)) => self.gave_up(gop_id, frame_id).await,
                    None => true,
                };
            }
        };
        let header = picture.header;
        let frame_id = picture.frame_id();
        let verdict = self.gate.picture(
            header.gop_id,
            frame_id,
            picture.prev_frame_id,
            picture.keyframe(),
            picture.continuation(),
        );
        match verdict {
            Verdict::Deliver => {
                for record in picture.records {
                    let event = PictureEvent::Record {
                        header,
                        record,
                        received_us,
                    };
                    if self.events.send(event).await.is_err() {
                        return false;
                    }
                }
                self.pictures += 1;
                // After the records: an acknowledged picture reaches the
                // decoder before any continuation that references it.
                let ack = FrameAck {
                    gop_id: header.gop_id,
                    frame_id,
                    received_us,
                };
                self.acks.send(ViewerFrame::FrameAck(ack)).await.is_ok()
            }
            Verdict::Skip => {
                self.skipped += 1;
                true
            }
            Verdict::Lost(loss) => {
                self.skipped += 1;
                self.report_loss(loss).await
            }
        }
    }

    /// A picture given up incomplete: the loss is reported once per chain.
    pub async fn gave_up(&mut self, gop_id: u64, frame_id: u64) -> bool {
        match self.gate.gave_up(gop_id, frame_id) {
            Some(loss) => self.report_loss(loss).await,
            None => true,
        }
    }

    async fn report_loss(&mut self, loss: FrameLoss) -> bool {
        if self.acks.send(ViewerFrame::FrameLoss(loss)).await.is_err() {
            return false;
        }
        let broken = PictureEvent::Broken {
            gop_id: loss.gop_id,
        };
        self.events.send(broken).await.is_ok()
    }
}
