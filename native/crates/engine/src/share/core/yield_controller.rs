//! "Cubic with yielding" (plan 2026-11-14 E2): the congestion controller of
//! file connections. Without an interactive session in the process it is
//! exactly its inner controller (Cubic, or what `SE_SHARE_QUIC_CC` names).
//! While one runs (`interactive_presence.rs`), or a person's metadata
//! operation to the connection's device runs or ended less than the hold
//! time ago (`metadata_presence.rs`, `yield_engagement.rs`), it limits its window so that
//! the queueing delay it measures stays at the target `T`, the way LEDBAT
//! does (RFC 6817, retrieved 2026-11-14):
//!
//! - queueing delay = filtered current delay − base delay (§2.4.2); the base
//!   delay is the minimum over `BASE_HISTORY` = 10 one-minute minima
//!   (§2.4.2, §2.5); the current delay is a MIN filter over at most
//!   `CURRENT_FILTER` recent samples, none older than one round trip (§2.4.2:
//!   "A simple MIN filter applied over a small window"; §2.5: samples "not
//!   older than an RTT");
//! - below the target the window grows by `GAIN · off_target · bytes · MSS /
//!   window` with `GAIN` = 1; an application-limited sender is held at its
//!   flight size + 1 MSS, and the flight never falls below 2 MSS (§2.4.2,
//!   §2.5; `ALLOWED_INCREASE` and `MIN_CWND` state both as noq windows);
//! - above the target it shrinks multiplicatively once per round trip by
//!   `W · (delay / T − 1)`, capped at `W / 2` (LEDBAT++,
//!   draft-irtf-iccrg-ledbat-plus-plus-06 §4.2, retrieved 2026-11-14). RFC
//!   6817's additive decrease (one MSS per round trip) would need thousands
//!   of round trips to drain a Cubic window of a fast link;
//! - a loss halves it at most once per round trip (§2.4.2);
//! - the window in force is the minimum of the inner window and this one,
//!   so it is never more aggressive than Cubic (§4.2);
//! - while this window is the one in force, the inner controller does not
//!   grow its own: the acknowledgements belong to a flight it did not allow,
//!   and a window grown on them would be sent as one burst when the
//!   interactive session ends.
//!
//! The flight size of `max_allowed_cwnd` is the data outstanding before an
//! acknowledgement (§2.4.2). noq reports the bytes still in flight after an
//! ACK frame (`Controller::on_end_acks`) and handles every received datagram
//! before it sends again, so the bytes acknowledged since the last packet
//! left are added back. The bound holds only while noq reports the path
//! application limited (§2.4.2: "When the sender is application-limited, the
//! sender's cwnd is clamped down"): applied to a window in use, it pinned
//! that window to the next multiple of the MSS (measurement M2, third run).
//!
//! Delay samples (G1, 2026-11-14): RFC 6817 measures the one-way delay of the
//! data (§2.4.1, §3.4.1), without the time the receiver holds its
//! acknowledgement back. QUIC gives a round trip per ACK frame
//! (`Controller::on_rtt_sample`, vendor patch G1, RFC 9002 §5.1). A sample is
//! that round trip less the ACK delay the peer reported, by the rule of RFC
//! 9002 §5.3. Without the subtraction a delayed ACK (up to the peer's
//! `max_ack_delay`, 25 ms by default, as large as `T`) looked like queueing;
//! queueing on the return path still counts (a known limit of M2).
use std::any::Any;
use std::sync::Arc;
use std::time::{Duration, Instant};

use noq_proto::congestion::{Controller, ControllerFactory, ControllerMetrics};
use noq_proto::RttEstimator;

use super::yield_delay::{network_delay, DelayTracker};
pub(crate) use super::yield_engagement::Engagement;
use super::yield_engagement::{hold_time, PeerHold};

/// Queueing delay the window aims at while interactive sessions run. RFC
/// 6817 §2.5 requires 100 ms or less; measurement M2 decides among 15, 25
/// and 50 ms (estimate until then).
pub(crate) const DEFAULT_TARGET: Duration = Duration::from_millis(25);
/// RFC 6817 §2.5 `MIN_CWND` (2) and `ALLOWED_INCREASE` (1) as noq windows,
/// in MSS. noq 1.3.0 starts a datagram only while `in_flight + MSS < window`
/// (`path_congestion_check`): a window of n MSS carries n − 1 packets, so
/// each bound is one MSS above the flight it stands for.
const MIN_CWND: u64 = 2 + 1;
const ALLOWED_INCREASE: u64 = 1 + 1;
/// The largest multiplicative decrease per round trip (LEDBAT++ §4.2).
const MAX_DECREASE: f64 = 0.5;

/// The yielding window of one path, apart from the inner controller.
#[derive(Clone, Debug)]
pub(crate) struct YieldWindow {
    window: u64,
    mss: u64,
    target: Duration,
    delays: DelayTracker,
    last_decrease: Option<Instant>,
    /// Bytes acknowledged since the last packet left (`on_sent`): with the
    /// bytes still in flight they give the flight size at that send.
    acked: u64,
    /// Growth below one byte not yet added: RFC 6817's increase is a real
    /// number, under one byte per packet above a window of MSS² (2.1 MB).
    carry: f64,
}

impl YieldWindow {
    pub(crate) fn new(initial: u64, mtu: u16, target: Duration) -> Self {
        let mss = u64::from(mtu.max(1));
        Self {
            window: initial.max(MIN_CWND * mss),
            mss,
            target: target.max(Duration::from_millis(1)),
            delays: DelayTracker::default(),
            last_decrease: None,
            acked: 0,
            carry: 0.0,
        }
    }

    pub(crate) fn window(&self) -> u64 {
        self.window
    }

    fn floor(&self) -> u64 {
        MIN_CWND * self.mss
    }

    fn may_decrease(&self, now: Instant, round_trip: Duration) -> bool {
        self.last_decrease
            .is_none_or(|at| now.saturating_duration_since(at) >= round_trip)
    }

    /// One delay sample (`network_delay`) of one ACK frame, recorded also
    /// while not engaged: the base delay is known when a session starts.
    pub(crate) fn on_delay_sample(&mut self, now: Instant, delay: Duration, round_trip: Duration) {
        self.delays.record(now, delay, round_trip);
    }

    /// One acknowledged packet of `bytes`: `round_trip` is the smoothed round
    /// trip, `inner` the inner window, `engaged` whether the window yields
    /// now. The queueing delay is the one of the samples recorded so far.
    pub(crate) fn on_ack(
        &mut self,
        now: Instant,
        bytes: u64,
        round_trip: Duration,
        inner: u64,
        engaged: bool,
    ) {
        self.acked = self.acked.saturating_add(bytes);
        if !engaged {
            // Follows the inner window, so yielding starts from it.
            self.window = inner.max(self.floor());
            return;
        }
        self.window = self.window.min(inner).max(self.floor());
        let Some(queuing) = self.delays.queuing_delay() else {
            return;
        };
        let target = self.target.as_secs_f64();
        let delay = queuing.as_secs_f64();
        if delay <= target {
            let off_target = (target - delay) / target;
            self.carry += off_target * bytes as f64 * self.mss as f64 / self.window as f64;
            let whole = self.carry.floor();
            self.carry -= whole;
            self.window = self.window.saturating_add(whole as u64);
        } else if self.may_decrease(now, round_trip) {
            let share = (delay / target - 1.0).min(MAX_DECREASE);
            let cut = (self.window as f64 * share) as u64;
            self.window = self.window.saturating_sub(cut);
            self.carry = 0.0;
            self.last_decrease = Some(now);
        }
        self.window = self.window.max(self.floor());
    }

    /// The end of one ACK frame with `in_flight` bytes still outstanding
    /// after it, while noq reports the path `app_limited` (RFC 6817
    /// `max_allowed_cwnd`). The flight size of that rule is the one at the
    /// last send (`on_sent`): `in_flight` plus the bytes acknowledged since.
    pub(crate) fn on_end_acks(&mut self, in_flight: u64, engaged: bool, app_limited: bool) {
        if engaged && app_limited {
            let flight = in_flight.saturating_add(self.acked);
            let allowed = flight.saturating_add(ALLOWED_INCREASE * self.mss);
            self.window = self.window.min(allowed).max(self.floor());
        }
    }

    /// A packet that counts as in flight left.
    pub(crate) fn on_sent(&mut self) {
        self.acked = 0;
    }

    /// A congestion event: halve once per round trip.
    pub(crate) fn on_congestion(&mut self, now: Instant, round_trip: Duration) {
        if self.may_decrease(now, round_trip) {
            self.window = (self.window / 2).max(self.floor());
            self.last_decrease = Some(now);
        }
    }

    pub(crate) fn on_mtu(&mut self, mtu: u16) {
        self.mss = u64::from(mtu.max(1));
        self.window = self.window.max(self.floor());
    }

    /// The window in force: never above the inner one.
    pub(crate) fn effective(&self, inner: u64, engaged: bool) -> u64 {
        if engaged {
            inner.min(self.window)
        } else {
            inner
        }
    }
}

/// The inner controller plus the yielding window.
#[derive(Debug)]
pub(crate) struct YieldingController {
    inner: Box<dyn Controller>,
    yielding: YieldWindow,
    engagement: Engagement,
    /// Smoothed round trip of the last acknowledgement.
    round_trip: Duration,
    /// Smallest round trip sample so far, ACK delay included (RFC 9002
    /// §5.2 `min_rtt`; the same samples as the connection's estimator).
    min_rtt: Option<Duration>,
    /// The metadata presence of the connection's device (`yield_engagement.rs`).
    peer: PeerHold,
}

impl YieldingController {
    pub(crate) fn new(
        inner: Box<dyn Controller>,
        engagement: Engagement,
        target: Duration,
        mtu: u16,
    ) -> Self {
        let initial = inner.window();
        Self {
            inner,
            yielding: YieldWindow::new(initial, mtu, target),
            engagement,
            round_trip: Duration::from_millis(333),
            min_rtt: None,
            peer: PeerHold::default(),
        }
    }

    /// Whether the window yields now (`Engagement::engaged`).
    fn engaged(&self) -> bool {
        self.engagement.engaged(self.peer.engaged())
    }

    /// Takes in the device's metadata presence at `now`, with the hold this
    /// path needs now (`yield_engagement::hold_time`).
    fn refresh_peer(&mut self, now: Instant) {
        let hold = hold_time(
            self.round_trip,
            self.yielding.delays.base_delay(),
            self.yielding.target,
        );
        self.peer
            .refresh(self.engagement.peer_activity(), now, hold);
    }

    /// The part of `on_ack` that does not need an `RttEstimator` (tests).
    pub(crate) fn observe_ack(&mut self, now: Instant, bytes: u64, round_trip: Duration) {
        self.round_trip = round_trip;
        self.refresh_peer(now);
        let engaged = self.engaged();
        let inner = self.inner.window();
        self.yielding.on_ack(now, bytes, round_trip, inner, engaged);
    }

    /// One round-trip sample of an ACK frame (`Controller::on_rtt_sample`).
    pub(crate) fn observe_rtt_sample(
        &mut self,
        now: Instant,
        latest_rtt: Duration,
        ack_delay: Duration,
    ) {
        let min_rtt = self.min_rtt.map_or(latest_rtt, |min| min.min(latest_rtt));
        self.min_rtt = Some(min_rtt);
        let delay = network_delay(latest_rtt, ack_delay, min_rtt);
        self.yielding.on_delay_sample(now, delay, self.round_trip);
    }

    /// Whether the yielding window is the one in force now. The inner
    /// controller then sees its acknowledgements as application limited
    /// (Cubic does not grow on those): they acknowledge a flight the inner
    /// window did not allow, and a window grown on them would be released as
    /// one burst when the interactive session ends.
    fn holds_inner(&self) -> bool {
        self.engaged() && self.yielding.window() < self.inner.window()
    }
}

impl Clone for YieldingController {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone_box(),
            yielding: self.yielding.clone(),
            engagement: self.engagement.clone(),
            round_trip: self.round_trip,
            min_rtt: self.min_rtt,
            peer: self.peer,
        }
    }
}

impl Controller for YieldingController {
    fn on_sent(&mut self, now: Instant, bytes: u64, largest_pn: u64) {
        self.inner.on_sent(now, bytes, largest_pn);
    }

    fn on_packet_sent(&mut self, now: Instant, bytes: u16, pn: u64) {
        self.inner.on_packet_sent(now, bytes, pn);
        self.yielding.on_sent();
    }

    fn on_cwnd_limited(&mut self) {
        // Blocked by the yielding window is not blocked by the inner one.
        if !self.holds_inner() {
            self.inner.on_cwnd_limited();
        }
    }

    fn on_ack(
        &mut self,
        now: Instant,
        sent: Instant,
        bytes: u64,
        pn: u64,
        app_limited: bool,
        rtt: &RttEstimator,
    ) {
        let held = self.holds_inner();
        self.inner
            .on_ack(now, sent, bytes, pn, app_limited || held, rtt);
        self.observe_ack(now, bytes, rtt.get());
    }

    fn on_rtt_sample(&mut self, now: Instant, latest_rtt: Duration, ack_delay: Duration) {
        self.inner.on_rtt_sample(now, latest_rtt, ack_delay);
        self.observe_rtt_sample(now, latest_rtt, ack_delay);
    }

    fn on_end_acks(
        &mut self,
        now: Instant,
        in_flight: u64,
        app_limited: bool,
        largest_packet_num_acked: Option<u64>,
    ) {
        let held = self.holds_inner();
        self.inner.on_end_acks(
            now,
            in_flight,
            app_limited || held,
            largest_packet_num_acked,
        );
        self.refresh_peer(now);
        let engaged = self.engaged();
        self.yielding.on_end_acks(in_flight, engaged, app_limited);
    }

    fn on_congestion_event(
        &mut self,
        now: Instant,
        sent: Instant,
        is_persistent_congestion: bool,
        is_ecn: bool,
        lost_bytes: u64,
        largest_lost_pn: u64,
    ) {
        self.inner.on_congestion_event(
            now,
            sent,
            is_persistent_congestion,
            is_ecn,
            lost_bytes,
            largest_lost_pn,
        );
        if self.engaged() {
            self.yielding.on_congestion(now, self.round_trip);
        }
    }

    fn on_packet_lost(&mut self, lost_bytes: u16, pn: u64, now: Instant) {
        self.inner.on_packet_lost(lost_bytes, pn, now);
    }

    fn on_spurious_congestion_event(&mut self) {
        self.inner.on_spurious_congestion_event();
    }

    fn on_mtu_update(&mut self, new_mtu: u16) {
        self.inner.on_mtu_update(new_mtu);
        self.yielding.on_mtu(new_mtu);
    }

    fn on_ack_frequency_update(
        &mut self,
        ack_eliciting_threshold: u64,
        requested_max_ack_delay: Duration,
    ) {
        self.inner
            .on_ack_frequency_update(ack_eliciting_threshold, requested_max_ack_delay);
    }

    fn window(&self) -> u64 {
        self.yielding.effective(self.inner.window(), self.engaged())
    }

    fn metrics(&self) -> ControllerMetrics {
        let mut metrics = self.inner.metrics();
        metrics.congestion_window = self.window();
        metrics
    }

    fn clone_box(&self) -> Box<dyn Controller> {
        Box::new(self.clone())
    }

    fn initial_window(&self) -> u64 {
        self.inner.initial_window()
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

/// Builds a [`YieldingController`] around the inner factory's controller
/// for every path of one connection; all of them share the connection's
/// purpose slot.
pub(crate) struct YieldingFactory {
    pub(crate) inner: Arc<dyn ControllerFactory + Send + Sync>,
    pub(crate) engagement: Engagement,
    pub(crate) target: Duration,
}

impl ControllerFactory for YieldingFactory {
    fn build(self: Arc<Self>, now: Instant, current_mtu: u16) -> Box<dyn Controller> {
        let inner = Arc::clone(&self.inner).build(now, current_mtu);
        Box::new(YieldingController::new(
            inner,
            self.engagement.clone(),
            self.target,
            current_mtu,
        ))
    }
}

#[cfg(test)]
#[path = "yield_controller_tests.rs"]
mod tests;
