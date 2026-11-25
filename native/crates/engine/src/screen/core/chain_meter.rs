//! The station times and chain facts of one host session (plan 2026-11-14
//! „Bildkette“ BK0): the pipeline, its stage on the blocking pool and the
//! session record into one shared meter; `Pipeline::second` turns it into
//! the `STATS` fields of `frame_timing.rs`, and `publish` copies them into
//! the session as `se screen sessions --json` shows it. The meter always
//! records (bounded windows of the latest samples); only the viewer's
//! `FRAME_TIMING` request lets the station times leave the host.
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::frame_timing::{CaptureFacts, ChainReport, HostStations, SkewedUs, StationUs};
use super::registry::SessionView;
use super::wire_control::HostStats;

/// Samples per station: the latest ones.
pub const STATION_WINDOW: usize = 1024;

/// The latest samples of one station (µs, signed for clock differences).
#[derive(Clone, Debug, Default)]
pub struct Window {
    samples: VecDeque<i64>,
}

impl Window {
    pub fn record(&mut self, value: i64) {
        if self.samples.len() == STATION_WINDOW {
            self.samples.pop_front();
        }
        self.samples.push_back(value);
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Nearest-rank p50 and p95; `None` without samples.
    pub fn percentiles(&self) -> Option<(i64, i64)> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted: Vec<i64> = self.samples.iter().copied().collect();
        sorted.sort_unstable();
        let rank = |share: f64| {
            let index = (share * sorted.len() as f64).ceil() as usize;
            sorted[index.clamp(1, sorted.len()) - 1]
        };
        Some((rank(0.50), rank(0.95)))
    }

    pub fn station(&self) -> Option<StationUs> {
        self.percentiles().map(|(p50, p95)| StationUs {
            p50_us: u64::try_from(p50).unwrap_or(0),
            p95_us: u64::try_from(p95).unwrap_or(0),
        })
    }

    pub fn skewed(&self) -> Option<SkewedUs> {
        self.percentiles()
            .map(|(p50_us, p95_us)| SkewedUs { p50_us, p95_us })
    }
}

/// A station the meter times in µs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Station {
    Capture,
    /// Colour conversion as its own call (`convert_clock`, r1).
    Convert,
    Encode,
    WorkerHandoff,
    Send,
    InputInject,
}

fn signed(micros: u64) -> i64 {
    i64::try_from(micros).unwrap_or(i64::MAX)
}

/// The facts of the session's rate for `ChainReport` (`Pipeline::second`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RateFacts {
    pub target_fps: u32,
    pub wanted_fps: u32,
    pub capacity_fps: Option<u32>,
    pub sent_fps: u32,
    pub target_bps: u64,
    pub ceiling_bps: u64,
    pub user_ceiling_bps: Option<u64>,
    pub back_pressure: bool,
    pub resets: u64,
    /// What holds the target (`RateController::limit`, plan 2026-11-17).
    pub limit: Option<super::rate_control::RateLimit>,
    /// The rate the encoder is given (`RateController::encoder_bps`).
    pub encoder_bps: Option<u64>,
    /// Whether the encoder keeps it (`RateController::honour`).
    pub honour: Option<super::rate_control::honour::HonourVerdict>,
}

impl RateFacts {
    /// What holds the cadence below a higher rate.
    pub fn fps_limit(&self) -> &'static str {
        if self
            .capacity_fps
            .is_some_and(|capacity| capacity < self.wanted_fps)
        {
            return "encode_time";
        }
        // Fewer pictures than the cadence: the line pushed back, or the
        // source delivered no change (a still screen sends nothing).
        if u64::from(self.sent_fps) * 4 < u64::from(self.target_fps) * 3 {
            return if self.back_pressure { "link" } else { "source" };
        }
        "viewer"
    }

    /// What holds the target bit rate: the rate control's own reason; a
    /// pipeline without one (before plan 2026-11-17) names the ceilings, and
    /// `link` for any target below them.
    pub fn rate_limit(&self) -> &'static str {
        if let Some(limit) = self.limit {
            return limit.key();
        }
        if self.target_bps < self.ceiling_bps {
            return "link";
        }
        match self.user_ceiling_bps {
            Some(user) if user <= self.ceiling_bps => "ceiling",
            _ => "codec_ceiling",
        }
    }
}

/// The meter of one session.
#[derive(Debug, Default)]
pub struct ChainMeter {
    requested: bool,
    reset_at: Option<std::time::Instant>,
    capture: Window,
    convert: Window,
    encode: Window,
    handoff: Window,
    send: Window,
    tick_jitter: Window,
    input_inject: Window,
    network_raw: Window,
    input_raw: Window,
    last_capture_ts: Option<u64>,
    facts: CaptureFacts,
    /// Plan 2026-11-17 P9: step and result code of the capture's fallback.
    fallback_detail: Option<super::capture_fallback::FallbackDetail>,
    /// Read-backs of the host process since it started (`None`: unknown).
    readbacks: Option<u64>,
    readbacks_base: Option<u64>,
    continued: u64,
}

/// The meter shared by pipeline, stage and session.
pub type SharedChain = Arc<Mutex<ChainMeter>>;

pub fn lock(chain: &SharedChain) -> MutexGuard<'_, ChainMeter> {
    chain.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ChainMeter {
    /// `FRAME_TIMING` on (resets the station times) or off.
    pub fn request(&mut self, on: bool) {
        if on {
            for window in [
                &mut self.capture,
                &mut self.convert,
                &mut self.encode,
                &mut self.handoff,
                &mut self.send,
                &mut self.tick_jitter,
                &mut self.input_inject,
                &mut self.network_raw,
                &mut self.input_raw,
            ] {
                *window = Window::default();
            }
        }
        self.requested = on;
    }

    /// Wire `FRAME_TIMING`: bounds window wipes so a viewer cannot erase the
    /// host's own station samples (`se screen sessions`) continuously.
    pub fn request_throttled(&mut self, on: bool) {
        if !on {
            self.requested = false;
            return;
        }
        if self
            .reset_at
            .is_none_or(|at| at.elapsed() >= std::time::Duration::from_secs(1))
        {
            self.reset_at = Some(std::time::Instant::now());
            self.request(true);
        } else {
            self.requested = true;
        }
    }

    pub fn requested(&self) -> bool {
        self.requested
    }

    pub fn record_us(&mut self, station: Station, micros: u64) {
        let window = match station {
            Station::Capture => &mut self.capture,
            Station::Convert => &mut self.convert,
            Station::Encode => &mut self.encode,
            Station::WorkerHandoff => &mut self.handoff,
            Station::Send => &mut self.send,
            Station::InputInject => &mut self.input_inject,
        };
        window.record(signed(micros));
    }

    /// A picture captured at `capture_ts_us` (host clock) at a cadence of
    /// `fps`: its distance from the nearest multiple of the interval after
    /// the previous capture (pictures without change are never captured
    /// as frames, so gaps of whole intervals are no jitter).
    pub fn on_capture_ts(&mut self, capture_ts_us: u64, fps: u32) {
        let previous = self.last_capture_ts.replace(capture_ts_us);
        let Some(previous) = previous.filter(|previous| *previous < capture_ts_us) else {
            return;
        };
        let interval = 1_000_000 / u64::from(fps.max(1));
        let gap = capture_ts_us - previous;
        let ticks = ((gap + interval / 2) / interval).max(1);
        let jitter = gap.abs_diff(ticks * interval);
        self.tick_jitter.record(signed(jitter));
    }

    /// `FRAME_ACK`: the frame left the host at `sent_host_us` and its last
    /// record arrived at `received_viewer_us` (viewer clock).
    pub fn on_network(&mut self, sent_host_us: u64, received_viewer_us: u64) {
        self.network_raw
            .record(signed(received_viewer_us).saturating_sub(signed(sent_host_us)));
    }

    /// An input probe sent at `probe_viewer_us` handled at `handled_host_us`.
    pub fn on_input_probe(&mut self, probe_viewer_us: u64, handled_host_us: u64) {
        self.input_raw
            .record(signed(handled_host_us).saturating_sub(signed(probe_viewer_us)));
    }

    /// What the capture reports of itself; unknown fields keep their value.
    pub fn set_facts(&mut self, facts: CaptureFacts) {
        self.facts = self.facts.merged(facts);
    }

    /// The step and result code of the capture's fallback (P9); `None` once
    /// the capture is back on its first path.
    pub fn set_fallback_detail(&mut self, detail: Option<super::capture_fallback::FallbackDetail>) {
        self.fallback_detail = detail;
    }

    /// The read-back count of the host process (cumulative).
    pub fn set_readbacks(&mut self, total: u64) {
        self.readbacks_base.get_or_insert(total);
        self.readbacks = Some(total);
    }

    /// A reset recovered by continuing the encoder's chain (B-36).
    pub fn on_continued(&mut self) {
        self.continued += 1;
    }

    /// Station times measured so far; `None` without any.
    pub fn stations(&self) -> Option<HostStations> {
        let stations = HostStations {
            capture: self.capture.station(),
            convert: self.convert.station(),
            encode: self.encode.station(),
            worker_handoff: self.handoff.station(),
            send: self.send.station(),
            tick_jitter: self.tick_jitter.station(),
            input_inject: self.input_inject.station(),
            network_raw: self.network_raw.skewed(),
            input_raw: self.input_raw.skewed(),
        };
        (!stations.is_empty()).then_some(stations)
    }

    /// The stages and limits of the session.
    pub fn report(&self, rates: &RateFacts) -> ChainReport {
        let facts = self.facts;
        let convert = match (facts.on_gpu, self.readbacks_delta()) {
            (Some(true), Some(0) | None) => Some("gpu"),
            (Some(_), _) => Some("cpu"),
            (None, _) => None,
        };
        ChainReport {
            capture_path: facts.path.map(|path| path.key().to_string()),
            // P9: a CPU-pixel reason only beside `convert cpu`; a Windows
            // capture fallback with its failed step and result code, gone
            // once the capture is back on Desktop Duplication.
            capture_fallback: shown_fallback(facts.fallback, convert)
                .filter(|cause| !super::capture_fallback::fallback_ended(*cause, facts.path))
                .map(|cause| {
                    let detail = (!cause.is_conversion())
                        .then_some(self.fallback_detail)
                        .flatten();
                    super::capture_fallback::cause_text(cause.key(), detail)
                }),
            convert: convert.map(str::to_string),
            capture_rate_capped: facts.rate_capped,
            cursor_embedded: facts.cursor_embedded,
            readbacks: self.readbacks_delta(),
            recoveries_continued: Some(self.continued),
            recoveries_keyframe: Some(rates.resets.saturating_sub(self.continued)),
            target_fps: Some(rates.target_fps),
            wanted_fps: Some(rates.wanted_fps),
            encode_capacity_fps: rates.capacity_fps,
            fps_limit: Some(rates.fps_limit().to_string()),
            rate_limit: Some(rates.rate_limit().to_string()),
            rate_ceiling_kbps: rates.user_ceiling_bps.map(|bps| bps / 1000),
            encoder_rate_kbps: rates.encoder_bps.map(|bps| bps / 1000),
            rate_honour: rates.honour.map(|verdict| verdict.key().to_string()),
            virtual_display: None,
            // The host session fills them (`bandwidth_probe::LineLink::report`).
            measured_kbps: None,
            measured_age_ms: None,
            delivery: None,
        }
    }

    fn readbacks_delta(&self) -> Option<u64> {
        Some(self.readbacks?.saturating_sub(self.readbacks_base?))
    }
}

/// The fallback the report shows: a cause of the conversion (P9, why a
/// picture went as CPU pixels) only while the pictures do; a picture on the
/// GPU leaves an older one behind, as the facts keep what a newer mark does
/// not name.
pub(crate) fn shown_fallback(
    fallback: Option<super::frame_timing::CaptureFallback>,
    convert: Option<&str>,
) -> Option<super::frame_timing::CaptureFallback> {
    fallback.filter(|cause| !cause.is_conversion() || convert != Some("gpu"))
}

/// Copies the chain facts of `stats` into the session `view` (`se screen
/// sessions --json`) and leaves the station times in `stats` only when the
/// viewer asked for them. The raw clock differences stay out of the view:
/// without the viewer's clock offset they mean nothing on the host. A
/// session of the test picture names it as its capture path.
pub fn publish(view: &mut SessionView, stats: &mut HostStats, requested: bool) {
    // P5: the display this session's viewer asked for.
    if let Some(chain) = stats.chain.as_mut() {
        chain.virtual_display = super::virtual_display_host::chain_key(&view.id);
    }
    if view.source == "synthetic" {
        if let Some(chain) = stats.chain.as_mut() {
            chain.capture_path = Some(super::frame_timing::CapturePath::TestPicture.key().into());
            chain.capture_fallback = None;
        }
    }
    view.chain = stats.chain.clone();
    view.stations = stats.stations.clone().map(|mut stations| {
        stations.network_raw = None;
        stations.input_raw = None;
        stations
    });
    if !requested {
        stats.stations = None;
    }
}

#[cfg(test)]
#[path = "bildkette_stats_task_tests.rs"]
mod tests;
