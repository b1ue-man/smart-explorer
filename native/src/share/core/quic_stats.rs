//! QUIC statistics of one Share connection over a transfer burst (diagnosis of
//! the speed stage, E13): which counters are read every tick, when traffic
//! counts as a burst, and what a finished burst reports. Pure logic and data;
//! `quic_stats_hub.rs` reads the connections and writes the lines.
//!
//! A burst starts in the first tick that moved at least `BUSY_BYTES_PER_TICK`
//! UDP payload bytes (both directions together, so keepalives and browsing do
//! not count) and ends after `IDLE_TICKS_TO_END` quiet ticks. Every number of
//! a report is the difference between the tick before the burst and the last
//! busy tick, except the ones named final, min, max or mean.
use std::time::Duration;

use iroh::endpoint::{ConnectionStats, FrameStats, PathStats};
use serde::Serialize;

use super::quic_stats_proc::UdpCounters;
use super::transfer_meter::MeterSnapshot;

/// Length of one tick of the reporter.
pub(super) const TICK: Duration = Duration::from_millis(500);
/// UDP payload bytes (sent plus received) per tick that make a tick busy:
/// 0.5 MB/s, far above keepalives and control requests.
pub(super) const BUSY_BYTES_PER_TICK: u64 = 256 * 1024;
/// Quiet ticks in a row that end a burst.
pub(super) const IDLE_TICKS_TO_END: u32 = 2;
/// Busy ticks a burst needs to be reported (a short listing is not one).
pub(super) const MIN_BURST_TICKS: u32 = 2;

/// Which kind of path carried the connection's data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum PathKind {
    #[default]
    None,
    Ip,
    Relay,
}

/// The frame counters that explain a stall: flow-control blocks, the number
/// of acknowledgements, and the stream frames themselves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(super) struct Frames {
    pub(super) stream: u64,
    pub(super) acks: u64,
    pub(super) ack_frequency: u64,
    pub(super) immediate_ack: u64,
    pub(super) ping: u64,
    pub(super) data_blocked: u64,
    pub(super) stream_data_blocked: u64,
    pub(super) streams_blocked_bidi: u64,
    pub(super) max_data: u64,
    pub(super) max_stream_data: u64,
}

impl Frames {
    fn from_noq(frames: &FrameStats) -> Self {
        Self {
            stream: frames.stream,
            acks: frames.acks,
            ack_frequency: frames.ack_frequency,
            immediate_ack: frames.immediate_ack,
            ping: frames.ping,
            data_blocked: frames.data_blocked,
            stream_data_blocked: frames.stream_data_blocked,
            streams_blocked_bidi: frames.streams_blocked_bidi,
            max_data: frames.max_data,
            max_stream_data: frames.max_stream_data,
        }
    }

    fn since(self, earlier: Self) -> Self {
        Self {
            stream: self.stream.saturating_sub(earlier.stream),
            acks: self.acks.saturating_sub(earlier.acks),
            ack_frequency: self.ack_frequency.saturating_sub(earlier.ack_frequency),
            immediate_ack: self.immediate_ack.saturating_sub(earlier.immediate_ack),
            ping: self.ping.saturating_sub(earlier.ping),
            data_blocked: self.data_blocked.saturating_sub(earlier.data_blocked),
            stream_data_blocked: self
                .stream_data_blocked
                .saturating_sub(earlier.stream_data_blocked),
            streams_blocked_bidi: self
                .streams_blocked_bidi
                .saturating_sub(earlier.streams_blocked_bidi),
            max_data: self.max_data.saturating_sub(earlier.max_data),
            max_stream_data: self.max_stream_data.saturating_sub(earlier.max_stream_data),
        }
    }
}

/// The counters of one connection at one tick: totals (they only grow) and
/// the values of the moment (RTT, window, MTU).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Sample {
    pub(super) udp_tx_datagrams: u64,
    pub(super) udp_tx_bytes: u64,
    pub(super) udp_rx_datagrams: u64,
    pub(super) udp_rx_bytes: u64,
    pub(super) lost_packets: u64,
    pub(super) lost_bytes: u64,
    pub(super) congestion_events: u64,
    pub(super) spurious_congestion_events: u64,
    pub(super) plpmtud_probes_sent: u64,
    pub(super) plpmtud_probes_lost: u64,
    pub(super) black_holes: u64,
    pub(super) frames_tx: Frames,
    pub(super) frames_rx: Frames,
    pub(super) rtt: Duration,
    pub(super) cwnd: u64,
    pub(super) mtu: u16,
    pub(super) paths_open: u32,
    pub(super) path_kind: PathKind,
}

impl Sample {
    /// The totals come from the connection (all paths it ever had), the
    /// values of the moment and the congestion counters from the path that
    /// carries the data.
    pub(super) fn from_noq(
        connection: &ConnectionStats,
        path: &PathStats,
        kind: PathKind,
        paths_open: usize,
    ) -> Self {
        Self {
            udp_tx_datagrams: connection.udp_tx.datagrams,
            udp_tx_bytes: connection.udp_tx.bytes,
            udp_rx_datagrams: connection.udp_rx.datagrams,
            udp_rx_bytes: connection.udp_rx.bytes,
            lost_packets: connection.lost_packets,
            lost_bytes: connection.lost_bytes,
            congestion_events: path.congestion_events,
            spurious_congestion_events: path.spurious_congestion_events,
            plpmtud_probes_sent: path.sent_plpmtud_probes,
            plpmtud_probes_lost: path.lost_plpmtud_probes,
            black_holes: path.black_holes_detected,
            frames_tx: Frames::from_noq(&connection.frame_tx),
            frames_rx: Frames::from_noq(&connection.frame_rx),
            rtt: path.rtt,
            cwnd: path.cwnd,
            mtu: path.current_mtu,
            paths_open: u32::try_from(paths_open).unwrap_or(u32::MAX),
            path_kind: kind,
        }
    }

    /// UDP payload bytes, sent plus received, since `earlier`.
    pub(super) fn bytes_moved_since(&self, earlier: &Self) -> u64 {
        self.udp_tx_bytes
            .saturating_sub(earlier.udp_tx_bytes)
            .saturating_add(self.udp_rx_bytes.saturating_sub(earlier.udp_rx_bytes))
    }
}

/// What is read once per tick besides the connections: the transfer meter and
/// the process and kernel counters (absent where the system has none).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Ambient {
    pub(super) meter: MeterSnapshot,
    pub(super) cpu_ms: Option<u64>,
    pub(super) udp: Option<UdpCounters>,
}

/// The meter of a burst in milliseconds. Times add up over all streams and
/// threads of the process, so they can exceed the burst's duration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub(super) struct MeterReport {
    pub(super) raw_bytes: u64,
    pub(super) wire_bytes: u64,
    pub(super) lz4_blocks: u64,
    pub(super) lz4_ms: f64,
    pub(super) disk_read_ms: f64,
    pub(super) stage_write_ms: f64,
    pub(super) stage_wait_ms: f64,
    pub(super) net_wait_ms: f64,
    pub(super) disk_wait_ms: f64,
}

fn millis(nanos: u64) -> f64 {
    (nanos / 1_000) as f64 / 1_000.0
}

fn micros_to_millis(micros: u64) -> f64 {
    micros as f64 / 1_000.0
}

impl MeterReport {
    fn from_delta(delta: &MeterSnapshot) -> Self {
        Self {
            raw_bytes: delta.raw_bytes,
            wire_bytes: delta.wire_bytes,
            lz4_blocks: delta.lz4_blocks,
            lz4_ms: millis(delta.lz4_ns),
            disk_read_ms: millis(delta.disk_read_ns),
            stage_write_ms: millis(delta.stage_write_ns),
            stage_wait_ms: millis(delta.stage_wait_ns),
            net_wait_ms: millis(delta.net_wait_ns),
            disk_wait_ms: millis(delta.disk_wait_ns),
        }
    }
}

/// What a finished burst reports.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(super) struct Report {
    pub(super) duration_ms: u64,
    pub(super) busy_ticks: u32,
    pub(super) path_kind: PathKind,
    pub(super) paths_open: u32,
    /// Smoothed round-trip time at the end of the burst and its extremes
    /// over the busy ticks.
    pub(super) rtt_ms: f64,
    pub(super) rtt_ms_min: f64,
    pub(super) rtt_ms_max: f64,
    /// Congestion window at the end of the burst, its extremes and its mean
    /// over the busy ticks.
    pub(super) cwnd_bytes: u64,
    pub(super) cwnd_bytes_min: u64,
    pub(super) cwnd_bytes_max: u64,
    pub(super) cwnd_bytes_mean: u64,
    pub(super) mtu: u16,
    pub(super) congestion_events: u64,
    pub(super) spurious_congestion_events: u64,
    pub(super) lost_packets: u64,
    pub(super) lost_bytes: u64,
    pub(super) udp_tx_datagrams: u64,
    pub(super) udp_tx_bytes: u64,
    pub(super) udp_rx_datagrams: u64,
    pub(super) udp_rx_bytes: u64,
    pub(super) udp_tx_mb_per_s: f64,
    pub(super) udp_rx_mb_per_s: f64,
    pub(super) plpmtud_probes_sent: u64,
    pub(super) plpmtud_probes_lost: u64,
    pub(super) black_holes_detected: u64,
    pub(super) frames_tx: Frames,
    pub(super) frames_rx: Frames,
    /// Content streams this process served or opened during the burst.
    pub(super) streams: u64,
    pub(super) meter: MeterReport,
    /// CPU time of this process (user plus system) during the burst.
    pub(super) process_cpu_ms: Option<u64>,
    /// System-wide UDP counters during the burst (`RcvbufErrors` = datagrams
    /// dropped by a full receive buffer).
    pub(super) kernel_udp: Option<UdpCounters>,
}

/// Minimum, maximum and sum of a series of values.
#[derive(Clone, Copy, Debug, Default)]
struct Extremes {
    min: u64,
    max: u64,
    sum: u128,
    count: u32,
}

impl Extremes {
    fn record(&mut self, value: u64) {
        if self.count == 0 || value < self.min {
            self.min = value;
        }
        self.max = self.max.max(value);
        self.sum += u128::from(value);
        self.count += 1;
    }

    fn mean(&self) -> u64 {
        u64::try_from(self.sum / u128::from(self.count.max(1))).unwrap_or(u64::MAX)
    }
}

struct Burst {
    base: (Sample, Ambient),
    base_ms: u64,
    last: (Sample, Ambient),
    last_ms: u64,
    ticks: u32,
    cwnd: Extremes,
    rtt_micros: Extremes,
}

impl Burst {
    fn new(base: (Sample, Ambient), base_ms: u64) -> Self {
        Self {
            base,
            base_ms,
            last: base,
            last_ms: base_ms,
            ticks: 0,
            cwnd: Extremes::default(),
            rtt_micros: Extremes::default(),
        }
    }

    fn record(&mut self, now_ms: u64, sample: Sample, ambient: Ambient) {
        self.last = (sample, ambient);
        self.last_ms = now_ms;
        self.ticks += 1;
        self.cwnd.record(sample.cwnd);
        if !sample.rtt.is_zero() {
            self.rtt_micros
                .record(u64::try_from(sample.rtt.as_micros()).unwrap_or(u64::MAX));
        }
    }

    fn into_report(self) -> Option<Report> {
        if self.ticks < MIN_BURST_TICKS {
            return None;
        }
        let ((base, base_ambient), (last, last_ambient)) = (self.base, self.last);
        let duration_ms = self.last_ms.saturating_sub(self.base_ms).max(1);
        let seconds = duration_ms as f64 / 1_000.0;
        let tx_bytes = last.udp_tx_bytes.saturating_sub(base.udp_tx_bytes);
        let rx_bytes = last.udp_rx_bytes.saturating_sub(base.udp_rx_bytes);
        let meter = last_ambient.meter.since(base_ambient.meter);
        Some(Report {
            duration_ms,
            busy_ticks: self.ticks,
            path_kind: last.path_kind,
            paths_open: last.paths_open,
            rtt_ms: micros_to_millis(u64::try_from(last.rtt.as_micros()).unwrap_or(u64::MAX)),
            rtt_ms_min: micros_to_millis(self.rtt_micros.min),
            rtt_ms_max: micros_to_millis(self.rtt_micros.max),
            cwnd_bytes: last.cwnd,
            cwnd_bytes_min: self.cwnd.min,
            cwnd_bytes_max: self.cwnd.max,
            cwnd_bytes_mean: self.cwnd.mean(),
            mtu: last.mtu,
            congestion_events: last
                .congestion_events
                .saturating_sub(base.congestion_events),
            spurious_congestion_events: last
                .spurious_congestion_events
                .saturating_sub(base.spurious_congestion_events),
            lost_packets: last.lost_packets.saturating_sub(base.lost_packets),
            lost_bytes: last.lost_bytes.saturating_sub(base.lost_bytes),
            udp_tx_datagrams: last.udp_tx_datagrams.saturating_sub(base.udp_tx_datagrams),
            udp_tx_bytes: tx_bytes,
            udp_rx_datagrams: last.udp_rx_datagrams.saturating_sub(base.udp_rx_datagrams),
            udp_rx_bytes: rx_bytes,
            udp_tx_mb_per_s: tx_bytes as f64 / seconds / 1e6,
            udp_rx_mb_per_s: rx_bytes as f64 / seconds / 1e6,
            plpmtud_probes_sent: last
                .plpmtud_probes_sent
                .saturating_sub(base.plpmtud_probes_sent),
            plpmtud_probes_lost: last
                .plpmtud_probes_lost
                .saturating_sub(base.plpmtud_probes_lost),
            black_holes_detected: last.black_holes.saturating_sub(base.black_holes),
            frames_tx: last.frames_tx.since(base.frames_tx),
            frames_rx: last.frames_rx.since(base.frames_rx),
            streams: meter.streams,
            meter: MeterReport::from_delta(&meter),
            process_cpu_ms: match (last_ambient.cpu_ms, base_ambient.cpu_ms) {
                (Some(after), Some(before)) => Some(after.saturating_sub(before)),
                _ => None,
            },
            kernel_udp: match (last_ambient.udp, base_ambient.udp) {
                (Some(after), Some(before)) => Some(after.since(before)),
                _ => None,
            },
        })
    }
}

/// What one tick did to the tracker.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Step {
    /// A burst began in this tick (the reporter notes the thread CPU times).
    pub(super) started: bool,
    /// A burst ended in this tick.
    pub(super) report: Option<Report>,
}

/// Follows one connection tick by tick.
#[derive(Default)]
pub(super) struct Tracker {
    previous: Option<(Sample, Ambient)>,
    burst: Option<Burst>,
    quiet_ticks: u32,
}

impl Tracker {
    /// Feeds the counters of the tick at `now_ms` (milliseconds on a clock
    /// that only the reporter reads).
    pub(super) fn observe(&mut self, now_ms: u64, sample: Sample, ambient: Ambient) -> Step {
        let Some(before) = self.previous.replace((sample, ambient)) else {
            return Step::default();
        };
        let busy = sample.bytes_moved_since(&before.0) >= BUSY_BYTES_PER_TICK;
        let mut step = Step::default();
        if busy {
            self.quiet_ticks = 0;
            let burst = self.burst.get_or_insert_with(|| {
                step.started = true;
                let tick_ms = u64::try_from(TICK.as_millis()).unwrap_or(u64::MAX);
                Burst::new(before, now_ms.saturating_sub(tick_ms))
            });
            burst.record(now_ms, sample, ambient);
        } else if self.burst.is_some() {
            self.quiet_ticks += 1;
            if self.quiet_ticks >= IDLE_TICKS_TO_END {
                step.report = self.finish();
            }
        }
        step
    }

    /// The connection is gone (or the reporter stops): the open burst ends.
    pub(super) fn finish(&mut self) -> Option<Report> {
        self.quiet_ticks = 0;
        self.burst.take().and_then(Burst::into_report)
    }
}
