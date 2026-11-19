//! The rate model of a picture stream (plan 2026-11-17, user of 2026-11-17:
//! "latency below 20 ms like Shadow PC", a sharp picture, „av1 ist zu viel.
//! punkt.“ after AV1 reached about 84 Mbit/s, „h.264 capped bei 800kbit/s
//! … und ruckelt … nicht ok“). Only numbers and decisions: no clock, no
//! connection, no encoder.
//!
//! - Start (`start_table_bps`): the default bit rate of Moonlight
//!   (moonlight-qt `StreamingPreferences::getDefaultBitrate`, read on
//!   2026-11-17): 1, 2, 5, 10, 20 and 40 Mbit/s for 360p, 480p, 720p, 1080p,
//!   1440p and 2160p at 30 frames/s, linear between two rows and never above
//!   the last one; times frames/30 up to 60 frames/s and √(fps/60) × 2 above;
//!   doubled for 4:4:4. Unlike Moonlight the value is not rounded to whole
//!   Mbit/s (1 frame/s would round to 0). Then times the share of the codec
//!   (`codec_share`: H.264 1.0, HEVC 0.7, AV1 0.6, the values of the plan;
//!   the lossless tiles take the H.264 value, their encoder ignores it).
//! - Session start (`session_start_bps`): the rate measured for the peer and
//!   path when one is remembered, else the table, on the relay at most
//!   `START_RELAY_BPS` (the relay guard applies to the unmeasured start only,
//!   a remembered relay rate is a measurement of that path), within the
//!   ceilings.
//! - Growth (`grown`) and decrease (`decreased`): a sender that fills the
//!   target grows by at most 1.25 per interval (2 during the start ramp) and
//!   to at most 1.5 × what was delivered + 10 kbit/s (libwebrtc
//!   `AimdRateControl`, GCC draft-ietf-rmcat-gcc-02 5.5); during the ramp to
//!   at most twice what was delivered (slow start, RFC 5681 3.1). An
//!   application-limited sender never grows above the larger of the table
//!   and that delivery bound. A persistent overuse lowers a filling sender
//!   to 0.85 × the receive rate and an application-limited one to 0.85 × its
//!   current target, never to the rate of its content.
//! - Encoder budgets (`frame_budget_bytes`, `mf_buffer_bytes`): the one-frame
//!   buffer of Media Foundation is set in bytes (Microsoft,
//!   `CODECAPI_AVEncCommonBufferSize`: "For H.264 video … the size of the
//!   buffer is in bytes"; the H.265 encoder page says the same).
//! - `RateLimit`: what holds the target (`classify`).
//! - Plan 2026-11-17 P9, sharp mode: the encoders of the GPU take a constant
//!   quality (`SHARP_TARGET_QUALITY`, the CQ/QVBR level 18) with the session
//!   rate as their ceiling instead of a constant bit rate; the host process
//!   holds the target of its one session (`set_session_quality`) and the
//!   encoder factories read it when they open (`session_quality`). Media
//!   Foundation takes no quality target: its sharp mode is peak-constrained
//!   VBR at the session rate (`os/windows/mf_rate.rs`, review 2026-11-19).
use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

use super::{MIN_BITRATE_BPS, START_RELAY_BPS};
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::pixel_convert::FrameSize;

/// Moonlight's table: pixels and Mbit/s at 30 frames/s.
const RESOLUTION_TABLE: [(u64, f64); 6] = [
    (640 * 360, 1.0),
    (854 * 480, 2.0),
    (1280 * 720, 5.0),
    (1920 * 1080, 10.0),
    (2560 * 1440, 20.0),
    (3840 * 2160, 40.0),
];
/// Growth per interval after the start ramp.
pub const INTERVAL_GROWTH: f64 = 1.25;
/// Share of the delivered rate a sender may grow to (libwebrtc, GCC).
pub const DELIVERY_GROWTH: f64 = 1.5;
/// Added to the delivery bound (libwebrtc `AimdRateControl::ClampBitrate`).
pub const DELIVERY_ALLOWANCE_BPS: f64 = 10_000.0;
/// What a persistent overuse leaves (C3).
pub const DECREASE_FACTOR: f64 = 0.85;

/// Mbit/s of Moonlight's table for `pixels` at 30 frames/s.
pub fn resolution_factor(pixels: u64) -> f64 {
    let (first_pixels, first_factor) = RESOLUTION_TABLE[0];
    if pixels <= first_pixels {
        return first_factor;
    }
    for pair in RESOLUTION_TABLE.windows(2) {
        let ((low, low_factor), (high, high_factor)) = (pair[0], pair[1]);
        if pixels <= high {
            let share = (pixels - low) as f64 / (high - low) as f64;
            return low_factor + share * (high_factor - low_factor);
        }
    }
    RESOLUTION_TABLE[RESOLUTION_TABLE.len() - 1].1
}

/// Moonlight's frame-rate factor: fps/30 up to 60, √(fps/60) × 60/30 above.
pub fn frame_rate_factor(fps: u32) -> f64 {
    let fps = f64::from(fps.max(1));
    if fps <= 60.0 {
        fps / 30.0
    } else {
        (fps / 60.0).sqrt() * 60.0 / 30.0
    }
}

/// The share of H.264's rate a codec needs for the same picture.
pub fn codec_share(codec: CodecKind) -> f64 {
    match codec {
        CodecKind::H264 | CodecKind::Tiles => 1.0,
        CodecKind::Hevc => 0.7,
        CodecKind::Av1 => 0.6,
    }
}

/// The start rate of a stream of `size` at `fps` in `codec`, never below
/// `MIN_BITRATE_BPS`.
pub fn start_table_bps(size: FrameSize, fps: u32, codec: CodecKind, chroma_444: bool) -> u64 {
    let chroma = if chroma_444 { 2.0 } else { 1.0 };
    let mbps = resolution_factor(size.pixels()) * chroma * frame_rate_factor(fps);
    let bps = (mbps * 1_000_000.0 * codec_share(codec)).round();
    (bps as u64).max(MIN_BITRATE_BPS)
}

/// The start of a session: `remembered` when the peer and path have one,
/// else `table` (on the relay at most `START_RELAY_BPS`), else the old start
/// of the path; at most `ceiling_bps`, at least `MIN_BITRATE_BPS`.
pub fn session_start_bps(
    table: Option<u64>,
    remembered: Option<u64>,
    relay: bool,
    path_start_bps: u64,
    ceiling_bps: u64,
) -> u64 {
    let unmeasured = table.unwrap_or(path_start_bps);
    let unmeasured = if relay {
        unmeasured.min(START_RELAY_BPS)
    } else {
        unmeasured
    };
    remembered
        .unwrap_or(unmeasured)
        .min(ceiling_bps)
        .max(MIN_BITRATE_BPS)
}

/// 1.5 × `delivered_bps` + 10 kbit/s.
pub fn delivery_cap_bps(delivered_bps: u64) -> f64 {
    DELIVERY_GROWTH * delivered_bps as f64 + DELIVERY_ALLOWANCE_BPS
}

/// What one growth step sees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrowthFacts {
    pub current: f64,
    /// 2 during the start ramp, else `INTERVAL_GROWTH`.
    pub factor: f64,
    pub ramping: bool,
    pub app_limited: bool,
    /// Receive rate of the last second.
    pub delivered: Option<u64>,
    /// `start_table_bps` of the stream, `None` before the pipeline names it.
    pub table: Option<u64>,
    pub back_pressure: bool,
}

/// The target after one growth step; `None` holds it. Never below `current`.
pub fn grown(facts: GrowthFacts) -> Option<f64> {
    let wanted = facts.current * facts.factor;
    let cap = if facts.app_limited {
        if facts.back_pressure {
            return None;
        }
        let table = facts.table.map_or(0.0, |table| table as f64);
        table.max(facts.delivered.map_or(0.0, delivery_cap_bps))
    } else {
        let delivered = facts.delivered?;
        if facts.ramping {
            // Slow start: at most twice what was delivered.
            facts.factor * delivered as f64
        } else {
            delivery_cap_bps(delivered)
        }
    };
    Some(wanted.min(cap).max(facts.current))
}

/// The target after a persistent overuse: 0.85 × the receive rate of a
/// sender that filled the target, 0.85 × the target of one that did not.
pub fn decreased(current: f64, app_limited: bool, received: Option<u64>) -> f64 {
    let base = if app_limited {
        current
    } else {
        received.map_or(current, |rate| rate as f64)
    };
    (DECREASE_FACTOR * base).min(current)
}

/// The bytes a constant-rate encoder spends on one frame at `bps` and `fps`.
pub fn frame_budget_bytes(bps: u64, fps: u32) -> u64 {
    (bps / 8 / u64::from(fps.max(1))).max(1)
}

/// `CODECAPI_AVEncCommonBufferSize` of one frame in bytes.
pub fn mf_buffer_bytes(bps: u32, fps: u32) -> u32 {
    (u64::from(bps) / 8 / u64::from(fps.max(1))).clamp(1, u64::from(u32::MAX)) as u32
}

/// What holds the target bit rate (`STATS` `chain.rate_limit`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateLimit {
    /// The start ramp still runs.
    Ramp,
    /// The content does not fill the target (application-limited).
    Content,
    /// The line: an overuse, loss or a stale stream lowered the target.
    Link,
    /// The viewer's „Höchste Datenrate“.
    Ceiling,
    /// The codec pairing's ceiling.
    CodecCeiling,
    /// The ceiling the encoder reports of itself.
    EncoderCeiling,
    /// The encoder does not keep the rate: se commands it less.
    EncoderIgnores,
}

impl RateLimit {
    pub fn key(self) -> &'static str {
        match self {
            Self::Ramp => "ramp",
            Self::Content => "content",
            Self::Link => "link",
            Self::Ceiling => "ceiling",
            Self::CodecCeiling => "codec_ceiling",
            Self::EncoderCeiling => "encoder_ceiling",
            Self::EncoderIgnores => "encoder_ignores",
        }
    }
}

/// What `classify` decides from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LimitFacts {
    pub target_bps: u64,
    pub user_ceiling_bps: Option<u64>,
    pub codec_ceiling_bps: Option<u64>,
    pub encoder_ceiling_bps: Option<u64>,
    pub ramping: bool,
    pub app_limited: bool,
    /// The encoder exceeded its budget: se corrects or replaces it.
    pub encoder_corrected: bool,
}

/// The first holding reason in this order: the encoder, the viewer's
/// ceiling, the codec's, the encoder's own, the ramp, the content, the line.
pub fn classify(facts: &LimitFacts) -> RateLimit {
    let reached = |ceiling: Option<u64>| ceiling.is_some_and(|bps| facts.target_bps >= bps);
    let codec = facts.codec_ceiling_bps.unwrap_or(u64::MAX);
    if facts.encoder_corrected {
        RateLimit::EncoderIgnores
    } else if reached(facts.user_ceiling_bps) && facts.user_ceiling_bps <= Some(codec) {
        RateLimit::Ceiling
    } else if reached(facts.codec_ceiling_bps) {
        RateLimit::CodecCeiling
    } else if reached(facts.encoder_ceiling_bps) {
        RateLimit::EncoderCeiling
    } else if facts.ramping {
        RateLimit::Ramp
    } else if facts.app_limited {
        RateLimit::Content
    } else {
        RateLimit::Link
    }
}

/// The constant quality of sharp mode on the encoders of the GPU (NVENC
/// `targetQuality`, AMF `QvbrQualityLevel`; both 1–51, lower is better).
pub const SHARP_TARGET_QUALITY: u8 = 18;

/// The quality target of this host process's session; 0: none (constant
/// bit rate). One host process serves one session.
static SESSION_QUALITY: AtomicU8 = AtomicU8::new(0);

/// The session now wants `target` (sharp mode) or a constant bit rate.
pub fn set_session_quality(target: Option<u8>) {
    SESSION_QUALITY.store(target.unwrap_or(0).min(51), Ordering::Relaxed);
}

/// The quality target an encoder of this process opens with.
pub fn session_quality() -> Option<u8> {
    Some(SESSION_QUALITY.load(Ordering::Relaxed)).filter(|target| *target > 0)
}
