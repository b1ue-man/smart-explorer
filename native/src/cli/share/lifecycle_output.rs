use crate::share::{
    DirectAccessState, DirectDecisionKind, DirectEnvelopeKind, DirectFailure, DirectGrantState,
    DirectRelayOutcome, DirectRequestDirection, DirectRequestEntry, DirectRetryState,
    ShareProfiles, ShareStatus,
};

pub(in crate::cli) fn request_value(
    entry: &DirectRequestEntry,
    profiles: &ShareProfiles,
) -> serde_json::Value {
    let request = &entry.record.request;
    let (peer, peer_role) = peer(entry);
    let (authorization_active, authorization_basis) = authorization(entry, profiles);
    let (connectivity_state, connectivity_label) = connectivity(entry, profiles);
    let (relay_envelope, relay_retry) = current_relay(entry);
    serde_json::json!({
        "request_id": request.request_id.as_str(),
        "direction": direction_code(entry.direction),
        "contact_id": entry.contact_id,
        "local_lookup_id": entry.local_lookup_id,
        "peer": {
            "role": peer_role,
            "device_id": peer.device_id,
            "device_name": peer.device_name,
            "node_id": peer.node_id,
            "public_key": peer.public_key,
            "fingerprint": peer.fingerprint,
        },
        "created_at": request.created_at,
        "expires_at": request.expires_at,
        "message": request.message,
        "delivery": {
            "state": entry.record.delivery.state.code(),
            "changed_at": entry.record.delivery.changed_at,
            "error": failure_value(entry.record.delivery.failure.as_ref()),
        },
        "relay": {
            "envelope": relay_envelope,
            "outcome": relay_retry.and_then(|retry| retry.relay_outcome.map(relay_code)),
            "changed_at": relay_retry.and_then(|retry| retry.relay_changed_at),
        },
        "peer_receipt": {
            "request": request_peer_receipt(entry),
            "decision": decision_peer_receipt(entry),
        },
        "receipts": {
            "request": entry.request_receipt.as_ref().map(|receipt| serde_json::json!({
                "present": true,
                "received_at": receipt.received_at,
                "expires_at": receipt.expires_at,
            })).unwrap_or_else(|| serde_json::json!({"present": false})),
            "decision": entry.decision_receipt.as_ref().map(|receipt| serde_json::json!({
                "present": true,
                "decision_revision": receipt.decision_revision,
                "received_at": receipt.received_at,
                "expires_at": receipt.expires_at,
            })).unwrap_or_else(|| serde_json::json!({"present": false})),
        },
        "decision": {
            "state": entry.record.decision.state.code(),
            "revision": entry.record.decision.revision,
            "changed_at": entry.record.decision.changed_at,
            "message": entry.record.decision.message,
            "error": failure_value(entry.record.decision.failure.as_ref()),
        },
        "decision_delivery": {
            "state": entry.record.decision_delivery.state.code(),
            "revision": entry.record.decision_delivery.revision,
            "changed_at": entry.record.decision_delivery.changed_at,
            "error": failure_value(entry.record.decision_delivery.failure.as_ref()),
        },
        "attempts": {
            "request": retry_value(&entry.retries.request),
            "request_receipt": retry_value(&entry.retries.request_receipt),
            "decision": retry_value(&entry.retries.decision),
            "decision_receipt": retry_value(&entry.retries.decision_receipt),
        },
        "authorization": {
            "state": if authorization_active { "active" } else { "inactive" },
            "active": authorization_active,
            "basis": authorization_basis,
        },
        "connectivity": {
            "state": connectivity_state,
            "label": connectivity_label,
        },
    })
}

pub(super) fn print_request(entry: &DirectRequestEntry, profiles: &ShareProfiles, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&request_value(entry, profiles))
                .unwrap_or_else(|_| "{}".to_string())
        );
    } else {
        for line in request_text(entry, profiles) {
            println!("{line}");
        }
    }
}

pub(super) fn request_text(entry: &DirectRequestEntry, profiles: &ShareProfiles) -> Vec<String> {
    let request = &entry.record.request;
    let (peer, _) = peer(entry);
    let (active, basis) = authorization(entry, profiles);
    let (connectivity_state, connectivity_label) = connectivity(entry, profiles);
    let (relay_envelope, relay_retry) = current_relay(entry);
    let relay = relay_retry
        .and_then(|retry| retry.relay_outcome.map(relay_code))
        .unwrap_or("unconfirmed");
    let relay_at = relay_retry.and_then(|retry| retry.relay_changed_at);
    let mut lines = vec![format!(
        "request\t{}\tdirection={}\tdelivery={}\tdelivery_at={}\trelay_envelope={}\trelay={}\trelay_at={}\trequest_peer_receipt={}\tdecision={}\tdecision_revision={}\tdecision_at={}\tdecision_delivery={}\tdecision_delivery_at={}\tdecision_peer_receipt={}\tauthorization={}\tconnectivity={}",
        request.request_id,
        direction_code(entry.direction),
        entry.record.delivery.state.code(),
        entry.record.delivery.changed_at,
        relay_envelope,
        relay,
        option_i64(relay_at),
        receipt_state(&request_peer_receipt(entry)),
        entry.record.decision.state.code(),
        entry.record.decision.revision,
        entry.record.decision.changed_at,
        entry.record.decision_delivery.state.code(),
        entry.record.decision_delivery.changed_at,
        receipt_state(&decision_peer_receipt(entry)),
        if active { "active" } else { "inactive" },
        connectivity_state,
    )];
    lines.push(format!(
        "request_peer\t{}\tdevice_id={}\tdevice_name={}\tfingerprint={}\tnode_id={}\tcreated_at={}\texpires_at={}\tauthorization_basis={}\tconnectivity_label={}",
        request.request_id,
        clean(&peer.device_id),
        clean(&peer.device_name),
        clean(&peer.fingerprint),
        clean(&peer.node_id),
        request.created_at,
        request.expires_at,
        basis,
        clean(&connectivity_label),
    ));
    lines.push(format!(
        "request_errors\t{}\tdelivery={}\tdecision={}\tdecision_delivery={}",
        request.request_id,
        failure_text(entry.record.delivery.failure.as_ref()),
        failure_text(entry.record.decision.failure.as_ref()),
        failure_text(entry.record.decision_delivery.failure.as_ref()),
    ));
    for (kind, retry) in [
        (DirectEnvelopeKind::Request, &entry.retries.request),
        (
            DirectEnvelopeKind::RequestReceipt,
            &entry.retries.request_receipt,
        ),
        (DirectEnvelopeKind::Decision, &entry.retries.decision),
        (
            DirectEnvelopeKind::DecisionReceipt,
            &entry.retries.decision_receipt,
        ),
    ] {
        lines.push(format!(
            "request_attempt\t{}\tenvelope={}\tattempt_count={}\tlast_attempt_at={}\trelay={}\trelay_at={}\terror={}",
            request.request_id,
            envelope_code(kind),
            retry.attempt_count,
            option_i64(retry.last_attempt_at),
            retry.relay_outcome.map(relay_code).unwrap_or("unconfirmed"),
            option_i64(retry.relay_changed_at),
            failure_text(retry.last_error.as_ref()),
        ));
    }
    lines
}

pub(super) fn direction_code(direction: DirectRequestDirection) -> &'static str {
    match direction {
        DirectRequestDirection::Outgoing => "outgoing",
        DirectRequestDirection::Incoming => "incoming",
    }
}

pub(super) fn envelope_code(kind: DirectEnvelopeKind) -> &'static str {
    match kind {
        DirectEnvelopeKind::Request => "request",
        DirectEnvelopeKind::RequestReceipt => "request_receipt",
        DirectEnvelopeKind::Decision => "decision",
        DirectEnvelopeKind::DecisionReceipt => "decision_receipt",
    }
}

fn retry_value(retry: &DirectRetryState) -> serde_json::Value {
    serde_json::json!({
        "attempt_count": retry.attempt_count,
        "last_attempt_at": retry.last_attempt_at,
        "relay_outcome": retry.relay_outcome.map(relay_code),
        "relay_changed_at": retry.relay_changed_at,
        "last_error": failure_value(retry.last_error.as_ref()),
    })
}

fn peer(entry: &DirectRequestEntry) -> (&crate::share::DirectPeerIdentity, &'static str) {
    match entry.direction {
        DirectRequestDirection::Outgoing => (
            entry
                .decision
                .as_ref()
                .map(|decision| &decision.target)
                .or_else(|| {
                    entry
                        .request_receipt
                        .as_ref()
                        .map(|receipt| &receipt.target)
                })
                .unwrap_or(&entry.record.request.target),
            "target",
        ),
        DirectRequestDirection::Incoming => (&entry.record.request.requester, "requester"),
    }
}

fn failure_value(failure: Option<&DirectFailure>) -> serde_json::Value {
    failure.map_or(
        serde_json::Value::Null,
        |failure| serde_json::json!({"code": failure.code, "message": failure.message}),
    )
}

fn request_peer_receipt(entry: &DirectRequestEntry) -> serde_json::Value {
    if entry.direction == DirectRequestDirection::Incoming {
        return serde_json::json!({"state": "not_applicable", "received_at": null});
    }
    entry.request_receipt.as_ref().map_or_else(
        || serde_json::json!({"state": "unconfirmed", "received_at": null}),
        |receipt| serde_json::json!({"state": "received", "received_at": receipt.received_at}),
    )
}

fn decision_peer_receipt(entry: &DirectRequestEntry) -> serde_json::Value {
    if entry.direction == DirectRequestDirection::Outgoing || entry.decision.is_none() {
        return serde_json::json!({"state": "not_applicable", "received_at": null});
    }
    entry.decision_receipt.as_ref().map_or_else(
        || serde_json::json!({"state": "unconfirmed", "received_at": null}),
        |receipt| serde_json::json!({"state": "received", "received_at": receipt.received_at}),
    )
}

fn receipt_state(value: &serde_json::Value) -> &str {
    value
        .get("state")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unconfirmed")
}

fn authorization(entry: &DirectRequestEntry, profiles: &ShareProfiles) -> (bool, &'static str) {
    let accepted = entry
        .decision
        .as_ref()
        .is_some_and(|decision| decision.decision == DirectDecisionKind::Accepted);
    match entry.direction {
        DirectRequestDirection::Outgoing => {
            let active = accepted
                && entry.contact_id.as_ref().is_some_and(|contact_id| {
                    profiles.direct_contacts.iter().any(|contact| {
                        contact.id == *contact_id
                            && contact.access_state == DirectAccessState::Accepted
                    })
                });
            (active, "local_contact_projection")
        }
        DirectRequestDirection::Incoming => {
            let requester = &entry.record.request.requester;
            let active = accepted
                && profiles.direct_grants.iter().any(|grant| {
                    grant.device_id == requester.device_id
                        && grant.public_key == requester.public_key
                        && grant.state == DirectGrantState::Accepted
                });
            (active, "local_grant_projection")
        }
    }
}

fn connectivity(entry: &DirectRequestEntry, profiles: &ShareProfiles) -> (&'static str, String) {
    if entry.direction == DirectRequestDirection::Incoming {
        return (
            "unknown",
            "No per-request incoming transport session is tracked".to_string(),
        );
    }
    let status = entry
        .contact_id
        .as_ref()
        .and_then(|id| {
            profiles
                .direct_contacts
                .iter()
                .find(|contact| &contact.id == id)
        })
        .map(|contact| &contact.status);
    status.map_or(("unknown", "Unknown".to_string()), |status| {
        (share_status_code(status), status.label())
    })
}

fn share_status_code(status: &ShareStatus) -> &'static str {
    match status {
        ShareStatus::Offline => "offline",
        ShareStatus::Waiting => "waiting",
        ShareStatus::WaitingForAccess => "waiting_for_access",
        ShareStatus::Available => "available",
        ShareStatus::Connecting => "connecting",
        ShareStatus::Connected => "connected",
        ShareStatus::ConnectedDirect => "connected_direct",
        ShareStatus::ConnectedRelay => "connected_relay",
        ShareStatus::Failed(_) => "failed",
        ShareStatus::IdentityConflict => "identity_conflict",
    }
}

fn current_relay(entry: &DirectRequestEntry) -> (&'static str, Option<&DirectRetryState>) {
    match entry.direction {
        DirectRequestDirection::Outgoing if entry.decision.is_none() => {
            ("request", Some(&entry.retries.request))
        }
        DirectRequestDirection::Incoming if entry.decision.is_some() => {
            ("decision", Some(&entry.retries.decision))
        }
        DirectRequestDirection::Incoming => {
            ("request_receipt", Some(&entry.retries.request_receipt))
        }
        DirectRequestDirection::Outgoing => {
            ("decision_receipt", Some(&entry.retries.decision_receipt))
        }
    }
}

fn relay_code(outcome: DirectRelayOutcome) -> &'static str {
    match outcome {
        DirectRelayOutcome::Forwarded => "forwarded",
        DirectRelayOutcome::TargetOffline => "target_offline",
    }
}

fn option_i64(value: Option<i64>) -> String {
    value.map_or_else(|| "-".to_string(), |value| value.to_string())
}

fn failure_text(failure: Option<&DirectFailure>) -> String {
    failure.map_or_else(
        || "-".to_string(),
        |failure| format!("{}:{}", clean(&failure.code), clean(&failure.message)),
    )
}

fn clean(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use super::{direction_code, envelope_code, request_text, request_value};
    use crate::share::{
        DirectAccessState, DirectContact, DirectEnvelopeKind, DirectPeerIdentity,
        DirectRequestDirection, DirectRequestId, ShareProfiles, ShareStatus, SignedDirectRequest,
        SignedDirectRequestReceipt,
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
    fn output_separates_delivery_receipt_authorization_and_connectivity() {
        let requester_secret = iroh::SecretKey::from_bytes(&[3; 32]);
        let target_secret = iroh::SecretKey::from_bytes(&[7; 32]);
        let requester =
            DirectPeerIdentity::from_secret("local-device", "Local Device", &requester_secret);
        let target =
            DirectPeerIdentity::from_secret("remote-device", "Remote Device", &target_secret);
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
    }
}
