use super::{authorized_device_views, decision_label, request_views, timestamp, transport_state};
use crate::share::{
    DirectAccessState, DirectContact, DirectDecisionKind, DirectDecisionState, DirectEnvelopeKind,
    DirectFailure, DirectGrant, DirectGrantState, DirectPeerIdentity, DirectRelayOutcome,
    DirectRequestDirection, DirectRequestEntry, DirectRequestId, DirectRequestRecord,
    DirectRequestRetries, DirectRetryState, ShareProfiles, ShareStatus, SignedDirectDecision,
    SignedDirectDecisionReceipt, SignedDirectRequest, SignedDirectRequestReceipt,
};

const SECRET: [u8; 32] = [42; 32];

#[test]
fn local_transport_never_calls_relay_forwarding_peer_received() {
    let mut retry = DirectRetryState::default();
    assert!(transport_state(&retry).starts_with("queued"));
    retry.attempt_count = 1;
    retry.last_attempt_at = Some(10);
    assert!(transport_state(&retry).starts_with("sent"));
    retry.relay_outcome = Some(DirectRelayOutcome::Forwarded);
    let label = transport_state(&retry);
    assert!(label.starts_with("relay_forwarded"));
    assert!(label.contains("nicht bestaetigt"));
    retry.relay_outcome = Some(DirectRelayOutcome::LegacyForwarded);
    let label = transport_state(&retry);
    assert!(label.starts_with("legacy_forwarded"));
    assert!(label.contains("keine signierte Empfangsquittung"));
    retry.last_error = Some(DirectFailure {
        code: "offline".into(),
        message: "peer offline".into(),
    });
    assert!(retry.last_error.is_some());
}

#[test]
fn decision_and_timestamp_labels_are_stable() {
    assert_eq!(decision_label(DirectDecisionState::Accepted), "accepted");
    assert_eq!(decision_label(DirectDecisionState::Revoked), "revoked");
    assert_eq!(timestamp(0), "1970-01-01 00:00:00 UTC");
}

#[test]
fn outgoing_projection_separates_relay_forwarding_from_peer_receipt() {
    let requester_key = iroh::SecretKey::from_bytes(&[21; 32]);
    let target_key = iroh::SecretKey::from_bytes(&[22; 32]);
    let target = DirectPeerIdentity::from_secret("target", "Target", &target_key);
    let request = signed_request(&requester_key, &target_key);
    let mut entry = DirectRequestEntry {
        direction: DirectRequestDirection::Outgoing,
        contact_id: Some("missing-contact".into()),
        local_lookup_id: None,
        record: DirectRequestRecord::new(request),
        request_receipt: None,
        decision: None,
        decision_receipt: None,
        retries: DirectRequestRetries::default(),
    };
    entry.retries.request.attempt_count = 1;
    entry.retries.request.relay_outcome = Some(DirectRelayOutcome::Forwarded);
    let mut profiles = ShareProfiles::default();
    profiles.direct_requests.push(entry);

    let (_, outgoing) = request_views(&profiles, 200);
    let facts = &outgoing[0].facts;
    assert!(facts
        .iter()
        .find(|fact| fact.label == "Lokaler Versand Anfrage")
        .unwrap()
        .value
        .starts_with("relay_forwarded"));
    assert!(facts
        .iter()
        .find(|fact| fact.label == "Peer-Empfang Anfrage")
        .unwrap()
        .value
        .starts_with("unconfirmed"));

    profiles.direct_requests[0].contact_id = Some("legacy-contact".into());
    profiles.direct_requests[0].retries.request.relay_outcome =
        Some(DirectRelayOutcome::LegacyForwarded);
    profiles.direct_contacts.push(DirectContact {
        id: "legacy-contact".into(),
        display_name: "Target".into(),
        lookup_id: "lookup-target".into(),
        expected_fingerprint: target.fingerprint.clone(),
        expected_node_id: target.node_id.clone(),
        remote_device_id: Some(target.device_id.clone()),
        remote_public_key: Some(target.public_key.clone()),
        auto_connect: true,
        auto_open: false,
        last_seen: Some(200),
        status: ShareStatus::Available,
        last_error: None,
        presence: None,
        access_state: DirectAccessState::Accepted,
        request_sent_at: Some(100),
        accepted_at: Some(200),
        accepted_public_key: Some(target.public_key.clone()),
    });
    let (_, outgoing) = request_views(&profiles, 201);
    assert_eq!(outgoing[0].decision, DirectDecisionState::Accepted);
    assert!(outgoing[0].can_retry);
    assert!(outgoing[0]
        .facts
        .iter()
        .find(|fact| fact.label == "Entscheidung vom Peer")
        .unwrap()
        .value
        .contains("Altclient-Beziehungsfreigabe"));
    assert_eq!(target.device_name, "Target");
}

#[test]
fn incoming_delete_stays_available_pending_and_waits_for_terminal_history() {
    let requester_key = iroh::SecretKey::from_bytes(&[21; 32]);
    let target_key = iroh::SecretKey::from_bytes(&[22; 32]);
    let request = signed_request(&requester_key, &target_key);
    let mut profiles = ShareProfiles::default();
    profiles
        .record_incoming_direct_request("lookup-target", request.clone(), 110)
        .unwrap();
    let (incoming, _) = request_views(&profiles, 111);
    assert!(incoming[0].can_decide);
    assert!(incoming[0].can_delete);

    let target = DirectPeerIdentity::from_secret("target", "Target", &target_key);
    let request_receipt = SignedDirectRequestReceipt::sign_with_nonce(
        &request,
        target.clone(),
        120,
        "request-receipt",
        None,
        &SECRET,
        &target_key,
    )
    .unwrap();
    profiles
        .record_direct_request_receipt(request_receipt)
        .unwrap();
    profiles
        .record_direct_relay_ack(
            &request.request_id,
            DirectEnvelopeKind::RequestReceipt,
            DirectRelayOutcome::Forwarded,
            121,
        )
        .unwrap();
    let decision = SignedDirectDecision::sign_with_nonce(
        &request,
        target,
        DirectDecisionKind::Rejected,
        1,
        130,
        330,
        "decision",
        None,
        &SECRET,
        &target_key,
    )
    .unwrap();
    profiles
        .record_direct_decision(decision.clone(), 130)
        .unwrap();
    let (incoming, _) = request_views(&profiles, 131);
    assert!(!incoming[0].can_decide);
    assert!(!incoming[0].can_delete);

    let receipt = SignedDirectDecisionReceipt::sign_with_nonce(
        &decision,
        140,
        "decision-receipt",
        None,
        &SECRET,
        &requester_key,
    )
    .unwrap();
    profiles.record_direct_decision_receipt(receipt).unwrap();
    let (incoming, _) = request_views(&profiles, 141);
    assert!(incoming[0].can_delete);
}

#[test]
fn authorized_card_never_links_same_device_id_with_a_different_key() {
    let requester_a = iroh::SecretKey::from_bytes(&[21; 32]);
    let requester_b = iroh::SecretKey::from_bytes(&[23; 32]);
    let target_key = iroh::SecretKey::from_bytes(&[22; 32]);
    let mut request_a = signed_request(&requester_a, &target_key);
    request_a.requester.device_id = "shared-device".into();
    let mut entry_a = DirectRequestEntry {
        direction: DirectRequestDirection::Incoming,
        contact_id: None,
        local_lookup_id: Some("lookup-target".into()),
        record: DirectRequestRecord::new(request_a),
        request_receipt: None,
        decision: None,
        decision_receipt: None,
        retries: DirectRequestRetries::default(),
    };
    entry_a.record.decision.state = DirectDecisionState::Accepted;
    entry_a.record.decision.revision = 1;
    entry_a.record.decision.changed_at = 120;
    let peer_b = DirectPeerIdentity::from_secret("shared-device", "Requester B", &requester_b);
    let mut profiles = ShareProfiles::default();
    profiles.direct_requests.push(entry_a);
    profiles.direct_grants.push(DirectGrant {
        device_id: peer_b.device_id,
        device_name: peer_b.device_name,
        public_key: peer_b.public_key,
        fingerprint: peer_b.fingerprint,
        node_id: peer_b.node_id,
        state: DirectGrantState::Accepted,
        updated_at: 130,
        exec: Default::default(),
    });

    let cards = authorized_device_views(&profiles);
    assert_eq!(cards.len(), 1);
    assert!(cards[0].accepted_request.is_none());
    let (incoming, _) = request_views(&profiles, 140);
    assert_eq!(
        incoming[0]
            .facts
            .iter()
            .find(|fact| fact.label == "Autorisierung")
            .unwrap()
            .value,
        "inactive — keine Freigabe"
    );
}

#[test]
fn incoming_identity_conflict_keeps_reject_and_delete_but_disables_accept() {
    let requester_a = iroh::SecretKey::from_bytes(&[21; 32]);
    let requester_b = iroh::SecretKey::from_bytes(&[23; 32]);
    let target_key = iroh::SecretKey::from_bytes(&[22; 32]);
    let mut profiles = ShareProfiles::default();
    for request in [
        signed_request_with_id(
            &requester_a,
            &target_key,
            "123e4567-e89b-42d3-a456-426614174000",
        ),
        signed_request_with_id(
            &requester_b,
            &target_key,
            "223e4567-e89b-42d3-a456-426614174000",
        ),
    ] {
        profiles.direct_requests.push(DirectRequestEntry {
            direction: DirectRequestDirection::Incoming,
            contact_id: None,
            local_lookup_id: Some("lookup-target".into()),
            record: DirectRequestRecord::new(request),
            request_receipt: None,
            decision: None,
            decision_receipt: None,
            retries: Default::default(),
        });
    }

    let (incoming, _) = request_views(&profiles, 101);
    assert_eq!(incoming.len(), 2);
    assert!(incoming.iter().all(|request| request.identity_conflict));
    assert!(incoming.iter().all(|request| request.can_decide));
    assert!(incoming.iter().all(|request| !request.can_accept));
    assert!(incoming.iter().all(|request| request.can_delete));
    assert!(incoming.iter().all(|request| request
        .facts
        .iter()
        .any(|fact| fact.label == "Identitaetskonflikt")));
}

#[test]
fn active_old_grant_is_named_as_the_resolution_for_a_new_identity() {
    let requester = iroh::SecretKey::from_bytes(&[21; 32]);
    let old_requester = iroh::SecretKey::from_bytes(&[23; 32]);
    let target = iroh::SecretKey::from_bytes(&[22; 32]);
    let request = signed_request(&requester, &target);
    let old_peer = DirectPeerIdentity::from_secret("requester", "Old Peer", &old_requester);
    let old_fingerprint = old_peer.fingerprint.clone();
    let mut profiles = ShareProfiles::default();
    profiles.direct_requests.push(DirectRequestEntry {
        direction: DirectRequestDirection::Incoming,
        contact_id: None,
        local_lookup_id: Some("lookup-target".into()),
        record: DirectRequestRecord::new(request),
        request_receipt: None,
        decision: None,
        decision_receipt: None,
        retries: Default::default(),
    });
    profiles.direct_grants.push(DirectGrant {
        device_id: old_peer.device_id,
        device_name: old_peer.device_name,
        public_key: old_peer.public_key,
        fingerprint: old_peer.fingerprint,
        node_id: old_peer.node_id,
        state: DirectGrantState::Accepted,
        updated_at: 100,
        exec: Default::default(),
    });

    let (incoming, _) = request_views(&profiles, 101);
    assert!(!incoming[0].can_accept);
    let conflict = incoming[0]
        .facts
        .iter()
        .find(|fact| fact.label == "Identitaetskonflikt")
        .unwrap();
    assert!(conflict.value.contains("Old Peer"));
    assert!(conflict.value.contains(&old_fingerprint));
    assert!(conflict.value.contains("widerrufen"));
}

fn signed_request(
    requester_key: &iroh::SecretKey,
    target_key: &iroh::SecretKey,
) -> SignedDirectRequest {
    signed_request_with_id(
        requester_key,
        target_key,
        "123e4567-e89b-42d3-a456-426614174000",
    )
}

fn signed_request_with_id(
    requester_key: &iroh::SecretKey,
    target_key: &iroh::SecretKey,
    request_id: &str,
) -> SignedDirectRequest {
    let target = DirectPeerIdentity::from_secret("target", "Target", target_key);
    SignedDirectRequest::sign_with_nonce(
        DirectRequestId::parse(request_id).unwrap(),
        "lookup-target",
        DirectPeerIdentity::from_secret("requester", "Requester", requester_key),
        DirectPeerIdentity::pinned_target(target.node_id, target.fingerprint),
        100,
        1_000,
        "nonce",
        None,
        &SECRET,
        requester_key,
    )
    .unwrap()
}
