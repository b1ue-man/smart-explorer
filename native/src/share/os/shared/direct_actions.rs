use super::direct_ledger::{DirectRequestDirection, DirectRequestEntry};
use super::direct_lifecycle::DirectDecisionState;
use super::direct_protocol::{
    DirectDecisionKind, DirectPeerIdentity, DirectRequestId, SignedDirectDecision,
    SignedDirectRequest, MAX_TRACKED_DIRECT_ENVELOPE_LIFETIME_SECS,
};
use super::identity::ShareIdentity;
use super::profiles::ShareProfiles;
use super::types::{DirectAccessState, ShareStatus};

#[derive(Clone, Debug)]
pub struct DirectRequestAction {
    pub entry: DirectRequestEntry,
    pub created: bool,
}

/// Creates one signed durable request, or makes the existing pending request
/// immediately retryable. The signed request ID is generated before the
/// transaction and remains stable if the optimistic save has to be replayed.
pub fn queue_direct_request_for_contact(
    default_home: Option<String>,
    identity: &ShareIdentity,
    contact_id: &str,
    message: Option<String>,
) -> Result<DirectRequestAction, String> {
    ShareIdentity::with_current_locked(identity.device_name.clone(), |current| {
        super::identity_store::with_matching_identity_generation(identity, current, |locked| {
            queue_direct_request_for_contact_locked(default_home, locked, contact_id, message)
        })
    })
}

fn queue_direct_request_for_contact_locked(
    default_home: Option<String>,
    identity: &ShareIdentity,
    contact_id: &str,
    message: Option<String>,
) -> Result<DirectRequestAction, String> {
    let now = super::core::now_secs();
    let profiles = ShareProfiles::load_checked(default_home.clone())?;
    let contact = profiles
        .direct_contacts
        .iter()
        .find(|contact| contact.id == contact_id)
        .cloned()
        .ok_or_else(|| format!("peer contact not found: {contact_id}"))?;
    if contact.access_state == DirectAccessState::Accepted {
        return Err(format!("peer contact is already authorized: {contact_id}"));
    }
    let relation_secret = ShareProfiles::direct_secret_checked(&contact)?
        .ok_or_else(|| format!("direct relation secret is missing for peer {contact_id}"))?;

    let request_id = DirectRequestId::generate().map_err(|error| error.to_string())?;
    let request = SignedDirectRequest::sign(
        request_id.clone(),
        contact.lookup_id.clone(),
        local_peer(identity),
        DirectPeerIdentity::pinned_target(
            contact.expected_node_id.clone(),
            contact.expected_fingerprint.clone(),
        ),
        now,
        now.saturating_add(MAX_TRACKED_DIRECT_ENVELOPE_LIFETIME_SECS),
        message,
        &relation_secret,
        &identity.iroh_secret,
    )
    .map_err(|error| format!("sign direct request: {error}"))?;
    let committed = ShareProfiles::mutate_persisted(default_home, |candidate| {
        require_contact_matches(candidate, &contact)?;
        if let Some(existing_id) = reusable_request_id(candidate, contact_id, now) {
            let retryable = candidate
                .direct_request(&existing_id)
                .ok_or_else(|| format!("direct request disappeared: {existing_id}"))?
                .manually_retryable_outboxes(now);
            for envelope in retryable {
                candidate
                    .retry_direct_envelope_now(&existing_id, envelope, now)
                    .map_err(|error| error.to_string())?;
            }
            mark_contact_pending(candidate, contact_id, now)?;
            return Ok(());
        }
        mark_contact_pending(candidate, contact_id, now)?;
        candidate
            .queue_outgoing_direct_request(contact_id, request.clone())
            .map_err(|error| error.to_string())?;
        Ok(())
    })?;
    if committed.direct_request(&request_id).is_some() {
        action_from(committed, &request_id, true)
    } else {
        let converged_id = reusable_request_id(&committed, contact_id, now).ok_or_else(|| {
            format!("concurrent direct request was not persisted for peer {contact_id}")
        })?;
        action_from(committed, &converged_id, false)
    }
}

pub fn decide_direct_request(
    default_home: Option<String>,
    identity: &ShareIdentity,
    request_id: &DirectRequestId,
    expected_fingerprint: &str,
    decision: DirectDecisionKind,
    message: Option<String>,
) -> Result<DirectRequestEntry, String> {
    ShareIdentity::with_current_locked(identity.device_name.clone(), |current| {
        super::identity_store::with_matching_identity_generation(identity, current, |locked| {
            decide_direct_request_locked(
                default_home,
                locked,
                request_id,
                expected_fingerprint,
                decision,
                message,
            )
        })
    })
}

fn decide_direct_request_locked(
    default_home: Option<String>,
    identity: &ShareIdentity,
    request_id: &DirectRequestId,
    expected_fingerprint: &str,
    decision: DirectDecisionKind,
    message: Option<String>,
) -> Result<DirectRequestEntry, String> {
    let now = super::core::now_secs();
    let profiles = ShareProfiles::load_checked(default_home.clone())?;
    let entry = profiles
        .direct_request(request_id)
        .cloned()
        .ok_or_else(|| format!("direct request not found: {request_id}"))?;
    require_incoming_request(&entry, identity, expected_fingerprint)?;
    let revision = entry
        .decision
        .as_ref()
        .map(|decision| decision.decision_revision)
        .unwrap_or(entry.record.decision.revision)
        .saturating_add(1);
    let signed = SignedDirectDecision::sign(
        &entry.record.request,
        local_peer(identity),
        decision,
        revision,
        now,
        now.saturating_add(MAX_TRACKED_DIRECT_ENVELOPE_LIFETIME_SECS),
        message,
        &identity.direct_secret(),
        &identity.iroh_secret,
    )
    .map_err(|error| format!("sign direct decision: {error}"))?;
    let expected_request = entry.record.request.clone();
    let committed = ShareProfiles::mutate_persisted(default_home, |candidate| {
        let current = candidate
            .direct_request(request_id)
            .ok_or_else(|| format!("direct request disappeared: {request_id}"))?;
        if current.direction != DirectRequestDirection::Incoming
            || current.record.request != expected_request
        {
            return Err(format!("direct request changed concurrently: {request_id}"));
        }
        if decision == DirectDecisionKind::Accepted
            && candidate.tracked_identity_conflict(request_id)
        {
            return Err(format!(
                "direct request has an identity conflict and cannot be accepted: {request_id}"
            ));
        }
        candidate
            .record_direct_decision(signed.clone(), now)
            .map_err(|error| error.to_string())?;
        Ok(())
    })?;
    committed
        .direct_request(request_id)
        .cloned()
        .ok_or_else(|| format!("persisted direct request is missing: {request_id}"))
}

pub fn retry_direct_request_now(
    default_home: Option<String>,
    request_id: &DirectRequestId,
) -> Result<DirectRequestEntry, String> {
    let now = super::core::now_secs();
    let committed = ShareProfiles::mutate_persisted(default_home, |candidate| {
        let retryable = candidate
            .direct_request(request_id)
            .ok_or_else(|| format!("direct request not found: {request_id}"))?
            .manually_retryable_outboxes(now);
        if retryable.is_empty() {
            return Err(format!(
                "direct request has no retryable envelope: {request_id}"
            ));
        }
        for envelope in retryable {
            candidate
                .retry_direct_envelope_now(request_id, envelope, now)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    })?;
    committed
        .direct_request(request_id)
        .cloned()
        .ok_or_else(|| format!("persisted direct request is missing: {request_id}"))
}

pub fn delete_direct_request_history(
    default_home: Option<String>,
    request_id: &DirectRequestId,
) -> Result<(), String> {
    let now = super::core::now_secs();
    ShareProfiles::mutate_persisted(default_home, |profiles| {
        profiles
            .delete_direct_request_locally(request_id, now)
            .map_err(|error| error.to_string())?;
        Ok(())
    })?;
    Ok(())
}

fn reusable_request_id(
    profiles: &ShareProfiles,
    contact_id: &str,
    now: i64,
) -> Option<DirectRequestId> {
    profiles
        .direct_requests
        .iter()
        .filter(|entry| {
            entry.direction == DirectRequestDirection::Outgoing
                && entry.contact_id.as_deref() == Some(contact_id)
                && entry.record.decision.state == DirectDecisionState::Pending
                && now <= entry.record.request.expires_at
        })
        .max_by_key(|entry| entry.record.request.created_at)
        .map(|entry| entry.record.request.request_id.clone())
}

fn require_contact_matches(
    profiles: &ShareProfiles,
    expected: &super::types::DirectContact,
) -> Result<(), String> {
    let current = profiles
        .direct_contacts
        .iter()
        .find(|contact| contact.id == expected.id)
        .ok_or_else(|| format!("peer contact was removed: {}", expected.id))?;
    if current.lookup_id != expected.lookup_id
        || current.expected_node_id != expected.expected_node_id
        || current.expected_fingerprint != expected.expected_fingerprint
    {
        return Err(format!(
            "peer contact changed concurrently: {}",
            expected.id
        ));
    }
    if current.access_state == DirectAccessState::Accepted {
        return Err(format!(
            "peer contact is already authorized: {}",
            expected.id
        ));
    }
    Ok(())
}

fn mark_contact_pending(
    profiles: &mut ShareProfiles,
    contact_id: &str,
    requested_at: i64,
) -> Result<(), String> {
    let contact = profiles
        .direct_contacts
        .iter_mut()
        .find(|contact| contact.id == contact_id)
        .ok_or_else(|| format!("peer contact not found: {contact_id}"))?;
    contact.auto_connect = true;
    contact.auto_open = false;
    contact.status = ShareStatus::WaitingForAccess;
    contact.access_state = DirectAccessState::Pending;
    contact.request_sent_at = Some(requested_at);
    Ok(())
}

fn require_incoming_request(
    entry: &DirectRequestEntry,
    identity: &ShareIdentity,
    expected_fingerprint: &str,
) -> Result<(), String> {
    if entry.direction != DirectRequestDirection::Incoming {
        return Err(format!(
            "direct request is not incoming: {}",
            entry.record.request.request_id
        ));
    }
    if entry.local_lookup_id.as_deref() != Some(identity.direct_lookup_id.as_str()) {
        return Err("direct request belongs to a different local identity".to_string());
    }
    if !entry
        .record
        .request
        .requester
        .fingerprint
        .eq_ignore_ascii_case(expected_fingerprint.trim())
    {
        return Err(format!(
            "fingerprint mismatch for {}: expected {}",
            entry.record.request.request_id, entry.record.request.requester.fingerprint
        ));
    }
    Ok(())
}

fn action_from(
    profiles: ShareProfiles,
    request_id: &DirectRequestId,
    created: bool,
) -> Result<DirectRequestAction, String> {
    let entry = profiles
        .direct_request(request_id)
        .cloned()
        .ok_or_else(|| format!("persisted direct request is missing: {request_id}"))?;
    Ok(DirectRequestAction { entry, created })
}

fn local_peer(identity: &ShareIdentity) -> DirectPeerIdentity {
    DirectPeerIdentity::from_secret(
        identity.device_id.clone(),
        identity.device_name.clone(),
        &identity.iroh_secret,
    )
}

#[cfg(test)]
mod tests {
    use super::reusable_request_id;
    use crate::share::{DirectDecisionState, ShareProfiles};

    #[test]
    fn no_empty_ledger_request_is_reused() {
        assert!(reusable_request_id(&ShareProfiles::default(), "peer", 10).is_none());
        assert_eq!(DirectDecisionState::Pending.code(), "pending");
    }
}
