//! The measurements of one `se screen bench` run (moved out of `bench.rs`
//! for BK0, plan 2026-11-14 „Bildkette“): shown pictures with their
//! barcode, latency, rates, recoveries, the static block's PSNR and the
//! station samples. With the host's real capture (`--source`) there is no
//! barcode: every decoded picture counts as shown and no PSNR is taken.
//!
//! Plan 2026-11-17 „Windows-Bild“ P3: the barcode is read against the
//! picture the decoder says it gave (`decoded_frame`), not against the
//! record just passed in, so a decoder that answers one call late counts
//! late (`late_frames`), not corrupt; decode errors and decoders that did
//! not open are counted, and the report keeps every counter when no
//! picture arrived intact (`failure`).
use std::time::{Duration, Instant};

use super::super::decoded_frame::DecodedFrame;
use super::super::gop_stream::now_us;
use super::super::metrics::{psnr_db, ClockSync, LatencySamples, RateMeter, RecoveryTracker};
use super::super::negotiate::CodecKind;
use super::super::pixel_convert::{downscale_bgra, BgraImage, FrameSize, Rect};
use super::super::synthetic::{read_barcode, render, static_block, BarcodeRead};
use super::{BenchReport, BitrateKbps, LatencyMs, StageSamples};

/// Interval of the PSNR samples (a reference picture costs a render).
const PSNR_EVERY: Duration = Duration::from_secs(1);

/// The measurements of one bench run.
pub(super) struct Measure {
    pub(super) size: FrameSize,
    pub(super) clock: ClockSync,
    pub(super) latency: LatencySamples,
    pub(super) shown: RateMeter,
    pub(super) received: RateMeter,
    pub(super) recovery: RecoveryTracker,
    pub(super) highest_frame: u64,
    pub(super) keyframes: u64,
    /// A key frame was shown since the last look (answers the open request).
    pub(super) fresh_keyframe: bool,
    pub(super) corrupt: u64,
    /// Pictures that came out of a later decoder call than their own.
    pub(super) late: u64,
    pub(super) decode_errors: u64,
    pub(super) open_failures: u64,
    pub(super) psnr: Option<f64>,
    pub(super) psnr_at: Option<Instant>,
    pub(super) codec: Option<CodecKind>,
    pub(super) decoder: String,
    pub(super) stages: StageSamples,
    /// The host captures its screen, not the test picture (`--source`).
    pub(super) real: bool,
}

impl Measure {
    pub(super) fn new(size: FrameSize) -> Self {
        Self {
            size,
            clock: ClockSync::default(),
            latency: LatencySamples::default(),
            shown: RateMeter::default(),
            received: RateMeter::default(),
            recovery: RecoveryTracker::default(),
            highest_frame: 0,
            keyframes: 0,
            fresh_keyframe: false,
            corrupt: 0,
            late: 0,
            decode_errors: 0,
            open_failures: 0,
            psnr: None,
            psnr_at: None,
            codec: None,
            decoder: "tiles".into(),
            stages: StageSamples::default(),
            real: false,
        }
    }

    /// One decoded picture `frame` (the decoder's own identity of it),
    /// completed by the record of `submitted`, shown at `shown_us` (viewer
    /// clock); `local` pictures share the host clock.
    pub(super) fn shown(
        &mut self,
        picture: &BgraImage,
        frame: DecodedFrame,
        submitted: u64,
        shown_us: u64,
        local: bool,
    ) {
        if frame.frame_id < submitted {
            self.late += 1;
        }
        if !self.real {
            match read_barcode(&picture.view(), self.size) {
                Ok(BarcodeRead::Valid(id)) if id == frame.frame_id => {}
                _ => {
                    self.corrupt += 1;
                    self.recovery.on_disruption(shown_us);
                    return;
                }
            }
        }
        if frame.keyframe {
            self.keyframes += 1;
            self.fresh_keyframe = true;
        }
        self.recovery.on_good_frame(shown_us);
        self.shown.record(shown_us, 0);
        let latency = if local {
            Some(shown_us.saturating_sub(frame.capture_ts_us))
        } else {
            self.clock.latency_us(frame.capture_ts_us, shown_us)
        };
        if let Some(latency) = latency {
            self.latency.record(latency);
        }
        if self.real {
            return;
        }
        if self.psnr_at.is_none_or(|at| at.elapsed() >= PSNR_EVERY) {
            self.psnr_at = Some(Instant::now());
            if let Some(value) = static_psnr(picture, frame.frame_id, self.size) {
                self.psnr = Some(self.psnr.map_or(value, |lowest| lowest.min(value)));
            }
        }
    }

    /// The report; without any shown picture with `failure` set and every
    /// counter kept (P3).
    pub(super) fn report(&self, encoder: &str, path: &str) -> BenchReport {
        let now = now_us();
        let shown = self.shown.summary(now);
        let failure = (shown.frames == 0).then(|| {
            if self.real {
                "no picture of the host's screen arrived"
            } else {
                "no frame of the test picture arrived intact"
            }
            .to_string()
        });
        let bytes = self.received.summary(now);
        let latency = self.latency.summary();
        let milliseconds = |pick: fn(&super::super::metrics::LatencySummary) -> f64| {
            latency.as_ref().map_or(0.0, pick)
        };
        BenchReport {
            codec: self.codec.map_or("tiles", CodecKind::key).into(),
            encoder: encoder.into(),
            decoder: self.decoder.clone(),
            path: path.into(),
            frames_sent: self.highest_frame,
            frames_shown: shown.frames,
            fps_mean: shown.fps_mean,
            fps_min: shown.fps_min,
            latency_ms: LatencyMs {
                p50: milliseconds(|summary| summary.p50_ms),
                p95: milliseconds(|summary| summary.p95_ms),
                max: milliseconds(|summary| summary.max_ms),
            },
            bitrate_kbps: BitrateKbps {
                mean: bytes.kbps_mean,
                max: bytes.kbps_max,
            },
            keyframes: self.keyframes,
            recoveries: self.recovery.recoveries_ms(),
            corrupt_frames: self.corrupt,
            late_frames: self.late,
            decode_errors: self.decode_errors,
            decoder_open_failures: self.open_failures,
            static_psnr_db: self.psnr.filter(|value| value.is_finite()),
            stages_ms: self.stages.summary(),
            source: None,
            host_timing: None,
            host_chain: None,
            failure,
        }
    }
}

/// PSNR of the static block of `picture` against the reference of
/// `frame_id` drawn at the source size `source` and scaled like `picture`.
fn static_psnr(picture: &BgraImage, frame_id: u64, source: FrameSize) -> Option<f64> {
    let mut reference = BgraImage::new(source).ok()?;
    render(frame_id, &mut reference).ok()?;
    let size = picture.size();
    let reference = if size == source {
        reference
    } else {
        downscale_bgra(&reference.view(), size).ok()?
    };
    let block = static_block(source);
    let scale = |value: u32, from: u32, to: u32| {
        (u64::from(value) * u64::from(to) / u64::from(from.max(1))) as u32
    };
    let rect = Rect {
        x: scale(block.x, source.width, size.width),
        y: scale(block.y, source.height, size.height),
        width: scale(block.width, source.width, size.width).max(1),
        height: scale(block.height, source.height, size.height).max(1),
    };
    psnr_db(&picture.view(), &reference.view(), rect).ok()
}
