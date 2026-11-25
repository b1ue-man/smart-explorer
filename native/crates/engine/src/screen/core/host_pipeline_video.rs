//! The pipeline's pictures as messages (`video_transport.rs`): which
//! carrier a picture takes (under „UDP“ the direct UDP pair or QUIC
//! datagrams, under „TCP“ the direct TCP link, else the picture streams),
//! the switch between kinds of carrier (a new group with a key frame), the
//! send of one message group's picture, and the answer to a lost picture:
//! the viewer's `FRAME_LOSS`, or a picture the carrier did not take, ends
//! the group; the next one continues from the confirmed picture (B-36) or
//! starts with a key frame, as after a stale stream (`reset_stale_stream`).
use super::*;
use crate::screen::core::chain_meter;
use crate::screen::core::delivery::Delivery;
use crate::screen::core::gop_continue::ResetFacts;
use crate::screen::core::rate_control::PathKind;
use crate::screen::core::sender_budget::EncodePermit;
use crate::screen::core::video_transport::video_route::{loss_recovery, LossRecovery};
use crate::screen::core::video_transport::video_sink::{MessageCarrier, MessageGroup, SinkKind};
use crate::screen::core::video_transport::FrameLoss;

/// One encoded picture: frame, id, capture time, size.
pub(super) type Picture = (EncodedFrame, u64, u64, FrameSize);

impl Pipeline {
    /// The carrier of the next picture for a `video` picture of a video
    /// codec without hybrid regions: under „UDP“ the direct pair or QUIC
    /// datagrams the viewer offered, under „TCP“ the direct TCP link;
    /// `None`: the picture streams.
    pub(super) fn message_carrier(&self, connection: &Connection) -> Option<MessageCarrier> {
        let video = self.kind == StreamKind::Video
            && self.choice.codec.is_video()
            && !self.frame_contract.is_some_and(|c| c.hybrid_regions);
        if !video {
            return None;
        }
        match self.pacer.delivery() {
            Delivery::Realtime => self.work.video().carrier(connection),
            Delivery::Reliable => self.work.video().record_carrier(),
        }
    }

    /// The open group is of another kind of carrier (`kind`): it ends, and
    /// a key frame opens the next one. `Some`: the picture (no key frame)
    /// does not go and the tick ends with it.
    pub(super) fn switch_carrier(
        &mut self,
        connection: &Connection,
        kind: SinkKind,
        keyframe: bool,
        frame_id: u64,
    ) -> Option<Produced> {
        let datagrams = kind == SinkKind::Packets;
        if self.pacer.datagrams() != datagrams {
            self.pacer.forget_in_flight();
            self.pacer.set_datagrams(datagrams);
        }
        if kind == SinkKind::Stream {
            let relay = path_of(connection).0 == PathKind::Relay;
            let reliable = self.pacer.delivery() == Delivery::Reliable;
            self.work.video().note_streams(relay, reliable);
        }
        let open = self.gop.as_ref().map(PictureSink::kind);
        if open.is_none_or(|open| open == kind) {
            return None;
        }
        self.note_restart(match kind {
            SinkKind::Packets => "pictures as datagrams: a new group",
            SinkKind::Records => "pictures on the direct TCP link: a new group",
            SinkKind::Stream => "pictures on the streams: a new group",
        });
        self.new_group();
        if keyframe {
            return None;
        }
        // Gemini 8/3: the encoded picture keeps its id.
        self.next_frame_id = self.next_frame_id.max(frame_id.saturating_add(1));
        Some(Produced::Reset)
    }

    /// Sends one picture of the open message group (opened here when none
    /// is): all its packets at once, no pace, nothing sent again.
    pub(super) async fn send_message(
        &mut self,
        connection: &Connection,
        carrier: MessageCarrier,
        (frame, frame_id, capture_ts_us, size): Picture,
        permit: EncodePermit,
        work_epoch: u64,
    ) -> Result<Produced, PipelineError> {
        let work = self.work.clone();
        let keyframe = frame.keyframe;
        let mut continued = false;
        if self.gop.is_none() {
            let header = GopHeader {
                gop_id: self.next_gop_id,
                frame_contract: self.frame_contract,
                family_layout_revision: self
                    .stage
                    .as_ref()
                    .and_then(|stage| stage.source.family_layout_revision()),
                kind: self.kind,
                codec: self.choice.codec,
                profile: self.choice.profile,
                size,
            };
            self.next_gop_id += 1;
            let mut group = MessageGroup::new(header, carrier.clone());
            continued = !keyframe && self.continue_from.is_some();
            if continued {
                group.mark_continuation();
                chain_meter::lock(&self.chain).on_continued();
            }
            self.confirmed.opened(header.gop_id, continued);
            if !continued {
                self.key_gop = header.gop_id;
            }
            self.continue_from = None;
            self.gop = Some(PictureSink::Messages(group));
        }
        self.next_frame_id += 1;
        self.rate.shared_budget().control_turn().await;
        let picture_write = self.rate.picture_write();
        let send_started_us = now_us();
        let send_started = std::time::Instant::now();
        let Some(PictureSink::Messages(group)) = self.gop.as_mut() else {
            return Ok(Produced::Idle);
        };
        let records = carrier.kind() == SinkKind::Records;
        group.set_carrier(carrier);
        let gop_id = group.header().gop_id;
        let writing = group.write_picture(frame_id, capture_ts_us, keyframe, frame.records);
        let written = tokio::select! {biased;
            _ = work.cancelled(work_epoch) => Err(io::Error::from(io::ErrorKind::Interrupted)),
            result = writing => result,
        };
        let booked = written.as_ref().map_or(0, |sent| sent.bytes);
        self.rate.on_stream_sent(send_started_us, booked, true);
        drop(picture_write);
        let sent = match written {
            Ok(sent) => sent,
            _ if connection.close_reason().is_some() => return Ok(Produced::Closed),
            // Overtaken (a new source, mode or route): no loss, a new group.
            _ if !work.current(work_epoch) => {
                self.reset_stream();
                return Ok(Produced::Reset);
            }
            Err(error) => {
                // Not taken whole: the picture is lost; recovered at once.
                // A TCP link that failed is given up (the streams follow).
                if records {
                    self.work.video().record_ended(&error.to_string());
                } else {
                    self.work.video().note_failed();
                }
                self.note_restart(&format!(
                    "picture {frame_id} not sent as a message: {error}"
                ));
                self.apply_loss(FrameLoss { gop_id, frame_id });
                return Ok(Produced::Reset);
            }
        };
        if !work.current(work_epoch) {
            self.reset_stream();
            return Ok(Produced::Reset);
        }
        let micros = u64::try_from(send_started.elapsed().as_micros()).unwrap_or(u64::MAX);
        chain_meter::lock(&self.chain).record_us(chain_meter::Station::Send, micros);
        self.work
            .video()
            .note_sent(sent.carrier, sent.packets, gop_id, frame_id);
        let bytes = sent.bytes;
        // A continuation starts the presentation of its group like a key frame.
        self.presentation
            .sent(gop_id, frame_id, keyframe || continued);
        permit.sent(gop_id, frame_id, bytes);
        let sent_us = now_us();
        self.pacer.on_frame_sent(SentFrame {
            gop_id,
            frame_id,
            bytes,
            sent_us,
            keyframe,
        });
        self.rate.on_frame_sent(bytes, sent_us);
        self.totals.bytes += bytes;
        self.totals.frames += 1;
        self.totals.keyframes += u64::from(keyframe);
        self.second_bytes += bytes;
        self.second_frames += 1;
        Ok(Produced::Sent)
    }

    /// The viewer's `FRAME_LOSS`, taken between two pictures.
    pub(super) fn apply_video_events(&mut self) {
        if let Some(loss) = self.work.video().take_loss() {
            self.apply_loss(loss);
        }
    }

    fn apply_loss(&mut self, loss: FrameLoss) {
        let open = self
            .gop
            .as_ref()
            .filter(|gop| gop.is_messages())
            .map(|gop| gop.header().gop_id);
        let facts = self.reset_facts();
        match loss_recovery(open, loss, &facts) {
            LossRecovery::Ignore => {}
            LossRecovery::Continue(last_good) => {
                self.note_restart(&format!(
                    "picture {} of group {} lost: continuing from {last_good}",
                    loss.frame_id, loss.gop_id
                ));
                self.continue_after_reset(last_good);
            }
            LossRecovery::KeyFrame => {
                self.note_restart(&format!(
                    "picture {} of group {} lost: a key frame",
                    loss.frame_id, loss.gop_id
                ));
                self.reset_stream();
            }
        }
    }

    /// What decides whether the next group continues the chain (B-36).
    pub(super) fn reset_facts(&mut self) -> ResetFacts {
        ResetFacts {
            negotiated: self.continuation_offered(),
            video: self.kind == StreamKind::Video
                && self.choice.codec.is_video()
                && !self.frame_contract.is_some_and(|c| c.hybrid_regions),
            keyframe_due: self.pacer.keyframe_due(),
            encoder_running: self.running.is_some(),
            stage_continues: self.stage.as_mut().is_some_and(|stage| {
                stage
                    .source
                    .host_stage()
                    .is_none_or(|host| host.continues_references())
            }),
            last_good: self.confirmed.last_good(),
        }
    }

    /// The group goes, the encoder stays: the next group continues from
    /// `last_good` (B-36).
    pub(super) fn continue_after_reset(&mut self, last_good: u64) {
        if let Some(gop) = self.gop.take() {
            gop.reset();
        }
        self.confirmed.stream_reset();
        self.pacer.on_stream_reset_continued();
        self.sender.reset();
        self.presentation.restart_group();
        self.continue_from = Some(last_good);
    }
}
