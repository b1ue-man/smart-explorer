//! Station times of `se screen bench` (plan 2026-11-14 P7, C5): where in the
//! picture path the time of one frame goes, as p50 and p95 in milliseconds per
//! station. `BenchReport::stages_ms` carries them; the field is left out of
//! the JSON while it is empty, so existing comparisons of the report stay as
//! they were, and `se screen bench` shows one `stage_ms` line per station.
//!
//! Stations and who can measure them:
//!
//! | station | meaning | `--local` | session with a device |
//! |---|---|---|---|
//! | `capture` | drawing the test picture (stands in for the capture) | yes | host side only |
//! | `encode` | encoder call of one frame | yes | host side only |
//! | `send` | handing all records of a frame to the picture stream | no | host side only |
//! | `network` | last record handed over until it arrived | no | pair in one process |
//! | `decode` | decoder call that completed a frame | yes | yes |
//! | `capture_to_arrival` | capture (host clock) until the last record arrived | no | yes |
//!
//! The viewer of a session with another device sees only what the wire
//! carries: the capture time stamp of each frame and its own clock. It
//! cannot split the host stations, so it reports `decode` and the sum
//! `capture_to_arrival` (capture, encode, send and network together, corrected
//! by the clock offset of `PING`/`PONG`). The tests of
//! `verbindungen_bild_messung_task_tests.rs` play the host in the same
//! process and fill every station.
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::screen::core::metrics::LatencySamples;

pub const CAPTURE: &str = "capture";
pub const ENCODE: &str = "encode";
pub const SEND: &str = "send";
pub const NETWORK: &str = "network";
pub const DECODE: &str = "decode";
pub const CAPTURE_TO_ARRIVAL: &str = "capture_to_arrival";

/// Percentiles of one station in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StageMs {
    pub p50: f64,
    pub p95: f64,
}

/// The samples of every station of one measurement; each station keeps its
/// samples bounded like the latency samples do.
#[derive(Clone, Debug, Default)]
pub struct StageSamples {
    stations: BTreeMap<&'static str, LatencySamples>,
}

impl StageSamples {
    /// One sample of `station` in microseconds.
    pub fn record_us(&mut self, station: &'static str, micros: u64) {
        self.stations.entry(station).or_default().record(micros);
    }

    /// One sample of `station`: the time since `started`.
    pub fn record_since(&mut self, station: &'static str, started: Instant) {
        self.record_duration(station, started.elapsed());
    }

    pub fn record_duration(&mut self, station: &'static str, elapsed: Duration) {
        let micros = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
        self.record_us(station, micros);
    }

    /// Samples of `station` so far.
    pub fn count(&self, station: &str) -> u64 {
        self.stations.get(station).map_or(0, LatencySamples::count)
    }

    /// p50 and p95 per station that has samples.
    pub fn summary(&self) -> BTreeMap<String, StageMs> {
        self.stations
            .iter()
            .filter_map(|(name, samples)| {
                samples.summary().map(|summary| {
                    (
                        (*name).to_string(),
                        StageMs {
                            p50: summary.p50_ms,
                            p95: summary.p95_ms,
                        },
                    )
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::{BenchReport, BitrateKbps, LatencyMs};
    use super::*;

    fn report(stages_ms: BTreeMap<String, StageMs>) -> BenchReport {
        BenchReport {
            codec: "tiles".into(),
            encoder: "tiles".into(),
            decoder: "tiles".into(),
            path: "local".into(),
            frames_sent: 10,
            frames_shown: 10,
            fps_mean: 30.0,
            fps_min: 29.0,
            latency_ms: LatencyMs {
                p50: 1.0,
                p95: 2.0,
                max: 3.0,
            },
            bitrate_kbps: BitrateKbps {
                mean: 100.0,
                max: 200.0,
            },
            keyframes: 1,
            recoveries: Vec::new(),
            corrupt_frames: 0,
            static_psnr_db: None,
            stages_ms,
        }
    }

    #[test]
    fn verbindungen_task_bench_stages_summary_gives_p50_and_p95_in_milliseconds() {
        let mut samples = StageSamples::default();
        for micros in [1_000, 2_000, 3_000, 4_000, 100_000] {
            samples.record_us(DECODE, micros);
        }
        samples.record_duration(ENCODE, Duration::from_millis(7));
        assert_eq!(samples.count(DECODE), 5);
        assert_eq!(samples.count(CAPTURE), 0);
        let summary = samples.summary();
        assert_eq!(summary.len(), 2, "{summary:?}");
        // Nearest rank: the third of five for p50, the last for p95.
        assert_eq!(
            summary[DECODE],
            StageMs {
                p50: 3.0,
                p95: 100.0
            }
        );
        assert_eq!(summary[ENCODE], StageMs { p50: 7.0, p95: 7.0 });
        assert!(StageSamples::default().summary().is_empty());
    }

    #[test]
    fn verbindungen_task_bench_stages_field_is_left_out_while_empty() {
        let plain = serde_json::to_value(report(BTreeMap::new())).unwrap();
        assert!(plain.get("stages_ms").is_none(), "{plain}");
        // An older writer's report reads back with no stations.
        let back: BenchReport = serde_json::from_value(plain).unwrap();
        assert!(back.stages_ms.is_empty());

        let mut stations = BTreeMap::new();
        stations.insert("decode".to_string(), StageMs { p50: 1.5, p95: 2.0 });
        let filled = serde_json::to_value(report(stations.clone())).unwrap();
        assert_eq!(filled["stages_ms"]["decode"]["p50"], 1.5);
        assert_eq!(filled["stages_ms"]["decode"]["p95"], 2.0);
        let back: BenchReport = serde_json::from_value(filled).unwrap();
        assert_eq!(back.stages_ms, stations);
    }
}
