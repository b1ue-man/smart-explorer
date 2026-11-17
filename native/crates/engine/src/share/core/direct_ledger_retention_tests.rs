use super::core::public_fingerprint;
use super::direct_ledger::{
    DirectEnvelopeKind, DirectLedgerError, DirectRelayOutcome, MAX_DIRECT_REQUEST_ENTRIES,
};
use super::direct_protocol::{
    DirectDecisionKind, DirectPeerIdentity, DirectRequestId, SignedDirectDecision,
    SignedDirectDecisionReceipt, SignedDirectRequest,
};
use super::profiles::ShareProfiles;
use super::types::{DirectAccessState, DirectContact, DirectGrantState, ShareStatus};

const SECRET: [u8; 32] = [0x77; 32];

fn key(byte: u8) -> iroh::SecretKey {
    iroh::SecretKey::from_bytes(&[byte; 32])
}

fn peer_key(index: usize) -> iroh::SecretKey {
    let mut bytes = [0x11; 32];
    bytes[..8].copy_from_slice(&(index as u64 + 1).to_le_bytes());
    iroh::SecretKey::from_bytes(&bytes)
}

fn request(index: usize, created_at: i64) -> SignedDirectRequest {
    let target_key = key(2);
    let target_public = target_key.public().to_string();
    SignedDirectRequest::sign_with_nonce(
        DirectRequestId::parse(format!(
            "123e4567-e89b-42d3-a{:03x}-{:012x}",
            index & 0xfff,
            index
        ))
        .unwrap(),
        "lookup-a",
        DirectPeerIdentity::from_secret("requester-a", "Requester", &key(1)),
        DirectPeerIdentity::pinned_target(
            target_public.clone(),
            public_fingerprint(target_public.as_bytes()),
        ),
        created_at,
        created_at + 100,
        format!("nonce-{index}"),
        None,
        &SECRET,
        &key(1),
    )
    .unwrap()
}

fn profiles() -> ShareProfiles {
    let target = DirectPeerIdentity::from_secret("target-a", "Target", &key(2));
    let mut profiles = ShareProfiles::default();
    profiles.direct_contacts.push(DirectContact {
        id: "contact-a".into(),
        display_name: "Target".into(),
        lookup_id: "lookup-a".into(),
        expected_fingerprint: target.fingerprint,
        expected_node_id: target.node_id,
        remote_device_id: None,
        remote_public_key: None,
        auto_connect: true,
        auto_open: false,
        last_seen: None,
        status: ShareStatus::WaitingForAccess,
        last_error: None,
        presence: None,
        access_state: DirectAccessState::Pending,
        request_sent_at: None,
        accepted_at: None,
        accepted_public_key: None,
        lan_candidates: Vec::new(),
        lan_seen_at: None,
        lan_uplink: None,
        relation: Default::default(),
    });
    profiles
}

fn fill_pending(profiles: &mut ShareProfiles, start: usize, end: usize) {
    for index in start..end {
        profiles
            .queue_outgoing_direct_request("contact-a", request(index, 100))
            .unwrap();
    }
}

#[test]
fn full_ledger_never_evicts_pending_requests() {
    let mut profiles = profiles();
    fill_pending(&mut profiles, 0, MAX_DIRECT_REQUEST_ENTRIES);
    let error = profiles
        .queue_outgoing_direct_request("contact-a", request(MAX_DIRECT_REQUEST_ENTRIES, 200))
        .unwrap_err();
    assert_eq!(error, DirectLedgerError::LedgerFull);
    assert_eq!(profiles.direct_requests.len(), MAX_DIRECT_REQUEST_ENTRIES);

    let mut unconfirmed = ShareProfiles::default();
    for index in 0..MAX_DIRECT_REQUEST_ENTRIES {
        accept_incoming(&mut unconfirmed, index, 100 + index as i64 * 10, false);
    }
    let blocked = incoming_request(MAX_DIRECT_REQUEST_ENTRIES, 500);
    assert_eq!(
        unconfirmed.record_incoming_direct_request("lookup-host", blocked, 501),
        Err(DirectLedgerError::LedgerFull)
    );
    assert_eq!(
        unconfirmed.direct_requests.len(),
        MAX_DIRECT_REQUEST_ENTRIES
    );
}

#[test]
fn capacity_prunes_terminal_inactive_history_before_new_request() {
    let mut profiles = profiles();
    let terminal = request(0, 100);
    profiles
        .queue_outgoing_direct_request("contact-a", terminal.clone())
        .unwrap();
    let rejected = SignedDirectDecision::sign_with_nonce(
        &terminal,
        DirectPeerIdentity::from_secret("target-a", "Target", &key(2)),
        DirectDecisionKind::Rejected,
        1,
        130,
        300,
        "decision-nonce",
        None,
        &SECRET,
        &key(2),
    )
    .unwrap();
    profiles
        .record_direct_decision(rejected.clone(), 140)
        .unwrap();
    let receipt = SignedDirectDecisionReceipt::sign_with_nonce(
        &rejected,
        150,
        "receipt-nonce",
        None,
        &SECRET,
        &key(1),
    )
    .unwrap();
    profiles.record_direct_decision_receipt(receipt).unwrap();
    profiles
        .record_direct_relay_ack(
            &terminal.request_id,
            DirectEnvelopeKind::DecisionReceipt,
            DirectRelayOutcome::Forwarded,
            151,
        )
        .unwrap();
    fill_pending(&mut profiles, 1, MAX_DIRECT_REQUEST_ENTRIES);

    let replacement = request(MAX_DIRECT_REQUEST_ENTRIES, 200);
    assert!(profiles
        .queue_outgoing_direct_request("contact-a", replacement.clone())
        .unwrap());
    assert_eq!(profiles.direct_requests.len(), MAX_DIRECT_REQUEST_ENTRIES);
    assert!(profiles.direct_request(&terminal.request_id).is_none());
    assert!(profiles.direct_request(&replacement.request_id).is_some());
    assert!(
        profiles
            .direct_requests
            .iter()
            .filter(|entry| entry.record.decision.state == super::DirectDecisionState::Pending)
            .count()
            >= MAX_DIRECT_REQUEST_ENTRIES - 1
    );

    assert_terminal_evicted_before_older_accepted();
    assert_host_accepts_beyond_ledger_capacity_preserving_grants();
    assert_pruned_outgoing_accept_keeps_remote_revocation_tombstone();
}

fn assert_terminal_evicted_before_older_accepted() {
    let mut profiles = profiles();
    let accepted = accept_incoming(&mut profiles, 0, 100, true);
    let terminal = request(1, 120);
    profiles
        .queue_outgoing_direct_request("contact-a", terminal.clone())
        .unwrap();
    let rejected = SignedDirectDecision::sign_with_nonce(
        &terminal,
        DirectPeerIdentity::from_secret("target-a", "Target", &key(2)),
        DirectDecisionKind::Rejected,
        1,
        140,
        300,
        "terminal-decision",
        None,
        &SECRET,
        &key(2),
    )
    .unwrap();
    profiles
        .record_direct_decision(rejected.clone(), 145)
        .unwrap();
    let receipt = SignedDirectDecisionReceipt::sign_with_nonce(
        &rejected,
        150,
        "terminal-receipt",
        None,
        &SECRET,
        &key(1),
    )
    .unwrap();
    profiles.record_direct_decision_receipt(receipt).unwrap();
    profiles
        .record_direct_relay_ack(
            &terminal.request_id,
            DirectEnvelopeKind::DecisionReceipt,
            DirectRelayOutcome::Forwarded,
            151,
        )
        .unwrap();
    fill_pending(&mut profiles, 2, MAX_DIRECT_REQUEST_ENTRIES);

    let next = request(MAX_DIRECT_REQUEST_ENTRIES, 200);
    assert!(profiles
        .queue_outgoing_direct_request("contact-a", next.clone())
        .unwrap());
    assert!(profiles.direct_request(&terminal.request_id).is_none());
    assert!(profiles.direct_request(&accepted.request_id).is_some());
    assert!(profiles.direct_request(&next.request_id).is_some());
    profiles.validate_direct_ledger().unwrap();
}

fn assert_host_accepts_beyond_ledger_capacity_preserving_grants() {
    const TOTAL_PEERS: usize = 128;
    let mut host = ShareProfiles::default();
    let mut requests = Vec::with_capacity(TOTAL_PEERS);
    for index in 0..TOTAL_PEERS {
        let base = 1_000 + index as i64 * 10;
        requests.push(accept_incoming(&mut host, index, base, true));
    }

    assert_eq!(host.direct_requests.len(), MAX_DIRECT_REQUEST_ENTRIES);
    assert_eq!(host.direct_grants.len(), TOTAL_PEERS);
    assert!(host
        .direct_grants
        .iter()
        .all(|grant| grant.state == DirectGrantState::Accepted));
    assert_eq!(host.direct_code_holders().len(), TOTAL_PEERS);
    assert!(host.direct_request_tombstones.is_empty());
    for evicted in &requests[..TOTAL_PEERS - MAX_DIRECT_REQUEST_ENTRIES] {
        assert!(host.direct_request(&evicted.request_id).is_none());
        assert!(!host.direct_auto_accept_denied("lookup-host", &evicted.requester));
    }
    for retained in &requests[TOTAL_PEERS - MAX_DIRECT_REQUEST_ENTRIES..] {
        assert!(host.direct_request(&retained.request_id).is_some());
    }
    host.validate_direct_ledger().unwrap();
}

fn assert_pruned_outgoing_accept_keeps_remote_revocation_tombstone() {
    let mut client = profiles();
    let (outgoing, host_key) = accept_outgoing(&mut client, 0, 100);
    let incoming = accept_incoming(&mut client, 1, 120, true);
    fill_pending(&mut client, 2, MAX_DIRECT_REQUEST_ENTRIES);

    assert!(client
        .queue_outgoing_direct_request("contact-a", request(MAX_DIRECT_REQUEST_ENTRIES, 200))
        .unwrap());
    assert!(client.direct_request(&incoming.request_id).is_none());
    assert!(client.direct_request(&outgoing.request_id).is_some());

    assert!(client
        .queue_outgoing_direct_request("contact-a", request(MAX_DIRECT_REQUEST_ENTRIES + 1, 210))
        .unwrap());
    assert_eq!(client.direct_requests.len(), MAX_DIRECT_REQUEST_ENTRIES);
    assert!(client.direct_request(&outgoing.request_id).is_none());
    assert!(client
        .direct_request_tombstone(&outgoing.request_id)
        .is_some());

    let revoked = SignedDirectDecision::sign_with_nonce(
        &outgoing,
        DirectPeerIdentity::from_secret("remote-host-0", "Remote Host 0", &host_key),
        DirectDecisionKind::Revoked,
        2,
        400,
        900,
        "remote-revoke",
        None,
        &SECRET,
        &host_key,
    )
    .unwrap();
    assert!(client.record_direct_decision(revoked, 401).unwrap());
    let contact = client
        .direct_contacts
        .iter()
        .find(|contact| contact.id == "out-0")
        .unwrap();
    assert_eq!(contact.access_state, DirectAccessState::Ignored);
    client.validate_direct_ledger().unwrap();
}

fn incoming_request(index: usize, created_at: i64) -> SignedDirectRequest {
    let requester_key = peer_key(index);
    let host_public = key(2).public().to_string();
    SignedDirectRequest::sign_with_nonce(
        DirectRequestId::parse(format!(
            "223e4567-e89b-42d3-a{:03x}-{:012x}",
            index & 0xfff,
            index
        ))
        .unwrap(),
        "lookup-host",
        DirectPeerIdentity::from_secret(
            format!("peer-{index}"),
            format!("Peer {index}"),
            &requester_key,
        ),
        DirectPeerIdentity::pinned_target(
            host_public.clone(),
            public_fingerprint(host_public.as_bytes()),
        ),
        created_at,
        created_at + 1_000,
        format!("in-nonce-{index}"),
        None,
        &SECRET,
        &requester_key,
    )
    .unwrap()
}

fn accept_incoming(
    profiles: &mut ShareProfiles,
    index: usize,
    base_time: i64,
    with_receipt: bool,
) -> SignedDirectRequest {
    let req = incoming_request(index, base_time);
    assert!(profiles
        .record_incoming_direct_request("lookup-host", req.clone(), base_time + 1)
        .unwrap());
    let accepted = SignedDirectDecision::sign_with_nonce(
        &req,
        DirectPeerIdentity::from_secret("target-a", "Target", &key(2)),
        DirectDecisionKind::Accepted,
        1,
        base_time + 2,
        base_time + 1_000,
        format!("in-dec-{index}"),
        None,
        &SECRET,
        &key(2),
    )
    .unwrap();
    assert!(profiles
        .record_direct_decision(accepted.clone(), base_time + 2)
        .unwrap());
    if with_receipt {
        let receipt = SignedDirectDecisionReceipt::sign_with_nonce(
            &accepted,
            base_time + 3,
            format!("in-rcpt-{index}"),
            None,
            &SECRET,
            &peer_key(index),
        )
        .unwrap();
        assert!(profiles.record_direct_decision_receipt(receipt).unwrap());
    }
    req
}

fn accept_outgoing(
    profiles: &mut ShareProfiles,
    index: usize,
    base_time: i64,
) -> (SignedDirectRequest, iroh::SecretKey) {
    let host_key = peer_key(index + 500);
    let host = DirectPeerIdentity::from_secret(
        format!("remote-host-{index}"),
        format!("Remote Host {index}"),
        &host_key,
    );
    let contact_id = format!("out-{index}");
    let lookup_id = format!("lookup-out-{index}");
    profiles.direct_contacts.push(DirectContact {
        id: contact_id.clone(),
        display_name: host.device_name.clone(),
        lookup_id: lookup_id.clone(),
        expected_fingerprint: host.fingerprint.clone(),
        expected_node_id: host.node_id.clone(),
        remote_device_id: None,
        remote_public_key: None,
        auto_connect: true,
        auto_open: false,
        last_seen: None,
        status: ShareStatus::WaitingForAccess,
        last_error: None,
        presence: None,
        access_state: DirectAccessState::Pending,
        request_sent_at: None,
        accepted_at: None,
        accepted_public_key: None,
        lan_candidates: Vec::new(),
        lan_seen_at: None,
        lan_uplink: None,
        relation: Default::default(),
    });
    let req = SignedDirectRequest::sign_with_nonce(
        DirectRequestId::parse(format!(
            "323e4567-e89b-42d3-a{:03x}-{:012x}",
            index & 0xfff,
            index
        ))
        .unwrap(),
        lookup_id,
        DirectPeerIdentity::from_secret("requester-a", "Requester", &key(1)),
        DirectPeerIdentity::pinned_target(host.node_id.clone(), host.fingerprint.clone()),
        base_time,
        base_time + 1_000,
        format!("out-nonce-{index}"),
        None,
        &SECRET,
        &key(1),
    )
    .unwrap();
    assert!(profiles
        .queue_outgoing_direct_request(&contact_id, req.clone())
        .unwrap());
    let accepted = SignedDirectDecision::sign_with_nonce(
        &req,
        host,
        DirectDecisionKind::Accepted,
        1,
        base_time + 2,
        base_time + 1_000,
        format!("out-dec-{index}"),
        None,
        &SECRET,
        &host_key,
    )
    .unwrap();
    assert!(profiles
        .record_direct_decision(accepted.clone(), base_time + 2)
        .unwrap());
    let receipt = SignedDirectDecisionReceipt::sign_with_nonce(
        &accepted,
        base_time + 3,
        format!("out-rcpt-{index}"),
        None,
        &SECRET,
        &key(1),
    )
    .unwrap();
    assert!(profiles.record_direct_decision_receipt(receipt).unwrap());
    assert!(profiles
        .record_direct_relay_ack(
            &req.request_id,
            DirectEnvelopeKind::DecisionReceipt,
            DirectRelayOutcome::Forwarded,
            base_time + 4,
        )
        .unwrap());
    (req, host_key)
}
