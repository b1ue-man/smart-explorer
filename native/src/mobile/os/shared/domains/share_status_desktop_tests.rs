//! DSK-M1: the online rule moved to `share::device_view` (shared with the
//! se Desktop tiles); the bridge JSON of a device stays byte-identical.
use super::*;
use crate::share::{
    DirectAccessState, DirectGrant, DirectGrantState, ExecGrant, ShareStatus, LAN_PRESENCE_TTL_SECS,
};

const NOW: i64 = 1_800_000_000;

fn contact(status: ShareStatus, lan_seen_at: Option<i64>, lan: &[&str]) -> DirectContact {
    DirectContact {
        id: "contact-a".into(),
        display_name: "Laptop".into(),
        lookup_id: "lookup-a".into(),
        expected_fingerprint: "fp-a".into(),
        expected_node_id: "node-a".into(),
        remote_device_id: Some("device-a".into()),
        remote_public_key: Some("key-a".into()),
        auto_connect: true,
        auto_open: false,
        last_seen: Some(NOW - 60),
        status,
        last_error: None,
        presence: None,
        access_state: DirectAccessState::Accepted,
        request_sent_at: None,
        accepted_at: None,
        accepted_public_key: None,
        lan_candidates: lan.iter().map(|address| address.to_string()).collect(),
        lan_seen_at,
        lan_uplink: None,
        relation: Default::default(),
    }
}

fn grant(write: bool) -> DirectGrant {
    DirectGrant {
        device_id: "device-a".into(),
        device_name: "Laptop".into(),
        public_key: "key-a".into(),
        fingerprint: "fp-a".into(),
        node_id: "node-a".into(),
        state: DirectGrantState::Accepted,
        updated_at: 1,
        exec: ExecGrant::default(),
        write,
    }
}

fn every_status() -> [ShareStatus; 10] {
    [
        ShareStatus::Offline,
        ShareStatus::Waiting,
        ShareStatus::WaitingForAccess,
        ShareStatus::Available,
        ShareStatus::Connecting,
        ShareStatus::Connected,
        ShareStatus::ConnectedDirect,
        ShareStatus::ConnectedRelay,
        ShareStatus::Failed("Zeitüberschreitung".into()),
        ShareStatus::IdentityConflict,
    ]
}

/// v0.5.182: `online = reachable || lan` with these four reachable states.
fn legacy_online(status: &ShareStatus, lan: bool) -> bool {
    matches!(
        status,
        ShareStatus::Available
            | ShareStatus::Connected
            | ShareStatus::ConnectedDirect
            | ShareStatus::ConnectedRelay
    ) || lan
}

/// v0.5.182 `status_code`.
fn legacy_code(status: &ShareStatus) -> &'static str {
    match status {
        ShareStatus::Offline => "offline",
        ShareStatus::Waiting => "waiting",
        ShareStatus::WaitingForAccess => "waitingForAccess",
        ShareStatus::Available => "available",
        ShareStatus::Connecting => "connecting",
        ShareStatus::Connected => "connected",
        ShareStatus::ConnectedDirect => "connectedDirect",
        ShareStatus::ConnectedRelay => "connectedRelay",
        ShareStatus::Failed(_) => "failed",
        ShareStatus::IdentityConflict => "identityConflict",
    }
}

/// The device object of v0.5.182, with its own formulas, plus the pinned
/// identity fields `deviceId` and `publicKey` (c5d3e899, `device_json`).
fn legacy_device_json(profiles: &ShareProfiles, contact: &DirectContact, lan: bool) -> Value {
    let target = PeerOpenTarget::Direct {
        contact_id: contact.id.clone(),
    };
    json!({
        "contactId": contact.id,
        "deviceId": contact.remote_device_id,
        "publicKey": contact.remote_public_key,
        "name": contact.display_name,
        "status": legacy_code(&contact.status),
        "statusText": contact.status.label(),
        "online": legacy_online(&contact.status, lan),
        "location": target.endpoint_prefix(),
        "lan": lan,
        "shareBack": contact.relation.share_back,
        "write": policy_status::contact_write(profiles, contact),
        "autoConnect": contact.auto_connect,
        "autoOpen": contact.auto_open,
    })
}

#[test]
fn desktop_task_online_rule_matches_bridge() {
    let fresh = Some(NOW);
    let expired = Some(NOW - LAN_PRESENCE_TTL_SECS - 1);
    let lan_cases: [(Option<i64>, &[&str], bool); 4] = [
        (None, &["192.0.2.7:4433"], false),
        (fresh, &["192.0.2.7:4433"], true),
        (expired, &["192.0.2.7:4433"], false),
        (fresh, &[], false),
    ];
    for write in [None, Some(true), Some(false)] {
        let profiles = ShareProfiles {
            direct_grants: write.map(grant).into_iter().collect(),
            ..ShareProfiles::default()
        };
        for status in every_status() {
            for (seen, addresses, lan) in lan_cases {
                let contact = contact(status.clone(), seen, addresses);
                let current = device_json(&profiles, &contact, NOW);
                let before = legacy_device_json(&profiles, &contact, lan);
                // Same keys in the same order: the text is byte-identical.
                assert_eq!(
                    serde_json::to_string(&current).unwrap(),
                    serde_json::to_string(&before).unwrap()
                );
                assert_eq!(
                    device_view::online(&contact, NOW),
                    legacy_online(&status, lan)
                );
                assert_eq!(device_view::seen_on_lan(&contact, NOW), lan);
                assert_eq!(device_view::status_code(&status), legacy_code(&status));
                // The tiles read `write` from `device_view`; it agrees with the bridge.
                let bridge_write = policy_status::contact_write(&profiles, &contact);
                assert_eq!(
                    device_view::granted_write(&profiles, &contact),
                    bridge_write
                );
                assert_eq!(bridge_write, write);
            }
        }
    }
}

#[test]
fn desktop_task_share_status_device_shape_unchanged() {
    let profiles = ShareProfiles::default();
    let device = device_json(&profiles, &contact(ShareStatus::Available, None, &[]), NOW);
    let mut keys: Vec<&str> = device
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "autoConnect",
            "autoOpen",
            "contactId",
            "deviceId",
            "lan",
            "location",
            "name",
            "online",
            "publicKey",
            "shareBack",
            "status",
            "statusText",
            "write",
        ]
    );
    assert_eq!(device["location"], "share://direct/contact-a");
    assert_eq!(device["status"], "available");
    assert_eq!(device["statusText"], "Online");
    assert_eq!(device["online"], Value::Bool(true));
    assert_eq!(device["write"], Value::Null);
}
