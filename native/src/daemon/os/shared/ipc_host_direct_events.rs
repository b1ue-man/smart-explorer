use crate::share::{
    DirectDecisionKind, DirectGrantState, DirectLedgerError, DirectPeerIdentity,
    DirectProtocolError, DirectRequestDirection, DirectRequestPolicy, DirectSignalEvent,
    ShareIdentity, ShareProfiles, SignedDirectDecision, SignedDirectDecisionReceipt,
    SignedDirectRequestReceipt, MAX_TRACKED_DIRECT_ENVELOPE_LIFETIME_SECS,
};
use std::{cell::RefCell, fmt};

pub(super) fn persist_group(
    identity: &ShareIdentity,
    events: &[DirectSignalEvent],
) -> Result<ShareProfiles, GroupPersistError> {
    let apply_error = RefCell::new(None);
    let policy = DirectRequestPolicy::current();
    let result = ShareProfiles::mutate_persisted(Some(super::default_home()), |profiles| {
        apply_error.take();
        for event in events {
            if let Err(error) = apply(profiles, identity, event, policy) {
                let message = error.to_string();
                apply_error.replace(Some(error));
                return Err(message);
            }
        }
        Ok(())
    });
    match result {
        Ok(profiles) => Ok(profiles),
        Err(error) => match apply_error.take() {
            Some(error) if error.is_retryable() => {
                Err(GroupPersistError::Retryable(error.to_string()))
            }
            Some(error) => Err(GroupPersistError::Permanent(error.to_string())),
            None if retryable_persistence_error(&error) => Err(GroupPersistError::Retryable(error)),
            None => Err(GroupPersistError::Permanent(error)),
        },
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum GroupPersistError {
    Permanent(String),
    Retryable(String),
}

fn retryable_persistence_error(error: &str) -> bool {
    let permanent = [
        "Share-Profile sind beschaedigt",
        "Nicht unterstuetzte Share-Profilversion",
        "Share-Profile kodieren",
        "exceed their byte budget",
        "not a regular file",
        "not valid UTF-8",
    ]
    .iter()
    .any(|marker| error.contains(marker));
    !permanent
        && (error.starts_with("Share-Profile lesen:")
            || error.starts_with("Share-Profile speichern:")
            || error.contains("changed concurrently"))
}

fn apply(
    profiles: &mut ShareProfiles,
    identity: &ShareIdentity,
    event: &DirectSignalEvent,
    policy: DirectRequestPolicy,
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
            if profiles.direct_request(&request.request_id).is_none() {
                return Ok(());
            }
            ensure_request_receipt(profiles, identity, request, *received_at)?;
            ensure_authenticated_request_decision(
                profiles,
                identity,
                request,
                *received_at,
                policy,
            )?;
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
            if profiles
                .direct_request_tombstone(&receipt.request_id)
                .is_some()
            {
                return Ok(());
            }
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

fn ensure_authenticated_request_decision(
    profiles: &mut ShareProfiles,
    identity: &ShareIdentity,
    request: &crate::share::SignedDirectRequest,
    now: i64,
    policy: DirectRequestPolicy,
) -> Result<(), ApplyError> {
    if profiles
        .direct_request(&request.request_id)
        .and_then(|entry| entry.decision.as_ref())
        .is_some()
    {
        return Ok(());
    }
    let Some(decision) = automatic_decision(profiles, request, policy) else {
        // FC5: a device without a grant waits in the inbox for the user.
        return Ok(());
    };
    let signed = SignedDirectDecision::sign(
        request,
        local_peer(identity),
        decision,
        1,
        now,
        now.saturating_add(MAX_TRACKED_DIRECT_ENVELOPE_LIFETIME_SECS),
        None,
        &identity.direct_secret(),
        &identity.iroh_secret,
    )?;
    profiles.record_direct_decision(signed, now)?;
    Ok(())
}

/// The decision this device takes without asking. The current grant of the
/// exact identity wins over older history; a suspended grant („neu
/// bestätigen“) is confirmed again by a request authenticated with the
/// current code. A device without a grant is accepted automatically only with
/// `AutoAccept` (FC5) and never when the user removed or blocked its key.
fn automatic_decision(
    profiles: &ShareProfiles,
    request: &crate::share::SignedDirectRequest,
    policy: DirectRequestPolicy,
) -> Option<DirectDecisionKind> {
    if profiles.tracked_identity_conflict(&request.request_id) {
        return Some(DirectDecisionKind::Rejected);
    }
    let requester = &request.requester;
    if profiles.direct_key_denied(requester) {
        return Some(DirectDecisionKind::Rejected);
    }
    match profiles.grant_for(&requester.device_id) {
        Some(grant)
            if grant.public_key == requester.public_key
                && grant.node_id == requester.node_id
                && grant.fingerprint == requester.fingerprint =>
        {
            Some(match grant.state {
                DirectGrantState::Accepted | DirectGrantState::Reconfirm => {
                    DirectDecisionKind::Accepted
                }
                DirectGrantState::Ignored => DirectDecisionKind::Rejected,
            })
        }
        Some(_) => Some(DirectDecisionKind::Rejected),
        None if profiles.direct_auto_accept_denied(&request.lookup_id, requester) => {
            Some(DirectDecisionKind::Rejected)
        }
        None if policy == DirectRequestPolicy::AutoAccept => Some(DirectDecisionKind::Accepted),
        None => None,
    }
}

fn outgoing_request_and_secret(
    profiles: &ShareProfiles,
    request_id: &crate::share::DirectRequestId,
) -> Result<(crate::share::SignedDirectRequest, Vec<u8>), ApplyError> {
    let (request, contact_id) = match profiles.direct_request(request_id) {
        Some(entry) if entry.direction == DirectRequestDirection::Outgoing => (
            entry.record.request.clone(),
            entry
                .contact_id
                .clone()
                .ok_or("direct request contact is missing")?,
        ),
        Some(_) => return Err("direct request has the wrong direction".into()),
        None => profiles
            .tombstoned_outgoing_request(request_id)
            .map(|(request, contact_id)| (request.clone(), contact_id.to_string()))
            .ok_or("unknown direct request")?,
    };
    let contact = profiles
        .direct_contacts
        .iter()
        .find(|contact| contact.id == contact_id)
        .ok_or("direct request contact was removed")?;
    let secret = ShareProfiles::direct_secret_checked(contact)
        .map_err(ApplyError::relation_secret)?
        .ok_or("direct request relation secret is missing")?;
    Ok((request, secret))
}

fn local_peer(identity: &ShareIdentity) -> DirectPeerIdentity {
    DirectPeerIdentity::from_secret(
        identity.device_id.clone(),
        identity.device_name.clone(),
        &identity.iroh_secret,
    )
}

#[derive(Clone, Debug)]
enum ApplyError {
    Permanent(String),
    Retryable(String),
}

impl ApplyError {
    fn relation_secret(error: String) -> Self {
        if error.contains("ist ungueltig") || error.contains("hat nicht 32 Bytes") {
            Self::Permanent(error)
        } else {
            Self::Retryable(error)
        }
    }

    fn is_retryable(&self) -> bool {
        matches!(self, Self::Retryable(_))
    }
}

impl fmt::Display for ApplyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Permanent(error) | Self::Retryable(error) => formatter.write_str(error),
        }
    }
}

impl std::error::Error for ApplyError {}

impl From<DirectProtocolError> for ApplyError {
    fn from(error: DirectProtocolError) -> Self {
        if error == DirectProtocolError::EntropyUnavailable {
            Self::Retryable(error.to_string())
        } else {
            Self::Permanent(error.to_string())
        }
    }
}

impl From<DirectLedgerError> for ApplyError {
    fn from(error: DirectLedgerError) -> Self {
        if matches!(
            error,
            DirectLedgerError::Protocol(DirectProtocolError::EntropyUnavailable)
        ) {
            Self::Retryable(error.to_string())
        } else {
            Self::Permanent(error.to_string())
        }
    }
}

impl From<String> for ApplyError {
    fn from(error: String) -> Self {
        Self::Permanent(error)
    }
}

impl From<&str> for ApplyError {
    fn from(error: &str) -> Self {
        Self::Permanent(error.to_string())
    }
}
