//! G18 / E17 (suite-2-entdeckung): an own outgoing legacy request sent
//! before a Direct-code renewal stays confirmable while its old lookup id
//! runs out; once the old data are dropped it is refused as changed. The
//! current id behaves as before.
use zeroize::Zeroizing;

use super::*;
use crate::share::backend_capabilities::is_legacy_direct;
use crate::share::core::public_fingerprint;
use crate::share::direct_ledger::{DirectRequestRetries, DirectRetryState};
use crate::share::direct_lifecycle::DirectRequestRecord;
use crate::share::direct_protocol::{DirectRequestId, SignedDirectRequest};
use crate::share::identity::code_renewal::CodeRenewalStatus;
use crate::share::identity::code_renewal_registry::{install, RetiringMaterial};
use crate::share::power::test_support::{auth_state, identity};
use crate::share::types::{PeerPresence, ShareStatus};

const OLD_LOOKUP: &str = "old-lookup-of-the-renewal";

struct Rig {
    local: ShareIdentity,
    state: ShareAuthState,
    target: PeerOpenTarget,
    endpoint: PeerEndpoint,
    contact: DirectContact,
    entry: DirectRequestEntry,
}

/// A pending outgoing legacy request toward an accepted unsigned peer,
/// recorded with `local_lookup_id`.
fn rig(local_lookup_id: Option<&str>) -> Rig {
    let local = identity("legacy probe local").expect("identity");
    let peer_key = iroh::SecretKey::from_bytes(&[51; 32]);
    let peer_node = peer_key.public().to_string();
    let fingerprint = public_fingerprint(peer_node.as_bytes());
    let relation_secret = vec![17u8; 32];
    let contact = DirectContact {
        id: "contact-legacy".into(),
        display_name: "Legacy peer".into(),
        lookup_id: "peer-lookup".into(),
        expected_fingerprint: fingerprint.clone(),
        expected_node_id: peer_node.clone(),
        remote_device_id: Some("legacy-peer".into()),
        remote_public_key: Some(peer_node.clone()),
        auto_connect: false,
        auto_open: false,
        last_seen: None,
        status: ShareStatus::Waiting,
        last_error: None,
        presence: None,
        access_state: DirectAccessState::Accepted,
        request_sent_at: None,
        accepted_at: Some(1),
        accepted_public_key: Some(peer_node.clone()),
        lan_candidates: Vec::new(),
        lan_seen_at: None,
        lan_uplink: None,
        relation: Default::default(),
    };
    let presence = PeerPresence {
        kind: "direct".into(),
        relation_id: "peer-lookup".into(),
        device_id: "legacy-peer".into(),
        device_name: "Legacy peer".into(),
        public_key: peer_node.clone(),
        fingerprint: fingerprint.clone(),
        node_id: peer_node.clone(),
        relay_url: String::new(),
        candidates: Vec::new(),
        expires_at: now_secs() + 300,
        nonce: "unsigned-nonce".into(),
        proof: String::new(),
    };
    let endpoint = PeerEndpoint {
        label: "legacy peer".into(),
        scope: ShareScope::Direct {
            contact_id: contact.id.clone(),
        },
        presence,
        relation_secret: relation_secret.clone(),
        expected_node_id: Some(peer_node.clone()),
    };
    let now = now_secs();
    let request = SignedDirectRequest::sign_with_nonce(
        DirectRequestId::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
        "peer-lookup",
        DirectPeerIdentity::from_secret(
            local.device_id.clone(),
            local.device_name.clone(),
            &local.iroh_secret,
        ),
        DirectPeerIdentity::pinned_target(peer_node, fingerprint),
        now,
        now + 300,
        "request-nonce",
        None,
        &relation_secret,
        &local.iroh_secret,
    )
    .expect("signed request");
    let entry = DirectRequestEntry {
        direction: DirectRequestDirection::Outgoing,
        contact_id: Some(contact.id.clone()),
        local_lookup_id: local_lookup_id.map(str::to_string),
        record: DirectRequestRecord::new(request),
        request_receipt: None,
        decision: None,
        decision_receipt: None,
        retries: DirectRequestRetries {
            request: DirectRetryState {
                relay_outcome: Some(DirectRelayOutcome::LegacyForwarded),
                ..Default::default()
            },
            ..Default::default()
        },
    };
    let mut state = auth_state(&local);
    state.direct_contacts.push(contact.clone());
    state.direct_requests.push(entry.clone());
    Rig {
        target: PeerOpenTarget::Direct {
            contact_id: contact.id.clone(),
        },
        local,
        state,
        endpoint,
        contact,
        entry,
    }
}

/// The renewal view of the local node: `retiring` runs out.
fn renewal(rig: &Rig, retiring: &[&str]) {
    install(
        &rig.local.node_id,
        &rig.local.direct_lookup_id,
        retiring
            .iter()
            .map(|lookup| RetiringMaterial {
                lookup_id: (*lookup).to_string(),
                secret: Zeroizing::new(vec![23u8; 32]),
                pending: Vec::new(),
            })
            .collect(),
        Vec::new(),
        CodeRenewalStatus::default(),
    );
}

fn probe(rig: &Rig) -> PendingLegacyProbe {
    PendingLegacyProbe {
        identity: rig.local.clone(),
        endpoint: rig.endpoint.clone(),
        contact: rig.contact.clone(),
        request: rig.entry.clone(),
    }
}

#[test]
fn entdeckung_task_a_request_sent_before_the_renewal_stays_confirmable_until_the_old_data_go() {
    let rig = rig(Some(OLD_LOOKUP));
    let probe = probe(&rig);
    renewal(&rig, &[OLD_LOOKUP]);
    assert!(own_lookup(&rig.local, OLD_LOOKUP));
    probe
        .check_request(&rig.entry)
        .expect("the request of the running renewal still binds");
    assert!(is_legacy_direct(&rig.state, &rig.target, &rig.endpoint).expect("classified"));

    // The old data were dropped: the request no longer matches this device.
    renewal(&rig, &[]);
    assert!(!own_lookup(&rig.local, OLD_LOOKUP));
    assert_eq!(probe.check_request(&rig.entry).unwrap_err(), CHANGED);
    assert!(!is_legacy_direct(&rig.state, &rig.target, &rig.endpoint).expect("classified"));
}

#[test]
fn entdeckung_task_a_request_under_the_current_id_or_another_device_id_behaves_as_before() {
    // The current id never needs a renewal view.
    let current = rig(None);
    let current_id = current.local.direct_lookup_id.clone();
    let mut entry = current.entry.clone();
    entry.local_lookup_id = Some(current_id);
    let mut state = current.state.clone();
    state.direct_requests = vec![entry.clone()];
    let mut bound = probe(&current);
    bound.request = entry.clone();
    bound.check_request(&entry).expect("current id");
    assert!(is_legacy_direct(&state, &current.target, &current.endpoint).expect("classified"));

    // An id that is neither current nor running out is still refused.
    let other = rig(Some("a-lookup-that-never-was-ours"));
    renewal(&other, &[OLD_LOOKUP]);
    assert_eq!(
        probe(&other).check_request(&other.entry).unwrap_err(),
        CHANGED
    );
    assert!(!is_legacy_direct(&other.state, &other.target, &other.endpoint).expect("classified"));
}
