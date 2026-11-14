//! P2 of plan `docs/plaene/2026-11-14-verbindungen-je-zweck` (E2): the
//! yielding controller. Remote suite only (AGENTS.md).
use super::*;
use crate::share::keepalive::interactive_presence::InteractiveKind;
use crate::share::keepalive::transport_purpose::TransportPurpose;

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

/// Acknowledges `count` full packets, each `delay` after it was sent, one
/// every millisecond from `start`; returns the time after the last one.
fn acks(c: &mut YieldingController, start: Instant, count: u32, delay: Duration) -> Instant {
    let mut now = start;
    for _ in 0..count {
        now += MS;
        c.observe_ack(now, now - delay, MSS, delay);
        c.on_end_acks(now, u64::MAX / 2, false, None);
    }
    now
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
    assert_eq!(window.window(), 2 * MSS, "never below MIN_CWND");
    // Flight size bounds growth (RFC 6817 max_allowed_cwnd).
    let mut capped = YieldWindow::new(100 * MSS, MTU, DEFAULT_TARGET);
    capped.on_end_acks(10 * MSS, true);
    assert_eq!(capped.window(), 11 * MSS);
    capped.on_end_acks(0, false);
    assert_eq!(capped.window(), 11 * MSS, "not engaged: unchanged");
}

#[test]
fn verbindungen_task_yield_full_window_survives_acknowledgements_in_batches() {
    // A sender that keeps its window full, acknowledged ten packets at a
    // time at the base delay. noq reports the bytes in flight after each
    // batch; the cap of RFC 6817 is the flight size before it plus one MSS.
    let mut window = YieldWindow::new(100 * MSS, MTU, DEFAULT_TARGET);
    let delay = 10 * MS;
    let mut now = Instant::now() + Duration::from_secs(1);
    for batch in 0..200 {
        let before = window.window();
        for _ in 0..10 {
            now += MS;
            window.on_ack(now, now - delay, MSS, delay, u64::MAX, true);
        }
        window.on_end_acks(before - 10 * MSS, true);
        assert!(
            window.window() >= before,
            "batch {batch}: {} < {before}",
            window.window()
        );
        assert!(
            window.window() <= before + MSS,
            "batch {batch}: more than one MSS per batch"
        );
    }
    assert!(window.window() > 100 * MSS, "{}", window.window());
    // A flight far below the window still caps it (application limited).
    window.on_end_acks(3 * MSS, true);
    assert_eq!(window.window(), 4 * MSS);
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
