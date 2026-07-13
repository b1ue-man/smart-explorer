use std::collections::HashSet;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use super::limits::{validate_presence, SourceKey, MAX_ROOM_MEMBERS, MAX_WRITER_QUEUED_BYTES};
use super::state::{cleanup, join_room, leave_room, lock_state, register_client, State};
use super::writer::{outbound_fits, QueuedMessage};
use super::{tracked_direct, Out, PeerPresence, Writer};

#[test]
fn non_owner_cannot_forge_direct_offline_event() {
    let state = Arc::new(Mutex::new(State::default()));
    let (owner, _owner_rx, owner_id) = add_client(&state, 1, "owner");
    let (_attacker, attacker_rx, attacker_id) = add_client(&state, 2, "attacker");
    let lookup_id = "lookup";
    {
        let mut state = lock_state(&state);
        state
            .direct
            .insert(lookup_id.into(), (owner_id, presence("owner", lookup_id)));
        state
            .clients
            .get_mut(&owner_id)
            .unwrap()
            .direct_lookup_ids
            .insert(lookup_id.into());
        state
            .watchers
            .insert(lookup_id.into(), HashSet::from([attacker_id]));
    }

    tracked_direct::unpublish(attacker_id, lookup_id, &state);
    assert_eq!(lock_state(&state).direct[lookup_id].0, owner_id);
    assert!(attacker_rx.try_recv().is_err());

    tracked_direct::unpublish(owner_id, lookup_id, &state);
    assert!(!lock_state(&state).direct.contains_key(lookup_id));
    assert!(matches!(
        decode(attacker_rx.recv().unwrap()),
        Out::DirectOffline { lookup_id: got } if got == lookup_id
    ));
    drop(owner);
}

#[test]
fn non_member_cannot_forge_room_left_event() {
    let state = Arc::new(Mutex::new(State::default()));
    let (first, first_rx, first_id) = add_client(&state, 1, "first");
    let (second, second_rx, second_id) = add_client(&state, 2, "second");
    let (_attacker, _attacker_rx, attacker_id) = add_client(&state, 3, "attacker");
    join_room(first_id, &first, "room", presence("first", "room"), &state);
    join_room(
        second_id,
        &second,
        "room",
        presence("second", "room"),
        &state,
    );
    drain(&first_rx);
    drain(&second_rx);

    leave_room(attacker_id, "room", &state);

    assert_eq!(lock_state(&state).rooms["room"].len(), 2);
    assert!(first_rx.try_recv().is_err());
    assert!(second_rx.try_recv().is_err());
}

#[test]
fn same_device_replacement_works_in_full_room_without_false_cleanup_event() {
    let state = Arc::new(Mutex::new(State::default()));
    let mut writers = Vec::new();
    let mut receivers = Vec::new();
    let mut ids = Vec::new();
    for index in 0..MAX_ROOM_MEMBERS {
        let device_id = format!("device-{index}");
        let (writer, receiver, id) = add_client(&state, index, &device_id);
        join_room(id, &writer, "room", presence(&device_id, "room"), &state);
        writers.push(writer);
        receivers.push(receiver);
        ids.push(id);
    }
    assert_eq!(lock_state(&state).rooms["room"].len(), MAX_ROOM_MEMBERS);
    for receiver in &receivers {
        drain(receiver);
    }

    let (replacement, replacement_rx, replacement_id) =
        add_client(&state, MAX_ROOM_MEMBERS, "device-0");
    join_room(
        replacement_id,
        &replacement,
        "room",
        presence("device-0", "room"),
        &state,
    );
    assert_eq!(lock_state(&state).rooms["room"].len(), MAX_ROOM_MEMBERS);
    assert_eq!(
        lock_state(&state).rooms["room"]["device-0"].0,
        replacement_id
    );
    assert!(!lock_state(&state).clients[&ids[0]].rooms.contains("room"));
    drain(&replacement_rx);
    for receiver in &receivers {
        drain(receiver);
    }

    cleanup(ids[0], &state);
    for receiver in receivers.iter().skip(1) {
        assert!(receiver.try_recv().is_err());
    }
    assert!(replacement_rx.try_recv().is_err());
    drop(writers);
}

#[test]
fn identical_room_rejoin_does_not_fan_out_duplicate_join() {
    let state = Arc::new(Mutex::new(State::default()));
    let (first, first_rx, first_id) = add_client(&state, 1, "first");
    let (second, second_rx, second_id) = add_client(&state, 2, "second");
    join_room(first_id, &first, "room", presence("first", "room"), &state);
    let second_presence = presence("second", "room");
    join_room(second_id, &second, "room", second_presence.clone(), &state);
    drain(&first_rx);
    drain(&second_rx);

    join_room(second_id, &second, "room", second_presence, &state);

    assert!(first_rx.try_recv().is_err());
    assert!(matches!(
        decode(second_rx.recv().unwrap()),
        Out::RoomRoster { .. }
    ));
}

#[test]
fn configured_room_count_with_normal_presences_fits_one_roster_message() {
    let members = (0..MAX_ROOM_MEMBERS - 1)
        .map(|index| presence(&format!("device-{index}"), "room"))
        .collect();
    assert!(outbound_fits(&Out::RoomRoster {
        room_id: "room".into(),
        members,
    }));
}

#[test]
fn oversized_room_roster_is_rejected_before_state_mutation() {
    let state = Arc::new(Mutex::new(State::default()));
    let (writer, receiver, id) = add_client(&state, 1, "joiner");
    {
        let mut state = lock_state(&state);
        let members = state.rooms.entry("room".into()).or_default();
        for index in 0..MAX_ROOM_MEMBERS - 1 {
            let mut member = presence(&format!("member-{index}"), "room");
            member.device_name = "\\".repeat(1024);
            member.relay_url = "\\".repeat(2048);
            member.candidates = (0..32).map(|_| "\\".repeat(256)).collect();
            validate_presence(&member).unwrap();
            members.insert(member.device_id.clone(), (index as u64 + 100, member));
        }
    }
    let before = state_counts(&state);

    join_room(id, &writer, "room", presence("joiner", "room"), &state);

    assert_eq!(state_counts(&state), before);
    assert!(!lock_state(&state).clients[&id].rooms.contains("room"));
    assert!(matches!(
        decode(receiver.recv().unwrap()),
        Out::Error { scope, .. } if scope == "room"
    ));
    assert!(!writer.is_closed());
}

fn add_client(
    state: &Arc<Mutex<State>>,
    source_index: usize,
    device_id: &str,
) -> (Writer, Receiver<QueuedMessage>, u64) {
    let (writer, receiver) = Writer::test_raw_channel(MAX_ROOM_MEMBERS * 2);
    let source = SourceKey::Ipv4([
        192,
        0,
        (source_index / 256) as u8,
        (source_index % 256) as u8,
    ]);
    let id = register_client(
        state,
        writer.clone(),
        source,
        device_id.into(),
        HashSet::new(),
    )
    .unwrap();
    (writer, receiver, id)
}

fn presence(device_id: &str, relation_id: &str) -> PeerPresence {
    PeerPresence {
        kind: "room".into(),
        relation_id: relation_id.into(),
        device_id: device_id.into(),
        device_name: device_id.into(),
        public_key: "pk".into(),
        fingerprint: "fp".into(),
        node_id: "node".into(),
        relay_url: "http://127.0.0.1:51821".into(),
        candidates: vec!["127.0.0.1:1".into()],
        expires_at: 99,
        nonce: "nonce".into(),
        proof: "proof".into(),
    }
}

fn decode(message: QueuedMessage) -> Out {
    serde_json::from_slice(message.json()).unwrap()
}

fn drain(receiver: &Receiver<QueuedMessage>) {
    while receiver.try_recv().is_ok() {}
}

fn state_counts(state: &Arc<Mutex<State>>) -> (usize, usize, usize, usize) {
    let state = lock_state(state);
    (
        state.clients.len(),
        state.direct.len(),
        state.watchers.len(),
        state.rooms.len(),
    )
}

#[test]
fn large_test_queue_stays_below_the_production_byte_budget() {
    let normal_roster_upper_bound = MAX_ROOM_MEMBERS * 2 * 1024;
    assert!(normal_roster_upper_bound < MAX_WRITER_QUEUED_BYTES);
}
