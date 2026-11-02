use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::clock::{ManualClock, PowerClock};
use super::{hold_needed, PowerHub, ProbeOutcome, SignalPowerStatus, HOLD_THROTTLE_MS};

fn hub(clock: &Arc<ManualClock>) -> Arc<PowerHub> {
    let clock: Arc<dyn PowerClock> = clock.clone();
    Arc::new(PowerHub::new(clock))
}

#[test]
fn android_background_task_probe_without_worker_fails_at_once() {
    let clock = Arc::new(ManualClock::new(1_000_000));
    let hub = hub(&clock);
    let started = Instant::now();
    let ticket = hub.request_probe(true);
    assert!(ticket.is_answered());
    assert_eq!(ticket.wait(), ProbeOutcome::default());
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn android_background_task_probe_waits_for_every_worker_and_wakes_them() {
    let clock = Arc::new(ManualClock::new(1_000_000));
    let hub = hub(&clock);
    let (first_wake, first_woken) = crossbeam_channel::bounded(1);
    let (second_wake, second_woken) = crossbeam_channel::bounded(1);
    let first = hub.subscribe(first_wake);
    let second = hub.subscribe(second_wake);

    let ticket = hub.request_probe(false);
    assert!(first_woken.try_recv().is_ok());
    assert!(second_woken.try_recv().is_ok());
    assert!(!ticket.is_answered());

    let batch = first.take_probe().expect("first worker has a probe");
    assert!(!batch.network_changed);
    assert!(first.take_probe().is_none(), "a probe is taken once");
    batch.answer(ProbeOutcome {
        ok: true,
        reconnected: true,
    });
    assert!(!ticket.is_answered(), "the second worker has not answered");
    // A worker that stops answers its share as failed.
    drop(second);
    assert_eq!(
        ticket.wait_timeout(Duration::from_secs(5)),
        ProbeOutcome {
            ok: true,
            reconnected: true,
        }
    );
}

#[test]
fn android_background_task_probe_ticket_is_bounded_and_merges_network_change() {
    let clock = Arc::new(ManualClock::new(1_000_000));
    let hub = hub(&clock);
    let (wake, _woken) = crossbeam_channel::bounded(1);
    let subscription = hub.subscribe(wake);
    let first = hub.request_probe(false);
    let second = hub.request_probe(true);
    let started = Instant::now();
    assert_eq!(
        first.wait_timeout(Duration::from_millis(50)),
        ProbeOutcome::default()
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    let batch = subscription.take_probe().expect("merged probes");
    assert!(
        batch.network_changed,
        "a network change in any probe counts"
    );
    batch.answer(ProbeOutcome {
        ok: true,
        reconnected: false,
    });
    assert_eq!(
        second.wait_timeout(Duration::from_secs(5)),
        ProbeOutcome {
            ok: true,
            reconnected: false,
        }
    );
}

#[test]
fn android_background_task_low_power_change_wakes_workers_once() {
    let clock = Arc::new(ManualClock::new(1_000_000));
    let hub = hub(&clock);
    let (wake, woken) = crossbeam_channel::bounded(1);
    let _subscription = hub.subscribe(wake);
    assert!(!hub.low_power());
    hub.set_low_power(true);
    assert!(hub.low_power());
    assert!(woken.try_recv().is_ok());
    hub.set_low_power(true);
    assert!(woken.try_recv().is_err(), "an unchanged state wakes nobody");
    hub.set_low_power(false);
    assert!(woken.try_recv().is_ok());
}

static THROTTLED_HOLDS: AtomicUsize = AtomicUsize::new(0);
static THROTTLED_LAST_MS: AtomicU32 = AtomicU32::new(0);

fn throttled_hook(hold_ms: u32) {
    THROTTLED_HOLDS.fetch_add(1, Ordering::SeqCst);
    THROTTLED_LAST_MS.store(hold_ms, Ordering::SeqCst);
}

#[test]
fn android_background_task_holds_are_throttled_and_only_in_low_power() {
    let clock = Arc::new(ManualClock::new(1_000_000));
    let hub = hub(&clock);
    hub.set_activity_hook(throttled_hook);
    hub.request_hold(5_000);
    assert_eq!(
        THROTTLED_HOLDS.load(Ordering::SeqCst),
        0,
        "normal operation"
    );

    hub.set_low_power(true);
    hub.request_hold(5_000);
    assert_eq!(THROTTLED_HOLDS.load(Ordering::SeqCst), 1);
    clock.advance(Duration::from_secs(1));
    hub.request_hold(5_000);
    assert_eq!(THROTTLED_HOLDS.load(Ordering::SeqCst), 1, "throttled");
    hub.request_hold(15_000);
    assert_eq!(
        THROTTLED_HOLDS.load(Ordering::SeqCst),
        2,
        "a longer hold wins"
    );
    assert_eq!(THROTTLED_LAST_MS.load(Ordering::SeqCst), 15_000);
    // After a suspend (wall clock only) the earlier hold has expired.
    clock.suspend(Duration::from_secs(60));
    hub.request_hold(5_000);
    assert_eq!(THROTTLED_HOLDS.load(Ordering::SeqCst), 3);
}

#[test]
fn android_background_task_hold_rule_is_bounded() {
    // Not covered, first hold.
    assert!(hold_needed(0, None, 1_000, 6_000));
    // Covered.
    assert!(!hold_needed(10_000, Some(5_000), 6_000, 9_000));
    // Within the interval and extending by at most the interval.
    assert!(!hold_needed(
        10_000,
        Some(5_000),
        8_000,
        10_000 + HOLD_THROTTLE_MS
    ));
    // Within the interval but extending further.
    assert!(hold_needed(
        10_000,
        Some(5_000),
        8_000,
        10_001 + HOLD_THROTTLE_MS
    ));
    // After the interval any extension counts.
    assert!(hold_needed(
        10_000,
        Some(5_000),
        5_000 + HOLD_THROTTLE_MS,
        10_001
    ));
}

#[test]
fn android_background_task_status_follows_the_latest_connection() {
    let clock = Arc::new(ManualClock::new(1_000_000));
    let hub = hub(&clock);
    let (old_wake, _old) = crossbeam_channel::bounded(1);
    let (new_wake, _new) = crossbeam_channel::bounded(1);
    let old = hub.subscribe(old_wake);
    let new = hub.subscribe(new_wake);
    old.claim_status(SignalPowerStatus {
        idle_supported: Some(false),
        ..SignalPowerStatus::default()
    });
    new.claim_status(SignalPowerStatus {
        idle_supported: Some(true),
        ..SignalPowerStatus::default()
    });
    old.update_status(|status| status.idle_active = true);
    new.update_status(|status| {
        status.keepalive_secs = Some(180);
        status.last_server_contact_unix = Some(1_000);
    });
    assert_eq!(
        hub.signal_status(),
        SignalPowerStatus {
            idle_supported: Some(true),
            idle_active: false,
            keepalive_secs: Some(180),
            last_server_contact_unix: Some(1_000),
        }
    );
    drop(old);
    assert_eq!(hub.signal_status().idle_supported, Some(true));
    drop(new);
    assert_eq!(hub.signal_status(), SignalPowerStatus::default());
}
