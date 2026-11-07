//! Which peers learn the class of a rejected file session and the host's
//! name (R-ZUGANG-02, R-ZUGANG-03 a). Every case runs the production
//! `authenticate_incoming_session` against a host state and reads
//! `denial_of` of the error, as `server.rs` does before it answers.
use std::io;
use std::sync::{Arc, Mutex};

use crate::share::core::{hmac_proof, public_fingerprint};
use crate::share::exec_policy::ExecGrant;
use crate::share::identity::ShareIdentity;
use crate::share::session::denial::{denial_of, SessionDenial};
use crate::share::session::peer_denials::{clear_incoming, host_rejection, rows};
use crate::share::session::{authenticate_incoming_session, session_payload};
use crate::share::types::{
    DirectAccessState, DirectContact, DirectGrant, DirectGrantState, RoomMember,
    RoomMemberAdmission, RoomMemberFlags, RoomPolicy, RoomProfile, ShareAuthState,
};
use crate::share::wire::{FsResponse, PeerHello};

const NONCE: &str = "gate-nonce-0123456789";
const WRONG_SECRET: &[u8] = b"not-the-secret-of-this-relation";
const UNLINKED: &str = "Gegenstelle ohne Kopplung";

fn identity(device_id: &str, seed: u8) -> ShareIdentity {
    let secret = iroh::SecretKey::from_bytes(&[seed; 32]);
    let node_id = secret.public().to_string();
    ShareIdentity {
        device_id: device_id.into(),
        device_name: format!("Name of {device_id}"),
        direct_lookup_id: format!("lookup-{device_id}"),
        public_key: node_id.clone(),
        fingerprint: public_fingerprint(node_id.as_bytes()),
        node_id,
        iroh_secret: secret,
        direct_secret: [seed; 32],
    }
}

fn state(host: &ShareIdentity) -> ShareAuthState {
    ShareAuthState {
        identity: host.clone(),
        direct_secret: host.direct_secret(),
        default_direct_exports: Default::default(),
        direct_contacts: Vec::new(),
        direct_grants: Vec::new(),
        rooms: Vec::new(),
        direct_requests: Vec::new(),
        direct_request_tombstones: Vec::new(),
        seen_nonces: Default::default(),
        direct_online: true,
        authorization_epoch: 1,
    }
}

fn grant(client: &ShareIdentity, grant_state: DirectGrantState) -> DirectGrant {
    DirectGrant {
        device_id: client.device_id.clone(),
        device_name: client.device_name.clone(),
        public_key: client.public_key.clone(),
        fingerprint: client.fingerprint.clone(),
        node_id: client.node_id.clone(),
        state: grant_state,
        updated_at: 1,
        exec: ExecGrant::default(),
        write: false,
    }
}

fn contact(peer: &ShareIdentity, remote_device_id: Option<&str>) -> DirectContact {
    DirectContact {
        id: format!("contact-{}", peer.device_id),
        display_name: format!("Contact {}", peer.device_id),
        lookup_id: format!("lookup-of-{}", peer.device_id),
        expected_fingerprint: peer.fingerprint.clone(),
        expected_node_id: peer.node_id.clone(),
        remote_device_id: remote_device_id.map(str::to_string),
        remote_public_key: Some(peer.public_key.clone()),
        auto_connect: false,
        auto_open: false,
        last_seen: None,
        status: Default::default(),
        last_error: None,
        presence: None,
        access_state: DirectAccessState::Accepted,
        request_sent_at: None,
        accepted_at: Some(1),
        accepted_public_key: Some(peer.public_key.clone()),
        lan_candidates: Vec::new(),
        lan_seen_at: None,
        lan_uplink: None,
        relation: Default::default(),
    }
}

fn room(room_id: &str, members: Vec<RoomMember>) -> RoomProfile {
    RoomProfile {
        id: format!("profile-{room_id}"),
        name: room_id.into(),
        room_id: room_id.into(),
        auto_join: true,
        last_seen: None,
        status: Default::default(),
        members,
        exports: Default::default(),
        policy: RoomPolicy::new_room(),
    }
}

fn member(peer: &ShareIdentity, blocked: bool, admission: RoomMemberAdmission) -> RoomMember {
    RoomMember {
        device_id: peer.device_id.clone(),
        device_name: peer.device_name.clone(),
        fingerprint: peer.fingerprint.clone(),
        public_key: peer.public_key.clone(),
        node_id: peer.node_id.clone(),
        relay_url: String::new(),
        candidates: Vec::new(),
        last_seen: None,
        status: Default::default(),
        blocked,
        exec: ExecGrant::default(),
        presence: None,
        relation: RoomMemberFlags {
            admission,
            ..RoomMemberFlags::default()
        },
    }
}

/// The hello of `client` with a session proof made with `secret`.
fn hello(
    host: &ShareIdentity,
    client: &ShareIdentity,
    kind: &str,
    relation_id: &str,
    secret: &[u8],
) -> PeerHello {
    let payload = session_payload(
        kind,
        relation_id,
        &client.device_id,
        &host.device_id,
        &client.node_id,
        &host.node_id,
        NONCE,
    );
    PeerHello {
        protocol_version: 3,
        relation_kind: kind.into(),
        relation_id: relation_id.into(),
        device_id: client.device_id.clone(),
        public_key: client.public_key.clone(),
        node_id: client.node_id.clone(),
        session_nonce: NONCE.into(),
        session_proof: hmac_proof(secret, &payload),
        requested_capabilities: Vec::new(),
    }
}

fn reject(auth: &Arc<Mutex<ShareAuthState>>, hello: &PeerHello) -> io::Error {
    authenticate_incoming_session(hello, &hello.node_id, auth).expect_err("the hello is rejected")
}

/// The class of the rejection and its text.
fn outcome(state: &ShareAuthState, hello: &PeerHello) -> (Option<SessionDenial>, String) {
    let auth = Arc::new(Mutex::new(state.clone()));
    let error = reject(&auth, hello);
    (denial_of(&error), error.to_string())
}

type Case = (
    &'static str,
    ShareAuthState,
    PeerHello,
    Option<SessionDenial>,
);

/// Runs every case; a class must come with the class text only for a peer
/// that may learn it, and the other text names the missing relation.
fn run(cases: Vec<Case>) {
    for (name, state, hello, expected) in cases {
        let (denial, text) = outcome(&state, &hello);
        assert_eq!(denial, expected, "{name}: class of the rejection ({text})");
        assert_eq!(
            text.contains(UNLINKED),
            expected.is_none(),
            "{name}: text of the rejection ({text})"
        );
    }
}

#[test]
fn zugang_task_denial_gate_hides_class_and_host_name_from_strangers() {
    let host = identity("host-gate-a", 61);
    let stranger = identity("stranger-gate-a", 62);
    let lookup = host.direct_lookup_id.clone();
    let secret = host.direct_secret();

    // Without any relation, a made-up lookup id (the code a stranger holds
    // is not this host's), and a proof that fails.
    let mut offline = state(&host);
    offline.direct_online = false;
    let in_room = {
        let mut state = state(&host);
        state.rooms.push(room("room-a", Vec::new()));
        state
    };
    run(vec![
        (
            "invented lookup id",
            state(&host),
            hello(&host, &stranger, "direct", "invented-lookup", &secret),
            None,
        ),
        (
            "Direct off, wrong proof",
            offline,
            hello(&host, &stranger, "direct", &lookup, WRONG_SECRET),
            None,
        ),
        (
            "no grant, wrong proof",
            state(&host),
            hello(&host, &stranger, "direct", &lookup, WRONG_SECRET),
            None,
        ),
        (
            "room without the sender, no secret",
            in_room.clone(),
            hello(&host, &stranger, "room", "room-a", WRONG_SECRET),
            None,
        ),
        (
            "room the host never joined",
            in_room,
            hello(&host, &stranger, "room", "room-b", WRONG_SECRET),
            None,
        ),
    ]);

    // The host answers such a peer with the unclassified text, without its
    // own name, and records nothing.
    let auth = Arc::new(Mutex::new(state(&host)));
    let stranger_hello = hello(&host, &stranger, "direct", "invented-lookup", &secret);
    let error = reject(&auth, &stranger_hello);
    let (response, _log) = host_rejection(&stranger_hello, &auth, &error);
    let FsResponse::Err { msg, denial, .. } = response else {
        panic!("a rejected hello answers with an error");
    };
    assert_eq!(denial, None);
    assert_eq!(msg, "Share-Anmeldung nicht autorisiert");
    assert!(!msg.contains(&host.device_name), "{msg}");
    assert!(rows("gate-local")
        .iter()
        .all(|row| row.node_id != stranger.node_id));
}

#[test]
fn zugang_task_denial_gate_keeps_the_class_for_pinned_peers() {
    let host = identity("host-gate-b", 63);
    let peer = identity("peer-gate-b", 64);
    let lookup = host.direct_lookup_id.clone();
    // A pinned peer needs no valid proof to learn why it is rejected.
    let direct = |name: &'static str, auth: ShareAuthState, expected| -> Case {
        (
            name,
            auth,
            hello(&host, &peer, "direct", &lookup, WRONG_SECRET),
            Some(expected),
        )
    };
    let with_grant = |grant_state| {
        let mut state = state(&host);
        state.direct_grants.push(grant(&peer, grant_state));
        state
    };
    let with_member = |blocked, admission| {
        let mut state = state(&host);
        state
            .rooms
            .push(room("room-b", vec![member(&peer, blocked, admission)]));
        state
    };
    let room_hello = hello(&host, &peer, "room", "room-b", WRONG_SECRET);
    let mut offline = with_grant(DirectGrantState::Accepted);
    offline.direct_online = false;
    let mut borrowed = hello(&host, &peer, "direct", &lookup, WRONG_SECRET);
    borrowed.device_id = "someone-else".into();

    run(vec![
        direct(
            "grant waiting for reconfirmation",
            with_grant(DirectGrantState::Reconfirm),
            SessionDenial::Reconfirm,
        ),
        direct(
            "blocked grant",
            with_grant(DirectGrantState::Ignored),
            SessionDenial::Blocked,
        ),
        direct(
            "Direct off for a pinned peer",
            offline,
            SessionDenial::Offline,
        ),
        (
            "room member waiting for admission",
            with_member(true, RoomMemberAdmission::Pending),
            room_hello.clone(),
            Some(SessionDenial::RoomAdmission),
        ),
        (
            "room member blocked by the user",
            with_member(true, RoomMemberAdmission::Admitted),
            room_hello,
            Some(SessionDenial::RoomAdmission),
        ),
        // The device id of a pinned peer, claimed with another identity's
        // proof, pins nothing: the pin is device, key and node together.
        (
            "device id borrowed from a grant",
            with_grant(DirectGrantState::Reconfirm),
            borrowed,
            None,
        ),
    ]);
}

#[test]
fn zugang_task_denial_gate_keeps_the_class_for_holders_of_the_code() {
    let host = identity("host-gate-c", 65);
    let newcomer = identity("newcomer-gate-c", 66);
    let lookup = host.direct_lookup_id.clone();
    let secret = host.direct_secret();
    let mut offline = state(&host);
    offline.direct_online = false;
    // A new device that holds the current Direct code proves it with the
    // session proof; a proof with any other secret does not count.
    run(vec![
        (
            "valid proof, no grant yet",
            state(&host),
            hello(&host, &newcomer, "direct", &lookup, &secret),
            Some(SessionDenial::NotGranted),
        ),
        (
            "valid proof, Direct off",
            offline,
            hello(&host, &newcomer, "direct", &lookup, &secret),
            Some(SessionDenial::Offline),
        ),
        (
            "proof with another secret",
            state(&host),
            hello(&host, &newcomer, "direct", &lookup, WRONG_SECRET),
            None,
        ),
    ]);
}

#[test]
fn zugang_task_denial_gate_needs_the_key_to_be_the_node_for_a_class() {
    let host = identity("host-gate-d", 67);
    let lookup = host.direct_lookup_id.clone();
    let local = "gate-local";

    // A grant pinning a node whose key is another string than the node id.
    let odd = identity("odd-gate-d", 68);
    let mut odd_grant = grant(&odd, DirectGrantState::Reconfirm);
    odd_grant.public_key = "key-that-is-not-the-node".into();
    odd_grant.fingerprint = public_fingerprint(odd_grant.public_key.as_bytes());
    let mut odd_state = state(&host);
    odd_state.direct_grants.push(odd_grant.clone());
    let mut odd_hello = hello(&host, &odd, "direct", &lookup, WRONG_SECRET);
    odd_hello.public_key = odd_grant.public_key.clone();
    let auth = Arc::new(Mutex::new(odd_state));
    let error = reject(&auth, &odd_hello);
    assert_eq!(denial_of(&error), None, "{error}");
    assert!(error.to_string().contains(UNLINKED), "{error}");
    // The host books no rejection for the node and answers generically.
    let (response, _log) = host_rejection(&odd_hello, &auth, &error);
    let FsResponse::Err { msg, denial, .. } = response else {
        panic!("a rejected hello answers with an error");
    };
    assert_eq!(denial, None);
    assert_eq!(msg, "Share-Anmeldung nicht autorisiert");
    assert!(rows(local).iter().all(|row| row.node_id != odd.node_id));

    // The same grant with the key as the node keeps its class and its
    // answer under the host's name.
    let same = identity("same-gate-d", 69);
    let mut same_state = state(&host);
    same_state
        .direct_grants
        .push(grant(&same, DirectGrantState::Reconfirm));
    let same_hello = hello(&host, &same, "direct", &lookup, WRONG_SECRET);
    let auth = Arc::new(Mutex::new(same_state));
    let error = reject(&auth, &same_hello);
    assert_eq!(denial_of(&error), Some(SessionDenial::Reconfirm), "{error}");
    let (response, _log) = host_rejection(&same_hello, &auth, &error);
    let FsResponse::Err { msg, denial, .. } = response else {
        panic!("a rejected hello answers with an error");
    };
    assert_eq!(denial, Some(SessionDenial::Reconfirm));
    assert!(msg.contains(&host.device_name), "{msg}");
    clear_incoming(&same.device_id, &same.public_key);
}

#[test]
fn zugang_task_denial_gate_contact_without_device_id_lends_no_name() {
    let host = identity("host-gate-e", 70);
    let peer = identity("peer-gate-e", 71);
    let other = identity("other-gate-e", 72);
    let lookup = host.direct_lookup_id.clone();
    let from_peer = hello(&host, &peer, "direct", &lookup, WRONG_SECRET);
    let unnamed = || {
        let mut state = state(&host);
        state.direct_contacts.push(contact(&peer, None));
        state
    };
    // The peer's device id is the one of a grant for another node.
    let mut named_by_grant = unnamed();
    let mut other_grant = grant(&other, DirectGrantState::Accepted);
    other_grant.device_id = peer.device_id.clone();
    named_by_grant.direct_grants.push(other_grant);
    // ... of another contact that pins another node.
    let mut named_by_contact = unnamed();
    named_by_contact
        .direct_contacts
        .push(contact(&other, Some(&peer.device_id)));
    // ... of a room member with another node.
    let mut named_by_member = unnamed();
    let mut room_member = member(&other, false, RoomMemberAdmission::Admitted);
    room_member.device_id = peer.device_id.clone();
    named_by_member
        .rooms
        .push(room("room-e", vec![room_member]));
    // A contact that names the peer's device id needs no such check.
    let mut named = named_by_grant.clone();
    named.direct_contacts = vec![contact(&peer, Some(&peer.device_id))];
    // A contact naming another device pins nothing for this one.
    let mut other_device = state(&host);
    other_device
        .direct_contacts
        .push(contact(&peer, Some("some-other-device")));

    run(vec![
        (
            "contact without device id, device id free",
            unnamed(),
            from_peer.clone(),
            Some(SessionDenial::NotGranted),
        ),
        (
            "device id named by a grant of another node",
            named_by_grant,
            from_peer.clone(),
            None,
        ),
        (
            "device id named by a contact of another node",
            named_by_contact,
            from_peer.clone(),
            None,
        ),
        (
            "device id named by a room member of another node",
            named_by_member,
            from_peer.clone(),
            None,
        ),
        (
            "contact names the peer's device id",
            named,
            from_peer.clone(),
            Some(SessionDenial::NotGranted),
        ),
        (
            "contact names another device id",
            other_device,
            from_peer,
            None,
        ),
    ]);
}
