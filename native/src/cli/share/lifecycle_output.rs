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
    let (effective_decision, decision_evidence) = effective_decision(entry, profiles);
    let (connectivity_state, connectivity_label) = connectivity(entry, profiles);
    let (relay_envelope, relay_retry) = current_relay(entry);
    let identity_conflict = profiles.tracked_identity_conflict(&request.request_id);
    let resolution_commands =
        super::request_selection::tracked_conflict_resolution_commands(profiles, entry);
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
            "effective_state": effective_decision,
            "evidence": decision_evidence,
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
        "identity_conflict": identity_conflict,
        "resolution_commands": resolution_commands,
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
    let (effective_decision, decision_evidence) = effective_decision(entry, profiles);
    let (connectivity_state, connectivity_label) = connectivity(entry, profiles);
    let (relay_envelope, relay_retry) = current_relay(entry);
    let identity_conflict = profiles.tracked_identity_conflict(&request.request_id);
    let relay = relay_retry
        .and_then(|retry| retry.relay_outcome.map(relay_code))
        .unwrap_or("unconfirmed");
    let relay_at = relay_retry.and_then(|retry| retry.relay_changed_at);
    let mut lines = vec![format!(
        "request\t{}\tdirection={}\tdelivery={}\tdelivery_at={}\trelay_envelope={}\trelay={}\trelay_at={}\trequest_peer_receipt={}\tdecision={}\teffective_decision={}\tdecision_evidence={}\tdecision_revision={}\tdecision_at={}\tdecision_delivery={}\tdecision_delivery_at={}\tdecision_peer_receipt={}\tauthorization={}\tconnectivity={}\tidentity_conflict={}",
        request.request_id,
        direction_code(entry.direction),
        entry.record.delivery.state.code(),
        entry.record.delivery.changed_at,
        relay_envelope,
        relay,
        option_i64(relay_at),
        receipt_state(&request_peer_receipt(entry)),
        entry.record.decision.state.code(),
        effective_decision,
        decision_evidence,
        entry.record.decision.revision,
        entry.record.decision.changed_at,
        entry.record.decision_delivery.state.code(),
        entry.record.decision_delivery.changed_at,
        receipt_state(&decision_peer_receipt(entry)),
        if active { "active" } else { "inactive" },
        connectivity_state,
        identity_conflict,
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
    for command in super::request_selection::tracked_conflict_resolution_commands(profiles, entry) {
        lines.push(format!(
            "request_resolution\t{}\t{}",
            request.request_id, command
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
            let contact_active = entry.contact_id.as_ref().is_some_and(|contact_id| {
                profiles.direct_contacts.iter().any(|contact| {
                    contact.id == *contact_id && contact.access_state == DirectAccessState::Accepted
                })
            });
            let legacy =
                entry.retries.request.relay_outcome == Some(DirectRelayOutcome::LegacyForwarded);
            (
                contact_active && (accepted || legacy),
                if legacy && entry.decision.is_none() {
                    "legacy_contact_projection"
                } else {
                    "local_contact_projection"
                },
            )
        }
        DirectRequestDirection::Incoming => {
            let requester = &entry.record.request.requester;
            let active = accepted
                && profiles.direct_grants.iter().any(|grant| {
                    grant.device_id == requester.device_id
                        && grant.public_key == requester.public_key
                        && grant.node_id == requester.node_id
                        && grant.fingerprint == requester.fingerprint
                        && grant.state == DirectGrantState::Accepted
                });
            (active, "local_grant_projection")
        }
    }
}

fn effective_decision(
    entry: &DirectRequestEntry,
    profiles: &ShareProfiles,
) -> (&'static str, &'static str) {
    if entry.decision.is_some() {
        return (entry.record.decision.state.code(), "signed_request");
    }
    if entry.direction != DirectRequestDirection::Outgoing
        || entry.retries.request.relay_outcome != Some(DirectRelayOutcome::LegacyForwarded)
    {
        return (entry.record.decision.state.code(), "none");
    }
    let state = entry
        .contact_id
        .as_ref()
        .and_then(|contact_id| {
            profiles
                .direct_contacts
                .iter()
                .find(|contact| contact.id == *contact_id)
        })
        .map(|contact| match contact.access_state {
            DirectAccessState::Accepted => "accepted",
            DirectAccessState::Ignored => "rejected",
            DirectAccessState::IdentityConflict => "identity_conflict",
            DirectAccessState::Pending => "pending",
        })
        .unwrap_or("unknown");
    (state, "legacy_relation")
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
        DirectRelayOutcome::LegacyForwarded => "legacy_forwarded",
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
#[path = "lifecycle_output_tests.rs"]
mod tests;
