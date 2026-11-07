//! R-ZUGANG-03 (c), G15 (suite-2-entdeckung): an incoming rejection names a
//! device and links its contact only through a relation that pins the
//! TLS-bound node of the same device; a claimed device id names nobody. Known
//! devices keep their name and class.
use std::sync::{Arc, Mutex};

use super::*;
use crate::share::exec_policy::ExecGrant;
use crate::share::power::test_support::{auth_state, identity};
use crate::share::session::denial::denied;
use crate::share::types::{
    DirectAccessState, DirectContact, DirectGrant, DirectGrantState, ShareStatus,
};
use crate::share::wire::PeerHello;

fn clear_all_incoming() {
    for row in peer_denials::rows("entdeckung_task_notices_clean") {
        if row.direction == "incoming" {
            peer_denials::clear_incoming(
                &row.device_id,
                row.public_key.as_deref().unwrap_or_default(),
            );
        }
    }
}

/// The rejection book is process-wide: empty before and after the test.
struct CleanBook;

impl CleanBook {
    fn new() -> Self {
        clear_all_incoming();
        Self
    }
}

impl Drop for CleanBook {
    fn drop(&mut self) {
        clear_all_incoming();
    }
}

fn hello(device_id: &str, node: &str) -> PeerHello {
    PeerHello {
        protocol_version: 3,
        relation_kind: "direct".into(),
        relation_id: "lookup".into(),
        device_id: device_id.into(),
        public_key: node.into(),
        node_id: node.into(),
        session_nonce: String::new(),
        session_proof: String::new(),
        requested_capabilities: Vec::new(),
    }
}

fn contact(id: &str, name: &str, device_id: &str, node: &str) -> DirectContact {
    DirectContact {
        id: id.into(),
        display_name: name.into(),
        lookup_id: format!("lookup-{id}"),
        expected_fingerprint: "fp".into(),
        expected_node_id: node.into(),
        remote_device_id: Some(device_id.into()),
        remote_public_key: Some(node.into()),
        auto_connect: false,
        auto_open: false,
        last_seen: None,
        status: ShareStatus::Offline,
        last_error: None,
        presence: None,
        access_state: DirectAccessState::Accepted,
        request_sent_at: None,
        accepted_at: Some(1),
        accepted_public_key: Some(node.into()),
        lan_candidates: Vec::new(),
        lan_seen_at: None,
        lan_uplink: None,
        relation: Default::default(),
    }
}

fn reject(
    auth: &Arc<Mutex<ShareAuthState>>,
    hello: &PeerHello,
    denial: SessionDenial,
) -> std::io::Error {
    let error = denied(denial, "rejected for the test");
    let (_response, log) = peer_denials::host_rejection(hello, auth, &error);
    log
}

fn incoming_notice(auth: &Arc<Mutex<ShareAuthState>>, node: &str, device_id: &str) -> AccessNotice {
    let state = auth.lock().expect("auth");
    let rows = peer_denials::rows(&state.identity.node_id);
    assert!(
        rows.iter()
            .any(|row| row.node_id == node && row.device_id == device_id),
        "the rejection of {node} is recorded"
    );
    notices(&state)
        .into_iter()
        .find(|notice| notice.direction == "incoming" && notice.device_id == device_id)
        .expect("a notice for the rejected device")
}

#[test]
fn entdeckung_task_an_unpinned_node_with_a_contacts_device_id_is_shown_as_unknown() {
    let _book = CleanBook::new();
    let host = identity("notice host").expect("identity");
    let mut state = auth_state(&host);
    let contact_node = "g15c-contact-node-abcdef";
    state.direct_contacts.push(contact(
        "contact-x",
        "Anna Laptop",
        "x-device",
        contact_node,
    ));
    let auth = Arc::new(Mutex::new(state));

    // An intruder holding a Direct code claims the contact's device id from
    // a node of its own.
    let intruder = "g15c-intruder-node-0123456789";
    let log = reject(
        &auth,
        &hello("x-device", intruder),
        SessionDenial::NotGranted,
    );
    assert!(
        !log.to_string().contains("Anna Laptop"),
        "the host log names nobody: {log}"
    );
    let notice = incoming_notice(&auth, intruder, "x-device");
    assert_eq!(notice.device_name, None);
    assert_eq!(notice.contact_id, None);
    assert!(
        notice
            .message
            .starts_with("Unbekanntes Gerät (g15c-int…) möchte"),
        "{}",
        notice.message
    );
    assert!(!notice.message.contains("Anna Laptop") && !notice.remedy.contains("Anna Laptop"));

    // The contact's own node keeps its name and its contact link.
    reject(
        &auth,
        &hello("x-device", contact_node),
        SessionDenial::NotGranted,
    );
    let state = auth.lock().expect("auth");
    let known = notices(&state)
        .into_iter()
        .find(|notice| {
            notice.direction == "incoming" && notice.device_name.as_deref() == Some("Anna Laptop")
        })
        .expect("the contact's own rejection");
    assert_eq!(known.contact_id.as_deref(), Some("contact-x"));
    assert!(known.message.starts_with("Anna Laptop möchte"));
}

#[test]
fn entdeckung_task_a_device_with_an_ignored_grant_keeps_its_name_and_class_blocked() {
    let _book = CleanBook::new();
    let host = identity("notice host two").expect("identity");
    let mut state = auth_state(&host);
    let node = "g15e-blocked-node-abcdef";
    state.direct_grants.push(DirectGrant {
        device_id: "blocked-device".into(),
        device_name: "Bernd Handy".into(),
        public_key: node.into(),
        fingerprint: "fp".into(),
        node_id: node.into(),
        state: DirectGrantState::Ignored,
        updated_at: 1,
        exec: ExecGrant::default(),
        write: false,
    });
    let auth = Arc::new(Mutex::new(state));
    reject(
        &auth,
        &hello("blocked-device", node),
        SessionDenial::Blocked,
    );
    let notice = incoming_notice(&auth, node, "blocked-device");
    assert_eq!(notice.device_name.as_deref(), Some("Bernd Handy"));
    assert_eq!(notice.reason, "blocked");
    assert_eq!(
        notice.message,
        "Bernd Handy möchte zugreifen, ist hier aber gesperrt"
    );
}
