//! Start of the bit rate of a picture stream (plan 2026-11-14 „Bildkette“,
//! decision B-05): the start value per peer and path is the rate this process
//! last measured as sustainable for them (kept in memory only, never stored);
//! without one the target ramps up, doubling once per max(round trip, frame
//! interval) while the delay-based detector sees no overuse, and the first
//! persistent overuse ends the ramp. The procedure is slow start (RFC 5681
//! section 3.1: the window doubles per round trip of acknowledged data, i.e.
//! at most twice what was delivered) applied to the target of the encoder, as
//! GCC probes above its estimate (draft-ietf-rmcat-gcc-02); no number of either
//! is taken over besides the factor two of slow start.
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, PoisonError};

use super::{PathKind, RateController, RateWindow};

/// Growth per interval during the ramp: slow start doubles per round trip.
pub const RAMP_FACTOR: f64 = 2.0;
/// What a stale picture stream leaves of the target (`on_stale_stream`).
pub const STALE_DECREASE: f64 = 0.5;

/// Sustainable rates of this process, one per peer and path that opened a
/// session; bounded by the paired devices of this installation.
fn memo() -> &'static Mutex<HashMap<(String, PathKind), u64>> {
    static MEMO: OnceLock<Mutex<HashMap<(String, PathKind), u64>>> = OnceLock::new();
    MEMO.get_or_init(Default::default)
}

/// The rate last measured as sustainable for `peer` on `path`.
pub fn remembered(peer: &str, path: PathKind) -> Option<u64> {
    memo()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&(peer.to_owned(), path))
        .copied()
}

/// Notes `bps` as the sustainable rate of `peer` on `path`.
pub fn remember(peer: &str, path: PathKind, bps: u64) {
    memo()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert((peer.to_owned(), path), bps);
}

/// Where the start of one controller stands.
#[derive(Clone, Debug)]
pub struct Probe {
    peer: Option<String>,
    /// The target ramps up (no measured start value, no overuse yet).
    ramping: bool,
    /// The target was measured against the path (an overuse lowered it, or it
    /// started from a measured value): it may be remembered.
    measured: bool,
}

impl Default for Probe {
    fn default() -> Self {
        Self::new()
    }
}

impl Probe {
    /// A controller without a known peer ramps from the path's start value.
    pub fn new() -> Self {
        Self {
            peer: None,
            ramping: true,
            measured: false,
        }
    }

    pub fn ramping(&self) -> bool {
        self.ramping
    }

    pub fn peer(&self) -> Option<&str> {
        self.peer.as_deref()
    }

    /// Binds the peer once; the remembered start value of `path`, if any.
    pub fn bind(&mut self, peer: &str, path: PathKind) -> Option<u64> {
        if self.peer.is_some() {
            return None;
        }
        self.peer = Some(peer.to_owned());
        self.start(path)
    }

    /// A new path (or the first one): the remembered value ends the ramp
    /// before it starts; without one the ramp starts over.
    pub fn start(&mut self, path: PathKind) -> Option<u64> {
        let known = self.peer.as_deref().and_then(|peer| remembered(peer, path));
        self.ramping = known.is_none();
        self.measured = known.is_some();
        known
    }

    /// Persistent overuse or high loss: the path's limit was found.
    pub fn on_limit(&mut self) {
        self.ramping = false;
        self.measured = true;
    }

    /// Remembers `bps` for the bound peer once it is a measured rate.
    pub fn note(&self, path: PathKind, bps: u64) {
        if let (Some(peer), true) = (self.peer.as_deref(), self.measured) {
            remember(peer, path, bps);
        }
    }
}

/// The start of the target per peer and path.
impl RateController {
    /// Binds the peer once: a rate remembered for it and this path replaces
    /// the start value and ends the ramp before it starts (B-05); the relay
    /// guard does not cut it (`model::session_start_bps`).
    pub fn bind_peer(&mut self, peer: &str) {
        if let Some(known) = self.probe.bind(peer, self.path) {
            self.delay_target = self.link_clamp(self.start_value(Some(known)) as f64);
            self.sustained = Some(self.delay_target);
        }
    }

    pub fn peer_bound(&self) -> bool {
        self.probe.peer().is_some()
    }

    /// The rate the path was last measured to carry: the target, at most
    /// what a limit left of it (persistent overuse, a stale stream) or what
    /// delivery backed since. The growth of an application-limited sender
    /// (plan 5.3a) raises the target without a measurement of the path and
    /// stays out of it.
    pub fn sustained_bps(&self) -> u64 {
        let target = self.target_bps();
        self.sustained
            .map_or(target, |bps| target.min(self.clamp(bps).round() as u64))
    }

    /// Growth that delivery backs (at most the growth factor times what was
    /// delivered): it raises the sustained rate, never lowers it.
    pub(super) fn note_delivered(&mut self, backed_bps: f64) {
        let proven = self.clamp(backed_bps).min(self.delay_target);
        self.sustained = Some(self.sustained.map_or(proven, |known| known.max(proven)));
    }

    /// Remembers the sustained rate for the bound peer once the target was
    /// measured against the path (once a second, `Pipeline::second`).
    pub fn remember(&self) {
        self.probe.note(self.path, self.sustained_bps());
    }

    /// The picture stream went stale (`pacing::stale_after_us`, B-35): the
    /// frames handed to it were not acknowledged within the time the line
    /// needs for them, so the path does not carry the target. No
    /// acknowledgement of these frames follows (the stream is reset and its
    /// frames are forgotten), so the delay-based controller never sees this
    /// overuse and the next frames would be sized for the same target and go
    /// stale as well. The target halves, as the sender of RFC 5681 (section
    /// 3.1, equation 4) halves after a loss it detects by timeout, the
    /// counterpart of the slow start that doubled it; the ramp ends, and the
    /// event is back pressure, so an application-limited sender does not
    /// grow again within one rate window. Plan 2026-11-17: at most once per
    /// `stale_us` (the stale time of the stream that went stale), so the
    /// resets of one congested period halve once, not once each.
    pub fn on_stale_stream(&mut self, now_us: u64, stale_us: u64) {
        self.pressure_us = Some(now_us);
        self.probe.on_limit();
        self.gradient.reset();
        self.overuse_streak = 0;
        self.last_update_us = now_us;
        let recent = self
            .stale_halved
            .is_some_and(|(at, window)| now_us.saturating_sub(at) < window);
        if recent {
            return;
        }
        self.stale_halved = Some((now_us, stale_us));
        self.delay_target = self.link_clamp(self.target_bps() as f64 * STALE_DECREASE);
        self.sustained = Some(self.delay_target);
        // The tiles keep the lowered target for a while (`tiles_rate`).
        self.tiles.on_congestion(now_us);
    }

    /// A new path starts over from its remembered rate or the start of the
    /// stream on it (`model::session_start_bps`) and discards the previous
    /// path's loss ceiling.
    pub fn set_path(&mut self, path: PathKind) {
        if path != self.path {
            self.path = path;
            let known = self.probe.start(path);
            self.delay_target = self.link_clamp(self.start_value(known) as f64);
            self.sustained = known.map(|_| self.delay_target);
            self.stale_halved = None;
            self.loss_target = f64::INFINITY;
            self.gradient.reset();
            self.received = RateWindow::default();
            self.sent = RateWindow::default();
            // P9: delivery rate and round trip belong to the old path.
            self.queue = super::queue::QueueState::default();
            self.loss_counters = None;
            self.congestion_events = None;
            self.overuse_streak = 0;
            self.pressure_us = None;
            self.loss_pressure = false;
        }
    }
}
