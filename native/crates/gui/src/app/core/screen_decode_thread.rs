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
//! of the group arrives its records are skipped. A group that continues the
//! chain (B-36) keeps the decoder, and a new „Dekodierer“ choice opens its
//! decoder at the next key frame (V1): `screen_decode_continue.rs`. The
//! decoder hands its picture over and gets the storage of shown pictures
//! back (`screen_decode_local.rs`).
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

pub use super::screen_decode_run::spawn;
use super::screen_decode_run::SendFrame;

/// One picture group with its decoder.
struct Gop {
    id: u64,
    /// The group whose key frame opened the decoder (B-36: later groups
    /// may continue it).
    first: u64,
    header: GopHeader,
    decoder: Box<dyn ScreenDecoder>,
    /// The decoder keeps planes instead of converting.
    yuv: bool,
    /// The decoder keeps D3D11VA pictures on the GPU for the presenter (P8).
    gpu: bool,
    /// Epoch captured when this actual input was submitted, retained over codec delay.
    pending: BTreeMap<u64, (DecodedFrame, super::screen_presented::PresentedFrame)>,
}

impl Gop {
    fn matches_continuation(&self, next: &GopHeader) -> bool {
        self.header.kind == next.kind
            && self.header.codec == next.codec
            && self.header.profile == next.profile
            && self.header.size == next.size
            && self.header.frame_contract == next.frame_contract
    }
}

/// What the decode thread keeps between two events.
pub struct DecodeState {
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
    chain: continuation::Chain,
    /// Watch over the negotiated codec (`screen_decode_guard.rs`).
    guard: crate::screen::core::decoder_health::DecoderGuard,
    /// Named codecs whose missing picture went to the viewer log once.
    reported: crate::screen::core::decoder_health::GivenUp,
    // Declared last: decoder/pending pictures must die before the session lease.
    shared: Arc<Shared>,
    memory: crate::screen::core::frame_memory::MemoryHold,
}

#[path = "screen_decode_continue.rs"]
mod continuation;
#[path = "screen_decode_events.rs"]
mod events;
#[path = "screen_decode_guard.rs"]
mod guard;

impl DecodeState {
    pub fn new(
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
            chain: Default::default(),
            guard: Default::default(),
            reported: Default::default(),
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
                // The statistics name the decoder and its stage (BK6, 4.10a).
                let name = decoder.name();
                self.shared.update(|info| info.decoder = Some(name));
                let yuv = self.shared.prefers_yuv();
                if yuv {
                    decoder.prefer_yuv(true);
                }
                let gpu = self.shared.direct.wanted();
                decoder.prefer_gpu(gpu);
                Some(Gop {
                    id: header.gop_id,
                    first: header.gop_id,
                    header: *header,
                    decoder,
                    yuv,
                    gpu,
                    pending: BTreeMap::new(),
                })
            }
            Err(error) => {
                self.guard_failure(header.codec);
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
            // Every record not decoded breaks the chain (B-36).
            self.chain.continuity.broken();
            return;
        }
        if let Some(family) = &self.shared.info().family {
            if family.snapshot.as_ref().is_none_or(|snapshot| {
                header.family_layout_revision != Some(snapshot.layout_revision)
            }) {
                self.chain.continuity.broken();
                return;
            }
        }
        if header.gop_id < self.latest_gop {
            if self
                .gop
                .as_ref()
                .is_some_and(|gop| header.gop_id >= gop.first)
            {
                self.chain.continuity.broken();
            }
            return;
        }
        let fresh = header.gop_id > self.latest_gop || self.last_record.is_none();
        let first_of_frame = self.last_record != Some((header.gop_id, record.frame_id));
        self.last_record = Some((header.gop_id, record.frame_id));
        let keyframe = record.flags & FLAG_KEYFRAME != 0;
        if fresh && !self.stream_start(&header, &record) {
            return;
        }
        if !fresh && keyframe && first_of_frame {
            self.chain.continuity.start(&record);
        }
        if self.broken == Some(header.gop_id) {
            if !keyframe
                || self
                    .broken_frame
                    .is_some_and(|frame| record.frame_id <= frame)
            {
                self.chain.continuity.broken();
                // The group goes on without its key frame: a request the
                // host took for answered (it named an older group, the host
                // had opened this one as a continuation) is asked again, at
                // most once per `KEYFRAME_RETRY_US` (`KeyframeRequester`).
                self.request_keyframe(KeyframeReason::Reset, header.gop_id);
                return;
            }
            self.broken = None;
            self.broken_frame = None;
            self.gop = None;
        }
        // Fix of 2026-11-18: the guard's silence counts from the first record
        // that goes to a decoder, not while records wait for a key frame.
        self.guard_record(header.codec);
        if self.reopens_at(keyframe, first_of_frame) {
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
                    self.chain.continuity.broken();
                    return;
                }
            },
        };
        let want_yuv = self.shared.prefers_yuv();
        if want_yuv != gop.yuv {
            gop.decoder.prefer_yuv(want_yuv);
            gop.yuv = want_yuv;
        }
        let want_gpu = self.shared.direct.wanted();
        if want_gpu != gop.gpu {
            gop.decoder.prefer_gpu(want_gpu);
            gop.gpu = want_gpu;
        }
        // Shown pictures' storage first: the decoder reads into it (V1).
        self.shared.local.pool.feed(gop.decoder.as_mut());
        let decode_started = Instant::now();
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
            // Pictures submitted after this one and still in the decoder (P3).
            Ok(Some((output, receipt, gop.pending.len())))
        })();
        self.chain.open_frame = record.has_more();
        match decoded {
            Ok(output) => {
                if let Some((output, receipt, lag)) = output {
                    // Station `decode` of the viewer (BK0).
                    let micros =
                        u64::try_from(decode_started.elapsed().as_micros()).unwrap_or(u64::MAX);
                    self.shared
                        .update(|info| info.latency.decode.record(micros));
                    if output.keyframe {
                        self.keyframes.on_keyframe();
                    }
                    self.guard_picture(header.codec, lag);
                    self.show(&mut gop, header.size, output.capture_ts_us, receipt);
                }
                self.gop = Some(gop);
            }
            Err(error) => {
                self.guard_failure(header.codec);
                // The failed picture was never shown; the shown one stays
                // without receipts until the requested key frame.
                self.shared.pause_receipts();
                self.chain.continuity.broken();
                self.broken = Some(header.gop_id);
                self.broken_frame = Some(record.frame_id);
                // A hybrid picture the decoder cannot place (its picture
                // came late or without its identity): „Automatisch“ leaves
                // the codec for this session (another codec first, the
                // tiles last; fix of 2026-11-18: before, straight to the
                // tiles), a named codec stays and shows the error, with the
                // reason in the window and the viewer log.
                if header.frame_contract.is_some_and(|c| c.hybrid_regions)
                    && DecoderFallback::from_error(&error).is_some()
                {
                    let reason = format!("kein passend zugeordnetes Hybridbild: {error}");
                    if self.leave_codec(header.codec, &reason) {
                        return;
                    }
                }
                self.request_keyframe(KeyframeReason::DecodeError, header.gop_id);
            }
        }
    }

    /// Hands the finished picture to the GUI thread.
    fn show(
        &mut self,
        gop: &mut Gop,
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
            // Left out by the preview's shortest gap (BK0: dropped pictures).
            info.latency.dropped += u64::from(!due);
        });
        if !due {
            return;
        }
        // P8: a D3D11VA picture goes to the presenter on this thread.
        if let Some(picture) = gop.gpu.then(|| gop.decoder.take_gpu_picture()).flatten() {
            self.seq += 1;
            self.last_shown = Some(Instant::now());
            if !super::screen_direct::deliver(&self.shared, &*self.send, picture, presented) {
                self.request_keyframe(KeyframeReason::DecodeError, gop.header.gop_id);
            }
            return;
        }
        if gop.gpu {
            // The presenter waits but this picture came as planes or BGRA.
            super::screen_direct::not_direct(&self.shared);
        }
        // Handed over, not copied (V1, BK9 U1); the storage comes back.
        let planes = if gop.yuv {
            gop.decoder.take_yuv_picture()
        } else {
            None
        };
        let data = match planes {
            Some(planes) => PictureData::Yuv(planes),
            None => PictureData::Bgra(gop.decoder.take_picture()),
        };
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
