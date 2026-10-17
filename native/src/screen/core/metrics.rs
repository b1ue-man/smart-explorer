//! Measures of a screen session and of `se screen bench` (decision C9):
//! end-to-end latency with p50, p95 and maximum, frame and bit rates per
//! second, the clock offset from `PING`/`PONG` with the smallest round trip,
//! recovery times after a loss, and the PSNR of the static text block
//! (`static_psnr_db`). Every store is bounded.
use std::collections::VecDeque;
use std::io;

use super::pixel_convert::{BgraView, Rect};
use super::wire::invalid;
use super::wire_control::Pong;

/// Latency samples kept exactly (1 MiB); beyond that a uniform reservoir.
pub(crate) const MAX_LATENCY_SAMPLES: usize = 1 << 18;
/// Seconds of rate history: one day of measurement.
pub(crate) const MAX_RATE_SECONDS: usize = 86_400;
/// A clock sample older than this is replaced by the next one even with a
/// longer round trip, so clock drift is followed.
pub(crate) const CLOCK_SAMPLE_MAX_AGE_US: u64 = 30_000_000;
/// Recovery times kept per session.
pub(crate) const MAX_RECOVERIES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LatencySummary {
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
}

#[derive(Clone, Debug)]
pub(crate) struct LatencySamples {
    samples: Vec<u32>,
    seen: u64,
    max_us: u64,
    random: u64,
}

impl Default for LatencySamples {
    fn default() -> Self {
        Self {
            samples: Vec::new(),
            seen: 0,
            max_us: 0,
            random: 0x9E37_79B9_7F4A_7C15,
        }
    }
}

impl LatencySamples {
    pub(crate) fn record(&mut self, latency_us: u64) {
        self.seen += 1;
        self.max_us = self.max_us.max(latency_us);
        let value = u32::try_from(latency_us).unwrap_or(u32::MAX);
        if self.samples.len() < MAX_LATENCY_SAMPLES {
            self.samples.push(value);
            return;
        }
        // xorshift64; the reservoir keeps each sample with equal chance.
        self.random ^= self.random << 13;
        self.random ^= self.random >> 7;
        self.random ^= self.random << 17;
        let slot = self.random % self.seen;
        if let Some(entry) = self.samples.get_mut(slot as usize) {
            *entry = value;
        }
    }

    pub(crate) fn count(&self) -> u64 {
        self.seen
    }

    /// Nearest-rank percentiles; `None` without samples.
    pub(crate) fn summary(&self) -> Option<LatencySummary> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted = self.samples.clone();
        sorted.sort_unstable();
        let rank = |share: f64| {
            let index = (share * sorted.len() as f64).ceil() as usize;
            f64::from(sorted[index.clamp(1, sorted.len()) - 1]) / 1000.0
        };
        Some(LatencySummary {
            p50_ms: rank(0.50),
            p95_ms: rank(0.95),
            max_ms: self.max_us as f64 / 1000.0,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RateSummary {
    pub frames: u64,
    pub fps_mean: f64,
    /// Lowest frame count of a completed second; the mean while no second
    /// completed.
    pub fps_min: f64,
    pub kbps_mean: f64,
    pub kbps_max: f64,
}

/// Frames and bytes per second of wall time.
#[derive(Clone, Debug, Default)]
pub(crate) struct RateMeter {
    start_us: Option<u64>,
    second: u64,
    frames: u64,
    bytes: u64,
    seconds: Vec<(u64, u64)>,
    total_frames: u64,
    total_bytes: u64,
}

impl RateMeter {
    pub(crate) fn record(&mut self, now_us: u64, bytes: u64) {
        let start = *self.start_us.get_or_insert(now_us);
        let second = now_us.saturating_sub(start) / 1_000_000;
        if second > self.second {
            self.close_seconds(second);
        }
        self.frames += 1;
        self.bytes += bytes;
        self.total_frames += 1;
        self.total_bytes += bytes;
    }

    fn close_seconds(&mut self, second: u64) {
        let mut pending = Some((self.frames, self.bytes));
        while self.second < second && self.seconds.len() < MAX_RATE_SECONDS {
            self.seconds.push(pending.take().unwrap_or_default());
            self.second += 1;
        }
        self.second = second;
        self.frames = 0;
        self.bytes = 0;
    }

    /// Means over the time since the first frame; minimum and maximum over
    /// the seconds completed by `now_us`, including seconds without frames.
    pub(crate) fn summary(&self, now_us: u64) -> RateSummary {
        let since_start = self
            .start_us
            .map_or(0, |start| now_us.saturating_sub(start));
        let seconds = since_start.max(1) as f64 / 1_000_000.0;
        let fps_mean = self.total_frames as f64 / seconds;
        let kbps_mean = self.total_bytes as f64 * 8.0 / 1000.0 / seconds;
        let now_second = since_start / 1_000_000;
        let open = [
            (now_second > self.second).then_some((self.frames, self.bytes)),
            (now_second > self.second + 1).then_some((0, 0)),
        ];
        let complete = self
            .seconds
            .iter()
            .copied()
            .chain(open.into_iter().flatten());
        let (min_frames, max_bytes) =
            complete.fold((None::<u64>, None::<u64>), |(min, max), (frames, bytes)| {
                (
                    Some(min.map_or(frames, |low: u64| low.min(frames))),
                    Some(max.map_or(bytes, |high: u64| high.max(bytes))),
                )
            });
        let fps_min = min_frames.map_or(fps_mean, |frames| frames as f64);
        let kbps_max = max_bytes.map_or(kbps_mean, |bytes| bytes as f64 * 8.0 / 1000.0);
        RateSummary {
            frames: self.total_frames,
            fps_mean,
            fps_min,
            kbps_mean,
            kbps_max,
        }
    }
}

/// One `PING`/`PONG` exchange: round trip without the host's turnaround, and
/// `offset_us` = host clock − viewer clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ClockSample {
    pub rtt_us: u64,
    pub offset_us: i64,
    pub at_us: u64,
}

/// Clock offset from the exchange with the smallest round trip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ClockSync {
    best: Option<ClockSample>,
}

impl ClockSync {
    /// `received_us` is the viewer clock when the `PONG` arrived. Exchanges
    /// whose clocks run backwards are ignored.
    pub(crate) fn on_pong(&mut self, pong: &Pong, received_us: u64) -> Option<ClockSample> {
        let (sent, host_in, host_out) = (pong.ping_sent_us, pong.received_us, pong.sent_us);
        if received_us < sent || host_out < host_in {
            return None;
        }
        let rtt_us = (received_us - sent).saturating_sub(host_out - host_in);
        let offset = ((i128::from(host_in) - i128::from(sent))
            + (i128::from(host_out) - i128::from(received_us)))
            / 2;
        let sample = ClockSample {
            rtt_us,
            offset_us: offset.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64,
            at_us: received_us,
        };
        let replace = self.best.is_none_or(|best| {
            rtt_us <= best.rtt_us
                || received_us.saturating_sub(best.at_us) > CLOCK_SAMPLE_MAX_AGE_US
        });
        if replace {
            self.best = Some(sample);
        }
        Some(sample)
    }

    pub(crate) fn offset_us(&self) -> Option<i64> {
        self.best.map(|best| best.offset_us)
    }

    /// Capture (host clock) to display (viewer clock); `None` without an
    /// offset or when the display seems to come first.
    pub(crate) fn latency_us(&self, captured_host_us: u64, shown_viewer_us: u64) -> Option<u64> {
        let offset = i128::from(self.offset_us()?);
        let latency = i128::from(shown_viewer_us) + offset - i128::from(captured_host_us);
        u64::try_from(latency).ok()
    }
}

/// Time from a disruption (reset, decode error, lost picture stream) to the
/// next good frame.
#[derive(Clone, Debug, Default)]
pub(crate) struct RecoveryTracker {
    disrupted_at_us: Option<u64>,
    recoveries_us: VecDeque<u64>,
}

impl RecoveryTracker {
    /// The first disruption counts; later ones before recovery add nothing.
    pub(crate) fn on_disruption(&mut self, now_us: u64) {
        if self.disrupted_at_us.is_none() {
            self.disrupted_at_us = Some(now_us);
        }
    }

    pub(crate) fn on_good_frame(&mut self, now_us: u64) -> Option<u64> {
        let since = self.disrupted_at_us.take()?;
        let duration = now_us.saturating_sub(since);
        if self.recoveries_us.len() == MAX_RECOVERIES {
            self.recoveries_us.pop_front();
        }
        self.recoveries_us.push_back(duration);
        Some(duration)
    }

    pub(crate) fn recoveries_ms(&self) -> Vec<f64> {
        self.recoveries_us
            .iter()
            .map(|duration| *duration as f64 / 1000.0)
            .collect()
    }
}

/// PSNR in dB of the colour channels of `rect` in both frames; infinite for
/// identical content (the lossless tile codec).
pub(crate) fn psnr_db(a: &BgraView, b: &BgraView, rect: Rect) -> io::Result<f64> {
    if !rect.fits(a.size()) || !rect.fits(b.size()) {
        return Err(invalid("PSNR region lies outside a frame"));
    }
    let (start, end) = (rect.x as usize * 4, (rect.x + rect.width) as usize * 4);
    let mut squared = 0u64;
    for y in rect.y..rect.y + rect.height {
        let pairs = a.row(y)[start..end]
            .chunks_exact(4)
            .zip(b.row(y)[start..end].chunks_exact(4));
        for (left, right) in pairs {
            for (first, second) in left[..3].iter().zip(&right[..3]) {
                let difference = u64::from(first.abs_diff(*second));
                squared += difference * difference;
            }
        }
    }
    if squared == 0 {
        return Ok(f64::INFINITY);
    }
    let mean = squared as f64 / (u64::from(rect.width) * u64::from(rect.height) * 3) as f64;
    Ok(10.0 * (255.0 * 255.0 / mean).log10())
}

#[cfg(test)]
#[path = "metrics_tests.rs"]
mod tests;
