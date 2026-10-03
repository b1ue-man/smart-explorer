//! Ownership, compatibility and fair PIN access acceptance signals for the remote suite.

use std::collections::HashSet;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iroh_base::SecretKey;

use crate::discovery_state::{prepare_exchange_locked, DiscoveryOffer};
use crate::limits::SourceKey;
use crate::protocol::{DiscoveryAdvertisement, DiscoveryKind};
use crate::state::{cleanup, register_client_with_identity, ClientIdentity, RegistrationError, State};
use crate::{direct_presence, discovery, login, rooms, Out, PeerPresence, Writer};

struct Peer {
    id: u64,
    device: String,
    key: SecretKey,
    writer: Writer,
    receiver: Receiver<Out>,
}

fn peer(state: &Arc<Mutex<State>>, device: &str, seed: u8, proven: bool) -> Peer {
    let key = SecretKey::from_bytes(&[seed; 32]);
    let identity = if proven { ClientIdentity::Proven(key.public()) } else { ClientIdentity::LegacyClaimed(key.public()) };
    let (writer, receiver) = Writer::test_channel(32);
    let id = register_client_with_identity(state, writer.clone(), SourceKey::Ipv4([192, 0, 2, seed]),
        device.into(), HashSet::from([discovery::CAPABILITY.into()]), identity).unwrap();
    Peer { id, device: device.into(), key, writer, receiver }
}

fn presence(peer: &Peer, kind: &str, relation: &str) -> PeerPresence {
    PeerPresence {
        kind: kind.into(), relation_id: relation.into(), device_id: peer.device.clone(),
        device_name: peer.device.clone(), public_key: peer.key.public().to_string(),
        fingerprint: "fp".into(), node_id: peer.key.public().to_string(),
        relay_url: "https://relay.example".into(), candidates: Vec::new(),
        expires_at: crate::discovery_state::unix_seconds() + 300, nonce: "nonce".into(), proof: "proof".into(),
    }
}

fn recv(peer: &Peer) -> Out { peer.receiver.recv_timeout(Duration::from_secs(2)).unwrap() }
fn rejected(peer: &Peer) { assert!(matches!(recv(peer), Out::Error { .. })); }

#[test]
fn review_task_proven_device_binding_displaces_impostor_and_survives_cleanup() {
    let state = Arc::new(Mutex::new(State::default()));
    let impostor = peer(&state, "device", 31, false);
    let owner = peer(&state, "device", 32, true);
    assert!(impostor.writer.is_closed());
    assert!(!state.lock().unwrap().clients.contains_key(&impostor.id));
    assert!(state.lock().unwrap().relay_admissions.admits(&owner.key.public()));
    cleanup(owner.id, &state);
    let (writer, _receiver) = Writer::test_channel(4);
    let registration = |identity| register_client_with_identity(&state, writer.clone(),
        SourceKey::Ipv4([198, 51, 100, 1]), "device".into(), HashSet::new(), identity);
    assert_eq!(registration(ClientIdentity::Proven(impostor.key.public())), Err(RegistrationError::DeviceBound));
    assert_eq!(registration(ClientIdentity::Legacy), Err(RegistrationError::DeviceBound));
    assert!(registration(ClientIdentity::Proven(owner.key.public())).is_ok());
}

#[test]
fn review_task_strict_mode_and_key_caps_cannot_be_bypassed_by_device_ids() {
    let mut initial = State::default();
    initial.policy.require_key_login = true;
    initial.policy.limits.max_clients_per_key = 1;
    let state = Arc::new(Mutex::new(initial));
    let owner = peer(&state, "owner", 35, true);
    let (writer, _receiver) = Writer::test_channel(4);
    let registration = |device: &str, identity| register_client_with_identity(&state, writer.clone(),
        SourceKey::Ipv4([198, 51, 100, 1]), device.into(), HashSet::new(), identity);
    assert_eq!(registration("other-id", ClientIdentity::Proven(owner.key.public())), Err(RegistrationError::KeyFull));
    assert_eq!(registration("old", ClientIdentity::Legacy), Err(RegistrationError::KeyLoginRequired));
}

#[test]
fn review_task_bound_lookup_requires_access_and_only_its_owner_mutates_it() {
    let state = Arc::new(Mutex::new(State::default()));
    let owner = peer(&state, "owner", 41, true);
    let watcher = peer(&state, "watcher", 42, true);
    let legacy = peer(&state, "legacy", 43, false);
    let proof = login::hex(&[5; 32]);
    let hash = login::hex(&login::sha256(&[5; 32]));
    direct_presence::publish(owner.id, &owner.writer, presence(&owner, "direct", "lookup"), Some(hash), &state);
    assert_eq!(state.lock().unwrap().direct["lookup"].0, owner.id);
    direct_presence::watch(watcher.id, &watcher.writer, "lookup", Some(login::hex(&[6; 32])), &state);
    rejected(&watcher);
    assert!(!state.lock().unwrap().clients[&watcher.id].watched_lookup_ids.contains("lookup"));
    direct_presence::watch(watcher.id, &watcher.writer, "lookup", Some(proof), &state);
    assert!(matches!(recv(&watcher), Out::DirectAvailable { .. }));
    direct_presence::watch(legacy.id, &legacy.writer, "lookup", None, &state);
    assert!(matches!(recv(&legacy), Out::DirectAvailable { .. }));
    direct_presence::publish(watcher.id, &watcher.writer, presence(&watcher, "direct", "lookup"),
        Some(login::hex(&[9; 32])), &state);
    rejected(&watcher);
    direct_presence::publish(legacy.id, &legacy.writer, presence(&legacy, "direct", "lookup"), None, &state);
    rejected(&legacy);
    crate::dispatch(watcher.id, &watcher.writer, crate::In::DirectAccessAccepted {
        lookup_id: "lookup".into(), requester_device_id: watcher.device.clone(), accepted: false,
        presence: Some(presence(&owner, "direct", "lookup")), msg: Some("forged".into()),
    }, &state);
    rejected(&watcher);
    direct_presence::unpublish(watcher.id, "lookup", &state);
    assert_eq!(state.lock().unwrap().direct["lookup"].0, owner.id);
    direct_presence::unpublish(owner.id, "lookup", &state);
    assert!(matches!(recv(&watcher), Out::DirectOffline { .. }));
    assert!(matches!(recv(&legacy), Out::DirectOffline { .. }));
    assert!(direct_presence::require_owner(&state, owner.id, &owner.writer, "lookup"));
}

#[test]
fn review_task_room_partitions_and_member_origin_preserve_mixed_clients() {
    let state = Arc::new(Mutex::new(State::default()));
    let owner = peer(&state, "owner", 51, true);
    let stranger = peer(&state, "stranger", 52, true);
    let member = peer(&state, "member", 53, true);
    let legacy = peer(&state, "legacy", 54, false);
    let proof = login::hex(&[5; 32]);
    rooms::join_with_access(owner.id, &owner.writer, "room", presence(&owner, "room", "room"), Some(proof.clone()), &state);
    assert!(matches!(recv(&owner), Out::RoomRoster { members, .. } if members.is_empty()));
    rooms::join_with_access(stranger.id, &stranger.writer, "room", presence(&owner, "room", "room"), Some(proof.clone()), &state);
    rejected(&stranger);
    rooms::join_with_access(stranger.id, &stranger.writer, "room", presence(&stranger, "room", "room"), Some(login::hex(&[6; 32])), &state);
    assert!(matches!(recv(&stranger), Out::RoomRoster { members, .. } if members.is_empty()));
    assert!(owner.receiver.try_recv().is_err());
    rooms::join_with_access(member.id, &member.writer, "room", presence(&member, "room", "room"), Some(proof), &state);
    assert!(matches!(recv(&member), Out::RoomRoster { members, .. } if members.len() == 1 && members[0].device_id == owner.device));
    assert!(matches!(recv(&owner), Out::RoomJoined { .. }));
    rooms::join_with_access(legacy.id, &legacy.writer, "room", presence(&legacy, "room", "room"), None, &state);
    assert!(matches!(recv(&legacy), Out::RoomRoster { members, .. } if members.len() == 3));
    for peer in [&owner, &member, &stranger] { assert!(matches!(recv(peer), Out::RoomJoined { .. })); }
    rooms::leave_room(owner.id, "room", &state);
    assert!(matches!(recv(&member), Out::RoomLeft { .. }));
    assert!(matches!(recv(&legacy), Out::RoomLeft { .. }));
    assert!(stranger.receiver.try_recv().is_err());
    assert!(state.lock().unwrap().rooms["room"].contains_key(&stranger.device));
}

#[test]
fn review_task_device_and_lookup_bindings_survive_state_file_restart() {
    let directory = std::env::temp_dir().join(format!("se-signal-state-{}", login::hex(&login::nonce().unwrap())));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("bindings.json");
    let mut initial = State::default();
    initial.binding_path = Some(path.clone());
    let state = Arc::new(Mutex::new(initial));
    let owner = peer(&state, "owner", 61, true);
    direct_presence::publish(owner.id, &owner.writer, presence(&owner, "direct", "lookup"),
        Some(login::hex(&login::sha256(&[5; 32]))), &state);
    let bindings = crate::bindings::Bindings::load(&path, i64::MAX).unwrap();
    let owner_key = owner.key.public().to_string();
    assert_eq!(bindings.device_key(&owner.device), Some(owner_key.as_str()));
    assert_eq!(bindings.lookup_key("lookup"), Some(owner_key.as_str()));
    let mut restarted = State::default();
    restarted.bindings = bindings;
    let state = Arc::new(Mutex::new(restarted));
    let intruder = peer(&state, "intruder", 62, true);
    direct_presence::publish(intruder.id, &intruder.writer, presence(&intruder, "direct", "lookup"),
        Some(login::hex(&[9; 32])), &state);
    rejected(&intruder);
    assert!(!state.lock().unwrap().direct.contains_key("lookup"));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn review_task_pin_start_budget_is_per_connector_key_and_preserves_other_access() {
    let state = Arc::new(Mutex::new(State::default()));
    let publisher = peer(&state, "publisher", 71, true);
    let first = peer(&state, "first", 72, true);
    let same_key = peer(&state, "same-key", 72, true);
    let other = peer(&state, "other", 73, true);
    let proxied_first = peer(&state, "proxied-first", 74, false);
    let proxied_second = peer(&state, "proxied-second", 75, false);
    let now = Instant::now();
    let mut state = state.lock().unwrap();
    for id in [proxied_first.id, proxied_second.id] {
        state.clients.get_mut(&id).unwrap().source = SourceKey::ExternallyLimitedProxy;
    }
    state.discovery_offers.insert("offer".into(), DiscoveryOffer::new(publisher.id, DiscoveryAdvertisement {
        discovery_id: "offer".into(), offer_id: "local-offer".into(), kind: DiscoveryKind::Direct,
        display_alias: "Alias".into(), suite: "opaque".into(), version: 1, expires_at: i64::MAX,
    }, now + Duration::from_secs(300)));
    assert!(prepare_exchange_locked(&mut state, first.id, "offer", "active", 3, now).is_ok());
    assert_eq!(prepare_exchange_locked(&mut state, same_key.id, "offer", "same-key-active", 3, now).err(),
        Some("connector already pairing with this offer"));
    assert!(prepare_exchange_locked(&mut state, other.id, "offer", "other-active", 3, now).is_ok());
    state.discovery_exchanges.remove("active");
    // Sequential attempts by this key exhaust only its own sliding window.
    for index in 1..12 {
        let exchange = format!("sequential-{index}");
        assert!(prepare_exchange_locked(&mut state, first.id, "offer", &exchange, 3, now).is_ok());
        state.discovery_exchanges.remove(&exchange);
    }
    assert_eq!(prepare_exchange_locked(&mut state, same_key.id, "offer", "exhausted", 3, now).err(),
        Some("discovery offer pairing attempt rate exceeded"));
    state.discovery_exchanges.remove("other-active");
    assert!(prepare_exchange_locked(&mut state, other.id, "offer", "other-new", 3, now).is_ok());
    assert!(prepare_exchange_locked(&mut state, first.id, "offer", "window-renewed", 3,
        now + Duration::from_secs(60)).is_ok());
    for (id, exchange) in [(proxied_first.id, "proxy-first"), (proxied_second.id, "proxy-second")] {
        assert!(prepare_exchange_locked(&mut state, id, "offer", exchange, 3,
            now + Duration::from_secs(60)).is_ok());
    }
}
