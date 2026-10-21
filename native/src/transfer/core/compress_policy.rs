//! Adaptive LZ4 between two se peers (Share content streams and the SSH
//! agent's `Data` frames, U44-10): pure decision logic, no I/O and no clock
//! of its own. Compressing overlaps sending, so the stream runs at
//! `min(core rate, wire rate ÷ ratio)` against the wire rate without it;
//! that gains only while the wire limits, the core is faster than the wire
//! and the data shrinks.
//!
//! The rule (all four hold):
//! 1. the writer waited for the wire in more than half of the last window
//!    (`writer_blocked_share > 0.5`): the line limits, not a disk or a core;
//! 2. one core compresses at least 1.3 times as fast as the wire carries
//!    (30 % reserve for the spread of the measurements);
//! 3. the sampled ratio `packed ÷ raw` is at most 0.85 (15 % fewer bytes);
//! 4. one core stays free for everything else (`free_cores >= 1.0`).
//!
//! A decision holds for 8 blocks (the block it was made for and the seven
//! after it), so a measurement that flickers does not flip the stream. The
//! ratio is sampled from the first 64 KiB of every file and then from every
//! 8th mebibyte.
use std::time::{Duration, Instant};

/// Blocks a decision holds.
pub(crate) const WINDOW_BLOCKS: u32 = 8;
/// Bytes of the sample taken at the start of every file.
pub(crate) const PROBE_BYTES: usize = 64 * 1024;
/// Distance of the later samples inside a file.
pub(crate) const PROBE_PERIOD: u64 = 8 * 1024 * 1024;
/// The writer waited for the wire longer than this share of a window.
pub(crate) const BLOCKED_SHARE_MIN: f64 = 0.5;
/// Core rate needed relative to the wire rate.
pub(crate) const CORE_OVER_WIRE: f64 = 1.3;
/// Largest ratio `packed ÷ raw` worth the effort.
pub(crate) const RATIO_MAX: f64 = 0.85;
/// Cores that must stay free for the rest of the machine.
pub(crate) const FREE_CORES_MIN: f64 = 1.0;
/// Length of one wire measurement window.
pub(crate) const METER_WINDOW: Duration = Duration::from_secs(1);
/// Weight of the newest measurement (like `BatchSizer::record`).
const NEWEST_WEIGHT: f64 = 0.3;

/// What one decision is made from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PolicyInput {
    /// Share of the last window the writer spent waiting for the wire.
    pub(crate) writer_blocked_share: f64,
    /// Bytes per second the writer sent in a window that was wire limited.
    pub(crate) wire_bps: u64,
    /// Bytes per second one core compresses (measured by the samples).
    pub(crate) core_bps: u64,
    /// Smoothed `packed ÷ raw` of the samples.
    pub(crate) ratio: f64,
    /// Cores of this machine that no process uses right now.
    pub(crate) free_cores: f64,
}

impl PolicyInput {
    /// The rule alone, without the hold of the previous decision.
    pub(crate) fn favours_compression(&self) -> bool {
        self.writer_blocked_share > BLOCKED_SHARE_MIN
            && self.core_bps as f64 >= CORE_OVER_WIRE * self.wire_bps as f64
            && self.ratio <= RATIO_MAX
            && self.free_cores >= FREE_CORES_MIN
    }
}

/// The measuring and deciding state of one stream direction.
#[derive(Debug)]
pub(crate) struct Policy {
    decision: bool,
    /// Blocks the current decision still holds.
    held: u32,
    ratio: Option<f64>,
    core_bps: Option<f64>,
    meter: LinkMeter,
    next_probe: u64,
}

impl Default for Policy {
    fn default() -> Self {
        Self::new()
    }
}

impl Policy {
    pub(crate) fn new() -> Self {
        Self {
            decision: false,
            held: 0,
            ratio: None,
            core_bps: None,
            meter: LinkMeter::default(),
            next_probe: 0,
        }
    }

    /// Whether the next block is compressed. Call exactly once per block:
    /// a decision holds for `WINDOW_BLOCKS` calls.
    pub(crate) fn should_compress(&mut self, input: PolicyInput) -> bool {
        if self.held > 0 {
            self.held -= 1;
            return self.decision;
        }
        self.decision = input.favours_compression();
        self.held = WINDOW_BLOCKS - 1;
        self.decision
    }

    /// `should_compress` from what this policy measured itself. The free
    /// cores are asked for only when a new decision is due.
    pub(crate) fn decide(&mut self, free_cores: impl FnOnce() -> f64) -> bool {
        let cores = if self.held > 0 { 0.0 } else { free_cores() };
        let input = self.input(cores);
        self.should_compress(input)
    }

    /// The decision input as the measurements stand now. Without a sample
    /// the ratio is 1.0 and the core rate 0: nothing is compressed before
    /// the first sample.
    pub(crate) fn input(&self, free_cores: f64) -> PolicyInput {
        PolicyInput {
            writer_blocked_share: self.meter.share(),
            wire_bps: self.meter.wire_bps(),
            core_bps: self.core_bps.map_or(0, |rate| rate as u64),
            ratio: self.ratio.unwrap_or(1.0),
            free_cores,
        }
    }

    /// A compression of `raw` bytes into `packed` bytes took `took`.
    pub(crate) fn observe_compression(&mut self, raw: usize, packed: usize, took: Duration) {
        if raw == 0 {
            return;
        }
        let ratio = packed as f64 / raw as f64;
        self.ratio = Some(blend(self.ratio, ratio));
        let seconds = took.as_secs_f64();
        if seconds > 0.0 {
            self.core_bps = Some(blend(self.core_bps, raw as f64 / seconds));
        }
    }

    /// A new file starts: its first block is sampled.
    pub(crate) fn start_file(&mut self) {
        self.next_probe = 0;
    }

    /// Whether the block that starts at `file_offset` is sampled: the first
    /// block of a file and then the first block at or after every 8th
    /// mebibyte.
    pub(crate) fn probe_due(&mut self, file_offset: u64) -> bool {
        if file_offset < self.next_probe {
            return false;
        }
        self.next_probe = (file_offset / PROBE_PERIOD + 1).saturating_mul(PROBE_PERIOD);
        true
    }

    /// `bytes` were handed to the wire; the writer waited `blocked` of the
    /// time for it. `now` closes a window once it is a second long.
    pub(crate) fn record_send(&mut self, bytes: u64, blocked: Duration, now: Instant) {
        self.meter.record(bytes, blocked, now);
    }

    /// Smoothed `packed ÷ raw`, if any sample exists.
    pub(crate) fn ratio(&self) -> Option<f64> {
        self.ratio
    }
}

fn blend(old: Option<f64>, new: f64) -> f64 {
    match old {
        Some(old) => old * (1.0 - NEWEST_WEIGHT) + new * NEWEST_WEIGHT,
        None => new,
    }
}

/// How long the writer waited for the wire, window by window.
#[derive(Debug, Default)]
struct LinkMeter {
    window_start: Option<Instant>,
    bytes: u64,
    blocked: Duration,
    /// Share of the last closed window.
    last_share: f64,
    /// Rate of the last closed window that the wire limited.
    last_wire_bps: u64,
}

impl LinkMeter {
    fn record(&mut self, bytes: u64, blocked: Duration, now: Instant) {
        let start = *self.window_start.get_or_insert(now);
        self.bytes = self.bytes.saturating_add(bytes);
        self.blocked = self.blocked.saturating_add(blocked);
        let length = now.saturating_duration_since(start);
        if length < METER_WINDOW {
            return;
        }
        let seconds = length.as_secs_f64();
        self.last_share = (self.blocked.as_secs_f64() / seconds).min(1.0);
        if self.last_share > BLOCKED_SHARE_MIN {
            self.last_wire_bps = (self.bytes as f64 / seconds) as u64;
        }
        self.window_start = Some(now);
        self.bytes = 0;
        self.blocked = Duration::ZERO;
    }

    fn share(&self) -> f64 {
        self.last_share
    }

    fn wire_bps(&self) -> u64 {
        self.last_wire_bps
    }
}
