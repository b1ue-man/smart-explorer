//! Ordered frame transmission and shared sender debt.
use super::*;

impl Pipeline {
    pub(super) async fn send(
        &mut self,
        connection: &Connection,
        frame: EncodedFrame,
        frame_id: u64,
        capture_ts_us: u64,
        size: FrameSize,
        permit: crate::screen::core::sender_budget::EncodePermit,
        work_epoch:u64,
    ) -> Result<Produced, PipelineError> {
        let work=self.work.clone();
        if !work.current(work_epoch) {self.reset_stream();return Ok(Produced::Reset);}
        if frame.records.is_empty() {
            return Ok(Produced::Idle);
        }
        let stale = Duration::from_micros(STALE_STREAM_US);
        if self.gop.is_none() {
            let header = GopHeader {
                gop_id: self.next_gop_id,
                frame_contract: self.frame_contract,
                family_layout_revision: self.stage.as_ref().and_then(|stage| stage.source.family_layout_revision()),
                kind: self.kind,
                codec: self.choice.codec,
                profile: self.choice.profile,
                size,
            };
            self.next_gop_id += 1;
            let opened=tokio::select! {biased;
                _=work.cancelled(work_epoch)=>Err(io::Error::from(io::ErrorKind::Interrupted)),
                result=crate::screen::core::gop_stream::write_with_route(connection,stale,GopWriter::open(connection,header))=>result,
            };
            match opened {
                Ok(gop) => self.gop = Some(gop),
                _ if connection.close_reason().is_some() => return Ok(Produced::Closed),
                // No free stream slot or credit yet: the frame is lost, the
                // next one is a key frame on a new stream.
                _ => {
                    self.reset_stream();
                    return Ok(Produced::Reset);
                }
            }
        }
        let Some(gop) = self.gop.as_mut() else {
            return Ok(Produced::Idle);
        };
        let gop_id = gop.header().gop_id;
        self.next_frame_id += 1;
        let encoded_bytes: u64 = frame.records.iter().map(|r| r.len() as u64).sum();
        let serialization_us = (u128::from(encoded_bytes) * 8 * 1_000_000 / u128::from(self.rate.target_bps().max(1))).min(u128::from(u64::MAX)) as u64;
        let active_budget = stale.saturating_add(Duration::from_micros(serialization_us));
        let writing=crate::screen::core::gop_stream::write_with_route(
            connection, active_budget,
            gop.write_frame(frame_id, capture_ts_us, frame.keyframe, frame.records),
        );
        let written=tokio::select! {biased;
            _=work.cancelled(work_epoch)=>Err(io::Error::from(io::ErrorKind::Interrupted)),
            result=writing=>result,
        };
        let bytes = match written {
            Ok(bytes) => bytes,
            _ if connection.close_reason().is_some() => return Ok(Produced::Closed),
            _ => {
                self.reset_stream();
                return Ok(Produced::Reset);
            }
        };
        if !work.current(work_epoch) {self.reset_stream();return Ok(Produced::Reset);}
        self.presentation.sent(gop_id,frame_id,frame.keyframe);
        permit.sent(gop_id,frame_id,bytes);
        let sent_us = now_us();
        self.pacer.on_frame_sent(SentFrame {
            gop_id,
            frame_id,
            bytes,
            sent_us,
            keyframe: frame.keyframe,
        });
        self.rate.on_frame_sent(bytes, sent_us);
        self.totals.bytes += bytes;
        self.totals.frames += 1;
        self.totals.keyframes += u64::from(frame.keyframe);
        self.second_bytes += bytes;
        self.second_frames += 1;
        if self.kind == StreamKind::Preview {
            self.preview_in_flight = Some(frame_id);
            self.preview_sent_us = sent_us;
        }
        let inject = self
            .inject_resets
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |left| {
                left.checked_sub(1)
            })
            .is_ok();
        if inject {
            self.reset_stream();
            return Ok(Produced::Reset);
        }
        Ok(Produced::Sent)
    }
}
