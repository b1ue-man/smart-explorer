//! Where the Share transfers of this process spend their time (diagnosis of
//! the speed stage, E13). Each content stream adds the time it waited for the
//! network, packed or unpacked LZ4 blocks, read from the disk or wrote into
//! the stage; the QUIC statistics reporter (`quic_stats_hub.rs`) takes the
//! difference of two snapshots for the length of a transfer burst.
//!
//! The meter is a measurement aid of development builds (`enabled`): a
//! release build, the unit tests of this crate and a process with
//! `SE_TEST_NO_SHARE_STATS` set neither read the clock nor count. It changes
//! no protocol byte and no decision of a transfer.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::Serialize;

/// The variable that switches the diagnosis off in a development build.
pub(super) const NO_STATS_VARIABLE: &str = "SE_TEST_NO_SHARE_STATS";

/// Whether this process measures: development builds only (like
/// `SE_TEST_NO_LZ4`, a release build ignores the variable), not the unit
/// tests of the crate (they must not write into a profile).
pub(super) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        cfg!(debug_assertions) && !cfg!(test) && std::env::var_os(NO_STATS_VARIABLE).is_none()
    })
}

/// What a span of time was spent on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    /// Packing or unpacking LZ4 blocks.
    Lz4,
    /// A host reading the file (`Read`, `ReadAt`).
    DiskRead,
    /// Writing into the stage or the target file.
    StageWrite,
    /// A receiver waiting for the writer (its queue was full).
    StageWait,
    /// Waiting for the network: sending a frame, or waiting for the next
    /// frame of the peer.
    NetWait,
    /// A sender waiting for the reader (its queue was empty).
    DiskWait,
}

/// The process-wide totals, in nanoseconds and counts.
#[derive(Default)]
pub(super) struct TransferMeter {
    streams: AtomicU64,
    raw_bytes: AtomicU64,
    wire_bytes: AtomicU64,
    lz4_blocks: AtomicU64,
    lz4_ns: AtomicU64,
    disk_read_ns: AtomicU64,
    stage_write_ns: AtomicU64,
    stage_wait_ns: AtomicU64,
    net_wait_ns: AtomicU64,
    disk_wait_ns: AtomicU64,
}

/// The totals at one instant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(super) struct MeterSnapshot {
    pub(super) streams: u64,
    pub(super) raw_bytes: u64,
    pub(super) wire_bytes: u64,
    pub(super) lz4_blocks: u64,
    pub(super) lz4_ns: u64,
    pub(super) disk_read_ns: u64,
    pub(super) stage_write_ns: u64,
    pub(super) stage_wait_ns: u64,
    pub(super) net_wait_ns: u64,
    pub(super) disk_wait_ns: u64,
}

impl MeterSnapshot {
    /// What happened between `earlier` and this snapshot.
    pub(super) fn since(self, earlier: Self) -> Self {
        Self {
            streams: self.streams.saturating_sub(earlier.streams),
            raw_bytes: self.raw_bytes.saturating_sub(earlier.raw_bytes),
            wire_bytes: self.wire_bytes.saturating_sub(earlier.wire_bytes),
            lz4_blocks: self.lz4_blocks.saturating_sub(earlier.lz4_blocks),
            lz4_ns: self.lz4_ns.saturating_sub(earlier.lz4_ns),
            disk_read_ns: self.disk_read_ns.saturating_sub(earlier.disk_read_ns),
            stage_write_ns: self.stage_write_ns.saturating_sub(earlier.stage_write_ns),
            stage_wait_ns: self.stage_wait_ns.saturating_sub(earlier.stage_wait_ns),
            net_wait_ns: self.net_wait_ns.saturating_sub(earlier.net_wait_ns),
            disk_wait_ns: self.disk_wait_ns.saturating_sub(earlier.disk_wait_ns),
        }
    }
}

impl TransferMeter {
    fn slot(&self, phase: Phase) -> &AtomicU64 {
        match phase {
            Phase::Lz4 => &self.lz4_ns,
            Phase::DiskRead => &self.disk_read_ns,
            Phase::StageWrite => &self.stage_write_ns,
            Phase::StageWait => &self.stage_wait_ns,
            Phase::NetWait => &self.net_wait_ns,
            Phase::DiskWait => &self.disk_wait_ns,
        }
    }

    /// `spent` went to `phase`.
    pub(super) fn add(&self, phase: Phase, spent: Duration) {
        let nanos = u64::try_from(spent.as_nanos()).unwrap_or(u64::MAX);
        self.slot(phase).fetch_add(nanos, Ordering::Relaxed);
    }

    /// A content stream started (counted on the side that serves or opens it).
    pub(super) fn stream_started(&self) {
        self.streams.fetch_add(1, Ordering::Relaxed);
    }

    /// `raw` bytes of file content travelled as `wire` bytes of frame payload.
    pub(super) fn bytes(&self, raw: usize, wire: usize) {
        self.raw_bytes.fetch_add(raw as u64, Ordering::Relaxed);
        self.wire_bytes.fetch_add(wire as u64, Ordering::Relaxed);
    }

    /// One block went out packed.
    pub(super) fn lz4_block(&self) {
        self.lz4_blocks.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn snapshot(&self) -> MeterSnapshot {
        let load = |value: &AtomicU64| value.load(Ordering::Relaxed);
        MeterSnapshot {
            streams: load(&self.streams),
            raw_bytes: load(&self.raw_bytes),
            wire_bytes: load(&self.wire_bytes),
            lz4_blocks: load(&self.lz4_blocks),
            lz4_ns: load(&self.lz4_ns),
            disk_read_ns: load(&self.disk_read_ns),
            stage_write_ns: load(&self.stage_write_ns),
            stage_wait_ns: load(&self.stage_wait_ns),
            net_wait_ns: load(&self.net_wait_ns),
            disk_wait_ns: load(&self.disk_wait_ns),
        }
    }
}

/// The meter of this process.
pub(super) fn meter() -> &'static TransferMeter {
    static METER: OnceLock<TransferMeter> = OnceLock::new();
    METER.get_or_init(TransferMeter::default)
}

/// A span of time on its way into the meter; inert where `enabled` is false.
pub(super) struct Timer(Option<Instant>);

impl Timer {
    pub(super) fn start() -> Self {
        Self(enabled().then(Instant::now))
    }

    /// The span went to `phase`.
    pub(super) fn stop(self, phase: Phase) {
        if let Some(started) = self.0 {
            meter().add(phase, started.elapsed());
        }
    }
}

/// Counts a started content stream and its bytes where measuring is on.
pub(super) fn stream_started() {
    if enabled() {
        meter().stream_started();
    }
}

/// Counts content bytes where measuring is on.
pub(super) fn bytes(raw: usize, wire: usize) {
    if enabled() {
        meter().bytes(raw, wire);
    }
}

/// Counts a packed block where measuring is on.
pub(super) fn lz4_block() {
    if enabled() {
        meter().lz4_block();
    }
}
