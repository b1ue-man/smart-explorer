use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::Connection;

use super::{closed_idle, sweep_step, IncomingActivity, QuietSnapshot, SweepStep, IDLE_QUIET_MS};
use crate::share::power::test_support::{manual_hub, wait_until, LoopbackPeers};
use crate::share::session::session_key;

#[test]
fn android_background_task_quiet_period_needs_two_minutes_of_unchanged_activity() {
    let first = sweep_step(None, 7, false, 1_000);
    let SweepStep::Keep(snapshot) = first else {
        panic!("a new connection is never closed");
    };
    assert_eq!(
        snapshot,
        QuietSnapshot {
            activity: 7,
            since_ms: 1_000
        }
    );
    assert_eq!(
        sweep_step(Some(snapshot), 7, false, 1_000 + IDLE_QUIET_MS - 1),
        SweepStep::Keep(snapshot)
    );
    assert_eq!(
        sweep_step(Some(snapshot), 7, false, 1_000 + IDLE_QUIET_MS),
        SweepStep::Close
    );
    // New stream frames, open streams or a lease restart the period.
    for (activity, busy) in [(8, false), (7, true)] {
        assert_eq!(
            sweep_step(Some(snapshot), activity, busy, 1_000 + IDLE_QUIET_MS),
            SweepStep::Keep(QuietSnapshot {
                activity,
                since_ms: 1_000 + IDLE_QUIET_MS
            })
        );
    }
    // A wall clock that went backwards starts over instead of closing.
    assert_eq!(
        sweep_step(Some(snapshot), 7, false, 500),
        SweepStep::Keep(QuietSnapshot {
            activity: 7,
            since_ms: 500
        })
    );
}

#[test]
fn android_background_task_mount_lease_keeps_incoming_connection() {
    let activity = IncomingActivity::default();
    assert!(activity.idle_close_allowed());
    activity.lease_used("se-mount-v2.token");
    activity.lease_used("se-mount-v2.token");
    assert!(!activity.idle_close_allowed());
    activity.lease_released("se-mount-v2.token");
    assert!(activity.idle_close_allowed());
}

fn client_connection(peers: &LoopbackPeers) -> Option<Connection> {
    let key = session_key(&peers.endpoint);
    peers.client.sessions.lock().ok()?.get(&key).cloned()
}

/// Sweeps the node until it closes (or asks to close) one connection,
/// advancing the fake wall time by the quiet period each round.
fn sweep_until_closed(sweep: impl Fn(i64) -> usize) {
    let mut now = 10_000_000;
    for _ in 0..20 {
        if sweep(now) == 1 {
            return;
        }
        now += IDLE_QUIET_MS;
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("the quiet connection was never closed");
}

#[test]
fn android_background_task_quiet_incoming_connection_closes_and_client_reconnects() {
    let (_clock, power) = manual_hub(1_000_000);
    let peers = LoopbackPeers::new(power).expect("loopback peers");
    peers.peer.probe_root().expect("first listing");
    let first = client_connection(&peers).expect("cached client connection");

    sweep_until_closed(|now| {
        let report = peers.host.sweep_idle_connections_at(now);
        assert_eq!(report.closed_outgoing, 0, "the host dialed nothing");
        report.closing_incoming
    });
    wait_until("the idle close at the client", || {
        first.close_reason().is_some()
    });
    assert!(closed_idle(&first), "{:?}", first.close_reason());

    // The next operation dials a new connection transparently.
    peers
        .peer
        .probe_root()
        .expect("listing after the idle close");
    let second = client_connection(&peers).expect("new client connection");
    assert_ne!(first.stable_id(), second.stable_id());
    assert!(second.close_reason().is_none());
}

#[test]
fn android_background_task_quiet_outgoing_connection_closes_and_redials() {
    let (_clock, power) = manual_hub(1_000_000);
    let peers = LoopbackPeers::new(power).expect("loopback peers");
    peers.peer.probe_root().expect("first listing");
    let first = client_connection(&peers).expect("cached client connection");

    sweep_until_closed(|now| {
        let report = peers.client.sweep_idle_connections_at(now);
        assert_eq!(report.closing_incoming, 0, "the client accepted nothing");
        report.closed_outgoing
    });
    assert!(first.close_reason().is_some());
    assert!(
        client_connection(&peers).is_none(),
        "the closed connection left the cache"
    );
    peers
        .peer
        .probe_root()
        .expect("listing after the own idle close");
    assert!(client_connection(&peers).is_some());
}

#[test]
fn android_background_task_active_connection_is_not_closed() {
    let (_clock, power) = manual_hub(1_000_000);
    let peers = LoopbackPeers::new(Arc::clone(&power)).expect("loopback peers");
    peers.peer.probe_root().expect("first listing");
    let mut now = 10_000_000;
    assert_eq!(
        peers.host.sweep_idle_connections_at(now).closing_incoming,
        0
    );
    for _ in 0..3 {
        // Traffic within every quiet period keeps the connection.
        peers.peer.probe_root().expect("listing");
        now += IDLE_QUIET_MS;
        assert_eq!(
            peers.host.sweep_idle_connections_at(now).closing_incoming,
            0
        );
    }
    let connection = client_connection(&peers).expect("cached client connection");
    assert!(connection.close_reason().is_none());
}
