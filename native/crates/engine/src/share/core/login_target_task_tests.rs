//! `login_target_v1` (E22): the target only from the pinned session of a
//! device that may steer, a withdrawal deletes it, an older peer gets a clear
//! message, and the viewer switches to the login view and back by itself.
use super::target_store::{
    choose_open, login_state, Change, LoginSwitch, LoginTargetBook, OpenChoice, SwitchAction,
    ViewerSurface, MAX_DEVICES,
};
use super::target_wire::{
    accept, host_message, Accepted, HostFacts, LoginTargetMessage, Refusal, SessionPeer,
    WithdrawReason, LOGIN_TARGET_FEATURE, LOGIN_TARGET_FORMAT,
};
use crate::login::core::{
    gateway_types::{GatewayTarget, MAX_NETWORK},
    model::VERSION,
};
use iroh::SecretKey;

fn key(byte: u8) -> String {
    SecretKey::from_bytes(&[byte; 32]).public().to_string()
}

fn target(byte: u8) -> GatewayTarget {
    GatewayTarget {
        format: VERSION,
        machine_id: format!("machine-{byte}"),
        public_key: key(byte),
        direct_addresses: vec!["192.0.2.10:4433".parse().expect("address")],
        relay_urls: vec!["https://relay.example.test/".into()],
        use_default_relays: false,
        allow_plaintext_relays: false,
    }
}

fn peer(device: &str, byte: u8) -> SessionPeer {
    SessionPeer {
        device_id: device.into(),
        public_key: key(byte),
        node_id: key(byte),
        device_name: format!("Gerät {device}"),
        relation_id: format!("contact-{device}"),
        pinned: true,
    }
}

fn offer(device: &str, machine: u8) -> LoginTargetMessage {
    LoginTargetMessage::Offer {
        format: LOGIN_TARGET_FORMAT,
        device_id: device.into(),
        target_user: "1000".into(),
        target: target(machine),
    }
}

fn withdrawn(device: &str, reason: WithdrawReason) -> LoginTargetMessage {
    LoginTargetMessage::Withdrawn {
        format: LOGIN_TARGET_FORMAT,
        device_id: device.into(),
        reason,
    }
}

#[test]
fn login_ziel_task_host_offers_only_with_steering_and_own_grant() {
    assert_eq!(LOGIN_TARGET_FEATURE, "login_target_v1");
    let installed = target(7);
    let grants = vec!["viewer".to_string()];
    let facts = |steering, account, target| HostFacts {
        host_device_id: "host",
        device_id: "viewer",
        steering,
        account,
        own_grants: &grants,
        target,
    };
    assert_eq!(
        host_message(&facts(false, Some("1000"), Some(&installed))),
        withdrawn("host", WithdrawReason::NoSteering)
    );
    assert_eq!(
        host_message(&facts(true, None, Some(&installed))),
        withdrawn("host", WithdrawReason::NotInstalled)
    );
    assert_eq!(
        host_message(&facts(true, Some("1000"), None)),
        withdrawn("host", WithdrawReason::NotInstalled)
    );
    let other = vec!["someone-else".to_string()];
    let ungranted = HostFacts {
        own_grants: &other,
        ..facts(true, Some("1000"), Some(&installed))
    };
    assert_eq!(
        host_message(&ungranted),
        withdrawn("host", WithdrawReason::NotGranted)
    );
    assert_eq!(
        host_message(&facts(true, Some("1000"), Some(&installed))),
        LoginTargetMessage::Offer {
            format: LOGIN_TARGET_FORMAT,
            device_id: "host".into(),
            target_user: "1000".into(),
            target: installed.clone(),
        }
    );
    // The hook asks the steering list itself: an unknown device never gets
    // an offer, whatever the broker reports.
    let broker = crate::login::core::self_grants::SelfReply::refused("1000", "test");
    let answer = super::host_login_target(
        "host",
        "login-ziel-task-device-that-never-steers",
        &key(9),
        Ok(broker),
    );
    assert_eq!(answer, withdrawn("host", WithdrawReason::NoSteering));
}

#[test]
fn login_ziel_task_viewer_accepts_only_the_pinned_session_of_the_device() {
    let session = peer("host", 1);
    let mut unpinned = session.clone();
    unpinned.pinned = false;
    assert_eq!(
        accept(&unpinned, &offer("host", 7)),
        Err(Refusal::NotPinned)
    );
    assert_eq!(
        accept(&session, &offer("third-device", 7)),
        Err(Refusal::ForeignDevice)
    );
    // A target with the device's own Share key is no machine target.
    assert_eq!(accept(&session, &offer("host", 1)), Err(Refusal::DeviceKey));
    let mut wrong_format = offer("host", 7);
    if let LoginTargetMessage::Offer { format, .. } = &mut wrong_format {
        *format = 2;
    }
    assert_eq!(accept(&session, &wrong_format), Err(Refusal::Invalid));
    assert_eq!(
        accept(&session, &offer("host", 7)),
        Ok(Accepted::Store {
            target_user: "1000".into(),
            target: target(7),
        })
    );

    let mut book = LoginTargetBook::default();
    assert_eq!(
        book.apply(&unpinned, &offer("host", 7), 10),
        Err(Refusal::NotPinned)
    );
    assert!(book.targets().is_empty());
    assert_eq!(
        book.apply(&session, &offer("host", 7), 10),
        Ok(Change::Stored)
    );
    assert_eq!(
        book.apply(&session, &offer("host", 7), 20),
        Ok(Change::Unchanged)
    );
    let stored = book
        .bound("host", &key(1))
        .expect("bound to device and key");
    assert_eq!(stored.target, target(7));
    assert_eq!(stored.target_user, "1000");
    assert!(
        book.bound("host", &key(2)).is_none(),
        "another key is another device"
    );
    // A new target only through a new message of the pinned session.
    assert_eq!(
        book.apply(&session, &offer("host", 8), 30),
        Ok(Change::Stored)
    );
    assert_eq!(book.targets().len(), 1);
    assert_eq!(book.targets()[0].target, target(8));
}

#[test]
fn login_ziel_task_revocation_deletes_the_target() {
    let session = peer("host", 1);
    let mut book = LoginTargetBook::default();
    book.apply(&session, &offer("host", 7), 10).expect("stored");
    assert_eq!(
        book.apply(&session, &withdrawn("host", WithdrawReason::NoSteering), 11),
        Ok(Change::Removed(WithdrawReason::NoSteering))
    );
    assert!(book.targets().is_empty());
    assert_eq!(
        book.resolve("host").map_err(|error| error.code),
        Err("login_target_missing")
    );
    // A withdrawal from a foreign session deletes nothing.
    book.apply(&session, &offer("host", 7), 12).expect("stored");
    let other = peer("other", 3);
    assert_eq!(
        book.apply(&other, &withdrawn("host", WithdrawReason::NoSteering), 13),
        Err(Refusal::ForeignDevice)
    );
    assert_eq!(book.targets().len(), 1);
    // Removing the contact, or a contact with a new key, deletes it too.
    assert!(book.retain(|device, public_key| device == "host" && public_key == key(2)));
    assert!(book.targets().is_empty());
    book.apply(&session, &offer("host", 7), 14).expect("stored");
    assert!(book.forget("host"));
    assert!(book.targets().is_empty());
    // The book stays bounded.
    for index in 0..=MAX_DEVICES {
        let device = format!("host-{index}");
        book.apply(&peer(&device, 1), &offer(&device, 7), index as u64)
            .expect("stored");
    }
    assert_eq!(book.targets().len(), MAX_DEVICES);
    assert!(book.resolve("host-0").is_err(), "the oldest made room");
}

#[test]
fn login_ziel_task_older_peer_gets_a_clear_message() {
    let session = peer("host", 1);
    let mut book = LoginTargetBook::default();
    book.apply(&session, &offer("host", 7), 10).expect("stored");
    assert!(book.note_legacy(&session, 11));
    let error = book.resolve("Gerät host").expect_err("legacy peer");
    assert_eq!(error.code, "login_target_legacy_peer");
    assert!(
        error.message.contains("login_target_v1"),
        "{}",
        error.message
    );
    assert!(error.to_string().starts_with("login_target_legacy_peer: "));
    let mut unpinned = session.clone();
    unpinned.pinned = false;
    assert!(!book.note_legacy(&unpinned, 12));
    // A newer version of the peer sends its target again.
    book.apply(&session, &offer("host", 7), 13).expect("stored");
    assert!(book.legacy().is_empty());
    assert!(book.resolve("contact-host").is_ok());
    // Names are unique or the ID is asked for.
    let mut twin = peer("twin", 4);
    twin.device_name = "Gerät host".into();
    book.apply(&twin, &offer("twin", 8), 14).expect("stored");
    assert_eq!(
        book.resolve("gerät host").map_err(|error| error.code),
        Err("login_target_ambiguous")
    );
    assert!(book.resolve("twin").is_ok());
}

#[test]
fn login_ziel_task_wire_stays_bounded_and_strict() {
    let message = offer("host", 7);
    let bytes = message.encode().expect("encoded");
    assert!(bytes.len() <= MAX_NETWORK);
    assert_eq!(
        LoginTargetMessage::decode(&bytes).expect("decoded"),
        message
    );
    assert!(LoginTargetMessage::decode(&vec![b' '; MAX_NETWORK + 1]).is_err());
    let mut extra: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    extra["admin"] = serde_json::json!(true);
    assert!(LoginTargetMessage::decode(&serde_json::to_vec(&extra).expect("bytes")).is_err());
    let later = br#"{"kind":"withdrawn","format":1,"device_id":"host","reason":"later_reason"}"#;
    assert_eq!(
        LoginTargetMessage::decode(later).expect("decoded"),
        withdrawn("host", WithdrawReason::Unknown)
    );
    let bad_user = LoginTargetMessage::Offer {
        format: LOGIN_TARGET_FORMAT,
        device_id: "host".into(),
        target_user: "S-1-5 21".into(),
        target: target(7),
    };
    assert!(bad_user.encode().is_err());
}

#[test]
fn login_ziel_task_switches_on_secure_desktop_and_back() {
    for state in ["secure_desktop", "host_locked", "session_inactive"] {
        assert!(login_state(state), "{state}");
    }
    for state in [
        "streaming",
        "window_minimized",
        "viewer_hidden",
        "no_display",
        "ended",
    ] {
        assert!(!login_state(state), "{state}");
    }
    let mut switch = LoginSwitch::default();
    assert_eq!(switch.observe("streaming", true), SwitchAction::Stay);
    assert_eq!(
        switch.observe("secure_desktop", true),
        SwitchAction::EnterLogin
    );
    assert_eq!(switch.surface(), ViewerSurface::Login);
    assert_eq!(switch.observe("secure_desktop", true), SwitchAction::Stay);
    assert_eq!(switch.observe("host_locked", true), SwitchAction::Stay);
    assert_eq!(switch.observe("streaming", true), SwitchAction::LeaveLogin);
    assert_eq!(switch.surface(), ViewerSurface::Screen);
    // Without a paired target the screen view stays.
    assert_eq!(switch.observe("host_locked", false), SwitchAction::Stay);
    assert_eq!(switch.observe("streaming", false), SwitchAction::Stay);
    // A failed login view is not reopened in the same pause.
    assert_eq!(
        switch.observe("session_inactive", true),
        SwitchAction::EnterLogin
    );
    assert_eq!(switch.login_failed(), SwitchAction::LeaveLogin);
    assert_eq!(switch.observe("session_inactive", true), SwitchAction::Stay);
    assert_eq!(switch.observe("streaming", true), SwitchAction::Stay);
    assert_eq!(
        switch.observe("secure_desktop", true),
        SwitchAction::EnterLogin
    );
}

#[test]
fn login_ziel_task_open_forms_keep_the_file_form() {
    let mut book = LoginTargetBook::default();
    book.apply(&peer("host", 1), &offer("host", 7), 10)
        .expect("stored");
    assert_eq!(
        choose_open(&book, "/tmp/gateway-target.json", true, true),
        Ok(OpenChoice::File)
    );
    match choose_open(&book, "Gerät host", false, false) {
        Ok(OpenChoice::Device(paired)) => assert_eq!(paired.target, target(7)),
        other => panic!("device form expected: {other:?}"),
    }
    assert_eq!(
        choose_open(&book, "gateway-target.json", false, true).map_err(|error| error.code),
        Err("machine_pin_acknowledgement_required")
    );
    assert_eq!(
        choose_open(&book, "unknown", false, false).map_err(|error| error.code),
        Err("login_target_missing")
    );
}

#[test]
fn login_ziel_task_book_survives_a_reload() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("login-targets.json");
    assert!(LoginTargetBook::load_from(&path)
        .expect("empty")
        .targets()
        .is_empty());
    let mut book = LoginTargetBook::default();
    book.apply(&peer("host", 1), &offer("host", 7), 10)
        .expect("stored");
    book.note_legacy(&peer("old", 2), 11);
    book.save_to(&path).expect("saved");
    assert_eq!(LoginTargetBook::load_from(&path).expect("loaded"), book);
    // An entry whose target carries the device key is left out on load.
    let mut text: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).expect("read")).expect("json");
    text["targets"][0]["target"]["public_key"] = serde_json::json!(key(1));
    std::fs::write(&path, serde_json::to_vec(&text).expect("bytes")).expect("written");
    let reloaded = LoginTargetBook::load_from(&path).expect("loaded");
    assert!(reloaded.targets().is_empty());
    assert_eq!(reloaded.legacy().len(), 1);
}
