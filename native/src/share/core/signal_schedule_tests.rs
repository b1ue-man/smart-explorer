use std::time::Duration;

use super::{SignalMode, SignalSchedule, PROBE_REPLY_DEADLINE};
use crate::share::power::clock::{ManualClock, PowerClock};

const DESKTOP: SignalMode = SignalMode {
    low_power: false,
    idle_secs: None,
    tracked: true,
};
const OLD_SERVER_IDLE: SignalMode = SignalMode {
    low_power: true,
    idle_secs: None,
    tracked: false,
};
const IDLE: SignalMode = SignalMode {
    low_power: true,
    idle_secs: Some(180),
    tracked: false,
};

#[test]
fn android_background_task_desktop_cadence_is_unchanged() {
    let clock = ManualClock::new(1_000_000);
    let mut schedule = SignalSchedule::new(clock.now());
    assert_eq!(
        schedule.next_wake(clock.now(), DESKTOP),
        Some(Duration::from_secs(2)),
        "tracked outbox every 2 s"
    );
    clock.advance(Duration::from_secs(20));
    assert!(schedule.heartbeat_due(clock.now(), DESKTOP));
    schedule.heartbeat_sent(clock.now());
    assert!(!schedule.heartbeat_due(clock.now(), DESKTOP));
    clock.advance(Duration::from_secs(40));
    assert!(schedule.pong_expired(clock.now()), "pong within 40 s");
    assert!(
        schedule.presence_due(clock.now(), DESKTOP),
        "presence every 60 s"
    );
    // A desktop never judges the server by the wall clock.
    clock.suspend(Duration::from_secs(3_600));
    assert!(!schedule.stale(clock.now(), DESKTOP));
}

#[test]
fn android_background_task_idle_mode_has_no_heartbeat_and_no_presence_timer() {
    let clock = ManualClock::new(1_000_000);
    let schedule = SignalSchedule::new(clock.now());
    clock.advance(Duration::from_secs(600));
    let now = clock.now();
    assert!(!schedule.heartbeat_due(now, IDLE));
    assert!(!schedule.presence_due(now, IDLE));
    assert!(!schedule.tracked_due(now, IDLE));
    // The only timer while awake is the silent-server check: K + 60 + 30 s.
    let fresh = SignalSchedule::new(clock.now());
    assert_eq!(
        fresh.next_wake(clock.now(), IDLE),
        Some(Duration::from_secs(180 + 60 + 30 + 1))
    );
}

#[test]
fn android_background_task_suspend_makes_a_silent_server_stale() {
    let clock = ManualClock::new(1_000_000);
    let schedule = SignalSchedule::new(clock.now());
    // The device sleeps: the monotonic clock stands, the wall clock runs.
    clock.suspend(Duration::from_secs(180 + 90));
    assert!(!schedule.stale(clock.now(), IDLE), "exactly at the limit");
    clock.suspend(Duration::from_secs(1));
    assert!(schedule.stale(clock.now(), IDLE));
    assert!(
        !schedule.heartbeat_due(clock.now(), OLD_SERVER_IDLE),
        "monotonic timers did not run"
    );
    // Without idle mode the server drops a silent client after 60 s.
    let legacy = SignalSchedule::new(clock.now());
    clock.suspend(Duration::from_secs(91));
    assert!(legacy.stale(clock.now(), OLD_SERVER_IDLE));
    assert!(
        legacy.presence_due(clock.now(), OLD_SERVER_IDLE),
        "presence age counts real time in low power"
    );
}

#[test]
fn android_background_task_idle_presence_survives_every_keepalive_round() {
    // K = 180: a presence lives 300 s; renewed at every keepalive whenever
    // it would not reach the next keepalive plus the reply window.
    let clock = ManualClock::new(1_000_000);
    let mut schedule = SignalSchedule::new(clock.now());
    let mut published_at = clock.now().wall_ms;
    for round in 0..20 {
        clock.suspend(Duration::from_secs(180));
        schedule.inbound(clock.now());
        let now = clock.now();
        assert!(
            now.wall_ms - published_at < 300_000,
            "presence expired before keepalive {round}"
        );
        if schedule.idle_presence_due(now, 180) {
            schedule.published(now);
            published_at = now.wall_ms;
        }
        // The copy at observers must outlive the next round and its reply.
        assert!(published_at + 300_000 >= now.wall_ms + 180_000 + 60_000);
    }
}

#[test]
fn android_background_task_idle_presence_renews_after_two_minutes() {
    let clock = ManualClock::new(1_000_000);
    let mut schedule = SignalSchedule::new(clock.now());
    clock.suspend(Duration::from_secs(10));
    schedule.inbound(clock.now());
    // Short keepalive: young presence stays.
    assert!(!schedule.idle_presence_due(clock.now(), 30));
    clock.suspend(Duration::from_secs(110));
    schedule.inbound(clock.now());
    assert!(
        schedule.idle_presence_due(clock.now(), 30),
        "older than 120 s"
    );
    schedule.published(clock.now());
    assert!(!schedule.idle_presence_due(clock.now(), 30));
}

#[test]
fn android_background_task_probe_heartbeat_has_ten_seconds() {
    let clock = ManualClock::new(1_000_000);
    let mut schedule = SignalSchedule::new(clock.now());
    schedule.heartbeat_sent(clock.now());
    schedule.probe_started(clock.now());
    assert!(schedule.probing());
    assert_eq!(
        schedule.next_wake(clock.now(), IDLE),
        Some(PROBE_REPLY_DEADLINE)
    );
    clock.advance(PROBE_REPLY_DEADLINE - Duration::from_millis(1));
    assert!(!schedule.probe_expired(clock.now()));
    assert!(schedule.pong_received(), "the pong answers the probe");
    assert!(!schedule.probing());
    assert!(!schedule.pong_expired(clock.now()));
    schedule.probe_started(clock.now());
    clock.advance(PROBE_REPLY_DEADLINE);
    assert!(schedule.probe_expired(clock.now()));
}

#[test]
fn android_background_task_leaving_idle_sends_a_heartbeat_at_once() {
    let clock = ManualClock::new(1_000_000);
    let mut schedule = SignalSchedule::new(clock.now());
    assert!(!schedule.heartbeat_due(clock.now(), OLD_SERVER_IDLE));
    schedule.heartbeat_soon();
    assert!(!schedule.heartbeat_due(clock.now(), IDLE), "still idle");
    assert!(schedule.heartbeat_due(clock.now(), DESKTOP));
    assert_eq!(
        schedule.next_wake(clock.now(), DESKTOP),
        Some(Duration::ZERO)
    );
    schedule.heartbeat_sent(clock.now());
    assert!(!schedule.heartbeat_due(clock.now(), DESKTOP));
}
