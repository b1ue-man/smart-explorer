use crate::share::{
    DirectDecisionKind, DirectGrantState, DirectLedgerError, DirectPeerIdentity,
    DirectProtocolError, DirectRequestDirection, DirectSignalEvent, ShareIdentity, ShareProfiles,
    SignedDirectDecision, SignedDirectDecisionReceipt, SignedDirectRequestReceipt,
};
use std::fmt;

const DECISION_LIFETIME_SECS: i64 = 30 * 24 * 60 * 60;

pub(crate) fn persist_all(
    identity: &ShareIdentity,
    events: &[DirectSignalEvent],
) -> Result<ShareProfiles, String> {
    ShareProfiles::mutate_persisted(Some(super::default_home()), |profiles| {
        for event in events {
            apply(profiles, identity, event).map_err(|error| error.to_string())?;
        }
        Ok(())
    })
}

fn apply(
    profiles: &mut ShareProfiles,
    identity: &ShareIdentity,
    event: &DirectSignalEvent,
) -> Result<(), ApplyError> {
    match event {
        DirectSignalEvent::RequestReceived {
            request,
            received_at,
        } => {
            verify_incoming_request(identity, request, *received_at)?;
            profiles.record_incoming_direct_request(
                &identity.direct_lookup_id,
                request.clone(),
                *received_at,
            )?;
            ensure_request_receipt(profiles, identity, request, *received_at)?;
            ensure_existing_grant_decision(profiles, identity, request, *received_at)?;
        }
        DirectSignalEvent::RequestReceiptReceived {
            receipt,
            received_at,
        } => {
            let (request, secret) = outgoing_request_and_secret(profiles, &receipt.request_id)?;
            receipt.verify_for(&request, &secret, *received_at)?;
            profiles.record_direct_request_receipt(receipt.clone())?;
        }
        DirectSignalEvent::DecisionReceived {
            decision,
            received_at,
        } => {
            let (request, secret) = outgoing_request_and_secret(profiles, &decision.request_id)?;
            decision.verify_for(&request, &secret, *received_at)?;
            profiles.record_direct_decision(decision.clone(), *received_at)?;
            let needs_receipt = profiles
                .direct_request(&decision.request_id)
                .and_then(|entry| entry.decision_receipt.as_ref())
                .is_none_or(|receipt| receipt.decision_revision != decision.decision_revision);
            if needs_receipt {
                let receipt = SignedDirectDecisionReceipt::sign(
                    decision,
                    *received_at,
                    None,
                    &secret,
                    &identity.iroh_secret,
                )?;
                profiles.record_direct_decision_receipt(receipt)?;
            }
        }
        DirectSignalEvent::DecisionReceiptReceived {
            receipt,
            received_at,
        } => {
            let entry = profiles
                .direct_request(&receipt.request_id)
                .ok_or("unknown direct request")?;
            if entry.direction != DirectRequestDirection::Incoming {
                return Err("decision receipt has the wrong direction".into());
            }
            let decision = entry
                .decision
                .clone()
                .ok_or("decision envelope is missing")?;
            let secret = identity.direct_secret();
            receipt.verify_for(&decision, &secret, *received_at)?;
            profiles.record_direct_decision_receipt(receipt.clone())?;
        }
        DirectSignalEvent::EnvelopeAttempted {
            request_id,
            envelope,
            attempt_count,
            at,
            failure,
        } => {
            profiles.record_direct_attempt(
                request_id,
                *envelope,
                *attempt_count,
                *at,
                failure.clone(),
            )?;
        }
        DirectSignalEvent::RelayAcknowledged {
            request_id,
            envelope,
            outcome,
            at,
        } => {
            profiles.record_direct_relay_ack(request_id, *envelope, *outcome, *at)?;
        }
    }
    Ok(())
}

fn verify_incoming_request(
    identity: &ShareIdentity,
    request: &crate::share::SignedDirectRequest,
    received_at: i64,
) -> Result<(), ApplyError> {
    let local = local_peer(identity);
    let target = &request.target;
    if request.lookup_id != identity.direct_lookup_id
        || target.node_id != local.node_id
        || target.public_key != local.public_key
        || target.fingerprint != local.fingerprint
        || (!target.device_id.is_empty() && target.device_id != local.device_id)
    {
        return Err("direct request target does not match this identity".into());
    }
    request.verify_at(&identity.direct_secret(), received_at)?;
    Ok(())
}

fn ensure_request_receipt(
    profiles: &mut ShareProfiles,
    identity: &ShareIdentity,
    request: &crate::share::SignedDirectRequest,
    received_at: i64,
) -> Result<(), ApplyError> {
    if profiles
        .direct_request(&request.request_id)
        .and_then(|entry| entry.request_receipt.as_ref())
        .is_some()
    {
        return Ok(());
    }
    let receipt = SignedDirectRequestReceipt::sign(
        request,
        local_peer(identity),
        received_at,
        None,
        &identity.direct_secret(),
        &identity.iroh_secret,
    )?;
    profiles.record_direct_request_receipt(receipt)?;
    Ok(())
}

fn ensure_existing_grant_decision(
    profiles: &mut ShareProfiles,
    identity: &ShareIdentity,
    request: &crate::share::SignedDirectRequest,
    now: i64,
) -> Result<(), ApplyError> {
    if profiles
        .direct_request(&request.request_id)
        .and_then(|entry| entry.decision.as_ref())
        .is_some()
    {
        return Ok(());
    }
    let Some(grant) = profiles.grant_for(&request.requester.device_id) else {
        return Ok(());
    };
    if grant.public_key != request.requester.public_key
        || grant.node_id != request.requester.node_id
    {
        return Ok(());
    }
    let decision = match grant.state {
        DirectGrantState::Accepted => DirectDecisionKind::Accepted,
        DirectGrantState::Ignored => DirectDecisionKind::Rejected,
    };
    let signed = SignedDirectDecision::sign(
        request,
        local_peer(identity),
        decision,
        1,
        now,
        now.saturating_add(DECISION_LIFETIME_SECS),
        None,
        &identity.direct_secret(),
        &identity.iroh_secret,
    )?;
    profiles.record_direct_decision(signed, now)?;
    Ok(())
}

fn outgoing_request_and_secret(
    profiles: &ShareProfiles,
    request_id: &crate::share::DirectRequestId,
) -> Result<(crate::share::SignedDirectRequest, Vec<u8>), ApplyError> {
    let entry = profiles
        .direct_request(request_id)
        .ok_or("unknown direct request")?;
    if entry.direction != DirectRequestDirection::Outgoing {
        return Err("direct request has the wrong direction".into());
    }
    let contact_id = entry
        .contact_id
        .as_deref()
        .ok_or("direct request contact is missing")?;
    let contact = profiles
        .direct_contacts
        .iter()
        .find(|contact| contact.id == contact_id)
        .ok_or("direct request contact was removed")?;
    let secret = ShareProfiles::direct_secret_checked(contact)?
        .ok_or("direct request relation secret is missing")?;
    Ok((entry.record.request.clone(), secret))
}

fn local_peer(identity: &ShareIdentity) -> DirectPeerIdentity {
    DirectPeerIdentity::from_secret(
        identity.device_id.clone(),
        identity.device_name.clone(),
        &identity.iroh_secret,
    )
}

#[derive(Debug)]
struct ApplyError(String);

impl fmt::Display for ApplyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ApplyError {}

impl From<DirectProtocolError> for ApplyError {
    fn from(error: DirectProtocolError) -> Self {
        Self(error.to_string())
    }
}

impl From<DirectLedgerError> for ApplyError {
    fn from(error: DirectLedgerError) -> Self {
        Self(error.to_string())
    }
}

impl From<String> for ApplyError {
    fn from(error: String) -> Self {
        Self(error)
    }
}

impl From<&str> for ApplyError {
    fn from(error: &str) -> Self {
        Self(error.to_string())
    }
}
