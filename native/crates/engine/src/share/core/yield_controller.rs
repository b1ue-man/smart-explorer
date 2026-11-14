//! "Cubic with yielding" (plan 2026-11-14 E2): the congestion controller of
//! file connections. Without an interactive session in the process it is
//! exactly its inner controller (Cubic, or what `SE_SHARE_QUIC_CC` names).
//! While one runs (`interactive_presence.rs`), it limits its window so that
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
//!   window` with `GAIN` = 1, at most to flight size + `ALLOWED_INCREASE` MSS,
//!   never below `MIN_CWND` = 2 MSS (§2.4.2, §2.5);
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
//! acknowledgement (§2.4.2). noq reports the bytes still in flight after it
//! (`Controller::on_end_acks`), so the bytes acknowledged since the last
//! report are added back.
//!
//! QUIC gives the controller round-trip samples only (`now − sent` of an
//! acknowledged packet), not RFC 6817's one-way delay: queueing on the return
//! path counts as delay too (a known limit of measurement M2).
use std::any::Any;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use noq_proto::congestion::{Controller, ControllerFactory, ControllerMetrics};
use noq_proto::RttEstimator;

use super::interactive_presence::InteractivePresence;
use super::transport_purpose::PurposeSlot;

/// Queueing delay the window aims at while interactive sessions run. RFC
/// 6817 §2.5 requires 100 ms or less; measurement M2 decides among 15, 25
/// and 50 ms (estimate until then).
pub(crate) const DEFAULT_TARGET: Duration = Duration::from_millis(25);
const BASE_HISTORY: usize = 10;
const BASE_INTERVAL: Duration = Duration::from_secs(60);
const CURRENT_FILTER: usize = 4;
const MIN_CWND: u64 = 2;
const ALLOWED_INCREASE: u64 = 1;
/// The largest multiplicative decrease per round trip (LEDBAT++ §4.2).
const MAX_DECREASE: f64 = 0.5;

/// Base and current delay of one path (RFC 6817 §2.4.2).
#[derive(Clone, Debug, Default)]
pub(crate) struct DelayTracker {
    /// One minimum per minute: (start of the minute, minimum).
    base: VecDeque<(Instant, Duration)>,
    /// Recent samples: (arrival, delay).
    current: VecDeque<(Instant, Duration)>,
}

impl DelayTracker {
    /// Records the delay `sample` seen at `now`; `round_trip` bounds the age
    /// of the current samples.
    pub(crate) fn record(&mut self, now: Instant, sample: Duration, round_trip: Duration) {
        match self.base.back_mut() {
            Some((start, minimum)) if now.saturating_duration_since(*start) < BASE_INTERVAL => {
                *minimum = (*minimum).min(sample);
            }
            _ => {
                self.base.push_back((now, sample));
                while self.base.len() > BASE_HISTORY {
                    self.base.pop_front();
                }
            }
        }
        self.current.push_back((now, sample));
        while self.current.len() > CURRENT_FILTER {
            self.current.pop_front();
        }
        while self.current.len() > 1
            && self
                .current
                .front()
                .is_some_and(|(at, _)| now.saturating_duration_since(*at) > round_trip)
        {
            self.current.pop_front();
        }
    }

    /// The smallest delay of the history.
    pub(crate) fn base_delay(&self) -> Option<Duration> {
        self.base.iter().map(|(_, minimum)| *minimum).min()
    }

    /// Filtered current delay minus base delay.
    pub(crate) fn queuing_delay(&self) -> Option<Duration> {
        let current = self.current.iter().map(|(_, delay)| *delay).min()?;
        Some(current.saturating_sub(self.base_delay()?))
    }
}

/// The yielding window of one path, apart from the inner controller.
#[derive(Clone, Debug)]
pub(crate) struct YieldWindow {
    window: u64,
    mss: u64,
    target: Duration,
    delays: DelayTracker,
    last_decrease: Option<Instant>,
    /// Bytes acknowledged since the last `on_end_acks`: with the bytes still
    /// in flight they give the flight size before these acknowledgements.
    acked: u64,
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

    /// One acknowledged packet: `sent` is its send time, `round_trip` the
    /// smoothed round trip, `inner` the inner window, `engaged` whether the
    /// window yields now. Delay history is kept also while not engaged, so
    /// the base delay is known when an interactive session starts.
    pub(crate) fn on_ack(
        &mut self,
        now: Instant,
        sent: Instant,
        bytes: u64,
        round_trip: Duration,
        inner: u64,
        engaged: bool,
    ) {
        self.acked = self.acked.saturating_add(bytes);
        self.delays
            .record(now, now.saturating_duration_since(sent), round_trip);
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
            let grow = off_target * bytes as f64 * self.mss as f64 / self.window as f64;
            self.window = self.window.saturating_add(grow as u64);
        } else if self.may_decrease(now, round_trip) {
            let share = (delay / target - 1.0).min(MAX_DECREASE);
            let cut = (self.window as f64 * share) as u64;
            self.window = self.window.saturating_sub(cut);
            self.last_decrease = Some(now);
        }
        self.window = self.window.max(self.floor());
    }

    /// The end of one batch of acknowledgements with `in_flight` bytes still
    /// outstanding after it (RFC 6817 `max_allowed_cwnd`). The flight size of
    /// that rule is the one before the batch: `in_flight` plus the bytes the
    /// batch acknowledged. With the bytes after it alone, every batch of two
    /// or more packets would shrink a fully used window to its floor.
    pub(crate) fn on_end_acks(&mut self, in_flight: u64, engaged: bool) {
        let flight = in_flight.saturating_add(std::mem::take(&mut self.acked));
        if engaged {
            let allowed = flight.saturating_add(ALLOWED_INCREASE * self.mss);
            self.window = self.window.min(allowed).max(self.floor());
        }
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

/// Whether a controller yields now: its connection's purpose yields, the
/// switch allows it and an interactive session runs.
#[derive(Clone, Debug)]
pub(crate) struct Engagement {
    pub(crate) slot: Arc<PurposeSlot>,
    pub(crate) presence: &'static InteractivePresence,
    /// Off: plain inner controller ("vorher" of measurement M2).
    pub(crate) enabled: bool,
}

impl Engagement {
    fn engaged(&self) -> bool {
        self.enabled && self.slot.yields() && self.presence.active()
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
        }
    }

    /// The part of `on_ack` that does not need an `RttEstimator` (tests).
    pub(crate) fn observe_ack(
        &mut self,
        now: Instant,
        sent: Instant,
        bytes: u64,
        round_trip: Duration,
    ) {
        self.round_trip = round_trip;
        let engaged = self.engagement.engaged();
        let inner = self.inner.window();
        self.yielding
            .on_ack(now, sent, bytes, round_trip, inner, engaged);
    }

    /// Whether the yielding window is the one in force now. The inner
    /// controller then sees its acknowledgements as application limited
    /// (Cubic does not grow on those): they acknowledge a flight the inner
    /// window did not allow, and a window grown on them would be released as
    /// one burst when the interactive session ends.
    fn holds_inner(&self) -> bool {
        self.engagement.engaged() && self.yielding.window() < self.inner.window()
    }
}

impl Clone for YieldingController {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone_box(),
            yielding: self.yielding.clone(),
            engagement: self.engagement.clone(),
            round_trip: self.round_trip,
        }
    }
}

impl Controller for YieldingController {
    fn on_sent(&mut self, now: Instant, bytes: u64, largest_pn: u64) {
        self.inner.on_sent(now, bytes, largest_pn);
    }

    fn on_packet_sent(&mut self, now: Instant, bytes: u16, pn: u64) {
        self.inner.on_packet_sent(now, bytes, pn);
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
        self.observe_ack(now, sent, bytes, rtt.get());
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
        let engaged = self.engagement.engaged();
        self.yielding.on_end_acks(in_flight, engaged);
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
        if self.engagement.engaged() {
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
        self.yielding
            .effective(self.inner.window(), self.engagement.engaged())
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
