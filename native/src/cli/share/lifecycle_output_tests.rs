use super::{authorization, direction_code, envelope_code, request_text, request_value};
use crate::share::{
    DirectAccessState, DirectContact, DirectDecisionKind, DirectEnvelopeKind, DirectGrant,
    DirectGrantState, DirectPeerIdentity, DirectRelayOutcome, DirectRequestDirection,
    DirectRequestEntry, DirectRequestId, DirectRequestRecord, DirectRequestRetries, ShareProfiles,
    ShareStatus, SignedDirectDecision, SignedDirectRequest, SignedDirectRequestReceipt,
};

#[test]
fn stable_codes_are_machine_friendly() {
    assert_eq!(direction_code(DirectRequestDirection::Outgoing), "outgoing");
    assert_eq!(
        envelope_code(DirectEnvelopeKind::DecisionReceipt),
        "decision_receipt"
    );
}

#[test]
fn incoming_authorization_requires_the_exact_tracked_request_identity() {
    let requester_secret = iroh::SecretKey::from_bytes(&[3; 32]);
    let conflicting_secret = iroh::SecretKey::from_bytes(&[4; 32]);
    let target_secret = iroh::SecretKey::from_bytes(&[7; 32]);
    let requester =
        DirectPeerIdentity::from_secret("shared-device", "Requester", &requester_secret);
    let target = DirectPeerIdentity::from_secret("target", "Target", &target_secret);
    let request = SignedDirectRequest::sign_with_nonce(
        DirectRequestId::parse("11234567-89ab-4def-8123-456789abcdef").unwrap(),
        "lookup",
        requester.clone(),
        DirectPeerIdentity::pinned_target(target.node_id.clone(), target.fingerprint.clone()),
        10,
        1_000,
        "request-nonce",
        None,
        &[9; 32],
        &requester_secret,
    )
    .unwrap();
    let decision = SignedDirectDecision::sign_with_nonce(
        &request,
        target,
        DirectDecisionKind::Accepted,
        1,
        20,
        1_000,
        "decision-nonce",
        None,
        &[9; 32],
        &target_secret,
    )
    .unwrap();
    let entry = DirectRequestEntry {
        direction: DirectRequestDirection::Incoming,
        contact_id: None,
        local_lookup_id: Some("lookup".into()),
        record: DirectRequestRecord::new(request),
        request_receipt: None,
        decision: Some(decision),
        decision_receipt: None,
        retries: DirectRequestRetries::default(),
    };
    let conflicting =
        DirectPeerIdentity::from_secret("shared-device", "Conflicting", &conflicting_secret);
    let mut profiles = ShareProfiles::default();
    profiles.direct_grants.push(grant(conflicting));
    assert!(!authorization(&entry, &profiles).0);
    profiles.direct_grants[0] = grant(requester);
    assert!(authorization(&entry, &profiles).0);
}

fn grant(peer: DirectPeerIdentity) -> DirectGrant {
    DirectGrant {
        device_id: peer.device_id,
        device_name: peer.device_name,
        public_key: peer.public_key,
        fingerprint: peer.fingerprint,
        node_id: peer.node_id,
        state: DirectGrantState::Accepted,
        updated_at: 20,
        exec: Default::default(),
    }
}

#[test]
fn output_separates_delivery_receipt_authorization_and_connectivity() {
    let requester_secret = iroh::SecretKey::from_bytes(&[3; 32]);
    let target_secret = iroh::SecretKey::from_bytes(&[7; 32]);
    let requester =
        DirectPeerIdentity::from_secret("local-device", "Local Device", &requester_secret);
    let target = DirectPeerIdentity::from_secret("remote-device", "Remote Device", &target_secret);
    let request = SignedDirectRequest::sign_with_nonce(
        DirectRequestId::parse("01234567-89ab-4def-8123-456789abcdef").unwrap(),
        "lookup",
        requester,
        DirectPeerIdentity::pinned_target(target.node_id.clone(), target.fingerprint.clone()),
        10,
        1_000,
        "request-nonce",
        None,
        &[9; 32],
        &requester_secret,
    )
    .unwrap();
    let mut profiles = ShareProfiles::default();
    profiles.direct_contacts.push(DirectContact {
        id: "contact".into(),
        display_name: "Peer".into(),
        lookup_id: "lookup".into(),
        expected_fingerprint: target.fingerprint.clone(),
        expected_node_id: target.node_id.clone(),
        remote_device_id: None,
        remote_public_key: None,
        auto_connect: true,
        auto_open: false,
        last_seen: None,
        status: ShareStatus::Offline,
        last_error: None,
        presence: None,
        access_state: DirectAccessState::Pending,
        request_sent_at: Some(10),
        accepted_at: None,
        accepted_public_key: None,
    });
    profiles
        .queue_outgoing_direct_request("contact", request.clone())
        .unwrap();
    let receipt = SignedDirectRequestReceipt::sign_with_nonce(
        &request,
        target,
        11,
        "receipt-nonce",
        None,
        &[9; 32],
        &target_secret,
    )
    .unwrap();
    profiles.record_direct_request_receipt(receipt).unwrap();

    let entry = &profiles.direct_requests[0];
    let value = request_value(entry, &profiles);
    assert_eq!(value["direction"], "outgoing");
    assert_eq!(value["delivery"]["state"], "received");
    assert_eq!(value["peer_receipt"]["request"]["state"], "received");
    assert_eq!(value["peer"]["device_name"], "Remote Device");
    assert_eq!(value["authorization"]["state"], "inactive");
    assert_eq!(value["connectivity"]["state"], "offline");
    assert!(request_text(entry, &profiles)
        .iter()
        .any(|line| line.contains("request_peer_receipt=received")));

    profiles.direct_requests[0].request_receipt = None;
    profiles.direct_requests[0].retries.request.relay_outcome =
        Some(DirectRelayOutcome::LegacyForwarded);
    profiles.direct_requests[0].retries.request.relay_changed_at = Some(12);
    profiles.direct_contacts[0].access_state = DirectAccessState::Accepted;
    let entry = &profiles.direct_requests[0];
    let value = request_value(entry, &profiles);
    assert_eq!(value["decision"]["state"], "pending");
    assert_eq!(value["decision"]["effective_state"], "accepted");
    assert_eq!(value["decision"]["evidence"], "legacy_relation");
    assert_eq!(value["authorization"]["state"], "active");
    assert_eq!(value["authorization"]["basis"], "legacy_contact_projection");
    assert!(request_text(entry, &profiles).iter().any(|line| {
        line.contains("effective_decision=accepted")
            && line.contains("decision_evidence=legacy_relation")
            && line.contains("authorization=active")
    }));
}

#[test]
fn tracked_identity_conflict_is_machine_visible_with_exact_resolution_commands() {
    let mut profiles = ShareProfiles::default();
    profiles
        .direct_requests
        .push(incoming_entry(3, "31234567-89ab-4def-8123-456789abcdef"));
    profiles
        .direct_requests
        .push(incoming_entry(4, "41234567-89ab-4def-8123-456789abcdef"));

    let entry = &profiles.direct_requests[0];
    let value = request_value(entry, &profiles);
    assert_eq!(value["identity_conflict"], true);
    let commands = value["resolution_commands"].as_array().unwrap();
    assert!(commands.iter().any(|command| {
        command.as_str() == Some("se share request reject 41234567-89ab-4def-8123-456789abcdef")
    }));
    assert!(commands.iter().any(|command| {
        command.as_str() == Some("se share request reject 31234567-89ab-4def-8123-456789abcdef")
    }));
    let lines = request_text(entry, &profiles);
    assert!(lines
        .iter()
        .any(|line| line.contains("identity_conflict=true")));
    assert!(lines.iter().any(|line| {
        line == "request_resolution\t31234567-89ab-4def-8123-456789abcdef\tse share request reject 31234567-89ab-4def-8123-456789abcdef"
    }));
}

fn incoming_entry(secret_byte: u8, request_id: &str) -> DirectRequestEntry {
    let requester_secret = iroh::SecretKey::from_bytes(&[secret_byte; 32]);
    let target_secret = iroh::SecretKey::from_bytes(&[7; 32]);
    let target = DirectPeerIdentity::from_secret("target", "Target", &target_secret);
    let request = SignedDirectRequest::sign_with_nonce(
        DirectRequestId::parse(request_id).unwrap(),
        "lookup",
        DirectPeerIdentity::from_secret("shared-device", "Requester", &requester_secret),
        DirectPeerIdentity::pinned_target(target.node_id, target.fingerprint),
        10,
        1_000,
        format!("request-nonce-{secret_byte}"),
        None,
        &[9; 32],
        &requester_secret,
    )
    .unwrap();
    DirectRequestEntry {
        direction: DirectRequestDirection::Incoming,
        contact_id: None,
        local_lookup_id: Some("lookup".into()),
        record: DirectRequestRecord::new(request),
        request_receipt: None,
        decision: None,
        decision_receipt: None,
        retries: DirectRequestRetries::default(),
    }
}
