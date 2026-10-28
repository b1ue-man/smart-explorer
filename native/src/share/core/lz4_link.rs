//! The LZ4 measurements of one connection (Z-P8). Every content stream of a
//! connection packs by the same rule as before (`compress_policy`), but the
//! sampled ratio, the core rate and the wire measurement belong to the
//! connection: a new stream (a small file, one part of a large one) starts
//! with what its connection learned instead of a second without samples, so
//! small files are packed too once the line is the limit.
//!
//! The wire window counts the waits of all writers of the connection; its
//! share and rate are per writer (divided by the writers seen in the
//! window), which for one stream is exactly the former per-stream meter.
//! No lock is held while a block is packed.
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use crate::transfer::compress_policy::{PolicyInput, BLOCKED_SHARE_MIN, METER_WINDOW};

use super::Lz4Stream;

/// Weight of the newest sample (as in `compress_policy`).
const NEWEST_WEIGHT: f64 = 0.3;
/// Links of outgoing connections kept by `link_for`; the older half goes
/// when the table is full (a link only carries measurements).
const LINKS_MAX: usize = 256;

#[derive(Default)]
struct Measurements {
    ratio: Option<f64>,
    core_bps: Option<f64>,
    next_writer: u64,
    window_start: Option<Instant>,
    bytes: u64,
    blocked: Duration,
    writers: HashSet<u64>,
    /// Per-writer share of the last closed window spent waiting.
    last_share: f64,
    /// Per-writer rate of the last closed window that the wire limited.
    last_wire_bps: u64,
}

/// The shared measurements of one connection direction.
#[derive(Clone, Default)]
pub(in crate::share) struct Lz4Link(Arc<Mutex<Measurements>>);

impl Lz4Link {
    /// A new stream of this connection.
    pub(in crate::share) fn stream(&self) -> Lz4Stream {
        let writer = {
            let mut measurements = self.lock();
            let writer = measurements.next_writer;
            measurements.next_writer = writer.wrapping_add(1);
            writer
        };
        Lz4Stream::on_link(self.clone(), writer)
    }

    fn lock(&self) -> MutexGuard<'_, Measurements> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The rule's input as the connection's measurements stand. Without a
    /// sample the ratio is 1.0 and the core rate 0: nothing is compressed.
    pub(super) fn input(&self, free_cores: f64) -> PolicyInput {
        let measurements = self.lock();
        PolicyInput {
            writer_blocked_share: measurements.last_share,
            wire_bps: measurements.last_wire_bps,
            core_bps: measurements.core_bps.map_or(0, |rate| rate as u64),
            ratio: measurements.ratio.unwrap_or(1.0),
            free_cores,
        }
    }

    /// A compression of `raw` bytes into `packed` bytes took `took`.
    pub(super) fn observe_compression(&self, raw: usize, packed: usize, took: Duration) {
        if raw == 0 {
            return;
        }
        let mut measurements = self.lock();
        let ratio = packed as f64 / raw as f64;
        measurements.ratio = Some(blend(measurements.ratio, ratio));
        let seconds = took.as_secs_f64();
        if seconds > 0.0 {
            measurements.core_bps = Some(blend(measurements.core_bps, raw as f64 / seconds));
        }
    }

    /// Writer `writer` handed `bytes` to the wire and waited `blocked` for
    /// it; `now` closes a window once it is `METER_WINDOW` long.
    pub(super) fn record_send(&self, writer: u64, bytes: usize, blocked: Duration, now: Instant) {
        let mut measurements = self.lock();
        let start = *measurements.window_start.get_or_insert(now);
        measurements.bytes = measurements.bytes.saturating_add(bytes as u64);
        measurements.blocked = measurements.blocked.saturating_add(blocked);
        measurements.writers.insert(writer);
        let length = now.saturating_duration_since(start);
        if length < METER_WINDOW {
            return;
        }
        let writer_seconds = length.as_secs_f64() * measurements.writers.len().max(1) as f64;
        measurements.last_share = (measurements.blocked.as_secs_f64() / writer_seconds).min(1.0);
        if measurements.last_share > BLOCKED_SHARE_MIN {
            measurements.last_wire_bps = (measurements.bytes as f64 / writer_seconds) as u64;
        }
        measurements.window_start = Some(now);
        measurements.bytes = 0;
        measurements.blocked = Duration::ZERO;
        measurements.writers.clear();
    }

    #[cfg(test)]
    pub(in crate::share) fn ratio(&self) -> Option<f64> {
        self.lock().ratio
    }
}

fn blend(old: Option<f64>, new: f64) -> f64 {
    match old {
        Some(old) => old * (1.0 - NEWEST_WEIGHT) + new * NEWEST_WEIGHT,
        None => new,
    }
}

/// The link of the outgoing connection `generation` of `session_key`: every
/// upload stream of that connection shares it.
pub(in crate::share) fn link_for(session_key: &str, generation: usize) -> Lz4Link {
    type Links = Mutex<HashMap<(String, usize), (Lz4Link, Instant)>>;
    static LINKS: OnceLock<Links> = OnceLock::new();
    let Ok(mut links) = LINKS.get_or_init(Links::default).lock() else {
        return Lz4Link::default();
    };
    let now = Instant::now();
    let key = (session_key.to_string(), generation);
    if let Some((link, used)) = links.get_mut(&key) {
        *used = now;
        return link.clone();
    }
    if links.len() >= LINKS_MAX {
        let mut used: Vec<Instant> = links.values().map(|(_, used)| *used).collect();
        used.sort_unstable();
        if let Some(cut) = used.get(used.len() / 2).copied() {
            links.retain(|_, (_, used)| *used > cut);
        }
    }
    let link = Lz4Link::default();
    links.insert(key, (link.clone(), now));
    link
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_task_lz4_share_link_shares_samples_between_streams() {
        let link = Lz4Link::default();
        let first = link.stream();
        // A text block on the first stream samples the ratio for all.
        let text: Vec<u8> = b"text packs well. "
            .iter()
            .copied()
            .cycle()
            .take(65536)
            .collect();
        assert!(
            first.pack(&text).is_none(),
            "no decision before a wire window"
        );
        let ratio = link.ratio().expect("the first block is sampled");
        assert!(ratio < 0.5, "{ratio}");
        let second = link.stream();
        assert!(second.pack(&text).is_none());
        assert_eq!(
            link_for("k", 1).ratio(),
            None,
            "an outgoing link starts empty"
        );
        let again = link_for("k", 1);
        again.observe_compression(100, 50, Duration::from_micros(10));
        assert_eq!(link_for("k", 1).ratio(), Some(0.5));
        assert_eq!(link_for("k", 2).ratio(), None);
    }

    #[test]
    fn speed_task_lz4_share_link_measures_per_writer() {
        let link = Lz4Link::default();
        let start = Instant::now();
        // Two writers, each waiting 30 % of one second: not wire limited,
        // although their waits add up to 60 %.
        link.record_send(1, 1_000_000, Duration::from_millis(300), start);
        link.record_send(2, 1_000_000, Duration::from_millis(300), start);
        link.record_send(1, 0, Duration::ZERO, start + METER_WINDOW);
        let input = link.input(4.0);
        assert!(input.writer_blocked_share < BLOCKED_SHARE_MIN, "{input:?}");
        // One writer waiting 90 %: wire limited at its own rate.
        link.record_send(
            1,
            10_000_000,
            Duration::from_millis(900),
            start + METER_WINDOW,
        );
        link.record_send(1, 0, Duration::ZERO, start + METER_WINDOW * 2);
        let input = link.input(4.0);
        assert!(input.writer_blocked_share > BLOCKED_SHARE_MIN, "{input:?}");
        assert!(input.wire_bps >= 9_000_000, "{input:?}");
    }
}
