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

/// The failure of a watched encoder that ignores its rate (review
/// 2026-11-19 L2): it reads as before (`encoder_ignores_rate: …`), and a
/// stage tells it from a passing failure (`RateGuard::gave_up`).
#[derive(Debug)]
struct IgnoresRate(String);

impl std::fmt::Display for IgnoresRate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for IgnoresRate {}

impl RateGuard {
    /// Whether `error` is the watch's verdict that the encoder ignores its
    /// rate (review 2026-11-19 L2): its factory goes onto the refused list
    /// at once; a fresh instance of it would start its watch from nothing
    /// and run another `HONOUR_WINDOW` of frames above the rate.
    pub fn gave_up(error: &io::Error) -> bool {
        error
            .get_ref()
            .is_some_and(|inner| inner.is::<IgnoresRate>())
    }

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
        let state = self.inner.rate_state();
        // Review 2026-11-19 (X3): an encoder that took a rate above the one
        // commanded (the smallest its device runs, `mf_rate::floored`) is
        // measured against the rate it runs; the stream's byte budget keeps
        // the ceiling.
        let measured = state
            .filter(|state| !state.refused && state.applied_bps > commanded_bps)
            .map_or(commanded_bps, |state| state.applied_bps);
        self.meter
            .on_frame(bytes, keyframe, measured, measured, fps);
        // X1: a refused rate gives the encoder up only while it runs above
        // the commanded one; a refused rise leaves it below and keeps the
        // ceiling (it was given up for the session before).
        // Codex round 5, finding 11: within 2 % above it the byte budget
        // keeps the ceiling (the band of the encoders' refused lowerings).
        let above = |state: &super::EncoderRateState| {
            u128::from(state.applied_bps) * 50 > u128::from(commanded_bps) * 51
        };
        if state.is_some_and(|state| state.refused && above(&state)) {
            self.meter.on_refused();
        }
        if self.can_demote && self.meter.verdict() == HonourVerdict::Ignores {
            let ratio = self
                .meter
                .ratio()
                .map_or("-".to_string(), |r| format!("{r:.2}"));
            return Err(io::Error::other(IgnoresRate(format!(
                "encoder_ignores_rate: {} (frame size / budget {ratio})",
                self.inner.name()
            ))));
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
