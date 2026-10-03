//! The signal worker against a fake Share server over TCP lines and
//! WebSocket, with a manual clock whose monotonic part stands still while
//! the "device" sleeps.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{unbounded, Receiver, Sender};

use super::idle::KeepaliveTuning;
use super::worker_power::WorkerPower;
use super::{connected, WorkerRuntime};
use crate::share::direct_reciprocal_coordinator::DirectReciprocalCoordinator;
use crate::share::discovery_exchange_port_impl::DiscoveryExchangePortImpl;
use crate::share::discovery_relation_store::InMemoryRelationStore;
use crate::share::discovery_signal_commands::DiscoverySignalRuntime;
use crate::share::discovery_signal_offline::{wait_offline_backoff, OfflineWait};
use crate::share::discovery_signal_port::direct_peer_from_identity;
use crate::share::node::ShareIrohNode;
use crate::share::power::clock::{ManualClock, PowerClock, SystemClock};
use crate::share::power::test_support::{auth_state, identity, wait_until, NO_RELAY};
use crate::share::power::{PowerHub, ProbeOutcome};
use crate::share::profiles::ShareProfiles;
use crate::share::signal_connection::SignalConnection;
use crate::share::signal_connector::NegotiatedSignal;
use crate::share::signal_handshake::SignalCapabilities;
use crate::share::tracked_signal_sender::AttemptCounters;
use crate::share::types::{PendingShareCmd, ShareAuthState, ShareCmd, ShareCmdResult, ShareEvent};

#[path = "signal_worker_test_server.rs"]
mod fake_server;

use self::fake_server::{tcp_pair, websocket_pair, FakeServer};

const OK: ProbeOutcome = ProbeOutcome {
    ok: true,
    reconnected: false,
};

struct Harness {
    clock: Arc<ManualClock>,
    power: Arc<PowerHub>,
    node: Arc<ShareIrohNode>,
    auth: Arc<Mutex<ShareAuthState>>,
    commands: (Sender<PendingShareCmd>, Receiver<PendingShareCmd>),
    events: (Sender<ShareEvent>, Receiver<ShareEvent>),
    stopped: AtomicBool,
}

impl Harness {
    fn manual() -> Self {
        let clock = Arc::new(ManualClock::new(1_700_000_000_000));
        let dynamic: Arc<dyn PowerClock> = clock.clone();
        Self::with_clock(clock, dynamic)
    }

    /// Real clock: every wait lasts as long as in production.
    fn real() -> Self {
        Self::with_clock(
            Arc::new(ManualClock::new(0)),
            Arc::new(SystemClock) as Arc<dyn PowerClock>,
        )
    }

    fn with_clock(clock: Arc<ManualClock>, hub_clock: Arc<dyn PowerClock>) -> Self {
        let power = Arc::new(PowerHub::new(hub_clock));
        let identity = identity("background phone").expect("identity");
        let auth = Arc::new(Mutex::new(auth_state(&identity)));
        let events = unbounded();
        let node = ShareIrohNode::start_with_power_for_test(
            NO_RELAY,
            &identity,
            auth.clone(),
            events.0.clone(),
            power.clone(),
        )
        .expect("node");
        Self {
            clock,
            power,
            node,
            auth,
            commands: unbounded(),
            events,
            stopped: AtomicBool::new(false),
        }
    }

    fn with_runtime<T>(&self, body: impl FnOnce(&mut WorkerRuntime<'_>) -> T) -> T {
        let (coordinator, completions) =
            DirectReciprocalCoordinator::start(self.node.clone(), 0).expect("coordinator");
        let mut direct_requests_sent = HashSet::new();
        let mut tracked_attempts = AttemptCounters::new();
        let mut discovery = discovery_runtime(&self.auth);
        let mut power = WorkerPower::subscribe(&self.node);
        let mut runtime = WorkerRuntime {
            auth: &self.auth,
            iroh: &self.node,
            commands: &self.commands.1,
            events: &self.events.0,
            stopped_flag: &self.stopped,
            direct_requests_sent: &mut direct_requests_sent,
            tracked_attempts: &mut tracked_attempts,
            discovery: &mut discovery,
            repair_completions: &completions,
            power: &mut power,
        };
        let result = body(&mut runtime);
        drop(coordinator);
        result
    }

    /// Waits until the fresh endpoint stopped changing its routes, so no
    /// route republish interleaves with the expected messages.
    fn settle_routes(&self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut revision = self.node.route_revision();
        let mut stable_since = Instant::now();
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
            let current = self.node.route_revision();
            if current != revision {
                revision = current;
                stable_since = Instant::now();
            } else if stable_since.elapsed() >= Duration::from_millis(500) {
                return;
            }
        }
    }

    /// Runs one connected session; true when it ended because of a stop.
    fn run_session(&self, connection: SignalConnection, idle_keepalive: bool) -> bool {
        self.with_runtime(|runtime| {
            let negotiated = NegotiatedSignal {
                connection,
                capabilities: SignalCapabilities {
                    idle_keepalive,
                    ..SignalCapabilities::default()
                },
                transport: "test".into(),
            };
            connected::run(negotiated, runtime, &mut KeepaliveTuning::default())
        })
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        self.node.wake_signal_worker();
    }

    fn command(&self, command: ShareCmd) -> Receiver<Result<ShareCmdResult, String>> {
        let (acknowledgement, acknowledged) = crossbeam_channel::bounded(1);
        self.commands
            .0
            .send(PendingShareCmd {
                command,
                acknowledgement,
                expires_at: Instant::now() + Duration::from_secs(10),
            })
            .expect("worker commands");
        acknowledged
    }

    /// The runtime state as the background reload would send it again.
    fn unchanged_profiles(&self) -> ShareCmd {
        let state = self.auth.lock().expect("auth").clone();
        let profiles = ShareProfiles {
            direct_contacts: state.direct_contacts.clone(),
            direct_grants: state.direct_grants.clone(),
            rooms: state.rooms.clone(),
            default_direct_exports: state.default_direct_exports.clone(),
            direct_requests: state.direct_requests.clone(),
            direct_request_tombstones: state.direct_request_tombstones.clone(),
            ..ShareProfiles::default()
        };
        ShareCmd::ConfigureProfiles {
            profiles: Box::new(profiles),
        }
    }
}

fn discovery_runtime(auth: &Arc<Mutex<ShareAuthState>>) -> DiscoverySignalRuntime {
    let direct_peer_auth = auth.clone();
    let port = DiscoveryExchangePortImpl::new(
        Box::new(move || {
            let state = direct_peer_auth
                .lock()
                .map_err(|_| "Share-State gesperrt".to_string())?;
            direct_peer_from_identity(&state.identity)
        }),
        Box::new(InMemoryRelationStore::default()),
    );
    DiscoverySignalRuntime::with_port(Box::new(port))
}

/// Idle mode end to end: no heartbeats, keepalives answered, presence
/// renewed by wall-clock age, probes, reload without traffic, wake-up.
fn idle_session(pair: fn() -> (SignalConnection, FakeServer)) {
    let harness = Harness::manual();
    harness.settle_routes();
    harness.power.set_low_power(true);
    let (connection, mut server) = pair();
    std::thread::scope(|scope| {
        let session = scope.spawn(|| harness.run_session(connection, true));
        server.expect("publish_direct");
        assert_eq!(server.expect("set_idle")["idle"], true);
        server.send(r#"{"t":"idle_ack","idle":true,"keepalive_secs":180}"#);
        wait_until("idle mode", || harness.power.signal_status().idle_active);
        assert_eq!(harness.power.signal_status().keepalive_secs, Some(180));

        // 25 s awake: a normal client would have sent a heartbeat.
        harness.clock.advance(Duration::from_secs(25));
        server.expect_silence(Duration::from_millis(300));

        // The background reload repeats the profiles: nothing is sent.
        let acknowledged = harness.command(harness.unchanged_profiles());
        assert!(matches!(
            acknowledged.recv_timeout(Duration::from_secs(5)),
            Ok(Ok(ShareCmdResult::Applied))
        ));
        server.expect_silence(Duration::from_millis(200));

        // A keepalive is answered; the young presence stays.
        server.send(r#"{"t":"keepalive"}"#);
        server.expect("keepalive_ack");
        server.expect_silence(Duration::from_millis(200));

        // Asleep for 150 s: the next keepalive renews the presence.
        harness.clock.suspend(Duration::from_secs(150));
        server.send(r#"{"t":"keepalive"}"#);
        server.expect("keepalive_ack");
        server.expect("publish_direct");

        // An alarm probe on a recently heard server sends nothing.
        assert_eq!(harness.power.request_probe(false).wait(), OK);
        server.expect_silence(Duration::from_millis(200));

        // The app becomes visible: normal mode with a heartbeat at once.
        harness.power.set_low_power(false);
        assert_eq!(server.expect("set_idle")["idle"], false);
        server.expect("heartbeat");
        assert!(!harness.power.signal_status().idle_active);

        harness.stop();
        assert!(
            session.join().expect("session"),
            "the stop ends the session"
        );
    });
}

#[test]
fn android_background_task_idle_session_over_tcp() {
    idle_session(tcp_pair);
}

#[test]
fn android_background_task_idle_session_over_websocket() {
    idle_session(websocket_pair);
}

#[test]
fn android_background_task_network_change_probe_needs_a_pong_within_ten_seconds() {
    let harness = Harness::manual();
    harness.settle_routes();
    harness.power.set_low_power(true);
    let (connection, mut server) = tcp_pair();
    std::thread::scope(|scope| {
        let session = scope.spawn(|| harness.run_session(connection, true));
        server.expect("publish_direct");
        server.expect("set_idle");
        server.send(r#"{"t":"idle_ack","idle":true,"keepalive_secs":180}"#);
        wait_until("idle mode", || harness.power.signal_status().idle_active);

        let ticket = harness.power.request_probe(true);
        server.expect_after_presence("heartbeat");
        server.send(r#"{"t":"pong"}"#);
        assert_eq!(ticket.wait(), OK);

        // Unanswered: after 10 s the connection is rebuilt.
        let ticket = harness.power.request_probe(true);
        server.expect_after_presence("heartbeat");
        harness.clock.advance(Duration::from_secs(10));
        assert!(!session.join().expect("session"), "a dead link reconnects");
        assert_eq!(
            ticket.wait_timeout(Duration::from_secs(5)),
            ProbeOutcome::default()
        );
    });
}

#[test]
fn android_background_task_probe_after_long_sleep_reconnects() {
    let harness = Harness::manual();
    harness.settle_routes();
    harness.power.set_low_power(true);
    let (connection, mut server) = tcp_pair();
    std::thread::scope(|scope| {
        let session = scope.spawn(|| harness.run_session(connection, true));
        server.expect("publish_direct");
        server.expect("set_idle");
        server.send(r#"{"t":"idle_ack","idle":true,"keepalive_secs":180}"#);
        wait_until("idle mode", || harness.power.signal_status().idle_active);
        // The device slept past the server's keepalive and reply window;
        // monotonic timers never fired.
        harness.clock.suspend(Duration::from_secs(180 + 90 + 1));
        let ticket = harness.power.request_probe(false);
        assert!(
            !session.join().expect("session"),
            "a stale connection ends for a reconnect"
        );
        // Without a worker the pending probe fails instead of hanging.
        assert_eq!(
            ticket.wait_timeout(Duration::from_secs(5)),
            ProbeOutcome::default()
        );
    });
}

#[test]
fn android_background_task_old_server_keeps_heartbeats_in_low_power() {
    let harness = Harness::manual();
    harness.settle_routes();
    harness.power.set_low_power(true);
    let (connection, mut server) = websocket_pair();
    std::thread::scope(|scope| {
        let session = scope.spawn(|| harness.run_session(connection, false));
        server.expect("publish_direct");
        wait_until("server status", || {
            harness.power.signal_status().idle_supported == Some(false)
        });
        // No set_idle for a server without the capability.
        server.expect_silence(Duration::from_millis(200));
        harness.clock.advance(Duration::from_secs(20));
        server.expect("heartbeat");
        server.send(r#"{"t":"pong"}"#);
        // Asleep beyond the old server's read deadline: the next wake-up
        // (here the alarm probe; in the field also the server's close)
        // reconnects. A pong the worker reads only after the jump counts as
        // fresh contact, so the test sleeps again until the probe sees the
        // silence; the ended session answers the probe with the default.
        let mut ended = false;
        for _ in 0..3 {
            harness.clock.suspend(Duration::from_secs(91));
            let outcome = harness
                .power
                .request_probe(false)
                .wait_timeout(Duration::from_secs(5));
            if outcome == ProbeOutcome::default() {
                ended = true;
                break;
            }
        }
        if !ended {
            harness.stop();
        }
        assert!(ended, "the probe after a long sleep did not reconnect");
        assert!(!session.join().expect("session"));
    });
}

#[test]
fn android_background_task_offline_wait_reacts_to_events_at_once() {
    // Real clock: the wait itself would last up to 30 s per round.
    let harness = Harness::real();
    std::thread::scope(|scope| {
        let waiter = scope.spawn(|| {
            harness.with_runtime(|runtime| {
                let probe = wait_offline_backoff(Duration::from_secs(3_600), runtime);
                runtime.power.answer(OK);
                let stopped = wait_offline_backoff(Duration::from_secs(3_600), runtime);
                (
                    matches!(probe, OfflineWait::Probe),
                    matches!(stopped, OfflineWait::Stopped),
                )
            })
        });
        let started = Instant::now();
        let acknowledged = harness.command(ShareCmd::Refresh);
        let acknowledgement = acknowledged.recv_timeout(Duration::from_secs(5));
        let applied = matches!(acknowledgement, Ok(Ok(ShareCmdResult::Applied)));
        if !applied {
            harness.stop();
        }
        assert!(applied, "offline command: {acknowledgement:?}");
        assert!(started.elapsed() < Duration::from_secs(5));
        // The acknowledged command proves the waiter is subscribed and
        // waiting; exactly one probe ends its first wait.
        let ticket = harness.power.request_probe(false);
        let answered = ticket.wait_timeout(Duration::from_secs(5));
        if answered != OK {
            harness.stop();
        }
        assert_eq!(answered, OK, "the probe woke the wait");
        let started = Instant::now();
        harness.stop();
        assert_eq!(waiter.join().expect("waiter"), (true, true));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the stop woke the wait"
        );
    });
}
