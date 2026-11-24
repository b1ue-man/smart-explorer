//! Frame rate and bit-rate ceiling of a picture stream (plan 2026-11-14
//! „Bildkette“, decisions B-02, B-03, B-06, B-07).
//!
//! The frame rate is the minimum of the rate the viewer wants (its display's
//! refresh rate, B-01), the rate of the source (a source without a new
//! picture answers `Unchanged` and costs no frame) and the encoding capacity
//! measured in this session: the 95th percentile of the time one frame took
//! from capture to encoded bytes over the last second (B-02). The bit rate
//! never sets the frame rate; it only sets the quantisation of the encoder.
//! A change of the frame rate keeps the running encoder (B-03): it was opened
//! with the highest allowed rate and simply gets fewer pictures.
//!
//! The ceiling of the bit rate comes from what the chosen encoder and decoder
//! report (B-06, B-07), not from a fixed value: for H.264 the level whose
//! macroblock rate both report, with its MaxBR (ITU-T H.264 (08/2021)
//! Table A-1) times the profile's cpbBrVclFactor (Table A-2); for the lossless
//! tile codec the size of an uncompressed BGRA picture per frame.
//!
//! `quality_for` and `adjust_quality` are the cadence of a v0.5.196 host
//! (frame rate = target / (pixels × 0.05 bit)). No host path uses them any
//! more; they stay for the tests that compare against that old behaviour.
use std::collections::VecDeque;

use super::{RateController, MIN_BITRATE_BPS};
use crate::screen::core::frame_size_caps::avc_limits;
use crate::screen::core::negotiate::{CodecChoice, CodecKind, H264Profile};
use crate::screen::core::pixel_convert::FrameSize;

/// Window of the measured encoding time: the same one second over which the
/// rate control measures the receive and send rates (decision C3).
pub const ENCODE_WINDOW_US: u64 = 1_000_000;
/// Percentile of the encoding time that limits the frame rate (B-02: p95).
const ENCODE_PERCENTILE: f64 = 0.95;
/// Entries of the window: one second at 240 frames/s, four times over (the
/// same bound as the rate windows of the rate control).
const MAX_ENCODE_ENTRIES: usize = 1024;

/// H.264 levels of ITU-T H.264 (08/2021) Table A-1 with their MaxBR in units
/// of cpbBrVclFactor bits per second. Level 1b (MaxBR 128) shares the
/// macroblock rate of level 1 and is not reported as a macroblock rate.
const AVC_MAX_BR: [(u32, u64); 19] = [
    (10, 64),
    (11, 192),
    (12, 384),
    (13, 768),
    (20, 2_000),
    (21, 4_000),
    (22, 4_000),
    (30, 10_000),
    (31, 14_000),
    (32, 20_000),
    (40, 20_000),
    (41, 50_000),
    (42, 50_000),
    (50, 135_000),
    (51, 240_000),
    (52, 240_000),
    (60, 240_000),
    (61, 480_000),
    (62, 800_000),
];
/// Bits per pixel of an uncompressed BGRA picture: the largest a lossless
/// tile frame can carry before its compression.
const BGRA_BITS_PER_PIXEL: u64 = 32;

/// Encoding times of the frames of the last second.
#[derive(Clone, Debug, Default)]
pub struct EncodeTiming {
    samples: VecDeque<(u64, u64)>,
}

impl EncodeTiming {
    /// One encoded frame that finished at `at_us` and took `elapsed_us`.
    pub fn push(&mut self, at_us: u64, elapsed_us: u64) {
        self.samples.push_back((at_us, elapsed_us.max(1)));
        while let Some(&(first, _)) = self.samples.front() {
            if at_us.saturating_sub(first) <= ENCODE_WINDOW_US
                && self.samples.len() <= MAX_ENCODE_ENTRIES
            {
                break;
            }
            self.samples.pop_front();
        }
    }

    /// The 95th percentile of the encoding time, `None` without a frame.
    pub fn p95_us(&self) -> Option<u64> {
        if self.samples.is_empty() {
            return None;
        }
        let mut times: Vec<u64> = self.samples.iter().map(|(_, elapsed)| *elapsed).collect();
        times.sort_unstable();
        let rank = (ENCODE_PERCENTILE * times.len() as f64).ceil() as usize;
        times.get(rank.saturating_sub(1)).copied()
    }

    /// Frames per second the chain from capture to encoded bytes carries.
    pub fn capacity_fps(&self) -> Option<u32> {
        let p95 = self.p95_us()?;
        Some(
            u32::try_from(1_000_000 / p95.max(1))
                .unwrap_or(u32::MAX)
                .max(1),
        )
    }
}

/// The frame rate of the next frames: what the viewer wants, at most what the
/// encoding carries; never below one frame per second.
pub fn cadence_fps(wanted: u32, capacity: Option<u32>) -> u32 {
    wanted.min(capacity.unwrap_or(wanted)).max(1)
}

/// MaxBR of the highest H.264 level whose macroblock rate is within
/// `macroblocks_per_second`, in bit/s for `profile`; the highest level of
/// Table A-1 when neither encoder nor decoder reports a rate.
pub fn avc_max_bitrate_bps(macroblocks_per_second: Option<u64>, profile: H264Profile) -> u64 {
    let level = match macroblocks_per_second {
        None => AVC_MAX_BR[AVC_MAX_BR.len() - 1],
        Some(rate) => AVC_MAX_BR
            .iter()
            .rev()
            .copied()
            .find(|(level, _)| {
                avc_limits(*level).is_some_and(|limits| limits.macroblocks_per_second <= rate)
            })
            .unwrap_or(AVC_MAX_BR[0]),
    };
    // cpbBrVclFactor (Table A-2): 1000 for Constrained Baseline, 1250 for High.
    let factor = match profile {
        H264Profile::ConstrainedBaseline => 1_000,
        H264Profile::High => 1_250,
    };
    level.1 * factor
}

/// MaxBR of the H.264 level `level` (`level_idc`, Table A-1) in bit/s for
/// `profile`; `None` for a level the table does not name. An encoder that
/// writes `level` into its stream keeps its rate below this (merge W2).
pub fn avc_level_max_bitrate_bps(level: u32, profile: H264Profile) -> Option<u64> {
    let (_, kbps) = AVC_MAX_BR
        .iter()
        .copied()
        .find(|(name, _)| *name == level)?;
    let factor = match profile {
        H264Profile::ConstrainedBaseline => 1_000,
        H264Profile::High => 1_250,
    };
    Some(kbps * factor)
}

/// The ceiling the codec pairing sets on the bit rate of `size` at `fps`.
pub fn codec_ceiling_bps(choice: &CodecChoice, size: FrameSize, fps: u32) -> u64 {
    match (choice.codec, choice.profile) {
        (CodecKind::H264, Some(profile)) => {
            avc_max_bitrate_bps(choice.max_macroblocks_per_second, profile)
        }
        (CodecKind::H264, None) => avc_max_bitrate_bps(
            choice.max_macroblocks_per_second,
            H264Profile::ConstrainedBaseline,
        ),
        // HEVC and AV1: MaxBR of the level the pairing reports (BK6,
        // `frame_size_hevc_av1.rs`), doubled for the 4:4:4 profile.
        (codec @ (CodecKind::Hevc | CodecKind::Av1), profile) => {
            crate::screen::core::frame_size_caps::video_max_bitrate_bps(
                codec,
                choice.max_macroblocks_per_second,
                codec.chroma_444(profile),
            )
        }
        (CodecKind::Tiles, _) => size
            .pixels()
            .max(1)
            .saturating_mul(BGRA_BITS_PER_PIXEL)
            .saturating_mul(u64::from(fps.max(1))),
    }
}

/// The ceiling of the target: the codec pairing (B-06) and the user (B-09).
impl RateController {
    /// The highest target: codec pairing and user, never below the floor
    /// (a user ceiling below it is held by the byte budget alone).
    pub fn ceiling_bps(&self) -> u64 {
        let codec = self.codec_ceiling_bps.unwrap_or(u64::MAX);
        // The pictures' share of the user's ceiling (the control stream
        // takes the rest, `user_picture_bps`).
        let user = self.user_picture_bps().unwrap_or(u64::MAX);
        codec.min(user).max(MIN_BITRATE_BPS)
    }

    /// „Höchste Datenrate“ of the viewer (B-09): `None` is automatic. Takes
    /// effect at once; a lower ceiling lowers the target with it, while the
    /// link estimate stays (plan 2026-11-17), so a raised ceiling returns to
    /// what the line was measured to carry. Review 2026-11-19: the ceiling
    /// is kept as given, also below `MIN_BITRATE_BPS` (a hand-edited
    /// `viewer.json` or another viewer sends that; it was raised to the
    /// floor without a word): the target E and the encoder stay at the
    /// floor, the byte budget of the stream holds the ceiling exactly
    /// (`stream_budget_bps`).
    pub fn set_ceiling(&mut self, ceiling_bps: Option<u64>) {
        self.user_ceiling_bps = ceiling_bps.map(|bps| bps.max(1));
        self.update_reserved();
    }

    pub fn user_ceiling_bps(&self) -> Option<u64> {
        self.user_ceiling_bps
    }

    /// What the codec pairing carries (B-06, `codec_ceiling_bps`).
    pub fn set_codec_ceiling(&mut self, ceiling_bps: u64) {
        self.codec_ceiling_bps = Some(ceiling_bps.max(MIN_BITRATE_BPS));
        self.delay_target = self.link_clamp(self.delay_target);
    }
}

/// Bits per pixel and frame of the v0.5.196 cadence (A1, not reproducible).
pub const BITS_PER_PIXEL: f64 = 0.05;
/// V1 configuration constant retained for consumers; native stills may use lower cadence.
pub const MIN_LADDER_FPS: u32 = 10;
/// The legacy preview edge; source pixels are no longer reduced by the rate controller.
pub const MIN_LONG_EDGE: u32 = 320;
/// Step of the v0.5.196 ladder, which restarted the encoder at every step.
const STEP_UP_HEADROOM: f64 = 1.25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quality {
    pub fps: u32,
    pub size: FrameSize,
}

impl Quality {
    fn load(self) -> u64 {
        self.size.pixels() * u64::from(self.fps)
    }
}

/// The v0.5.196 cadence: frame rate = target / (pixels × 0.05 bit).
pub fn quality_for(target_bps: u64, source: FrameSize, max_fps: u32) -> Quality {
    let max_fps = max_fps.max(1);
    let fps = (target_bps as f64 / (source.pixels().max(1) as f64 * BITS_PER_PIXEL)).floor() as u32;
    Quality {
        fps: fps.clamp(1, max_fps),
        size: source,
    }
}

/// The v0.5.196 ladder step from `current` (steps of at least 25 %).
pub fn adjust_quality(
    current: Quality,
    target_bps: u64,
    source: FrameSize,
    max_fps: u32,
) -> Quality {
    let fitting = quality_for(target_bps, source, max_fps);
    if current.size.width > source.width
        || current.size.height > source.height
        || fitting.load() as f64 * STEP_UP_HEADROOM <= current.load() as f64
    {
        return fitting;
    }
    let headroom = (target_bps as f64 / STEP_UP_HEADROOM) as u64;
    let better = quality_for(headroom, source, max_fps);
    let step = better.load() as f64 >= current.load() as f64 * STEP_UP_HEADROOM;
    if better.load() > current.load() && (step || better.fps == max_fps) {
        better
    } else {
        current
    }
}
