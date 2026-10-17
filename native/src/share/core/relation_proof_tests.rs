//! The relation proof (V7, E point 2) checks what the Exec handshake checked
//! before, in the same order and with the same outcome classes, and its
//! transcript for Exec is byte for byte the one of 0.5.182.
use super::*;
use crate::share::core::{hmac_proof, public_fingerprint};
use crate::share::exec_auth::{
    authorize_client_hello_in, relation_client, relation_server, transcript, ExecRejection,
};
use crate::share::exec_policy::ExecGrant;
use crate::share::exec_protocol::{ExecClientHello, ExecServerHello};
use crate::share::exec_types::{EXEC_CAPABILITY, EXEC_PROTOCOL_VERSION};
use crate::share::identity::ShareIdentity;
use crate::share::types::{DirectGrant, RoomMember, RoomMemberFlags, RoomPolicy, RoomProfile};

const NOW: i64 = 1_760_000_000;
const SECRET: &[u8] = b"direct-secret";

fn identity(device_id: &str, name: &str, seed: u8) -> ShareIdentity {
    let secret = iroh::SecretKey::from_bytes(&[seed; 32]);
    let node_id = secret.public().to_string();
    ShareIdentity {
        device_id: device_id.into(),
        device_name: name.into(),
        direct_lookup_id: format!("lookup-{device_id}"),
        public_key: node_id.clone(),
        fingerprint: public_fingerprint(node_id.as_bytes()),
        node_id,
        iroh_secret: secret,
        direct_secret: [seed; 32],
    }
}

fn enabled() -> ExecGrant {
    ExecGrant {
        enabled: true,
        policy_revision: 4,
        ..ExecGrant::default()
    }
}

fn state(host: &ShareIdentity, client: &ShareIdentity) -> ShareAuthState {
    ShareAuthState {
        identity: host.clone(),
        direct_secret: SECRET.to_vec(),
        default_direct_exports: Default::default(),
        direct_contacts: Vec::new(),
        direct_grants: vec![DirectGrant {
            device_id: client.device_id.clone(),
            device_name: client.device_name.clone(),
            public_key: client.public_key.clone(),
            fingerprint: client.fingerprint.clone(),
            node_id: client.node_id.clone(),
            state: DirectGrantState::Accepted,
            updated_at: 1,
            exec: enabled(),
            write: false,
        }],
        rooms: Vec::new(),
        direct_requests: Vec::new(),
        direct_request_tombstones: Vec::new(),
        seen_nonces: Default::default(),
        direct_online: true,
        authorization_epoch: 2,
    }
}

fn hellos(
    host: &ShareIdentity,
    client: &ShareIdentity,
    relation_id: &str,
) -> (ExecServerHello, ExecClientHello) {
    let server = ExecServerHello::new(
        "challenge-0123456789-abcdef".into(),
        host.device_id.clone(),
        host.public_key.clone(),
        host.fingerprint.clone(),
        host.node_id.clone(),
    );
    let mut hello = ExecClientHello {
        protocol_version: EXEC_PROTOCOL_VERSION,
        capability: EXEC_CAPABILITY.into(),
        relation_kind: "direct".into(),
        relation_id: relation_id.into(),
        device_id: client.device_id.clone(),
        device_name: client.device_name.clone(),
        public_key: client.public_key.clone(),
        fingerprint: client.fingerprint.clone(),
        node_id: client.node_id.clone(),
        client_nonce: "client-nonce-0123456789abc".into(),
        proof: String::new(),
        client_features: Vec::new(),
    };
    hello.proof = hmac_proof(SECRET, &transcript(&server, &hello));
    (server, hello)
}

/// Both outcomes as Exec classes: the proof alone, and the Exec handshake.
fn outcomes(
    server: &ExecServerHello,
    hello: &ExecClientHello,
    remote_node: &str,
    state: &ShareAuthState,
) -> (Result<(), ExecRejection>, Result<(), ExecRejection>) {
    let proof = prove_relation(
        &relation_server(server),
        &relation_client(hello),
        remote_node,
        state,
    )
    .map(|_| ())
    .map_err(ExecRejection::from);
    let exec = authorize_client_hello_in(server, hello, remote_node, state, NOW).map(|_| ());
    (proof, exec)
}

#[test]
fn perm_task_relation_proof_matches_exec_auth() {
    let host = identity("host", "Host", 3);
    let client = identity("client", "Client", 5);
    let state = state(&host, &client);
    let (server, hello) = hellos(&host, &client, &host.direct_lookup_id);

    // The transcript of 0.5.182, written out; the client features are not in it.
    let expected = format!(
        "smart-explorer/share-exec/v2|{}|{}|direct|{}|{}|{}|{}|{}|{}|{}|{}|{}|client-to-server|exec_stream_v2",
        server.challenge,
        hello.client_nonce,
        host.direct_lookup_id,
        client.device_id,
        client.public_key,
        client.fingerprint,
        client.node_id,
        host.device_id,
        host.public_key,
        host.fingerprint,
        host.node_id,
    );
    assert_eq!(transcript(&server, &hello), expected);
    let mut featured = hello.clone();
    featured.client_features = vec!["exec_approval_v1".into(), "exec_access_v1".into()];
    assert_eq!(transcript(&server, &featured), expected);
    assert_eq!(
        relation_transcript(&relation_server(&server), &relation_client(&hello)),
        expected
    );

    // Accepted: the proven principal is the claimed exact identity.
    let proven = prove_relation(
        &relation_server(&server),
        &relation_client(&hello),
        &client.node_id,
        &state,
    )
    .unwrap();
    assert_eq!(proven, ProvenPrincipal::from(&relation_client(&hello)));
    let authorized =
        authorize_client_hello_in(&server, &hello, &client.node_id, &state, NOW).unwrap();
    assert_eq!(ExecPrincipal::from(&proven), authorized.principal);
    assert_eq!(authorized.authorization.policy_revision, 4);

    let auth = ExecRejection::Authentication;
    let off = ExecRejection::NotAuthorized;
    let mut wrong_proof = hello.clone();
    wrong_proof.proof = hmac_proof(b"other-secret", &transcript(&server, &wrong_proof));
    let mut short_nonce = hello.clone();
    short_nonce.client_nonce = "short".into();
    short_nonce.proof = hmac_proof(SECRET, &transcript(&server, &short_nonce));
    let mut other_capability = hello.clone();
    other_capability.capability = "exec_stream_v1".into();
    let mut other_kind = hello.clone();
    other_kind.relation_kind = "contact".into();
    let mut unknown_device = state.clone();
    unknown_device.direct_grants.clear();
    let mut offline = state.clone();
    offline.direct_online = false;
    let cases: Vec<(
        &str,
        &ExecClientHello,
        &str,
        &ShareAuthState,
        Result<(), ExecRejection>,
    )> = vec![
        ("valid", &hello, &client.node_id, &state, Ok(())),
        (
            "wrong proof",
            &wrong_proof,
            &client.node_id,
            &state,
            Err(auth),
        ),
        (
            "short nonce",
            &short_nonce,
            &client.node_id,
            &state,
            Err(auth),
        ),
        (
            "other capability",
            &other_capability,
            &client.node_id,
            &state,
            Err(auth),
        ),
        (
            "other relation kind",
            &other_kind,
            &client.node_id,
            &state,
            Err(auth),
        ),
        (
            "transport node differs",
            &hello,
            &host.node_id,
            &state,
            Err(auth),
        ),
        (
            "unknown device",
            &hello,
            &client.node_id,
            &unknown_device,
            Err(auth),
        ),
        (
            "Direct offline",
            &hello,
            &client.node_id,
            &offline,
            Err(off),
        ),
    ];
    for (name, hello, remote_node, state, expected) in cases {
        let (proof, exec) = outcomes(&server, hello, remote_node, state);
        assert_eq!(proof, expected, "{name}: relation proof");
        assert_eq!(exec, expected, "{name}: Exec handshake");
    }

    // Exec switched off: the proof holds, Exec alone refuses, and that never
    // counts as a failed authentication.
    let mut exec_off = state.clone();
    exec_off.direct_grants[0].exec = ExecGrant::default();
    let (proof, exec) = outcomes(&server, &hello, &client.node_id, &exec_off);
    assert_eq!(proof, Ok(()));
    assert_eq!(exec, Err(off));
    assert!(!off.counts_as_failed_authentication());
    assert!(RelationRejection::Authentication.counts_as_failed_authentication());
    assert!(!RelationRejection::NotAuthorized.counts_as_failed_authentication());

    // Rooms: a room that is not joined, and a member waiting for admission,
    // fail before any secret is read.
    let room_hello = |relation_id: &str| {
        let mut room = hello.clone();
        room.relation_kind = "room".into();
        room.relation_id = relation_id.into();
        room
    };
    let mut room_state = state.clone();
    room_state.rooms.push(RoomProfile {
        id: "profile-room".into(),
        name: "Room".into(),
        room_id: "room-a".into(),
        auto_join: false,
        last_seen: None,
        status: Default::default(),
        members: vec![RoomMember {
            device_id: client.device_id.clone(),
            device_name: client.device_name.clone(),
            fingerprint: client.fingerprint.clone(),
            public_key: client.public_key.clone(),
            node_id: client.node_id.clone(),
            relay_url: String::new(),
            candidates: Vec::new(),
            last_seen: None,
            status: Default::default(),
            blocked: true,
            exec: enabled(),
            presence: None,
            relation: RoomMemberFlags::default(),
        }],
        exports: Default::default(),
        policy: RoomPolicy::new_room(),
    });
    let not_joined = outcomes(&server, &room_hello("room-a"), &client.node_id, &room_state);
    assert_eq!(not_joined, (Err(auth), Err(auth)));
    room_state.rooms[0].auto_join = true;
    let blocked = outcomes(&server, &room_hello("room-a"), &client.node_id, &room_state);
    assert_eq!(blocked, (Err(auth), Err(auth)));

    // Another protocol prefix separates the proofs: a screen proof never
    // opens Exec, and the reverse.
    let screen = RelationServerHello {
        transcript_prefix: "smart-explorer/share-screen/v1|".into(),
        ..relation_server(&server)
    };
    let mut screen_client = relation_client(&hello);
    screen_client.proof = hmac_proof(SECRET, &relation_transcript(&screen, &screen_client));
    assert!(prove_relation(&screen, &screen_client, &client.node_id, &state).is_ok());
    assert_eq!(
        prove_relation(
            &relation_server(&server),
            &screen_client,
            &client.node_id,
            &state
        ),
        Err(RelationRejection::Authentication)
    );
    assert_eq!(
        prove_relation(&screen, &relation_client(&hello), &client.node_id, &state),
        Err(RelationRejection::Authentication)
    );
}
