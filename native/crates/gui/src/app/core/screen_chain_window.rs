//! The viewer's own stations of a session (SCR-M7; plan 2026-11-14
//! „Bildkette“ BK0, moved out of `screen_session.rs`): capture-to-display
//! latency, decode time, the wait of a decoded picture until the drawing
//! thread takes it (the hand-over to the graphics), pictures dropped before
//! they were shown, pictures drawn again because no new one came (r1), and
//! when the host's station times were last asked for
//! (`FRAME_TIMING`, renewed every `TIMING_RENEW`, which also restarts the
//! host's window).
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Samples the latency figures of the statistics are taken from. It only
/// smooths the display: the figures follow the latest frames.
pub const LATENCY_WINDOW: usize = 200;

/// How often the viewer asks the host anew for its station times.
pub const TIMING_RENEW: Duration = Duration::from_secs(5);

/// The latest samples of one station in µs.
#[derive(Clone, Debug, Default)]
pub struct Samples {
    samples: VecDeque<u64>,
}

impl Samples {
    pub fn record(&mut self, micros: u64) {
        if self.samples.len() == LATENCY_WINDOW {
            self.samples.pop_front();
        }
        self.samples.push_back(micros);
    }

    /// Nearest-rank p50 and p95 in ms.
    pub fn percentiles_ms(&self) -> Option<(f64, f64)> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted: Vec<u64> = self.samples.iter().copied().collect();
        sorted.sort_unstable();
        let rank = |share: f64| {
            let index = (share * sorted.len() as f64).ceil() as usize;
            sorted[index.clamp(1, sorted.len()) - 1] as f64 / 1000.0
        };
        Some((rank(0.50), rank(0.95)))
    }
}

/// The latest capture-to-display times in µs, and the viewer's stations.
#[derive(Clone, Debug, Default)]
pub struct LatencyWindow {
    latency: Samples,
    /// Decoder calls that completed a picture.
    pub decode: Samples,
    /// A decoded picture waited this long until the drawing thread took it.
    pub handover: Samples,
    /// Decoded pictures replaced before the drawing thread took them, or
    /// left out by the preview's shortest gap.
    pub dropped: u64,
    /// Paints of the drawing thread that showed the previous picture again
    /// because no new one came (r1).
    pub repeated: u64,
    put_at: Option<Instant>,
    timing_asked: Option<Instant>,
}

impl LatencyWindow {
    pub fn record(&mut self, latency_us: u64) {
        self.latency.record(latency_us);
    }

    /// Nearest-rank p50 and p95 of the latency in ms.
    pub fn percentiles_ms(&self) -> Option<(f64, f64)> {
        self.latency.percentiles_ms()
    }

    /// A decoded picture went into the slot; `unshown` when it replaced one
    /// the drawing thread never took.
    pub fn on_put(&mut self, unshown: bool) {
        self.dropped += u64::from(unshown);
        self.put_at = Some(Instant::now());
    }

    /// The drawing thread drew the shown picture again: no new one came.
    pub fn on_repeated(&mut self) {
        self.repeated += 1;
    }

    /// The drawing thread took the picture of the slot.
    pub fn on_taken(&mut self) {
        if let Some(put) = self.put_at.take() {
            let micros = u64::try_from(put.elapsed().as_micros()).unwrap_or(u64::MAX);
            self.handover.record(micros);
        }
    }

    /// Whether the host's station times are to be asked for (again) now;
    /// marks the request as sent.
    pub fn timing_due(&mut self) -> bool {
        let due = self
            .timing_asked
            .is_none_or(|asked| asked.elapsed() >= TIMING_RENEW);
        if due {
            self.timing_asked = Some(Instant::now());
        }
        due
    }
}
