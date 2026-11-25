//! Peer addresses at connection setup and for direct retries (2026-11-25).
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::*;
use crate::share::core::now_secs;
use crate::share::endpoint_routes::NodeTransportOptions;
use crate::share::types::{DirectContact, ShareStatus};

fn presence(node: &str, relation: &str, candidates: &[&str], expires_at: i64) -> PeerPresence {
    PeerPresence {
        kind: "direct".into(),
        relation_id: relation.into(),
        device_id: "remote".into(),
        device_name: "Remote".into(),
        public_key: node.into(),
        fingerprint: "fingerprint".into(),
        node_id: node.into(),
        relay_url: "https://relay.example:51821/".into(),
        candidates: candidates
            .iter()
            .map(|candidate| candidate.to_string())
            .collect(),
        expires_at,
        nonce: format!("nonce-{expires_at}-{}", candidates.join(",")),
        proof: "proof".into(),
    }
}

fn node_id(seed: u8) -> String {
    iroh::SecretKey::from_bytes(&[seed; 32])
        .public()
        .to_string()
}

fn addrs(list: &[&str]) -> Vec<SocketAddr> {
    list.iter().map(|addr| addr.parse().unwrap()).collect()
}

fn book() -> PeerAddressBook {
    PeerAddressBook::new(crossbeam_channel::bounded(1).0)
}

#[test]
fn peer_dial_task_setup_dials_fresh_addresses_over_a_stale_cache() {
    let now = now_secs();
    let node = node_id(23);
    let options = NodeTransportOptions::default();
    let local = iroh::EndpointAddr::from_parts(
        iroh::SecretKey::from_bytes(&[24; 32]).public(),
        Vec::<iroh::TransportAddr>::new(),
    );
    // Cached before the peer's network change, with a LAN sighting.
    let stale = presence(
        &node,
        "lookup",
        &["203.0.113.5:4000", "192.168.1.20:4000"],
        now + 20,
    );
    let book = book();
    assert_eq!(
        merged_dial_presence(&stale, book.fresh(&stale).as_ref()),
        stale
    );
    // The server answered with the presence the peer published now.
    let fresh = presence(&node, "lookup", &["198.51.100.9:4100"], now + 300);
    book.record(&fresh);
    let merged = merged_dial_presence(&stale, book.fresh(&stale).as_ref());
    assert_eq!(
        merged.candidates,
        ["198.51.100.9:4100", "203.0.113.5:4000", "192.168.1.20:4000"]
    );
    assert_eq!(merged.expires_at, now + 300);
    let dialed = endpoint_addr(&merged, &local, &options).expect("dial address");
    let dialed: Vec<SocketAddr> = dialed.ip_addrs().copied().collect();
    assert!(
        dialed.contains(&addrs(&["198.51.100.9:4100"])[0]),
        "{dialed:?}"
    );
    // An expired cache is no obstacle once the server answered.
    let expired = presence(&node, "lookup", &["203.0.113.5:4000"], now - 1);
    assert!(endpoint_addr(&expired, &local, &options).is_err());
    let renewed = merged_dial_presence(&expired, book.fresh(&expired).as_ref());
    assert!(endpoint_addr(&renewed, &local, &options).is_ok());
    // Never from another relation, another device, or an older presence.
    let other = presence(&node, "other-lookup", &["203.0.113.5:4000"], now + 20);
    assert_eq!(
        merged_dial_presence(&other, book.fresh(&other).as_ref()),
        other
    );
    let mut impostor = fresh.clone();
    impostor.device_id = "other-device".into();
    assert_eq!(merged_dial_presence(&stale, Some(&impostor)), stale);
    let newer = presence(&node, "lookup", &["203.0.113.6:4000"], now + 400);
    assert_eq!(
        merged_dial_presence(&newer, book.fresh(&newer).as_ref()),
        newer
    );
    // A LAN-only cache (no server evidence) takes the server's presence.
    let mut lan_only = presence(&node, "lookup", &["192.168.1.20:4000"], now + 600);
    lan_only.relay_url.clear();
    let merged = merged_dial_presence(&lan_only, book.fresh(&lan_only).as_ref());
    assert_eq!(merged.relay_url, fresh.relay_url);
    assert_eq!(
        merged.candidates,
        ["198.51.100.9:4100", "192.168.1.20:4000"]
    );
}

#[test]
fn peer_dial_task_refresh_round_waits_for_the_server_answer() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    let (wake, woken) = crossbeam_channel::bounded(1);
    let book = Arc::new(PeerAddressBook::new(wake));
    // Without a signal session a setup never waits.
    assert_eq!(book.request("node"), None);
    book.session_started();
    let round = book.request("node").expect("round");
    assert!(woken.try_recv().is_ok(), "the worker is woken");
    assert_eq!(book.request("node"), Some(round));
    assert_eq!(book.take_round(), Some(vec!["node".to_string()]));
    assert_eq!(book.take_round(), None, "one round in flight");
    runtime.block_on(async {
        let waiting = book.clone();
        let waiter =
            tokio::spawn(async move { waiting.wait_round(round, Duration::from_secs(5)).await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        book.pong();
        assert!(
            waiter.await.expect("waiter"),
            "the pong completes the round"
        );
    });
    // Refreshed in this session: later setups dial at once.
    assert_eq!(book.request("node"), None);
    // A new session asks again; a silent server costs one bounded wait.
    book.session_ended();
    book.session_started();
    let round = book.request("node").expect("new session");
    assert!(book.take_round().is_some());
    runtime.block_on(async {
        let started = Instant::now();
        assert!(!book.wait_round(round, Duration::from_millis(50)).await);
        assert!(started.elapsed() < Duration::from_secs(2));
    });
    assert_eq!(book.request("other"), None, "no further waits while silent");
    book.pong();
    assert!(
        book.request("other").is_some(),
        "an answer resumes the refresh"
    );
    // A round in flight when the session ends goes out in the next one.
    assert_eq!(book.take_round(), Some(vec!["other".to_string()]));
    book.session_ended();
    assert_eq!(book.take_round(), None, "no session");
    book.session_started();
    assert_eq!(book.take_round(), Some(vec!["other".to_string()]));
}

fn contact(presence: PeerPresence, node: &str) -> DirectContact {
    DirectContact {
        id: "contact".into(),
        display_name: "Remote".into(),
        lookup_id: "lookup".into(),
        expected_fingerprint: "fingerprint".into(),
        expected_node_id: node.into(),
        remote_device_id: Some("remote".into()),
        remote_public_key: Some(node.into()),
        auto_connect: true,
        auto_open: false,
        last_seen: None,
        status: ShareStatus::Available,
        last_error: None,
        presence: Some(presence),
        access_state: DirectAccessState::Accepted,
        request_sent_at: None,
        accepted_at: None,
        accepted_public_key: Some(node.into()),
        lan_candidates: Vec::new(),
        lan_seen_at: None,
        lan_uplink: None,
        relation: Default::default(),
    }
}

fn state(contact: DirectContact) -> ShareAuthState {
    let secret = iroh::SecretKey::from_bytes(&[19; 32]);
    let node_id = secret.public().to_string();
    ShareAuthState {
        identity: crate::share::ShareIdentity {
            device_id: "local".into(),
            device_name: "Local".into(),
            direct_lookup_id: "local-lookup".into(),
            public_key: node_id.clone(),
            fingerprint: crate::share::core::public_fingerprint(node_id.as_bytes()),
            node_id,
            iroh_secret: secret,
            direct_secret: [0; 32],
        },
        direct_secret: vec![0; 32],
        default_direct_exports: Default::default(),
        direct_contacts: vec![contact],
        direct_grants: Vec::new(),
        rooms: Vec::new(),
        direct_requests: Vec::new(),
        direct_request_tombstones: Vec::new(),
        seen_nonces: Default::default(),
        direct_online: true,
        authorization_epoch: 0,
    }
}

#[test]
fn peer_dial_task_retry_addresses_follow_peer_republish() {
    let now = now_secs();
    let node = node_id(29);
    let cached = presence(&node, "lookup", &["203.0.113.5:4000"], now + 20);
    let mut contact = contact(cached, &node);
    contact.lan_candidates = vec!["192.168.1.20:4000".into()];
    contact.lan_seen_at = Some(now);
    let mut state = state(contact);
    let book = book();
    let mut changes = book.subscribe();
    let before = peer_addresses(&state, &book, &node, now);
    assert_eq!(before, addrs(&["192.168.1.20:4000", "203.0.113.5:4000"]));
    // The peer republished after its network change.
    book.record(&presence(
        &node,
        "lookup",
        &["198.51.100.9:4100"],
        now + 300,
    ));
    assert!(
        changes.has_changed().expect("book"),
        "a republish wakes the retries"
    );
    let after = peer_addresses(&state, &book, &node, now);
    assert_eq!(
        after,
        addrs(&["192.168.1.20:4000", "198.51.100.9:4100", "203.0.113.5:4000"])
    );
    // The daemon has not stored any server presence yet: the book alone.
    state.direct_contacts[0].presence = None;
    state.direct_contacts[0].lan_seen_at = None;
    assert_eq!(
        peer_addresses(&state, &book, &node, now),
        addrs(&["198.51.100.9:4100"])
    );
    // Other nodes and contacts without access get nothing.
    assert!(peer_addresses(&state, &book, &node_id(30), now).is_empty());
    state.direct_contacts[0].access_state = DirectAccessState::Ignored;
    assert!(peer_addresses(&state, &book, &node, now).is_empty());
    // A relation runtime update (LAN sighting) wakes them as well.
    drop(changes.borrow_and_update());
    book.changed();
    assert!(changes.has_changed().expect("book"));
}
