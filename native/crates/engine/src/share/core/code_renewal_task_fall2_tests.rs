//! Task suite of the Direct-code renewal (E17, M21), cases 2 (a)(b) and 4
//! (b)(c): the signed presence of a running out lookup id, the scheduling of
//! the renewal exchange and the early refusal of removed devices. Each test
//! starts one local Iroh node on loopback without a relay; no peer, no
//! server and no fixed port is involved.
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::share::core::{presence_payload, public_fingerprint, verify_hmac};
use crate::share::direct_reciprocal_coordinator::{
    DirectReciprocalCoordinator, DirectRepairCandidate, DirectRepairScheduleOutcome,
};
use crate::share::identity::code_renewal_registry as registry;
use crate::share::identity::ShareIdentity;
use crate::share::node::ShareIrohNode;
use crate::share::signal_presence::{build_presence, build_retiring_presence, PresenceSignature};
use crate::share::types::{
    DirectAccessState, DirectContact, DirectGrant, DirectGrantState, PeerEndpoint, PeerPresence,
    ShareAuthState, ShareEvent, ShareScope, ShareStatus,
};

const LOOKUP: &str = "cr-task-fall2-lookup";
const WAIT: Duration = Duration::from_secs(30);

/// Empties the process view of one host when the test ends, also on a panic.
struct ResetRegistry(String);

impl Drop for ResetRegistry {
    fn drop(&mut self) {
        registry::install(&self.0, "", Vec::new(), Vec::new(), Default::default());
    }
}

fn identity(device_id: &str, seed: u8) -> ShareIdentity {
    let key = iroh::SecretKey::from_bytes(&[seed; 32]);
    let node_id = key.public().to_string();
    ShareIdentity {
        device_id: device_id.into(),
        device_name: device_id.into(),
        direct_lookup_id: format!("lookup-{device_id}"),
        public_key: node_id.clone(),
        fingerprint: public_fingerprint(node_id.as_bytes()),
        node_id,
        iroh_secret: key,
        direct_secret: [seed; 32],
    }
}

fn grant(device: &ShareIdentity) -> DirectGrant {
    DirectGrant {
        device_id: device.device_id.clone(),
        device_name: device.device_name.clone(),
        public_key: device.public_key.clone(),
        fingerprint: device.fingerprint.clone(),
        node_id: device.node_id.clone(),
        state: DirectGrantState::Accepted,
        updated_at: 1,
        exec: Default::default(),
        write: false,
    }
}

/// A started local node whose authorization epoch is 0.
fn start(
    local: &ShareIdentity,
    grants: Vec<DirectGrant>,
) -> (Arc<ShareIrohNode>, crossbeam_channel::Receiver<ShareEvent>) {
    let auth = Arc::new(Mutex::new(ShareAuthState {
        identity: local.clone(),
        direct_secret: local.direct_secret(),
        default_direct_exports: Default::default(),
        direct_contacts: Vec::new(),
        direct_grants: grants,
        rooms: Vec::new(),
        direct_requests: Vec::new(),
        direct_request_tombstones: Vec::new(),
        seen_nonces: Default::default(),
        direct_online: true,
        authorization_epoch: 0,
    }));
    let (events, receiver) = crossbeam_channel::unbounded();
    let node = ShareIrohNode::start("127.0.0.1:0", local, auth, events).unwrap();
    (node, receiver)
}

fn payload_of(presence: &PeerPresence) -> String {
    presence_payload(
        &presence.kind,
        &presence.relation_id,
        &presence.device_id,
        &presence.public_key,
        &presence.node_id,
        &presence.relay_url,
        &presence.candidates,
        presence.expires_at,
        &presence.nonce,
    )
}

#[test]
fn code_renewal_task_retiring_presence_names_the_relay_only_and_is_signed() {
    let local = identity("host", 246);
    let (node, _events) = start(&local, Vec::new());
    let alt_secret = [7u8; 32];
    let retiring =
        build_retiring_presence("cr-task-fall2-alt", &local, &alt_secret, &node).unwrap();
    // No direct address, the relay of the node (empty while none is reachable).
    assert!(retiring.candidates.is_empty());
    assert_eq!(retiring.relay_url, node.relay_url());
    assert_eq!(retiring.kind, "direct");
    assert_eq!(retiring.relation_id, "cr-task-fall2-alt");
    assert_eq!(retiring.node_id, local.node_id);
    // Signed by the device, marked as a renewal announcement, still a route
    // policy presence for older peers, below the nonce limit of old servers.
    assert_eq!(retiring.signature_state(), PresenceSignature::Valid);
    assert!(retiring.announces_code_renewal());
    assert!(retiring.supports_route_policy());
    assert!(retiring.nonce.starts_with("rp1.cr1."));
    assert!(
        retiring.nonce.len() <= 126,
        "nonce {} bytes",
        retiring.nonce.len()
    );
    // The v1 proof is made with the running out secret, so holders of the old
    // data verify it unchanged.
    assert!(verify_hmac(
        &alt_secret,
        &payload_of(&retiring),
        &retiring.proof
    ));
    assert!(!verify_hmac(
        &[8u8; 32],
        &payload_of(&retiring),
        &retiring.proof
    ));
    // A direct address added by anyone breaks the signature and with it the
    // announcement.
    let mut forged = retiring.clone();
    forged.candidates.push("127.0.0.1:1".into());
    assert_eq!(forged.signature_state(), PresenceSignature::Invalid);
    assert!(!forged.announces_code_renewal());
    // The marker alone, without the device signature, announces nothing.
    let mut unsigned = retiring.clone();
    unsigned.nonce = "rp1.cr1.abcdefghijklmnop".into();
    assert!(!unsigned.announces_code_renewal());
    // The presence of the current data carries no such marker.
    let plain = build_presence("direct", "cr-task-fall2-neu", &local, &[9u8; 32], &node).unwrap();
    assert_eq!(plain.signature_state(), PresenceSignature::Valid);
    assert!(plain.supports_route_policy());
    assert!(!plain.announces_code_renewal());
    assert!(plain.nonce.starts_with("rp1."));
    assert!(!plain.nonce.starts_with("rp1.cr1."));
}

fn contact_of(remote: &ShareIdentity) -> DirectContact {
    DirectContact {
        id: "cr-task-fall2-contact".into(),
        display_name: remote.device_name.clone(),
        lookup_id: LOOKUP.into(),
        expected_fingerprint: remote.fingerprint.clone(),
        expected_node_id: remote.node_id.clone(),
        remote_device_id: Some(remote.device_id.clone()),
        remote_public_key: Some(remote.public_key.clone()),
        auto_connect: true,
        auto_open: false,
        last_seen: None,
        status: ShareStatus::Waiting,
        last_error: None,
        presence: None,
        access_state: DirectAccessState::Accepted,
        request_sent_at: None,
        accepted_at: Some(1),
        accepted_public_key: Some(remote.public_key.clone()),
        lan_candidates: Vec::new(),
        lan_seen_at: None,
        lan_uplink: None,
        relation: Default::default(),
    }
}

/// A repair candidate for `remote` whose presence is the retiring one when
/// `renewal` is set (the host announced its renewal), else an ordinary one.
fn candidate(
    node: &ShareIrohNode,
    local: &ShareIdentity,
    remote: &ShareIdentity,
    generation: u64,
    renewal: bool,
) -> DirectRepairCandidate {
    let secret = [9u8; 32];
    let built = if renewal {
        build_retiring_presence(LOOKUP, remote, &secret, node)
    } else {
        build_presence("direct", LOOKUP, remote, &secret, node)
    };
    let contact = contact_of(remote);
    let endpoint = PeerEndpoint {
        label: remote.device_name.clone(),
        scope: ShareScope::Direct {
            contact_id: contact.id.clone(),
        },
        presence: built.unwrap(),
        relation_secret: secret.to_vec(),
        expected_node_id: Some(remote.node_id.clone()),
    };
    DirectRepairCandidate::from_accepted_contact(generation, &contact, endpoint, local.clone())
        .unwrap()
}

/// Schedules candidates until the task reports `Suppressed`: its attempt has
/// ended and the task is closed for this kind of candidate.
fn schedule_until_suppressed(
    coordinator: &DirectReciprocalCoordinator,
    mut next: impl FnMut() -> DirectRepairCandidate,
) {
    let deadline = Instant::now() + WAIT;
    loop {
        let outcome = coordinator.schedule(next()).unwrap();
        if outcome == DirectRepairScheduleOutcome::Suppressed {
            return;
        }
        assert!(Instant::now() < deadline, "the repair attempt did not end");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn code_renewal_task_announced_renewal_reopens_a_closed_exchange_once() {
    let local = identity("host-b", 247);
    let remote = identity("device-remote", 248);
    let (node, _events) = start(&local, Vec::new());
    // The node's authorization epoch is 0, the coordinator runs at epoch 1:
    // the worker ends every attempt at once with a policy denial, which closes
    // the task exactly like a completed exchange does (`blocked`).
    let (coordinator, _completions) = DirectReciprocalCoordinator::start(node.clone(), 1).unwrap();
    let plain = || candidate(&node, &local, &remote, 1, false);
    let renewing = || candidate(&node, &local, &remote, 1, true);
    assert_eq!(
        coordinator.schedule(plain()).unwrap(),
        DirectRepairScheduleOutcome::Queued
    );
    schedule_until_suppressed(&coordinator, plain);
    // Closed, and an ordinary presence does not reopen it.
    assert_eq!(
        coordinator.schedule(plain()).unwrap(),
        DirectRepairScheduleOutcome::Suppressed
    );
    // The host's renewal announcement reopens it: the task is due again.
    assert_eq!(
        coordinator.schedule(renewing()).unwrap(),
        DirectRepairScheduleOutcome::Refreshed
    );
    // The new attempt ends and closes the task again; a second announcement
    // in the same epoch is suppressed, as is an ordinary presence.
    schedule_until_suppressed(&coordinator, renewing);
    assert_eq!(
        coordinator.schedule(renewing()).unwrap(),
        DirectRepairScheduleOutcome::Suppressed
    );
    assert_eq!(
        coordinator.schedule(plain()).unwrap(),
        DirectRepairScheduleOutcome::Suppressed
    );
    coordinator.request_stop();
    assert!(!coordinator.repair_in_flight());
}

#[test]
fn code_renewal_task_withdrawn_key_is_refused_before_tls_unless_a_grant_pins_it() {
    let local = identity("host-c", 249);
    let granted = identity("device-granted", 250);
    let removed = identity("device-removed", 251);
    let stranger = identity("device-stranger", 252);
    let _reset = ResetRegistry(local.node_id.clone());
    let (node, _events) = start(&local, vec![grant(&granted)]);
    // The removed device and, by an older removal that a grant has not yet
    // replaced, the granted one are both named in the process view.
    registry::install(
        &local.node_id,
        &local.direct_lookup_id,
        Vec::new(),
        vec![removed.node_id.clone(), granted.node_id.clone()],
        Default::default(),
    );
    // The accept loop refuses an endpoint named by the relay when it is not
    // pinned by a grant, contact or room member and is withdrawn
    // (`node_accept.rs`; `relay_names_withdrawn` and `IncomingAddr` are
    // private to that file, so its two inputs are checked here).
    let refused =
        |endpoint: &str| !node.known_endpoint(endpoint) && registry::is_withdrawn(endpoint);
    assert!(registry::is_withdrawn(&removed.node_id));
    assert!(!node.known_endpoint(&removed.node_id));
    assert!(refused(&removed.node_id));
    assert!(node.known_endpoint(&granted.node_id));
    assert!(registry::is_withdrawn(&granted.node_id));
    assert!(!refused(&granted.node_id));
    // A stranger the user never removed is not refused by this rule.
    assert!(!node.known_endpoint(&stranger.node_id));
    assert!(!registry::is_withdrawn(&stranger.node_id));
    assert!(!refused(&stranger.node_id));
    assert!(!refused(""));
}
