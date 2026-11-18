//! The watch next to an encoder (plan 2026-11-17, `rate_honour.rs`): every
//! encoder of a video codec that a stage opens (`CodecRegistry::
//! open_encoder_for_stage_admitted`, the host process and the daemon alike)
//! runs inside a `RateGuard`. It measures each frame against the rate the
//! encoder was given (`StageRequest::target_bps`, the session's target
//! times its correction) and asks the encoder whether it refused that rate
//! (`ScreenEncoder::rate_state`). An encoder that ignores the rate fails its
//! frame with `encoder_ignores_rate` when another encoder of the codec is
//! left (`can_demote`): every stage then puts its factory on the refused
//! list and opens the next one with a key frame (W4 B3, unchanged). Without
//! another encoder it keeps running at the session's correction.
use std::io;

use super::{FrameInterval, HonourMeter, HonourVerdict};
use crate::screen::core::capture_image::CapturedImage;
use crate::screen::core::capture_picture::Picture;
use crate::screen::core::host_backend::{CapturedEncodedFrame, EncodedFrame, ScreenEncoder};
use crate::screen::core::host_stage::StageRequest;
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::pixel_convert::{BgraImage, FrameSize};
use crate::screen::core::ref_recovery::Recovery;

/// `encoder` inside a watch; the lossless tiles keep no rate and stay bare.
pub fn guard(
    encoder: Box<dyn ScreenEncoder>,
    codec: CodecKind,
    can_demote: bool,
) -> Box<dyn ScreenEncoder> {
    if codec == CodecKind::Tiles {
        return encoder;
    }
    Box::new(RateGuard {
        inner: encoder,
        meter: HonourMeter::watching(),
        can_demote,
        interval: FrameInterval::default(),
        opened: std::time::Instant::now(),
    })
}

pub struct RateGuard {
    inner: Box<dyn ScreenEncoder>,
    meter: HonourMeter,
    /// Another encoder of the codec is left in the stage's rank.
    can_demote: bool,
    /// The interval of the frames it delivers (P9, `FrameInterval`).
    interval: FrameInterval,
    opened: std::time::Instant,
}

impl RateGuard {
    /// The verdict of the watch so far.
    pub fn verdict(&self) -> HonourVerdict {
        self.meter.verdict()
    }

    /// Counts one encoded frame; `Err` gives the encoder up.
    fn watch(
        &mut self,
        bytes: u64,
        keyframe: bool,
        commanded_bps: u64,
        fps: u32,
    ) -> io::Result<()> {
        let now_us = u64::try_from(self.opened.elapsed().as_micros()).unwrap_or(u64::MAX);
        self.interval.on_frame(now_us);
        let fps = self.interval.budget_fps(fps);
        self.meter
            .on_frame(bytes, keyframe, commanded_bps, commanded_bps, fps);
        if self.inner.rate_state().is_some_and(|state| state.refused) {
            self.meter.on_refused();
        }
        if self.can_demote && self.meter.verdict() == HonourVerdict::Ignores {
            let ratio = self
                .meter
                .ratio()
                .map_or("-".to_string(), |r| format!("{r:.2}"));
            return Err(io::Error::other(format!(
                "encoder_ignores_rate: {} (frame size / budget {ratio})",
                self.inner.name()
            )));
        }
        Ok(())
    }

    fn captured(
        &mut self,
        encoded: io::Result<CapturedEncodedFrame>,
        request: &StageRequest,
    ) -> io::Result<CapturedEncodedFrame> {
        let frame = encoded?;
        let bytes = frame.records.iter().map(|record| record.len() as u64).sum();
        self.watch(bytes, frame.keyframe, request.target_bps, request.fps)?;
        Ok(frame)
    }

    fn plain(
        &mut self,
        encoded: io::Result<EncodedFrame>,
        target_bps: u64,
        fps: u32,
    ) -> io::Result<EncodedFrame> {
        let frame = encoded?;
        let bytes = frame.records.iter().map(|record| record.len() as u64).sum();
        self.watch(bytes, frame.keyframe, target_bps, fps)?;
        Ok(frame)
    }
}

impl ScreenEncoder for RateGuard {
    fn encode_captured(
        &mut self,
        frame: &CapturedImage,
        request: &StageRequest,
    ) -> io::Result<CapturedEncodedFrame> {
        let encoded = self.inner.encode_captured(frame, request);
        self.captured(encoded, request)
    }

    fn encode_picture(
        &mut self,
        picture: &Picture,
        request: &StageRequest,
    ) -> io::Result<CapturedEncodedFrame> {
        let encoded = self.inner.encode_picture(picture, request);
        self.captured(encoded, request)
    }

    fn name(&self) -> String {
        self.inner.name()
    }

    fn size(&self) -> FrameSize {
        self.inner.size()
    }

    fn needs_refresh(&self) -> bool {
        self.inner.needs_refresh()
    }

    fn recover_references(&mut self, last_good: u64) -> Recovery {
        self.inner.recover_references(last_good)
    }

    fn rate_state(&self) -> Option<super::EncoderRateState> {
        self.inner.rate_state()
    }

    fn encode_frame(
        &mut self,
        frame: &BgraImage,
        request: &StageRequest,
    ) -> io::Result<EncodedFrame> {
        let encoded = self.inner.encode_frame(frame, request);
        self.plain(encoded, request.target_bps, request.fps)
    }

    /// Without a request the frame rate and so the budget per frame are
    /// unknown: the frame passes unwatched.
    fn encode(
        &mut self,
        frame: &BgraImage,
        keyframe: bool,
        target_bps: u64,
    ) -> io::Result<EncodedFrame> {
        self.inner.encode(frame, keyframe, target_bps)
    }
}
