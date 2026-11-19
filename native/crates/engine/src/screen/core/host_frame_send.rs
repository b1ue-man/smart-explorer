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
        work_epoch: u64,
    ) -> Result<Produced, PipelineError> {
        let work = self.work.clone();
        if !work.current(work_epoch) {
            self.reset_stream();
            return Ok(Produced::Reset);
        }
        if frame.records.is_empty() {
            return Ok(Produced::Idle);
        }
        let stale = Duration::from_micros(STALE_STREAM_US);
        // B-36: a new stream whose first frame is no key frame continues the
        // encoder's chain from the confirmed picture.
        let mut continued = false;
        // Review 2026-11-19 (Codex 3/9, 5/22): a new stream's header goes on
        // the line as well; booked with the picture as the writer wrote it.
        let mut header_bytes = 0u64;
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
            let opened = tokio::select! {biased;
                _=work.cancelled(work_epoch)=>Err(io::Error::from(io::ErrorKind::Interrupted)),
                result=crate::screen::core::gop_stream::write_with_route(connection,stale,GopWriter::open(connection,header))=>result,
            };
            match opened {
                Ok(mut gop) => {
                    header_bytes = gop.header_bytes();
                    continued = !frame.keyframe && self.continue_from.is_some();
                    if continued {
                        gop.mark_continuation();
                        super::super::chain_meter::lock(&self.chain).on_continued();
                    }
                    self.confirmed.opened(gop.header().gop_id, continued);
                    if !continued {
                        self.key_gop = gop.header().gop_id;
                    }
                    self.continue_from = None;
                    self.gop = Some(gop);
                }
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
        // At the rate the line was measured to carry (plan 2026-11-17): a
        // large lossless key frame on a line slower than the target gets the
        // time it needs.
        let serialization_us = (u128::from(encoded_bytes) * 8 * 1_000_000
            / u128::from(self.rate.sustained_bps().max(1)))
        .min(u128::from(u64::MAX)) as u64;
        // Review 2026-11-19 (Codex 13): the byte budget books what the writer
        // puts on the stream, the record headers included, when the send
        // starts (`rate_tiles.rs`); a cut-off write counts whole.
        let planned = encoded_bytes
            + super::super::wire_binary::RECORD_HEADER_BYTES as u64 * frame.records.len() as u64;
        let video = self.choice.codec.is_video();
        let send_started_us = now_us();
        // Codex 4/5: the records reach the stream within the byte budget,
        // after its saved credit at its rate (`GopWriter::pace_next_frame`);
        // the write gets the time that takes.
        let pace = self.rate.stream_pace(send_started_us, video);
        let paced_us = pace.map_or(0, |(rate, credit)| {
            let ahead = u128::from(planned.saturating_sub(credit));
            (ahead * 8 * 1_000_000 / u128::from(rate)).min(u128::from(u64::MAX)) as u64
        });
        // Codex 5/12: also without a budget now, a ceiling set while the
        // picture is written holds its rest (`pace.rate()`).
        let (rate, credit) = pace.unwrap_or((u64::MAX, 0));
        gop.pace_next_frame(rate, credit, Some(self.work.ceiling_cell()));
        let active_budget =
            stale.saturating_add(Duration::from_micros(serialization_us.max(paced_us)));
        let send_started = std::time::Instant::now();
        let writing = crate::screen::core::gop_stream::write_with_route(
            connection,
            active_budget,
            gop.write_frame_admitted(frame_id, capture_ts_us, frame.keyframe, frame.records),
        );
        let written = tokio::select! {biased;
            _=work.cancelled(work_epoch)=>Err(io::Error::from(io::ErrorKind::Interrupted)),
            result=writing=>result,
        };
        let booked = written.as_ref().map_or(planned, |bytes| *bytes) + header_bytes;
        self.rate.on_stream_sent(send_started_us, booked, video);
        let bytes = match written {
            Ok(bytes) => {
                // Station `send` (BK0): all records handed to the stream.
                let micros = u64::try_from(send_started.elapsed().as_micros()).unwrap_or(u64::MAX);
                super::super::chain_meter::lock(&self.chain)
                    .record_us(super::super::chain_meter::Station::Send, micros);
                bytes
            }
            _ if connection.close_reason().is_some() => return Ok(Produced::Closed),
            _ => {
                self.reset_stream();
                return Ok(Produced::Reset);
            }
        };
        if !work.current(work_epoch) {
            self.reset_stream();
            return Ok(Produced::Reset);
        }
        // A continuation starts the presentation of its stream like a key frame.
        self.presentation
            .sent(gop_id, frame_id, frame.keyframe || continued);
        permit.sent(gop_id, frame_id, bytes);
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
