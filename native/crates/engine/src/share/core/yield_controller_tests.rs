//! P2 of plan `docs/plaene/2026-11-14-verbindungen-je-zweck` (E2): the
//! yielding controller. Remote suite only (AGENTS.md).
use super::*;
use crate::share::keepalive::interactive_presence::{InteractiveKind, InteractivePresence};
use crate::share::keepalive::transport_purpose::{PurposeSlot, TransportPurpose};
use crate::share::keepalive::yield_delay::CURRENT_FILTER;

const MTU: u16 = 1200;
const MSS: u64 = MTU as u64;
const MS: Duration = Duration::from_millis(1);

/// An inner controller with a fixed window.
#[derive(Clone, Debug)]
struct Fixed(u64);

impl Controller for Fixed {
    fn on_congestion_event(&mut self, _: Instant, _: Instant, _: bool, _: bool, _: u64, _: u64) {}
    fn on_mtu_update(&mut self, _: u16) {}
    fn window(&self) -> u64 {
        self.0
    }
    fn clone_box(&self) -> Box<dyn Controller> {
        Box::new(self.clone())
    }
    fn initial_window(&self) -> u64 {
        self.0
    }
    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

/// An inner controller that grows by one packet on every batch of
/// acknowledgements that is not application limited, as Cubic grows only on
/// acknowledgements of a flight its own window limited.
#[derive(Clone, Debug)]
struct Growing(u64);

impl Controller for Growing {
    fn on_end_acks(&mut self, _: Instant, _: u64, app_limited: bool, _: Option<u64>) {
        if !app_limited {
            self.0 += MSS;
        }
    }
    fn on_congestion_event(&mut self, _: Instant, _: Instant, _: bool, _: bool, _: u64, _: u64) {}
    fn on_mtu_update(&mut self, _: u16) {}
    fn window(&self) -> u64 {
        self.0
    }
    fn clone_box(&self) -> Box<dyn Controller> {
        Box::new(self.clone())
    }
    fn initial_window(&self) -> u64 {
        self.0
    }
    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

fn presence() -> &'static InteractivePresence {
    Box::leak(Box::new(InteractivePresence::new()))
}

fn controller(
    inner: u64,
    purpose: TransportPurpose,
    presence: &'static InteractivePresence,
    enabled: bool,
) -> YieldingController {
    YieldingController::new(
        Box::new(Fixed(inner)),
        Engagement {
            slot: Arc::new(PurposeSlot::new(Some(purpose))),
            presence,
            enabled,
        },
        DEFAULT_TARGET,
        MTU,
    )
}

/// Acknowledges `count` full packets, one ACK frame each, one every
/// millisecond from `start`, in noq's order (`on_ack`, `on_end_acks`, then
/// the frame's round-trip sample `latest`, of which the peer reports
/// `ack_delay` as held back); returns the time after the last one.
fn acks_with(
    c: &mut YieldingController,
    start: Instant,
    count: u32,
    latest: Duration,
    ack_delay: Duration,
) -> Instant {
    let mut now = start;
    for _ in 0..count {
        now += MS;
        c.observe_ack(now, MSS, latest);
        c.on_end_acks(now, u64::MAX / 2, false, None);
        c.observe_rtt_sample(now, latest, ack_delay);
    }
    now
}

/// `acks_with` for a peer that acknowledges at once (`delay` is all path).
fn acks(c: &mut YieldingController, start: Instant, count: u32, delay: Duration) -> Instant {
    acks_with(c, start, count, delay, Duration::ZERO)
}

#[test]
fn verbindungen_task_yield_never_above_cubic() {
    let presence = presence();
    let inner = 400 * MSS;
    for engaged in [false, true] {
        let _guard = engaged.then(|| presence.enter(InteractiveKind::ScreenHost));
        let mut c = controller(inner, TransportPurpose::Files, presence, true);
        let mut now = Instant::now() + Duration::from_secs(1);
        for (round, delay) in [5u32, 5, 80, 200, 5, 5, 5, 30, 5].into_iter().enumerate() {
            now = acks(&mut c, now, 500, delay * MS);
            assert!(c.window() <= inner, "round {round}: {}", c.window());
            if round % 3 == 0 {
                c.on_congestion_event(now, now, false, false, MSS, 1);
                assert!(c.window() <= inner);
            }
        }
        // Growth below the target never passes the inner window either.
        now = acks(&mut c, now, 20_000, 5 * MS);
        assert!(c.window() <= inner && c.window() >= 2 * MSS);
        let _ = now;
    }
}

#[test]
fn verbindungen_task_yield_shrinks_only_with_presence() {
    let presence = presence();
    let inner = 400 * MSS;
    let start = Instant::now() + Duration::from_secs(1);
    // Base delay 10 ms, then 100 ms: 90 ms queueing, above the 25 ms target.
    let mut idle = controller(inner, TransportPurpose::Files, presence, true);
    let now = acks(&mut idle, start, 200, 10 * MS);
    acks(&mut idle, now, 2_000, 100 * MS);
    assert_eq!(
        idle.window(),
        inner,
        "no interactive session: plain inner window"
    );

    let guard = presence.enter(InteractiveKind::TcpPump);
    let mut busy = controller(inner, TransportPurpose::Files, presence, true);
    let now = acks(&mut busy, start, 200, 10 * MS);
    assert_eq!(busy.window(), inner, "below target the inner window holds");
    let now = acks(&mut busy, now, 2_000, 100 * MS);
    let shrunk = busy.window();
    assert!(
        shrunk < inner / 4,
        "queueing above target shrinks: {shrunk}"
    );
    assert!(shrunk >= 2 * MSS);
    // The session ends: the inner window is back at once.
    drop(guard);
    assert_eq!(busy.window(), inner);
    let _ = now;

    // Other purposes and the "vorher" switch never yield.
    let _guard = presence.enter(InteractiveKind::ScreenViewer);
    for (purpose, enabled) in [
        (TransportPurpose::Exec, true),
        (TransportPurpose::Tcp, true),
        (TransportPurpose::Files, false),
    ] {
        let mut c = controller(inner, purpose, presence, enabled);
        let now = acks(&mut c, start, 200, 10 * MS);
        acks(&mut c, now, 2_000, 100 * MS);
        assert_eq!(c.window(), inner, "{purpose:?} enabled={enabled}");
    }
}

#[test]
fn verbindungen_task_yield_grows_back_below_target() {
    let presence = presence();
    let _guard = presence.enter(InteractiveKind::ExecTerminal);
    let inner = 400 * MSS;
    let start = Instant::now() + Duration::from_secs(1);
    let mut c = controller(inner, TransportPurpose::Files, presence, true);
    let now = acks(&mut c, start, 200, 10 * MS);
    let now = acks(&mut c, now, 2_000, 100 * MS);
    let low = c.window();
    // The queue drains: delay back at the base, the window grows again.
    acks(&mut c, now, 20_000, 10 * MS);
    assert!(c.window() > low, "{} > {low}", c.window());
    assert!(c.window() <= inner);
}

#[test]
fn verbindungen_task_yield_loss_halves_once_per_round_trip() {
    let mut window = YieldWindow::new(100 * MSS, MTU, DEFAULT_TARGET);
    let now = Instant::now();
    let round_trip = 20 * MS;
    window.on_congestion(now, round_trip);
    assert_eq!(window.window(), 50 * MSS);
    window.on_congestion(now + MS, round_trip);
    assert_eq!(
        window.window(),
        50 * MSS,
        "second loss in the same round trip"
    );
    window.on_congestion(now + round_trip, round_trip);
    assert_eq!(window.window(), 25 * MSS);
    for step in 0..20 {
        window.on_congestion(now + round_trip * (2 + step), round_trip);
    }
    // RFC 6817 MIN_CWND, two packets in flight: noq 1.3.0 starts a datagram
    // only while in_flight + MSS < window, which takes a window of 3 MSS.
    assert_eq!(window.window(), 3 * MSS, "never below MIN_CWND");
    // The flight size of an application-limited sender bounds the window
    // (RFC 6817 max_allowed_cwnd): one packet more than the flight, which
    // under the same send gate is a window of the flight + 2 MSS.
    let mut capped = YieldWindow::new(100 * MSS, MTU, DEFAULT_TARGET);
    capped.on_end_acks(10 * MSS, true, true);
    assert_eq!(capped.window(), 12 * MSS);
    capped.on_end_acks(0, false, true);
    assert_eq!(capped.window(), 12 * MSS, "not engaged: unchanged");
    capped.on_end_acks(0, true, false);
    assert_eq!(capped.window(), 12 * MSS, "window in use: unchanged");
}

#[test]
fn verbindungen_task_yield_full_window_survives_acknowledgements_in_batches() {
    // A sender that keeps its window as full as noq 1.3.0 lets it, at the
    // base delay: a datagram starts only while in_flight + MSS < window, so
    // the flight is the largest multiple of the MSS below the window. The
    // connection driver handles every received datagram before it sends
    // again: the second ACK frame of a pass finds the flight the first one
    // drained. The path is not application limited, so neither holds the
    // window (with the bound on every ACK frame it stood at 100 MSS).
    let mut window = YieldWindow::new(100 * MSS, MTU, DEFAULT_TARGET);
    let delay = 10 * MS;
    let mut now = Instant::now() + Duration::from_secs(1);
    for pass in 0..200 {
        let before = window.window();
        let mut flight = (before - 1) / MSS * MSS;
        for _frame in 0..2 {
            for _ in 0..5 {
                now += MS;
                window.on_ack(now, MSS, delay, u64::MAX, true);
            }
            flight -= 5 * MSS;
            window.on_end_acks(flight, true, false);
            window.on_delay_sample(now, delay, delay);
        }
        window.on_sent();
        assert!(
            window.window() >= before,
            "pass {pass}: {} < {before}",
            window.window()
        );
        assert!(
            window.window() <= before + MSS,
            "pass {pass}: more than one MSS per pass"
        );
    }
    assert!(window.window() > 110 * MSS, "{}", window.window());
    // The application runs dry (noq reports the path application limited):
    // the window is held at one packet more than the flight at the last
    // send, whichever ACK frame of the pass reports it.
    for in_flight in [15 * MSS, 10 * MSS] {
        for _ in 0..5 {
            now += MS;
            window.on_ack(now, MSS, delay, u64::MAX, true);
        }
        window.on_end_acks(in_flight, true, true);
        assert_eq!(window.window(), 22 * MSS);
    }
    window.on_sent();
    window.on_end_acks(3 * MSS, true, true);
    assert_eq!(window.window(), 5 * MSS);
    // The controller passes noq's flag and its send report on.
    let presence = presence();
    let _guard = presence.enter(InteractiveKind::ScreenHost);
    let mut c = controller(400 * MSS, TransportPurpose::Files, presence, true);
    c.observe_ack(now, 5 * MSS, delay);
    c.on_end_acks(now, 15 * MSS, false, None);
    assert_eq!(c.window(), 400 * MSS, "window in use");
    c.on_end_acks(now, 15 * MSS, true, None);
    assert_eq!(c.window(), 22 * MSS, "application limited");
    c.on_packet_sent(now, MTU, 1);
    c.on_end_acks(now, 3 * MSS, true, None);
    assert_eq!(c.window(), 5 * MSS, "flight at the last send");
}

#[test]
fn verbindungen_task_yield_inner_window_is_held_while_the_yielding_one_is_in_force() {
    let presence = presence();
    let start = Instant::now() + Duration::from_secs(1);
    let mut c = YieldingController::new(
        Box::new(Growing(400 * MSS)),
        Engagement {
            slot: Arc::new(PurposeSlot::new(Some(TransportPurpose::Files))),
            presence,
            enabled: true,
        },
        DEFAULT_TARGET,
        MTU,
    );
    // No interactive session: the inner controller grows on every batch.
    let now = acks(&mut c, start, 100, 10 * MS);
    assert!(!c.holds_inner());
    assert_eq!(c.window(), 500 * MSS);
    // An interactive session and queueing above the target: the yielding
    // window is in force, and the inner one does not grow on acknowledgements
    // of a flight it did not allow.
    let guard = presence.enter(InteractiveKind::ScreenHost);
    let now = acks(&mut c, now, 2_000, 100 * MS);
    assert!(c.holds_inner());
    assert!(c.window() < 500 * MSS / 4, "{}", c.window());
    assert_eq!(c.inner.window(), 500 * MSS);
    // The session ends: the window in force is the inner one as it was, not
    // one grown by 2000 batches, and it grows again from there.
    drop(guard);
    assert_eq!(c.window(), 500 * MSS);
    acks(&mut c, now, 1, 100 * MS);
    assert_eq!(c.window(), 501 * MSS);
}

#[test]
fn verbindungen_task_yield_base_delay_from_minute_minima() {
    let mut delays = DelayTracker::default();
    let start = Instant::now();
    let round_trip = 50 * MS;
    assert_eq!(delays.queuing_delay(), None);
    delays.record(start, 50 * MS, round_trip);
    delays.record(start + Duration::from_secs(10), 20 * MS, round_trip);
    assert_eq!(delays.base_delay(), Some(20 * MS), "minimum of the minute");
    delays.record(start + Duration::from_secs(70), 40 * MS, round_trip);
    assert_eq!(delays.base_delay(), Some(20 * MS));
    // The current filter takes the minimum of recent samples only.
    assert_eq!(delays.queuing_delay(), Some(20 * MS));
    // Ten later minutes push the 20 ms minute out of the history.
    for minute in 2..=11u64 {
        delays.record(
            start + Duration::from_secs(60 * minute + 30),
            30 * MS,
            round_trip,
        );
    }
    assert_eq!(delays.base_delay(), Some(30 * MS));
    // Samples older than one round trip leave the current filter.
    let later = start + Duration::from_secs(60 * 12);
    delays.record(later, 90 * MS, round_trip);
    delays.record(later + 100 * MS, 35 * MS, round_trip);
    assert_eq!(delays.queuing_delay(), Some(5 * MS));
    // At most CURRENT_FILTER samples count.
    for step in 1..=CURRENT_FILTER as u32 {
        delays.record(later + 100 * MS + step * MS, 60 * MS, round_trip);
    }
    assert_eq!(delays.queuing_delay(), Some(30 * MS));
}

#[test]
fn verbindungen_task_yield_ack_delay_is_not_queueing() {
    // RFC 9002 §5.3: the reported ACK delay comes off unless the result would
    // fall below min_rtt.
    assert_eq!(network_delay(45 * MS, 25 * MS, 10 * MS), 20 * MS);
    assert_eq!(network_delay(30 * MS, 25 * MS, 10 * MS), 30 * MS);
    assert_eq!(network_delay(10 * MS, Duration::ZERO, 10 * MS), 10 * MS);

    let presence = presence();
    let _guard = presence.enter(InteractiveKind::ScreenHost);
    let inner = 400 * MSS;
    let start = Instant::now() + Duration::from_secs(1);
    // Path round trip 10 ms. The peer holds every ACK 25 ms back (its
    // max_ack_delay, as large as the target) and says so: 10 ms queueing at
    // most, below the target, so the window stays at the inner one.
    let mut held = controller(inner, TransportPurpose::Files, presence, true);
    let now = acks(&mut held, start, 200, 10 * MS);
    acks_with(&mut held, now, 2_000, 45 * MS, 25 * MS);
    assert_eq!(held.window(), inner, "a held ACK is not queueing");
    // The same 45 ms without a reported ACK delay are 35 ms of queueing,
    // above the target: the window yields.
    let mut queued = controller(inner, TransportPurpose::Files, presence, true);
    let now = acks(&mut queued, start, 200, 10 * MS);
    acks_with(&mut queued, now, 2_000, 45 * MS, Duration::ZERO);
    assert!(queued.window() < inner / 4, "{}", queued.window());
}

/// Decision of 2026-11-14 on E4: a person's metadata operation to the
/// connection's device engages the controller like a screen session, only
/// for that device, and for the hold after the last operation.
#[test]
fn verbindungen_task_yield_follows_the_metadata_presence_of_its_device() {
    use crate::share::keepalive::metadata_presence::{MetadataPresence, OperationOrigin};
    let register: &'static MetadataPresence = Box::leak(Box::new(MetadataPresence::new()));
    let device = iroh::SecretKey::from_bytes(&[11; 32]).public();
    let other = iroh::SecretKey::from_bytes(&[12; 32]).public();
    let inner = 400 * MSS;
    let to = |peer| {
        let slot = Arc::new(PurposeSlot::new(Some(TransportPurpose::Files)));
        slot.bind_activity(register.activity(peer));
        let engagement = Engagement {
            slot,
            presence: presence(),
            enabled: true,
        };
        YieldingController::new(Box::new(Fixed(inner)), engagement, DEFAULT_TARGET, MTU)
    };
    let (mut to_device, mut to_other) = (to(device), to(other));
    let start = Instant::now() + Duration::from_secs(1);
    // Base delay 10 ms, then 100 ms: 90 ms queueing, above the 25 ms target.
    let browsing = register.begin(device, OperationOrigin::User);
    let mut last = start;
    for c in [&mut to_device, &mut to_other] {
        let now = acks(c, start, 200, 10 * MS);
        last = acks(c, now, 2_000, 100 * MS);
    }
    assert!(to_device.window() < inner / 4, "{}", to_device.window());
    assert_eq!(to_other.window(), inner, "another device does not yield");
    // The operation ends; the hold (3 s here: 100 ms round trip, 10 ms base)
    // counts from the last time the controller saw it running.
    drop(browsing);
    let now = acks(&mut to_device, last, 2_000, 100 * MS);
    assert!(to_device.window() < inner / 4, "within the hold");
    acks(&mut to_device, now + Duration::from_secs(1), 10, 100 * MS);
    assert_eq!(to_device.window(), inner, "after the hold: inner window");
    // The job engine sets no presence.
    assert!(register.begin(device, OperationOrigin::Job).is_none());
    acks(
        &mut to_device,
        now + Duration::from_secs(2),
        2_000,
        100 * MS,
    );
    assert_eq!(to_device.window(), inner);
}
