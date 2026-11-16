//! The host's part of `se screen bench` (plan 2026-11-14 „Bildkette“ BK0):
//! the station times of a host that announced `frame_timing_v1` in `STATS`
//! (`frame_timing.rs`). The bench asks for them once, sends an input probe
//! with every later `STATS`, and adds the host's last report to
//! `stages_ms`: `capture`, `encode`, `worker_handoff`, `send`,
//! `tick_jitter`, `input_inject` as measured there, `network` and `input`
//! after the viewer's clock offset (`PING`/`PONG`). A host without the
//! announcement keeps the old fields only; `host_timing` then says
//! `unsupported_by_host`.
use super::super::frame_timing::{
    ChainReport, FrameTimingRequest, HostStations, StationUs, FRAME_TIMING_VERSION,
};
use super::super::metrics::ClockSync;
use super::super::wire_control::{HostStats, ViewerFrame};
use super::{stages, BenchReport, StageMs};

/// `host_timing`: the host reported its stations.
pub const HOST_TIMING_REPORTED: &str = "reported";
/// `host_timing`: the host does not announce `frame_timing_v1`.
pub const HOST_TIMING_UNSUPPORTED: &str = "unsupported_by_host";

/// What the bench learnt of the host's stations.
#[derive(Debug, Default)]
pub(super) struct HostTiming {
    announced: bool,
    requested: bool,
    stations: Option<HostStations>,
    chain: Option<ChainReport>,
}

fn ms(station: StationUs) -> StageMs {
    StageMs {
        p50: station.p50_us as f64 / 1000.0,
        p95: station.p95_us as f64 / 1000.0,
    }
}

impl HostTiming {
    /// One `STATS` arrived at `now_us` (viewer clock): the frame for the
    /// host, if any: the start of the station times on the first
    /// announcement, an input probe on every later one.
    pub(super) fn on_stats(&mut self, stats: HostStats, now_us: u64) -> Option<ViewerFrame> {
        if stats.chain.is_some() {
            self.chain = stats.chain;
        }
        if stats.frame_timing != Some(FRAME_TIMING_VERSION) {
            return None;
        }
        self.announced = true;
        if stats.stations.is_some() {
            self.stations = stats.stations;
        }
        if !self.requested {
            self.requested = true;
            return Some(ViewerFrame::FrameTiming(FrameTimingRequest::start()));
        }
        Some(ViewerFrame::FrameTiming(FrameTimingRequest::probe(now_us)))
    }

    /// Adds the host's stations and facts to `report`; the clock
    /// differences need the offset of `clock` and stay out without it.
    pub(super) fn finish(self, clock: &ClockSync, report: &mut BenchReport) {
        report.host_chain = self.chain;
        let state = if self.announced {
            HOST_TIMING_REPORTED
        } else {
            HOST_TIMING_UNSUPPORTED
        };
        report.host_timing = Some(state.into());
        let Some(stations) = self.stations else {
            return;
        };
        let offset = clock.offset_us();
        let measured = [
            (stages::CAPTURE, stations.capture),
            (stages::CONVERT, stations.convert),
            (stages::ENCODE, stations.encode),
            (stages::WORKER_HANDOFF, stations.worker_handoff),
            (stages::SEND, stations.send),
            (stages::TICK_JITTER, stations.tick_jitter),
            (stages::INPUT_INJECT, stations.input_inject),
            (
                stages::NETWORK,
                stations
                    .network_raw
                    .zip(offset)
                    .map(|(raw, offset)| raw.shifted(offset)),
            ),
            (
                stages::INPUT,
                stations
                    .input_raw
                    .zip(offset)
                    .map(|(raw, offset)| raw.shifted(offset.saturating_neg())),
            ),
        ];
        for (name, station) in measured {
            if let Some(station) = station {
                report.stages_ms.insert(name.to_string(), ms(station));
            }
        }
    }
}
