//! The threads of one viewer session (SCR-M7, U25-4). The decode thread
//! opens the relayed stream, reads its events in a loop and decodes the
//! pictures; the GUI thread only ever takes the newest picture out of a slot
//! (`Shared`) and wakes up for it. A sender thread writes the frames the GUI
//! queues (`SET_MODE`, `INPUT`, …), so a stalled socket never blocks the GUI
//! either. The relay acknowledges pictures and answers `PING` itself (SCR-M2);
//! the viewer sends neither `FRAME_ACK` nor `PREVIEW_ACK`.
//!
//! Picture groups: a record of a newer group drops the previous group and
//! opens a decoder for the new one; a record of an older group is stale and
//! ignored. A decode error or a broken group is answered with one key frame
//! request (`KeyframeRequester`: at most one open request); until a key frame
//! of the group arrives its records are skipped.
use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;
use std::time::Instant;

use super::screen_session::{DecoderOpener, EndInfo, Picture, PictureData, Shared};
use super::screen_texts::{end_text, error_text};
use crate::daemon::StreamEvent;
use crate::screen::core::decoded_frame::{DecodedFrame, DecoderFallback, MAX_PENDING_FRAMES};
use crate::screen::core::gop_stream::now_us;
use crate::screen::core::host_backend::ScreenDecoder;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::session_state::{KeyframeRequester, SessionState};
use crate::screen::core::wire_binary::{FrameRecord, GopHeader, FLAG_KEYFRAME};
use crate::screen::core::wire_control::{
    ControlAction, ControlRequest, HostFrame, KeyframeReason, KeyframeRequest, ViewerFrame,
};

pub(crate) use super::screen_decode_run::spawn;
use super::screen_decode_run::SendFrame;

/// One picture group with its decoder.
struct Gop {
    id: u64,
    decoder: Box<dyn ScreenDecoder>,
    /// The decoder keeps planes instead of converting.
    yuv: bool,
    /// Epoch captured when this actual input was submitted, retained over codec delay.
    pending: BTreeMap<u64, (DecodedFrame, super::screen_presented::PresentedFrame)>,
}

/// What the decode thread keeps between two events.
pub(crate) struct DecodeState {
    open_decoder: DecoderOpener,
    send: SendFrame,
    take_control: bool,
    gop: Option<Gop>,
    /// The newest group seen; records of older ones are stale.
    latest_gop: u64,
    /// A group whose records are skipped until its next key frame.
    broken: Option<u64>,
    /// A failed record cannot restart from a later fragment of that same frame.
    broken_frame: Option<u64>,
    last_record: Option<(u64, u64)>,
    keyframes: KeyframeRequester,
    seq: u64,
    last_shown: Option<Instant>,
    started: Instant,
    // Declared last: decoder/pending pictures must die before the session lease.
    shared: Arc<Shared>,
    memory: crate::screen::core::frame_memory::MemoryHold,
}

#[path = "screen_decode_events.rs"]
mod events;

impl DecodeState {
    pub(crate) fn new(
        shared: Arc<Shared>,
        open_decoder: DecoderOpener,
        send: SendFrame,
        take_control: bool,
    ) -> Self {
        let memory = shared
            .memory
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        Self {
            memory,
            shared,
            open_decoder,
            send,
            take_control,
            gop: None,
            latest_gop: 0,
            broken: None,
            broken_frame: None,
            last_record: None,
            keyframes: KeyframeRequester::default(),
            seq: 0,
            last_shown: None,
            started: Instant::now(),
        }
    }

    fn now(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_micros()).unwrap_or(u64::MAX)
    }

    /// At most one key frame request is open (`KEYFRAME_RETRY_US`).
    fn request_keyframe(&mut self, reason: KeyframeReason, gop_id: u64) {
        if self.keyframes.should_request(self.now()) {
            let request = KeyframeRequest {
                reason,
                gop_id: Some(gop_id),
            };
            let _ = (self.send)(&ViewerFrame::KeyframeRequest(request));
        }
    }

    fn open_gop(&mut self, header: &GopHeader) -> Option<Gop> {
        match (self.open_decoder)(header) {
            Ok(mut decoder) => {
                let yuv = self.shared.prefers_yuv();
                if yuv {
                    decoder.prefer_yuv(true);
                }
                Some(Gop {
                    id: header.gop_id,
                    decoder,
                    yuv,
                    pending: BTreeMap::new(),
                })
            }
            Err(error) => {
                self.shared.cancel_presentation();
                let text = format!("Das Bild kann nicht dekodiert werden: {error}");
                self.shared.update(|info| info.notice = Some(text));
                self.broken = Some(header.gop_id);
                self.request_keyframe(KeyframeReason::DecodeError, header.gop_id);
                None
            }
        }
    }

    fn record(&mut self, header: GopHeader, record: FrameRecord) {
        let admitted = self.shared.info().hello.as_ref().is_some_and(|hello| {
            crate::screen::core::frame_budget::contract_matches(
                header.frame_contract,
                hello.frame_contract,
            )
        });
        let exceeds = self
            .shared
            .memory
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(|memory| {
                let owners = if header.frame_contract.is_some() {
                    24
                } else {
                    8
                };
                header.size.pixels().saturating_mul(4 * owners) > memory.bytes()
            });
        if !admitted || exceeds {
            self.connection_lost();
            return;
        }
        if self
            .shared
            .lossless_fallback
            .load(std::sync::atomic::Ordering::Acquire)
            && header.codec != crate::screen::core::negotiate::CodecKind::Tiles
        {
            return;
        }
        if let Some(family) = &self.shared.info().family {
            if family.snapshot.as_ref().is_none_or(|snapshot| {
                header.family_layout_revision != Some(snapshot.layout_revision)
            }) {
                return;
            }
        }
        if header.gop_id < self.latest_gop {
            return;
        }
        self.last_record = Some((header.gop_id, record.frame_id));
        let keyframe = record.flags & FLAG_KEYFRAME != 0;
        if self.broken == Some(header.gop_id) {
            if !keyframe
                || self
                    .broken_frame
                    .is_some_and(|frame| record.frame_id <= frame)
            {
                return;
            }
            self.broken = None;
            self.broken_frame = None;
            self.gop = None;
        }
        self.latest_gop = header.gop_id;
        let presented = self.shared.begin_frame(header.gop_id, record.frame_id);
        let mut gop = match self.gop.take() {
            Some(gop) if gop.id == header.gop_id => gop,
            // The previous group is dropped with its decoder.
            _ => match self.open_gop(&header) {
                Some(gop) => gop,
                None => {
                    self.broken_frame = Some(record.frame_id);
                    return;
                }
            },
        };
        let want_yuv = self.shared.prefers_yuv();
        if want_yuv != gop.yuv {
            gop.decoder.prefer_yuv(want_yuv);
            gop.yuv = want_yuv;
        }
        let decoded = (|| -> io::Result<_> {
            let input = DecodedFrame::from_record(&record);
            if let Some((submitted, _)) = gop.pending.get(&input.frame_id) {
                if *submitted != input {
                    return Err(io::Error::other("frame fragments changed capture identity"));
                }
            } else {
                if gop.pending.len() >= MAX_PENDING_FRAMES {
                    return Err(io::Error::other("presentation identity window exhausted"));
                }
                gop.pending.insert(input.frame_id, (input, presented));
            }
            if !gop.decoder.decode(&record)? {
                return Ok(None);
            }
            let output = gop
                .decoder
                .decoded_frame()
                .ok_or_else(|| io::Error::other("decoded picture has no identity"))?;
            let (submitted, receipt) = gop
                .pending
                .remove(&output.frame_id)
                .ok_or_else(|| io::Error::other("decoded picture was not submitted in this GOP"))?;
            if output != submitted {
                return Err(io::Error::other("decoded picture changed capture identity"));
            }
            gop.pending
                .retain(|frame_id, _| *frame_id > output.frame_id);
            Ok(Some((output, receipt)))
        })();
        match decoded {
            Ok(output) => {
                if let Some((output, receipt)) = output {
                    if output.keyframe {
                        self.keyframes.on_keyframe();
                    }
                    self.show(&gop, header.size, output.capture_ts_us, receipt);
                }
                self.gop = Some(gop);
            }
            Err(error) => {
                // The failed picture was never shown; the shown one stays
                // without receipts until the requested key frame.
                self.shared.suspend_presentation();
                self.broken = Some(header.gop_id);
                self.broken_frame = Some(record.frame_id);
                if header.frame_contract.is_some_and(|c| c.hybrid_regions)
                    && DecoderFallback::from_error(&error).is_some()
                {
                    if let Some(mode) = self.shared.request_lossless() {
                        self.shared.update(|info| info.notice = Some("Der Videodecoder liefert kein passend zugeordnetes Hybridbild. Die Sitzung wechselt zur verlustfreien Kachelübertragung.".into()));
                        if (self.send)(&ViewerFrame::SetMode(mode)).is_err() {
                            self.connection_lost();
                        }
                    }
                    return;
                }
                self.request_keyframe(KeyframeReason::DecodeError, header.gop_id);
            }
        }
    }

    /// Hands the finished picture to the GUI thread.
    fn show(
        &mut self,
        gop: &Gop,
        size: FrameSize,
        capture_ts_us: u64,
        presented: super::screen_presented::PresentedFrame,
    ) {
        let shown_us = now_us();
        let gap = std::time::Duration::from_millis(self.shared.present_gap_ms());
        let due = self
            .last_shown
            .is_none_or(|shown| gap.is_zero() || shown.elapsed() >= gap);
        self.shared.update(|info| {
            let latency =
                i128::from(shown_us) + i128::from(info.clock_offset_us) - i128::from(capture_ts_us);
            if let Ok(latency) = u64::try_from(latency) {
                info.latency.record(latency);
            }
            info.pictures += 1;
        });
        if !due {
            return;
        }
        let planes = gop
            .yuv
            .then(|| gop.decoder.yuv_picture())
            .flatten()
            .map(PictureData::Yuv);
        let data = planes.unwrap_or_else(|| PictureData::Bgra(gop.decoder.picture().clone()));
        self.seq += 1;
        self.last_shown = Some(Instant::now());
        self.shared.put_picture(Picture {
            memory: self.memory.clone(),
            seq: self.seq,
            presented,
            size,
            data,
        });
    }
}
#[cfg(test)]
#[path = "batches_decode_lifetime_task_tests.rs"]
mod lifetime_tests;
