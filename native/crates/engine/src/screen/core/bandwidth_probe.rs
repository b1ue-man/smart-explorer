//! The line's available rate, measured as Shadow's „Auto-detect“ does
//! (user 2026-11-25: „alles drum herum an die verfügbare datenrate anpasst
//! die verfügbar ist, auch bei änderung“; support.shadow.tech „Configuring
//! Bitrate and Bandwidth in Shadow PC“, read 2026-11-25: the app measures the
//! connection and sets the bit rate, „Restart Auto-Detection“ measures
//! again). The encoder gets the result (`RateController::set_measured`).
//!
//! The host opens a probe stream on the session's connection: a QUIC uni
//! stream with the 15-byte head `SCRP`, version `u8`, `probe_id u64`,
//! `duration_ms u16` (big-endian), then filler bytes as fast as QUIC takes
//! them for `duration_ms`; then it resets the stream, so filler still in its
//! send buffer is dropped and never delays a picture. The probe stream's
//! priority (-1) is below the picture streams (0) and the control stream (1):
//! pictures go first, the probe fills what the line carries beyond them. The
//! viewer tells the probe stream from a picture stream (`SCRV`) by its first
//! four bytes, counts every byte that reaches the connection
//! (`ConnectionStats::udp_rx`: probe, pictures, control and sound, at the
//! moment they arrive, also while a picture reader waits for its decoder) in
//! windows of `WINDOW_US` and answers `PROBE_RESULT` with the largest
//! window's rate, the bottleneck rate. The count includes the QUIC packet
//! headers (a few percent): the line carries them too.
//!
//! Numbers, and why:
//! - Window 50 ms (`WINDOW_US`): at 10 Mbit/s that is 62 KB, about 45
//!   packets, so the timer's jitter (a few ms) and the bursts of a Wi-Fi
//!   aggregate (a few ms) change a window's rate by less than a tenth; a
//!   shorter window counts such bursts as rate, a longer one mixes the ramp
//!   of the congestion window into its average and lowers the maximum.
//! - Duration 10 smoothed round trips, at least 500 ms, at most 1.5 s
//!   (`probe_duration_us`): the probe starts from the congestion window the
//!   pictures left, as small as the initial window (about 12 KB) on a new
//!   connection. Slow start doubles it once per round trip; 100 Mbit/s at
//!   50 ms need about 6 doublings, 50 Mbit/s over a relay at 100 ms about 7.
//!   Ten round trips leave several full windows after the ramp; 500 ms keep
//!   about ten windows on a LAN, 1.5 s bound the load a probe puts on the
//!   line.
//! - Result timeout 3 s (`RESULT_TIMEOUT_US`): the longest probe, the
//!   viewer's grace (`READ_GRACE_US`) and the way back of its answer.
//!
//! Memory: the host writes the filler from one static buffer
//! (`Bytes::from_static`, no copy per write), the viewer drops every chunk it
//! reads, and its meter keeps a fixed handful of numbers, no samples.
//!
//! Older peers: a host announces `delivery_v1` in `SERVER_HELLO`; only then
//! does a viewer send `SET_DELIVERY` and `REMEASURE`. A viewer announces
//! `bandwidth_probe` in its hello; only then does a host open probe
//! streams, and a host without results keeps the stream's table rate. When
//! the host measures: `bandwidth_probe_schedule.rs`; the host session's side:
//! `bandwidth_probe_host.rs`; the streams: `bandwidth_probe_io.rs`.
use std::io;

use super::delivery::Delivery;
use super::gop_stream::VIEWER_UNI_STREAMS;
use super::wire::{invalid, BeReader};
use super::wire_hello::{ScreenClientHello, ScreenServerHello, SessionMode};

#[path = "bandwidth_probe_host.rs"]
mod host;
#[path = "bandwidth_probe_io.rs"]
mod probe_io;
#[path = "bandwidth_probe_schedule.rs"]
mod schedule;
pub use host::LineLink;
pub use probe_io::{read_probe, send_probe};
pub use schedule::{LineFacts, ProbeFacts, ProbeReason, ProbeSchedule, RouteKey};

/// `ScreenClientHello::bandwidth_probe`: the viewer reads probe streams and
/// answers `PROBE_RESULT`.
pub const BANDWIDTH_PROBE_V1: u16 = 1;
/// Viewer to host: the „Übertragung“, one byte (0 `realtime`, 1 `reliable`).
pub const SET_DELIVERY: u8 = 58;
/// Viewer to host: „Neu messen“, the JSON object `{}`.
pub const REMEASURE: u8 = 59;
/// Viewer to host: `probe_id`, `bps`, `bytes`, `duration_us`, each `u64`
/// big-endian.
pub const PROBE_RESULT: u8 = 60;
pub const SET_DELIVERY_BYTES: usize = 1;
pub const REMEASURE_BYTES: usize = 64;
pub const PROBE_RESULT_BYTES: usize = 32;

pub const PROBE_MAGIC: [u8; 4] = *b"SCRP";
pub const PROBE_VERSION: u8 = 1;
pub const PROBE_HEAD_BYTES: usize = 15;
/// Uni streams a viewer that announced the probe allows beyond the picture
/// streams (`VIEWER_UNI_STREAMS`): the probe never takes a picture's slot.
pub const PROBE_STREAMS: u32 = 1;
/// Below the picture streams (0) and the control stream (1).
pub const PROBE_STREAM_PRIORITY: i32 = -1;
/// The filler the host writes per call, from one static buffer.
pub const FILLER_BYTES: usize = 16 * 1024;
pub(crate) static FILLER: [u8; FILLER_BYTES] = [0; FILLER_BYTES];

pub const WINDOW_US: u64 = 50_000;
pub const MIN_PROBE_US: u64 = 500_000;
pub const MAX_PROBE_US: u64 = 1_500_000;
/// Smoothed round trips a probe lasts within `MIN_PROBE_US..=MAX_PROBE_US`.
pub const PROBE_RTTS: u64 = 10;
/// The viewer stops counting this long after the duration its head names,
/// when the host's reset has not arrived by then.
pub const READ_GRACE_US: u64 = 250_000;
/// The longest duration a viewer counts, whatever a head names.
pub const MAX_HEAD_MS: u16 = 5_000;
/// A probe without an answer this long after its start is dropped.
pub const RESULT_TIMEOUT_US: u64 = 3_000_000;

/// The duration of a probe on a path with the smoothed round trip `srtt_us`.
pub fn probe_duration_us(srtt_us: u64) -> u64 {
    srtt_us
        .saturating_mul(PROBE_RTTS)
        .clamp(MIN_PROBE_US, MAX_PROBE_US)
}

/// The head of a probe stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProbeHead {
    pub probe_id: u64,
    pub duration_ms: u16,
}

impl ProbeHead {
    /// The head of probe `probe_id` on a path with the round trip `srtt_us`.
    pub fn for_rtt(probe_id: u64, srtt_us: u64) -> Self {
        let duration_ms = u16::try_from(probe_duration_us(srtt_us) / 1000).unwrap_or(u16::MAX);
        Self {
            probe_id,
            duration_ms,
        }
    }

    pub fn duration_us(&self) -> u64 {
        u64::from(self.duration_ms) * 1000
    }

    pub fn encode(&self) -> [u8; PROBE_HEAD_BYTES] {
        let mut out = [0u8; PROBE_HEAD_BYTES];
        out[..4].copy_from_slice(&PROBE_MAGIC);
        out[4] = PROBE_VERSION;
        out[5..13].copy_from_slice(&self.probe_id.to_be_bytes());
        out[13..].copy_from_slice(&self.duration_ms.to_be_bytes());
        out
    }

    /// The head after its magic, which the viewer read to tell the stream
    /// from a picture stream.
    pub fn decode_after_magic(rest: &[u8; PROBE_HEAD_BYTES - 4]) -> io::Result<Self> {
        let mut reader = BeReader::new(rest);
        if reader.u8()? != PROBE_VERSION {
            return Err(invalid("probe stream of an unknown version"));
        }
        let probe_id = reader.u64()?;
        let duration_ms = reader.u16()?;
        reader.finish()?;
        if probe_id == 0 || duration_ms == 0 {
            return Err(invalid("probe stream needs an id and a duration"));
        }
        Ok(Self {
            probe_id,
            duration_ms: duration_ms.min(MAX_HEAD_MS),
        })
    }
}

/// `PROBE_RESULT`: what the viewer counted during probe `probe_id`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProbeResult {
    pub probe_id: u64,
    /// The largest window's rate in bit/s; 0 when no window completed.
    pub bps: u64,
    /// Every byte that reached the connection while the viewer counted.
    pub bytes: u64,
    /// How long it counted.
    pub duration_us: u64,
}

impl ProbeResult {
    pub fn validate(&self) -> io::Result<()> {
        if self.probe_id == 0 {
            return Err(invalid("probe result needs the probe's id"));
        }
        Ok(())
    }

    pub fn encode(&self) -> Vec<u8> {
        [self.probe_id, self.bps, self.bytes, self.duration_us]
            .iter()
            .flat_map(|value| value.to_be_bytes())
            .collect()
    }

    pub fn decode(payload: &[u8]) -> io::Result<Self> {
        let mut reader = BeReader::new(payload);
        let result = Self {
            probe_id: reader.u64()?,
            bps: reader.u64()?,
            bytes: reader.u64()?,
            duration_us: reader.u64()?,
        };
        reader.finish()?;
        result.validate()?;
        Ok(result)
    }
}

/// The byte of `SET_DELIVERY`.
pub fn delivery_code(delivery: Delivery) -> u8 {
    match delivery {
        Delivery::Realtime => 0,
        Delivery::Reliable => 1,
    }
}

/// The payload of `SET_DELIVERY`; another byte or length is refused.
pub fn delivery_of_payload(payload: &[u8]) -> io::Result<Delivery> {
    match payload {
        [0] => Ok(Delivery::Realtime),
        [1] => Ok(Delivery::Reliable),
        _ => Err(invalid("SET_DELIVERY carries an unknown delivery")),
    }
}

/// Whether the viewer may send `SET_DELIVERY` and `REMEASURE` to this host.
pub fn delivery_negotiated(server: &ScreenServerHello) -> bool {
    server.delivery_v1
}

/// The uni streams a viewer allows for `hello`, which it marks as reading
/// probe streams: none for a probe session, the picture streams for a
/// metadata channel, one more for the probe otherwise.
pub fn viewer_streams(hello: &mut ScreenClientHello, metadata: bool) -> u32 {
    if hello.mode == SessionMode::Probe {
        return 0;
    }
    if metadata {
        return VIEWER_UNI_STREAMS;
    }
    hello.bandwidth_probe = Some(BANDWIDTH_PROBE_V1);
    VIEWER_UNI_STREAMS + PROBE_STREAMS
}

/// The viewer's count during one probe: the bytes that reached the
/// connection (a cumulative total) per window of at least `WINDOW_US`, and
/// the largest window's rate. A fixed handful of numbers, no samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProbeMeter {
    start_us: u64,
    start_total: u64,
    window_us: u64,
    window_total: u64,
    last_us: u64,
    last_total: u64,
    max_bps: u64,
}

impl ProbeMeter {
    /// Starts at `now_us` with the connection's received total `total`.
    pub fn new(now_us: u64, total: u64) -> Self {
        Self {
            start_us: now_us,
            start_total: total,
            window_us: now_us,
            window_total: total,
            last_us: now_us,
            last_total: total,
            max_bps: 0,
        }
    }

    /// The total at `now_us`; a window of at least `WINDOW_US` closes and
    /// counts with its actual length (a late timer makes it longer, never
    /// its rate higher). A total below the last one counts as the last.
    pub fn sample(&mut self, now_us: u64, total: u64) {
        let total = total.max(self.last_total);
        let now_us = now_us.max(self.last_us);
        self.last_us = now_us;
        self.last_total = total;
        let elapsed = now_us - self.window_us;
        if elapsed < WINDOW_US {
            return;
        }
        let bits = u128::from(total - self.window_total) * 8 * 1_000_000;
        let bps = u64::try_from(bits / u128::from(elapsed)).unwrap_or(u64::MAX);
        self.max_bps = self.max_bps.max(bps);
        self.window_us = now_us;
        self.window_total = total;
    }

    /// The answer for probe `probe_id`.
    pub fn result(&self, probe_id: u64) -> ProbeResult {
        ProbeResult {
            probe_id,
            bps: self.max_bps,
            bytes: self.last_total - self.start_total,
            duration_us: self.last_us - self.start_us,
        }
    }
}

#[cfg(test)]
#[path = "bandwidth_probe_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "bandwidth_probe_line_tests.rs"]
mod line_tests;
