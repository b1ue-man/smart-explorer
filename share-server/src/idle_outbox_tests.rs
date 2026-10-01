//! Presence bundling for idle observers (contract V1, critic findings K2/K15/K16).

use std::collections::HashSet;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::idle::{Keepalive, SignalClock, TestClock, CAPABILITY};
use super::idle_outbox::IdleOutbox;
use super::limits::{SourceKey, MAX_WRITER_QUEUED_BYTES};
use super::state::{join_room, leave_room, register_client, State};
use super::writer::QueuedMessage;
use super::{dispatch, tracked_direct, In, Out, PeerPresence, Writer};

const START_UNIX: i64 = 1_800_000_000;
const LOOKUP: &str = "lookup-desktop";
const SECOND: Duration = Duration::from_secs(1);

struct Peer {
    id: u64,
    writer: Writer,
    receiver: Receiver<QueuedMessage>,
}

fn peer(state: &Arc<Mutex<State>>, clock: &Arc<TestClock>, device: &str, idle: bool) -> Peer {
    let (writer, receiver) = Writer::test_raw_channel(64);
    let mut capabilities = HashSet::new();
    if idle {
        writer.enable_idle(&clock.timing(Keepalive::default()));
        capabilities.insert(CAPABILITY.to_string());
    }
    let source = SourceKey::Ipv4([192, 0, 2, device.len() as u8]);
    let id = register_client(state, writer.clone(), source, device.into(), capabilities).unwrap();
    Peer {
        id,
        writer,
        receiver,
    }
}

fn presence(device: &str, relation: &str, clock: &TestClock, route: &str) -> PeerPresence {
    PeerPresence {
        kind: "direct".into(),
        relation_id: relation.into(),
        device_id: device.into(),
        device_name: device.into(),
        public_key: "pk".into(),
        fingerprint: "fp".into(),
        node_id: format!("node-{device}"),
        relay_url: "http://127.0.0.1:51821".into(),
        candidates: vec![route.into()],
        expires_at: clock.unix_secs() + 300,
        nonce: format!("nonce-{}", clock.unix_secs()),
        proof: "proof".into(),
    }
}

fn drain(receiver: &Receiver<QueuedMessage>) -> Vec<Value> {
    receiver
        .try_iter()
        .map(|message| serde_json::from_slice(message.json()).unwrap())
        .collect()
}

fn tags(receiver: &Receiver<QueuedMessage>) -> Vec<String> {
    drain(receiver)
        .into_iter()
        .map(|message| message["t"].as_str().unwrap().to_string())
        .collect()
}

fn set_idle(peer: &Peer, state: &Arc<Mutex<State>>, idle: bool, keepalive_secs: Option<u32>) {
    let message = In::SetIdle {
        idle,
        keepalive_secs,
    };
    dispatch(peer.id, &peer.writer, message, state);
}

fn watch(peer: &Peer, state: &Arc<Mutex<State>>) {
    let lookup_id = LOOKUP.to_string();
    dispatch(peer.id, &peer.writer, In::WatchDirect { lookup_id }, state);
}

fn publish(peer: &Peer, state: &Arc<Mutex<State>>, clock: &TestClock, route: &str) {
    let presence = presence("desktop", LOOKUP, clock, route);
    tracked_direct::publish(peer.id, &peer.writer, presence, state);
}

/// Desktop publisher plus an idle phone that already holds its presence.
fn idle_watcher() -> (Arc<TestClock>, Arc<Mutex<State>>, Peer, Peer) {
    let clock = TestClock::new(START_UNIX);
    let state = Arc::new(Mutex::new(State::default()));
    let desktop = peer(&state, &clock, "desktop", false);
    let phone = peer(&state, &clock, "phone", true);
    publish(&desktop, &state, &clock, "10.0.0.1:1");
    watch(&phone, &state);
    set_idle(&phone, &state, true, None);
    assert_eq!(tags(&phone.receiver), ["direct_available", "idle_ack"]);
    (clock, state, desktop, phone)
}

#[test]
fn android_background_task_idle_refresh_waits_for_keepalive_tick() {
    let (clock, state, desktop, phone) = idle_watcher();
    clock.advance(60 * SECOND);
    publish(&desktop, &state, &clock, "10.0.0.1:1");
    clock.advance(60 * SECOND);
    publish(&desktop, &state, &clock, "10.0.0.1:1");
    phone.writer.idle_tick(clock.now());
    assert!(
        tags(&phone.receiver).is_empty(),
        "pure refresh was not held"
    );
    assert_eq!(phone.writer.idle_deferred_len(), 1, "one refresh per key");

    clock.advance(60 * SECOND);
    phone.writer.idle_tick(clock.now());
    let delivered = drain(&phone.receiver);
    assert_eq!(delivered.len(), 2);
    assert_eq!(delivered[0]["t"], "direct_available");
    assert_eq!(
        delivered[0]["presence"]["expires_at"],
        START_UNIX + 120 + 300
    );
    assert_eq!(delivered[1]["t"], "keepalive");
}

#[test]
fn android_background_task_route_change_first_presence_and_expiry_are_immediate() {
    let (clock, state, desktop, phone) = idle_watcher();
    publish(&desktop, &state, &clock, "10.0.0.1:1");
    assert_eq!(phone.writer.idle_deferred_len(), 1);

    // A new route goes out at once and supersedes the held refresh.
    clock.advance(SECOND);
    publish(&desktop, &state, &clock, "10.0.0.2:1");
    assert_eq!(tags(&phone.receiver), ["direct_available"]);
    assert_eq!(phone.writer.idle_deferred_len(), 0);

    // First presence of another lookup on this connection: immediate, even
    // though it lives only 100 s.
    let laptop = peer(&state, &clock, "laptop", false);
    let lookup_id = "lookup-laptop".to_string();
    dispatch(
        phone.id,
        &phone.writer,
        In::WatchDirect { lookup_id },
        &state,
    );
    let mut short = presence("laptop", "lookup-laptop", &clock, "10.0.0.3:1");
    short.expires_at = clock.unix_secs() + 100;
    tracked_direct::publish(laptop.id, &laptop.writer, short, &state);
    assert_eq!(tags(&phone.receiver), ["direct_available"]);

    // The held copy would expire before the next tick plus margin: immediate.
    clock.advance(SECOND);
    let refresh = presence("laptop", "lookup-laptop", &clock, "10.0.0.3:1");
    tracked_direct::publish(laptop.id, &laptop.writer, refresh, &state);
    assert_eq!(tags(&phone.receiver), ["direct_available"]);
    assert_eq!(phone.writer.idle_deferred_len(), 0);
    // Now the copy lasts long enough, so the next pure refresh is held.
    let refresh = presence("laptop", "lookup-laptop", &clock, "10.0.0.3:1");
    tracked_direct::publish(laptop.id, &laptop.writer, refresh, &state);
    assert!(tags(&phone.receiver).is_empty());
    assert_eq!(phone.writer.idle_deferred_len(), 1);
}

#[test]
fn android_background_task_offline_unwatch_and_leave_drop_deferred_refreshes() {
    let (clock, state, desktop, phone) = idle_watcher();
    publish(&desktop, &state, &clock, "10.0.0.1:1");
    tracked_direct::unpublish(desktop.id, LOOKUP, &state);
    assert_eq!(tags(&phone.receiver), ["direct_offline"]);
    assert_eq!(phone.writer.idle_deferred_len(), 0);
    // After offline the next announcement is a first presence again.
    publish(&desktop, &state, &clock, "10.0.0.1:1");
    assert_eq!(tags(&phone.receiver), ["direct_available"]);

    publish(&desktop, &state, &clock, "10.0.0.1:1");
    let lookup_id = LOOKUP.to_string();
    dispatch(
        phone.id,
        &phone.writer,
        In::UnwatchDirect { lookup_id },
        &state,
    );
    assert_eq!(phone.writer.idle_deferred_len(), 0);
    watch(&phone, &state);
    assert_eq!(
        tags(&phone.receiver),
        ["direct_available"],
        "re-watch reply"
    );

    // Rooms: a member refresh is held, the member's leave drops it.
    let mut member = presence("desktop", "room", &clock, "10.0.0.1:1");
    member.kind = "room".into();
    join_room(desktop.id, &desktop.writer, "room", member.clone(), &state);
    let mut own = presence("phone", "room", &clock, "10.0.0.9:1");
    own.kind = "room".into();
    join_room(phone.id, &phone.writer, "room", own, &state);
    assert_eq!(tags(&phone.receiver), ["room_roster"]);
    clock.advance(SECOND);
    member.expires_at = clock.unix_secs() + 300;
    member.nonce = "refreshed".into();
    join_room(desktop.id, &desktop.writer, "room", member, &state);
    assert!(
        tags(&phone.receiver).is_empty(),
        "room refresh was not held"
    );
    assert_eq!(phone.writer.idle_deferred_len(), 1);
    leave_room(desktop.id, "room", &state);
    assert_eq!(tags(&phone.receiver), ["room_left"]);
    assert_eq!(phone.writer.idle_deferred_len(), 0);

    clock.advance(180 * SECOND);
    phone.writer.idle_tick(clock.now());
    assert_eq!(tags(&phone.receiver), ["keepalive"]);
}

#[test]
fn android_background_task_room_refresh_is_delivered_with_keepalive() {
    let clock = TestClock::new(START_UNIX);
    let state = Arc::new(Mutex::new(State::default()));
    let desktop = peer(&state, &clock, "desktop", false);
    let phone = peer(&state, &clock, "phone", true);
    let room_presence = |device: &str| {
        let mut presence = presence(device, "room", &clock, "10.0.0.1:1");
        presence.kind = "room".into();
        presence
    };
    join_room(
        desktop.id,
        &desktop.writer,
        "room",
        room_presence("desktop"),
        &state,
    );
    join_room(
        phone.id,
        &phone.writer,
        "room",
        room_presence("phone"),
        &state,
    );
    set_idle(&phone, &state, true, None);
    assert_eq!(tags(&phone.receiver), ["room_roster", "idle_ack"]);

    clock.advance(60 * SECOND);
    join_room(
        desktop.id,
        &desktop.writer,
        "room",
        room_presence("desktop"),
        &state,
    );
    assert!(tags(&phone.receiver).is_empty());
    clock.advance(120 * SECOND);
    phone.writer.idle_tick(clock.now());
    assert_eq!(tags(&phone.receiver), ["room_joined", "keepalive"]);
}

#[test]
fn android_background_task_waking_flushes_before_idle_ack() {
    let (clock, state, desktop, phone) = idle_watcher();
    publish(&desktop, &state, &clock, "10.0.0.1:1");
    set_idle(&phone, &state, false, None);
    let delivered = drain(&phone.receiver);
    assert_eq!(delivered.len(), 2);
    assert_eq!(delivered[0]["t"], "direct_available");
    assert_eq!(delivered[1]["t"], "idle_ack");
    assert_eq!(delivered[1]["idle"], false);
    assert_eq!(delivered[1]["keepalive_secs"], 180);
    // Not idle: refreshes are never held and no keepalive is due.
    publish(&desktop, &state, &clock, "10.0.0.1:1");
    clock.advance(400 * SECOND);
    phone.writer.idle_tick(clock.now());
    assert_eq!(tags(&phone.receiver), ["direct_available"]);
}

#[test]
fn android_background_task_keepalive_proposal_and_capability_gate() {
    let (_clock, state, desktop, phone) = idle_watcher();
    for (proposal, expected) in [(Some(90), 90), (Some(5), 30), (Some(900), 180), (None, 180)] {
        set_idle(&phone, &state, true, proposal);
        let ack = drain(&phone.receiver);
        assert_eq!(ack.len(), 1);
        assert_eq!(ack[0]["keepalive_secs"], expected, "proposal {proposal:?}");
    }

    // Without the capability `set_idle` is ignored: no ack, nothing held.
    let clock = TestClock::new(START_UNIX);
    let legacy = peer(&state, &clock, "legacy-phone", false);
    let lookup_id = LOOKUP.to_string();
    dispatch(
        legacy.id,
        &legacy.writer,
        In::WatchDirect { lookup_id },
        &state,
    );
    set_idle(&legacy, &state, true, None);
    publish(&desktop, &state, &clock, "10.0.0.1:1");
    assert_eq!(
        tags(&legacy.receiver),
        ["direct_available", "direct_available"]
    );
    assert_eq!(
        legacy.writer.idle_tick(clock.now()),
        super::idle_outbox::IdleStatus::Active
    );
}

/// Mandatory check from K2: K = 180 s, the publisher refreshes every 60 s.
#[test]
fn android_background_task_k180_bundles_one_refresh_per_tick_and_copy_never_expires() {
    let (clock, state, desktop, phone) = idle_watcher();
    let mut held_expiry = START_UNIX + 300;
    let mut refreshes_since_tick = 0;
    let mut ticks = 0;
    for second in 1..=1800 {
        clock.advance(SECOND);
        if second % 60 == 0 {
            publish(&desktop, &state, &clock, "10.0.0.1:1");
        }
        phone.writer.idle_tick(clock.now());
        for message in drain(&phone.receiver) {
            match message["t"].as_str().unwrap() {
                "direct_available" => {
                    held_expiry = message["presence"]["expires_at"].as_i64().unwrap();
                    refreshes_since_tick += 1;
                }
                "keepalive" => {
                    assert_eq!(refreshes_since_tick, 1, "tick at {second} s");
                    refreshes_since_tick = 0;
                    ticks += 1;
                }
                other => panic!("unexpected {other} at {second} s"),
            }
        }
        assert!(
            held_expiry > clock.unix_secs(),
            "observer copy expired at {second} s"
        );
    }
    assert_eq!(ticks, 10);
}

/// K16: the monotonic clock stood still while the wall clock ran on; the
/// observer's copy is judged on the wall clock and refreshed at once.
#[test]
fn android_background_task_wall_clock_jump_sends_refresh_immediately() {
    let (clock, state, desktop, phone) = idle_watcher();
    clock.advance_wall(250 * SECOND);
    publish(&desktop, &state, &clock, "10.0.0.1:1");
    assert_eq!(tags(&phone.receiver), ["direct_available"]);
    assert_eq!(phone.writer.idle_deferred_len(), 0);
}

#[test]
fn android_background_task_deferred_bytes_stay_within_one_writer_queue() {
    let start = Instant::now();
    let mut outbox = IdleOutbox::new(Keepalive::default());
    let mut sent = Vec::new();
    let mut enqueue = |json: Vec<u8>| {
        sent.push(json.len());
        true
    };
    let message = |lookup: &str| Out::DirectAvailable {
        lookup_id: lookup.into(),
        presence: presence("desktop", lookup, &TestClock::new(START_UNIX), "10.0.0.1:1"),
    };
    let half = MAX_WRITER_QUEUED_BYTES / 2;
    for lookup in ["a", "b", "c"] {
        assert!(outbox.send(&message(lookup), vec![b' '; 8], START_UNIX, &mut enqueue));
    }
    outbox.set_idle(true, None, start, &mut enqueue);
    for lookup in ["a", "b"] {
        outbox.offer(
            &message(lookup),
            vec![b' '; half],
            start,
            START_UNIX,
            &mut enqueue,
        );
    }
    assert_eq!(outbox.deferred_len(), 2);
    outbox.offer(
        &message("c"),
        vec![b' '; half],
        start,
        START_UNIX,
        &mut enqueue,
    );
    assert_eq!(outbox.deferred_len(), 2, "over budget: sent immediately");
    assert_eq!(sent.last(), Some(&half));
}
