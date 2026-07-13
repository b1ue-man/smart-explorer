use super::direct_ledger::{
    DirectEnvelopeKind, DirectLedgerError, DirectRelayOutcome, DirectRequestDirection,
    DirectRequestEntry, DirectRetryState,
};
use super::direct_lifecycle::{
    DirectDecisionDeliveryState, DirectDeliveryState, DirectFailure, DirectLifecycleEvent,
};
use super::direct_protocol::{
    DirectRequestId, SignedDirectDecision, SignedDirectDecisionReceipt, SignedDirectRequest,
    SignedDirectRequestReceipt,
};
use super::profiles::ShareProfiles;

impl ShareProfiles {
    pub fn direct_request(&self, request_id: &DirectRequestId) -> Option<&DirectRequestEntry> {
        self.direct_requests
            .iter()
            .find(|entry| entry.record.request.request_id == *request_id)
    }

    pub fn queue_outgoing_direct_request(
        &mut self,
        contact_id: &str,
        request: SignedDirectRequest,
    ) -> Result<bool, DirectLedgerError> {
        let contact = self
            .direct_contacts
            .iter()
            .find(|contact| contact.id == contact_id)
            .ok_or(DirectLedgerError::InvalidRelation)?;
        if contact.lookup_id != request.lookup_id
            || contact.expected_fingerprint != request.target.fingerprint
            || (!contact.expected_node_id.is_empty()
                && contact.expected_node_id != request.target.node_id)
        {
            return Err(DirectLedgerError::InvalidRelation);
        }
        request.digest()?;
        if self.tombstone_blocks_request(&request, DirectRequestDirection::Outgoing)? {
            return Err(DirectLedgerError::RequestIdConflict);
        }
        if let Some(existing) = self.direct_request(&request.request_id) {
            return same_request(
                existing,
                &request,
                DirectRequestDirection::Outgoing,
                contact_id,
            );
        }
        self.ensure_direct_request_capacity(request.created_at)?;
        self.direct_requests
            .push(DirectRequestEntry::outgoing(contact_id.into(), request));
        Ok(true)
    }

    pub fn record_incoming_direct_request(
        &mut self,
        local_lookup_id: &str,
        request: SignedDirectRequest,
        received_at: i64,
    ) -> Result<bool, DirectLedgerError> {
        if local_lookup_id.is_empty() || local_lookup_id != request.lookup_id {
            return Err(DirectLedgerError::InvalidRelation);
        }
        request.digest()?;
        if self.tombstone_blocks_request(&request, DirectRequestDirection::Incoming)? {
            return Ok(false);
        }
        if let Ok(index) = find_index(&self.direct_requests, &request.request_id) {
            same_request(
                &self.direct_requests[index],
                &request,
                DirectRequestDirection::Incoming,
                local_lookup_id,
            )?;
            let entry = &mut self.direct_requests[index];
            return Ok(entry.decision.is_none()
                && entry.requeue_forwarded_receipt(DirectEnvelopeKind::RequestReceipt));
        }
        self.ensure_direct_request_capacity(received_at)?;
        self.direct_requests.push(DirectRequestEntry::incoming(
            local_lookup_id.into(),
            request,
            received_at,
        )?);
        Ok(true)
    }

    pub fn record_direct_request_receipt(
        &mut self,
        receipt: SignedDirectRequestReceipt,
    ) -> Result<bool, DirectLedgerError> {
        if self.tombstone_blocks_digest(&receipt.request_id, &receipt.request_digest)? {
            return Ok(false);
        }
        let entry = find_mut(&mut self.direct_requests, &receipt.request_id)?;
        require_request_receipt(&entry.record.request, &receipt)?;
        if let Some(existing) = &entry.request_receipt {
            return if existing == &receipt {
                Ok(false)
            } else {
                Err(DirectLedgerError::EnvelopeConflict)
            };
        }
        if entry.direction == DirectRequestDirection::Outgoing {
            let observed_at = receipt.received_at.max(entry.record.delivery.changed_at);
            entry.record.apply(DirectLifecycleEvent::Delivery {
                request_id: receipt.request_id.clone(),
                state: DirectDeliveryState::Received,
                at: observed_at,
                failure: None,
            })?;
        }
        entry.request_receipt = Some(receipt);
        Ok(true)
    }

    pub fn record_direct_decision(
        &mut self,
        decision: SignedDirectDecision,
        observed_at: i64,
    ) -> Result<bool, DirectLedgerError> {
        if let Some(changed) = self.record_tombstoned_direct_decision(&decision)? {
            return Ok(changed);
        }
        let index = find_index(&self.direct_requests, &decision.request_id)?;
        require_decision(&self.direct_requests[index].record.request, &decision)?;
        let entry = &mut self.direct_requests[index];
        if let Some(existing) = &entry.decision {
            if decision.decision_revision < existing.decision_revision {
                return Ok(false);
            }
            if decision.decision_revision == existing.decision_revision {
                if existing != &decision {
                    return Err(DirectLedgerError::EnvelopeConflict);
                }
                let changed = entry.direction == DirectRequestDirection::Outgoing
                    && entry.requeue_forwarded_receipt(DirectEnvelopeKind::DecisionReceipt);
                return Ok(changed);
            }
        }
        entry.record.apply(DirectLifecycleEvent::Decision {
            request_id: decision.request_id.clone(),
            decision: decision.decision,
            revision: decision.decision_revision,
            at: decision.decided_at,
            message: decision.message.clone(),
        })?;
        if entry.direction == DirectRequestDirection::Outgoing {
            entry.record.apply(DirectLifecycleEvent::DecisionDelivery {
                request_id: decision.request_id.clone(),
                state: DirectDecisionDeliveryState::Received,
                revision: decision.decision_revision,
                at: observed_at,
                failure: None,
            })?;
        }
        entry.decision = Some(decision.clone());
        entry.decision_receipt = None;
        entry.retries.decision = DirectRetryState::default();
        entry.retries.decision_receipt = DirectRetryState::default();
        self.project_decision(index, &decision);
        Ok(true)
    }

    pub fn record_direct_decision_receipt(
        &mut self,
        receipt: SignedDirectDecisionReceipt,
    ) -> Result<bool, DirectLedgerError> {
        if self.direct_request_tombstone(&receipt.request_id).is_some() {
            return Ok(false);
        }
        let entry = find_mut(&mut self.direct_requests, &receipt.request_id)?;
        let decision = entry
            .decision
            .as_ref()
            .ok_or(DirectLedgerError::MissingEnvelope)?;
        if receipt.decision_revision < decision.decision_revision {
            return Ok(false);
        }
        require_decision_receipt(decision, &receipt)?;
        if let Some(existing) = &entry.decision_receipt {
            return if existing == &receipt {
                Ok(false)
            } else {
                Err(DirectLedgerError::EnvelopeConflict)
            };
        }
        if entry.direction == DirectRequestDirection::Incoming {
            let observed_at = receipt
                .received_at
                .max(entry.record.decision_delivery.changed_at);
            entry.record.apply(DirectLifecycleEvent::DecisionDelivery {
                request_id: receipt.request_id.clone(),
                state: DirectDecisionDeliveryState::Received,
                revision: receipt.decision_revision,
                at: observed_at,
                failure: None,
            })?;
        }
        entry.decision_receipt = Some(receipt);
        Ok(true)
    }

    pub fn record_direct_attempt(
        &mut self,
        request_id: &DirectRequestId,
        kind: DirectEnvelopeKind,
        attempt_count: u32,
        at: i64,
        error: Option<DirectFailure>,
    ) -> Result<bool, DirectLedgerError> {
        if self.direct_request_tombstone(request_id).is_some() {
            return Ok(false);
        }
        let entry = find_mut(&mut self.direct_requests, request_id)?;
        if !entry.has_outbox(kind) {
            return Err(DirectLedgerError::MissingEnvelope);
        }
        let previous = entry.retry(kind);
        if attempt_count <= previous.attempt_count {
            return Ok(false);
        }
        if attempt_count == 0 || at < 0 || previous.last_attempt_at.is_some_and(|last| at < last) {
            return Err(DirectLedgerError::InvalidAttempt);
        }
        let relay_outcome = previous.relay_outcome;
        let relay_changed_at = previous.relay_changed_at;
        match kind {
            DirectEnvelopeKind::Request => entry.record.apply(DirectLifecycleEvent::Delivery {
                request_id: request_id.clone(),
                state: DirectDeliveryState::Sent,
                at,
                failure: None,
            })?,
            DirectEnvelopeKind::Decision => {
                let revision = entry.record.decision.revision;
                entry.record.apply(DirectLifecycleEvent::DecisionDelivery {
                    request_id: request_id.clone(),
                    state: DirectDecisionDeliveryState::Sent,
                    revision,
                    at,
                    failure: None,
                })?
            }
            DirectEnvelopeKind::RequestReceipt | DirectEnvelopeKind::DecisionReceipt => false,
        };
        *entry.retry_mut(kind) = DirectRetryState {
            attempt_count,
            last_attempt_at: Some(at),
            relay_outcome,
            relay_changed_at,
            last_error: error,
        };
        Ok(true)
    }

    pub fn record_direct_relay_ack(
        &mut self,
        request_id: &DirectRequestId,
        kind: DirectEnvelopeKind,
        outcome: DirectRelayOutcome,
        at: i64,
    ) -> Result<bool, DirectLedgerError> {
        if self.direct_request_tombstone(request_id).is_some() {
            return Ok(false);
        }
        let entry = find_mut(&mut self.direct_requests, request_id)?;
        if !entry.has_outbox(kind) {
            return Err(DirectLedgerError::MissingEnvelope);
        }
        let retry = entry.retry(kind);
        if retry.relay_changed_at.is_some_and(|last| at < last) {
            return Ok(false);
        }
        if retry.relay_changed_at == Some(at) {
            match (retry.relay_outcome, outcome) {
                (Some(DirectRelayOutcome::Forwarded), DirectRelayOutcome::Forwarded)
                | (Some(DirectRelayOutcome::TargetOffline), DirectRelayOutcome::TargetOffline) => {
                    return Ok(false);
                }
                (Some(DirectRelayOutcome::Forwarded), DirectRelayOutcome::TargetOffline) => {
                    return Ok(false);
                }
                (Some(DirectRelayOutcome::TargetOffline), DirectRelayOutcome::Forwarded) => {}
                (None, _) => return Err(DirectLedgerError::EnvelopeConflict),
            }
        }
        if at < 0 {
            return Err(DirectLedgerError::InvalidTimestamp);
        }
        if outcome == DirectRelayOutcome::Forwarded {
            apply_relay_forwarded(entry, request_id, kind, at)?;
        }
        let retry = entry.retry_mut(kind);
        retry.relay_outcome = Some(outcome);
        retry.relay_changed_at = Some(at);
        retry.last_error = None;
        Ok(true)
    }

    /// Makes an existing durable outbox envelope immediately eligible for a
    /// manual retry. Resetting the absolute retry metadata is idempotent when
    /// this mutation is replayed after an optimistic-save conflict.
    pub fn retry_direct_envelope_now(
        &mut self,
        request_id: &DirectRequestId,
        kind: DirectEnvelopeKind,
        now: i64,
    ) -> Result<bool, DirectLedgerError> {
        if now < 0 {
            return Err(DirectLedgerError::InvalidTimestamp);
        }
        if self.direct_request_tombstone(request_id).is_some() {
            return Ok(false);
        }
        let entry = find_mut(&mut self.direct_requests, request_id)?;
        if !entry.has_outbox(kind) {
            return Err(DirectLedgerError::MissingEnvelope);
        }
        let retry = entry.retry_mut(kind);
        if retry == &DirectRetryState::default() {
            return Ok(false);
        }
        *retry = DirectRetryState::default();
        Ok(true)
    }
}

fn find_index(
    entries: &[DirectRequestEntry],
    request_id: &DirectRequestId,
) -> Result<usize, DirectLedgerError> {
    entries
        .iter()
        .position(|entry| entry.record.request.request_id == *request_id)
        .ok_or(DirectLedgerError::UnknownRequest)
}

fn find_mut<'a>(
    entries: &'a mut [DirectRequestEntry],
    request_id: &DirectRequestId,
) -> Result<&'a mut DirectRequestEntry, DirectLedgerError> {
    let index = find_index(entries, request_id)?;
    Ok(&mut entries[index])
}

fn same_request(
    existing: &DirectRequestEntry,
    request: &SignedDirectRequest,
    direction: DirectRequestDirection,
    relation: &str,
) -> Result<bool, DirectLedgerError> {
    let relation_matches = match direction {
        DirectRequestDirection::Outgoing => existing.contact_id.as_deref() == Some(relation),
        DirectRequestDirection::Incoming => existing.local_lookup_id.as_deref() == Some(relation),
    };
    if existing.direction != direction || !relation_matches {
        return Err(DirectLedgerError::RequestIdConflict);
    }
    if existing.record.request.digest()? == request.digest()? {
        Ok(false)
    } else {
        Err(DirectLedgerError::RequestIdConflict)
    }
}

fn apply_relay_forwarded(
    entry: &mut DirectRequestEntry,
    request_id: &DirectRequestId,
    kind: DirectEnvelopeKind,
    at: i64,
) -> Result<(), DirectLedgerError> {
    let event = match kind {
        DirectEnvelopeKind::Request => Some(DirectLifecycleEvent::Delivery {
            request_id: request_id.clone(),
            state: DirectDeliveryState::ServerQueued,
            at,
            failure: None,
        }),
        DirectEnvelopeKind::Decision => Some(DirectLifecycleEvent::DecisionDelivery {
            request_id: request_id.clone(),
            state: DirectDecisionDeliveryState::ServerQueued,
            revision: entry.record.decision.revision,
            at,
            failure: None,
        }),
        DirectEnvelopeKind::RequestReceipt | DirectEnvelopeKind::DecisionReceipt => None,
    };
    if let Some(event) = event {
        entry.record.apply(event)?;
    }
    Ok(())
}

pub(super) fn require_request_receipt(
    request: &SignedDirectRequest,
    receipt: &SignedDirectRequestReceipt,
) -> Result<(), DirectLedgerError> {
    if receipt.request_id != request.request_id
        || receipt.lookup_id != request.lookup_id
        || receipt.requester != request.requester
        || receipt.request_digest != request.digest()?
        || !target_matches_pin(&receipt.target, &request.target)
    {
        Err(DirectLedgerError::EnvelopeConflict)
    } else {
        Ok(())
    }
}

pub(super) fn require_decision(
    request: &SignedDirectRequest,
    decision: &SignedDirectDecision,
) -> Result<(), DirectLedgerError> {
    if decision.request_id != request.request_id
        || decision.lookup_id != request.lookup_id
        || decision.requester != request.requester
        || decision.request_digest != request.digest()?
        || !target_matches_pin(&decision.target, &request.target)
    {
        Err(DirectLedgerError::EnvelopeConflict)
    } else {
        Ok(())
    }
}

pub(super) fn require_decision_receipt(
    decision: &SignedDirectDecision,
    receipt: &SignedDirectDecisionReceipt,
) -> Result<(), DirectLedgerError> {
    if receipt.request_id != decision.request_id
        || receipt.lookup_id != decision.lookup_id
        || receipt.requester != decision.requester
        || receipt.target != decision.target
        || receipt.decision != decision.decision
        || receipt.decision_revision != decision.decision_revision
        || receipt.decision_digest != decision.digest()?
    {
        Err(DirectLedgerError::EnvelopeConflict)
    } else {
        Ok(())
    }
}

fn target_matches_pin(
    target: &super::direct_protocol::DirectPeerIdentity,
    pin: &super::direct_protocol::DirectPeerIdentity,
) -> bool {
    target.node_id == pin.node_id
        && target.public_key == pin.public_key
        && target.fingerprint == pin.fingerprint
}
