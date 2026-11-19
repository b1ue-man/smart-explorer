//! Whether an encoder keeps the bit rate se gives it (plan 2026-11-17, user:
//! "every encoder must provably honour the rate se sets"). A constant-rate
//! encoder with a buffer of one frame spends at most its frame budget
//! (`model::frame_budget_bytes`) on a frame whose content fills it; one that
//! spends far more ignores the rate (AV1 reached about 84 Mbit/s, an MFT
//! whose buffer was set in bits instead of bytes made bursts of eight frames).
//!
//! `HonourMeter` takes the non-key frames that fill at least half of the
//! budget the encoder was given (smaller ones say nothing about the rate)
//! and judges every `HONOUR_WINDOW` of them by the median of their size over
//! the budget of se's target:
//! - up to `HONOURED_MAX` honoured, up to `CORRECT_ABOVE` tolerated;
//! - above it the session's meter (`correcting`) commands the encoder
//!   `target / ratio` (the correction stays within `MIN_CORRECTION..=1` and
//!   grows back by `RELAX_STEP` once frames stay below `RELAX_BELOW`);
//! - above `DEMOTE_ABOVE`, a refused or misread rate (`EncoderRateState`),
//!   or a correction at its floor that still does not hold: the encoder
//!   ignores the rate. The stage's watch (`watching`, `rate_honour_guard.rs`)
//!   then gives the encoder up through the refused list of its stage, so the
//!   next encoder of the same codec opens with a key frame.
//!
//! The session's meter runs in the pipeline (`RateController::on_encoded`),
//! the watch next to the encoder; both see the same frames.
//!
//! Review 2026-11-19: at the user's „Höchste Datenrate“ nothing above it is
//! tolerated: while se's target is that ceiling (`set_bound`), the
//! session's meter corrects every window above `HONOURED_LIMIT_AT_CEILING`
//! instead of tolerating up to `CORRECT_ABOVE`. Key frames stay out of the
//! judgement (a key frame is several frame budgets by design); the byte
//! budget of the stream books them and every other byte against the ceiling
//! (`rate_tiles.rs`).
use serde::{Deserialize, Serialize};

use super::model::{frame_budget_bytes, LimitFacts};
use super::{RateController, MIN_BITRATE_BPS};

#[path = "rate_honour_guard.rs"]
mod watch;
pub use watch::{guard, RateGuard};

/// Frames of one judgement: about one second at 60 frames/s.
pub const HONOUR_WINDOW: usize = 60;
/// A frame of at least this share of its budget fills it.
pub const FILL_SHARE: f64 = 0.5;
pub const HONOURED_MAX: f64 = 1.3;
pub const CORRECT_ABOVE: f64 = 1.5;
/// While se's target is the user's ceiling: above this the session's meter
/// corrects (frames of the budget are the ceiling).
pub const HONOURED_LIMIT_AT_CEILING: f64 = 1.0;
pub const DEMOTE_ABOVE: f64 = 4.0;
pub const MIN_CORRECTION: f64 = 0.25;
/// A corrected encoder whose frames stay below this share of se's budget
/// gets `RELAX_STEP` more again.
pub const RELAX_BELOW: f64 = 0.8;
pub const RELAX_STEP: f64 = 1.25;
/// Weight of a new gap in the smoothed frame interval (`FrameInterval`).
pub const INTERVAL_WEIGHT: f64 = 0.125;

/// The smoothed interval between the frames an encoder delivers (P9). An
/// encoder opened for `fps` gets fewer frames when the cadence sinks or the
/// queue guard drops some (B-03 keeps it open); a frame then carries the
/// target over the longer interval, and judged by the budget of the open
/// rate it looked like an encoder that ignores its rate: at 60 frames/s
/// opened and 15 delivered a ratio of 4, a correction to the floor (0.25 ×
/// the target, 800 kbit/s of an H.264 target of 3.2 Mbit/s) and a demotion.
/// The budget therefore counts the measured interval when it is the longer
/// one (`budget_fps`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameInterval {
    last_us: Option<u64>,
    smoothed_us: Option<f64>,
}

impl FrameInterval {
    /// A frame delivered at `now_us`; a gap counts up to one second.
    pub fn on_frame(&mut self, now_us: u64) {
        if let Some(last) = self.last_us {
            let gap = now_us.saturating_sub(last).min(1_000_000) as f64;
            self.smoothed_us = Some(match self.smoothed_us {
                Some(smoothed) => smoothed + INTERVAL_WEIGHT * (gap - smoothed),
                None => gap,
            });
        }
        self.last_us = Some(now_us);
    }

    /// The frame rate of the budget: `fps`, at most the measured one.
    pub fn budget_fps(&self, fps: u32) -> u32 {
        let fps = fps.max(1);
        match self.smoothed_us {
            Some(smoothed) if smoothed >= 1.0 => {
                let measured = (1_000_000.0 / smoothed).floor().max(1.0);
                // `as` saturates a float above `u32::MAX`.
                fps.min(measured as u32)
            }
            _ => fps,
        }
    }
}

/// The judgement of the last window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HonourVerdict {
    /// Fewer than `HONOUR_WINDOW` frames that fill their budget so far.
    Pending,
    Honoured,
    Tolerated,
    /// se commands the encoder less than its target.
    Corrected,
    /// The encoder does not keep the rate; it is given up where another
    /// encoder of the codec is left.
    Ignores,
}

impl HonourVerdict {
    pub fn key(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Honoured => "honoured",
            Self::Tolerated => "tolerated",
            Self::Corrected => "corrected",
            Self::Ignores => "ignores",
        }
    }
}

/// What an encoder made of the last rate it was given.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EncoderRateState {
    /// The bit rate it runs at, as it reads it back where it tells.
    pub applied_bps: u64,
    /// It refused the last rate, or reads back another one. The watch gives
    /// it up only while `applied_bps` lies above the commanded rate (review
    /// 2026-11-19, X1).
    pub refused: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HonourMeter {
    correcting: bool,
    ratios: Vec<f64>,
    correction: f64,
    verdict: HonourVerdict,
    ratio: Option<f64>,
    /// se's target is the user's ceiling (`set_bound`).
    bound: bool,
}

impl HonourMeter {
    /// The session's meter: corrects the rate the encoder is given.
    pub fn correcting() -> Self {
        Self::new(true)
    }

    /// The watch next to the encoder: only decides whether it ignores.
    pub fn watching() -> Self {
        Self::new(false)
    }

    fn new(correcting: bool) -> Self {
        Self {
            correcting,
            ratios: Vec::with_capacity(HONOUR_WINDOW),
            correction: 1.0,
            verdict: HonourVerdict::Pending,
            ratio: None,
            bound: false,
        }
    }

    /// Whether se's target is the user's ceiling: a correcting meter then
    /// tolerates nothing above it (review 2026-11-19).
    pub fn set_bound(&mut self, bound: bool) {
        self.bound = bound;
    }

    /// One encoded frame of `bytes`: se's target was `target_bps`, the
    /// encoder was given `commanded_bps` at `fps`. The verdict after it.
    pub fn on_frame(
        &mut self,
        bytes: u64,
        keyframe: bool,
        target_bps: u64,
        commanded_bps: u64,
        fps: u32,
    ) -> HonourVerdict {
        let given = frame_budget_bytes(commanded_bps, fps) as f64;
        if keyframe || (bytes as f64) < FILL_SHARE * given {
            return self.verdict;
        }
        let budget = frame_budget_bytes(target_bps, fps) as f64;
        self.ratios.push(bytes as f64 / budget);
        if self.ratios.len() < HONOUR_WINDOW {
            return self.verdict;
        }
        self.ratios.sort_by(f64::total_cmp);
        let ratio = self.ratios[self.ratios.len() / 2];
        self.ratios.clear();
        self.judge(ratio)
    }

    fn judge(&mut self, ratio: f64) -> HonourVerdict {
        self.ratio = Some(ratio);
        let floor = self.correcting && self.correction <= MIN_CORRECTION;
        let at_ceiling = self.correcting && self.bound && ratio > HONOURED_LIMIT_AT_CEILING;
        self.verdict = if ratio > DEMOTE_ABOVE || (floor && ratio > CORRECT_ABOVE) {
            HonourVerdict::Ignores
        } else if ratio > CORRECT_ABOVE || at_ceiling {
            HonourVerdict::Corrected
        } else if ratio > HONOURED_MAX {
            HonourVerdict::Tolerated
        } else {
            HonourVerdict::Honoured
        };
        if self.correcting {
            self.correction = match self.verdict {
                HonourVerdict::Ignores => MIN_CORRECTION,
                HonourVerdict::Corrected => (self.correction / ratio).max(MIN_CORRECTION),
                HonourVerdict::Honoured if ratio < RELAX_BELOW => {
                    (self.correction * RELAX_STEP).min(1.0)
                }
                _ => self.correction,
            };
        }
        self.verdict
    }

    /// The encoder refused its rate or reads back another one.
    pub fn on_refused(&mut self) {
        self.verdict = HonourVerdict::Ignores;
        if self.correcting {
            self.correction = MIN_CORRECTION;
        }
    }

    /// The verdict of the last window.
    pub fn verdict(&self) -> HonourVerdict {
        self.verdict
    }

    /// The verdict as `STATS` shows it: corrected while the correction
    /// holds, even when the corrected rate is kept.
    pub fn state(&self) -> HonourVerdict {
        match self.verdict {
            HonourVerdict::Honoured | HonourVerdict::Tolerated if self.correction < 1.0 => {
                HonourVerdict::Corrected
            }
            verdict => verdict,
        }
    }

    /// The share of se's target the encoder is given (`MIN_CORRECTION..=1`).
    pub fn correction(&self) -> f64 {
        self.correction
    }

    /// The median frame size over se's budget of the last window.
    pub fn ratio(&self) -> Option<f64> {
        self.ratio
    }

    /// The rate to give the encoder for se's `target_bps`.
    pub fn commanded_bps(&self, target_bps: u64) -> u64 {
        let commanded = (target_bps as f64 * self.correction).round() as u64;
        commanded.clamp(MIN_BITRATE_BPS.min(target_bps), target_bps)
    }
}

/// The session's meter and the encoder it judges.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionHonour {
    meter: HonourMeter,
    encoder: Option<String>,
    /// The interval of the stream's encoded frames (P9).
    interval: FrameInterval,
}

impl Default for SessionHonour {
    fn default() -> Self {
        Self {
            meter: HonourMeter::correcting(),
            encoder: None,
            interval: FrameInterval::default(),
        }
    }
}

impl SessionHonour {
    /// A frame of the stream was encoded at `now_us` (`on_encode_at`).
    pub fn on_encoded_at(&mut self, now_us: u64) {
        self.interval.on_frame(now_us);
    }
}

/// The honour of the session's encoder (plan 2026-11-17).
impl RateController {
    /// The rate the encoder is given: the target times the correction.
    pub fn encoder_bps(&self) -> u64 {
        self.honour.meter.commanded_bps(self.target_bps())
    }

    /// One frame the stage encoded with `encoder` for se's `target_bps`,
    /// given `commanded_bps` at `fps`; the budget counts the measured frame
    /// interval when it is longer (`FrameInterval`, P9). Another encoder
    /// starts a new meter.
    pub fn on_encoded(
        &mut self,
        bytes: u64,
        keyframe: bool,
        target_bps: u64,
        commanded_bps: u64,
        fps: u32,
        encoder: &str,
    ) -> HonourVerdict {
        if self.honour.encoder.as_deref() != Some(encoder) {
            self.honour = SessionHonour {
                meter: HonourMeter::correcting(),
                encoder: Some(encoder.to_owned()),
                interval: self.honour.interval,
            };
        }
        let fps = self.honour.interval.budget_fps(fps);
        // Review 2026-11-19: at the user's ceiling nothing above it is
        // tolerated (`HONOURED_LIMIT_AT_CEILING`).
        let bound = self
            .user_picture_bps()
            .is_some_and(|ceiling| target_bps >= ceiling);
        self.honour.meter.set_bound(bound);
        self.honour
            .meter
            .on_frame(bytes, keyframe, target_bps, commanded_bps, fps)
    }

    /// The verdict on the session's encoder as `STATS` shows it.
    pub fn honour(&self) -> HonourVerdict {
        self.honour.meter.state()
    }

    /// What holds the target now (`model::classify`).
    pub fn limit(&self) -> super::model::RateLimit {
        let state = self.honour.meter.state();
        super::model::classify(&LimitFacts {
            target_bps: self.target_bps(),
            user_ceiling_bps: self.user_ceiling_bps,
            codec_ceiling_bps: self.codec_ceiling_bps,
            encoder_ceiling_bps: None,
            ramping: self.probe.ramping(),
            app_limited: self.app_limited_now,
            encoder_corrected: matches!(state, HonourVerdict::Corrected | HonourVerdict::Ignores),
        })
    }
}
