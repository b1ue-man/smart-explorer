//! E43 (L2): the machine login target crosses the Share session. The host
//! answers `FsRequest::LoginTarget` with `host_login_target`, the answer
//! travels as `FsResponse::LoginTarget`, and the viewer takes it with
//! `login_answer` into its book, bound to the host's device ID and Share
//! key. Host and viewer are two parties with their own identities; every
//! answer passes through its serialized frame like on the session.
use super::target_store::{Change, LoginTargetBook};
use super::target_wire::{
    host_message, HostFacts, LoginTargetMessage, SessionPeer, WithdrawReason, LOGIN_TARGET_FEATURE,
};
use super::{asks_login_target, host_login_target, login_answer, removed_contact_device};
use crate::login::core::{
    gateway_types::GatewayTarget,
    model::VERSION,
    self_grants::{SelfReply, SELF_FORMAT},
};
use crate::share::types::PeerPresence;
use crate::share::wire::{FsHostFeatures, FsRequest, FsResponse};
use iroh::SecretKey;

const HOST: &str = "login-ziel-weg-host";
const VIEWER: &str = "login-ziel-weg-viewer";

fn key(byte: u8) -> String {
    SecretKey::from_bytes(&[byte; 32]).public().to_string()
}

fn machine(byte: u8) -> GatewayTarget {
    GatewayTarget {
        format: VERSION,
        machine_id: format!("machine-{byte}"),
        public_key: key(byte),
        direct_addresses: vec!["192.0.2.20:4433".parse().expect("address")],
        relay_urls: vec!["https://relay.example.test/".into()],
        use_default_relays: false,
        allow_plaintext_relays: false,
    }
}

/// The viewer's pinned session to the host `device` with Share key `byte`.
fn session_to(device: &str, byte: u8) -> SessionPeer {
    SessionPeer {
        device_id: device.into(),
        public_key: key(byte),
        node_id: key(byte.wrapping_add(100)),
        device_name: format!("Gerät {device}"),
        relation_id: format!("contact-{device}"),
        pinned: true,
    }
}

/// The host broker's view: its gateway runs and holds a grant for `granted`.
fn broker(granted: &[&str], target: &GatewayTarget) -> SelfReply {
    SelfReply {
        format: SELF_FORMAT,
        state: "ok".into(),
        reason: None,
        release: "test".into(),
        account: "1000".into(),
        policy_revision: 1,
        own_grants: granted.iter().map(|device| device.to_string()).collect(),
        other_grants: 0,
        target: Some(target.clone()),
    }
}

/// What the host's dispatcher sends and the viewer's fetch reads.
fn over_the_session(message: LoginTargetMessage) -> LoginTargetMessage {
    let frame = serde_json::to_vec(&FsResponse::LoginTarget { message }).expect("encoded");
    let response: FsResponse = serde_json::from_slice(&frame).expect("decoded");
    login_answer(response).expect("a login target answer")
}

#[test]
fn login_ziel_task_target_arrives_bound_to_device_and_key() {
    let gateway = machine(31);
    let grants = vec![VIEWER.to_string()];
    let offered = host_message(&HostFacts {
        host_device_id: HOST,
        device_id: VIEWER,
        steering: true,
        account: Some("1000"),
        own_grants: &grants,
        target: Some(&gateway),
    });
    let message = over_the_session(offered);
    let mut book = LoginTargetBook::default();
    assert_eq!(
        book.apply(&session_to(HOST, 21), &message, 100),
        Ok(Change::Stored)
    );
    let stored = book.bound(HOST, &key(21)).expect("bound to device and key");
    assert_eq!(
        (&stored.target, stored.target_user.as_str()),
        (&gateway, "1000")
    );
    assert!(book.bound(HOST, &key(22)).is_none(), "another Share key");
    assert!(book.bound(VIEWER, &key(21)).is_none(), "another device");
    // An unpinned session changes nothing.
    let mut unpinned = session_to(HOST, 21);
    unpinned.pinned = false;
    let mut fresh = LoginTargetBook::default();
    assert!(fresh.apply(&unpinned, &message, 101).is_err());
    assert!(fresh.targets().is_empty());
    // Only the answer to this request is taken.
    assert!(login_answer(FsResponse::Ok).is_err());
    let refused = FsResponse::Err {
        kind: None,
        msg: "Share-Anfrage nicht autorisiert".into(),
        denial: None,
    };
    let error = login_answer(refused).expect_err("host error");
    assert_eq!(error.to_string(), "Share-Anfrage nicht autorisiert");
}

#[test]
fn login_ziel_task_without_steering_answers_without_target() {
    let gateway = machine(32);
    // The device never steers this host: the answer names no target, even
    // when the broker would hold a grant for it.
    let asking = "login-ziel-weg-device-without-steering";
    let answer = host_login_target(HOST, asking, &key(23), Ok(broker(&[asking], &gateway)));
    let frame = serde_json::to_string(&FsResponse::LoginTarget {
        message: answer.clone(),
    })
    .expect("encoded");
    assert!(!frame.contains("target_user") && !frame.contains(&gateway.public_key));
    let withdrawn = |reason| LoginTargetMessage::Withdrawn {
        format: 1,
        device_id: HOST.into(),
        reason,
    };
    assert_eq!(answer, withdrawn(WithdrawReason::NoSteering));
    // Steering without a grant of the gateway: `not_granted`, no target.
    let none: Vec<String> = Vec::new();
    let not_granted = host_message(&HostFacts {
        host_device_id: HOST,
        device_id: VIEWER,
        steering: true,
        account: Some("1000"),
        own_grants: &none,
        target: Some(&gateway),
    });
    assert_eq!(not_granted, withdrawn(WithdrawReason::NotGranted));
    // Either withdrawal deletes the target the viewer held.
    for reason in [WithdrawReason::NoSteering, WithdrawReason::NotGranted] {
        let mut book = LoginTargetBook::default();
        let offer = LoginTargetMessage::Offer {
            format: 1,
            device_id: HOST.into(),
            target_user: "1000".into(),
            target: gateway.clone(),
        };
        book.apply(&session_to(HOST, 24), &offer, 10)
            .expect("stored");
        let message = over_the_session(withdrawn(reason));
        assert_eq!(
            book.apply(&session_to(HOST, 24), &message, 11),
            Ok(Change::Removed(reason))
        );
        assert!(book.bound(HOST, &key(24)).is_none());
    }
}

#[test]
fn login_ziel_task_peer_without_feature_is_a_legacy_peer() {
    let current = FsHostFeatures::host();
    assert!(asks_login_target(&current));
    assert!(current.names().contains(&LOGIN_TARGET_FEATURE));
    let encoded = serde_json::to_string(&current).expect("encoded");
    assert!(encoded.contains(r#""login_target_v1":true"#), "{encoded}");
    // A host of an older version names no such feature: no request at all.
    let older: FsHostFeatures =
        serde_json::from_str(r#"{"remove_tree_v1":true,"code_renewal_v1":true}"#)
            .expect("older features");
    assert!(!asks_login_target(&older));
    assert_eq!(
        serde_json::to_value(FsRequest::LoginTarget).expect("request")["op"],
        "login_target"
    );
    assert!(!FsRequest::LoginTarget.mutates_filesystem());
    // The viewer notes the older host; every surface names the reason.
    let mut book = LoginTargetBook::default();
    assert!(book.note_legacy(&session_to(HOST, 25), 10));
    let error = book.resolve(HOST).expect_err("legacy peer");
    assert_eq!(error.code, "login_target_legacy_peer");
}

#[test]
fn login_ziel_task_removed_contact_deletes_the_target() {
    let presence = PeerPresence {
        kind: "direct".into(),
        relation_id: "lookup".into(),
        device_id: HOST.into(),
        device_name: "Host".into(),
        public_key: key(26),
        fingerprint: String::new(),
        node_id: key(126),
        relay_url: String::new(),
        candidates: Vec::new(),
        expires_at: 0,
        nonce: String::new(),
        proof: String::new(),
    };
    let device = |recorded: Option<&str>, presence: Option<&PeerPresence>| {
        removed_contact_device(recorded, presence)
    };
    assert_eq!(device(Some(HOST), None).as_deref(), Some(HOST));
    assert_eq!(device(None, Some(&presence)).as_deref(), Some(HOST));
    assert_eq!(device(Some(""), Some(&presence)).as_deref(), Some(HOST));
    assert_eq!(device(None, None), None);
    // The removed contact's target goes; another host's target stays.
    let mut book = LoginTargetBook::default();
    for (host, byte, gateway) in [(HOST, 26, 33), ("login-ziel-weg-other", 27, 34)] {
        let offer = LoginTargetMessage::Offer {
            format: 1,
            device_id: host.into(),
            target_user: "1000".into(),
            target: machine(gateway),
        };
        book.apply(&session_to(host, byte), &offer, 10)
            .expect("stored");
    }
    let removed = device(None, Some(&presence)).expect("device of the contact");
    assert!(book.forget(&removed));
    assert!(book.bound(HOST, &key(26)).is_none());
    assert!(book.bound("login-ziel-weg-other", &key(27)).is_some());
    assert_eq!(
        book.resolve(HOST).map_err(|error| error.code),
        Err("login_target_missing")
    );
}
