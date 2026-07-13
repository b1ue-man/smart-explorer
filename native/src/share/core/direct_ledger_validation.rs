use std::collections::HashSet;

use super::direct_ledger::{
    DirectLedgerError, DirectRelayOutcome, DirectRequestDirection, DirectRequestEntry,
    DirectRetryState, MAX_DIRECT_REQUEST_ENTRIES,
};
use super::direct_ledger_mutations::{
    require_decision, require_decision_receipt, require_request_receipt,
};
use super::direct_lifecycle::{
    DirectDecisionDeliveryState, DirectDecisionState, DirectDeliveryState,
};
use super::direct_protocol::validate_persisted_timestamp;
use super::profiles::ShareProfiles;

impl ShareProfiles {
    pub(super) fn validate_direct_ledger(&self) -> Result<(), String> {
        if self.direct_requests.len() > MAX_DIRECT_REQUEST_ENTRIES {
            return Err(format!(
                "zu viele direkte Requests (maximal {MAX_DIRECT_REQUEST_ENTRIES})"
            ));
        }
        let mut request_ids = HashSet::with_capacity(self.direct_requests.len());
        for entry in &self.direct_requests {
            let request_id = entry.record.request.request_id.as_str();
            if !request_ids.insert(request_id) {
                return Err(format!("doppelte direkte Request-ID {request_id}"));
            }
            entry
                .validate_persisted()
                .map_err(|error| format!("ungueltiger direkter Request {request_id}: {error}"))?;
        }
        for tombstone in &self.direct_request_tombstones {
            let request = &tombstone.request;
            validate_persisted_timestamp(request.created_at, request.expires_at).map_err(
                |error| {
                    format!(
                        "ungueltiger geloeschter direkter Request {}: direct protocol: {error}",
                        request.request_id
                    )
                },
            )?;
        }
        self.validate_direct_request_tombstones()?;
        Ok(())
    }
}

impl DirectRequestEntry {
    fn validate_persisted(&self) -> Result<(), DirectLedgerError> {
        validate_persisted_timestamp(
            self.record.request.created_at,
            self.record.request.expires_at,
        )?;
        self.record.request.digest()?;
        self.validate_relation()?;
        self.validate_statuses()?;
        validate_retry(
            &self.retries.request,
            self.direction == DirectRequestDirection::Outgoing,
        )?;
        validate_retry(&self.retries.request_receipt, false)?;
        validate_retry(&self.retries.decision, false)?;
        validate_retry(&self.retries.decision_receipt, false)?;

        if let Some(receipt) = &self.request_receipt {
            validate_persisted_timestamp(receipt.received_at, receipt.expires_at)?;
            require_request_receipt(&self.record.request, receipt)?;
        }
        if let Some(decision) = &self.decision {
            validate_persisted_timestamp(decision.decided_at, decision.expires_at)?;
            require_decision(&self.record.request, decision)?;
            if decision.decision_revision != self.record.decision.revision
                || self.record.decision.state != decision.decision.into()
            {
                return Err(DirectLedgerError::EnvelopeConflict);
            }
        } else if !matches!(
            self.record.decision.state,
            DirectDecisionState::Pending
                | DirectDecisionState::Failed
                | DirectDecisionState::Expired
        ) {
            return Err(DirectLedgerError::MissingEnvelope);
        }
        if let Some(receipt) = &self.decision_receipt {
            validate_persisted_timestamp(receipt.received_at, receipt.expires_at)?;
            let decision = self
                .decision
                .as_ref()
                .ok_or(DirectLedgerError::MissingEnvelope)?;
            require_decision_receipt(decision, receipt)?;
        }
        Ok(())
    }

    fn validate_relation(&self) -> Result<(), DirectLedgerError> {
        let valid = match self.direction {
            DirectRequestDirection::Outgoing => {
                self.contact_id.as_ref().is_some_and(|id| !id.is_empty())
                    && self.local_lookup_id.is_none()
            }
            DirectRequestDirection::Incoming => {
                self.contact_id.is_none()
                    && self.local_lookup_id.as_deref()
                        == Some(self.record.request.lookup_id.as_str())
            }
        };
        if valid {
            Ok(())
        } else {
            Err(DirectLedgerError::InvalidRelation)
        }
    }

    fn validate_statuses(&self) -> Result<(), DirectLedgerError> {
        let request = &self.record.request;
        let delivery = &self.record.delivery;
        if delivery.changed_at < request.created_at
            || (delivery.state == DirectDeliveryState::Expired
                && delivery.changed_at < request.expires_at)
            || (delivery.state != DirectDeliveryState::Expired
                && delivery.changed_at > request.expires_at)
        {
            return Err(DirectLedgerError::InvalidTimestamp);
        }
        let decision = &self.record.decision;
        if decision.changed_at < request.created_at
            || (decision.state == DirectDecisionState::Expired
                && decision.changed_at < request.expires_at)
        {
            return Err(DirectLedgerError::InvalidTimestamp);
        }
        if decision.state == DirectDecisionState::Pending && decision.revision != 0 {
            return Err(DirectLedgerError::EnvelopeConflict);
        }
        let delivery = &self.record.decision_delivery;
        if delivery.state == DirectDecisionDeliveryState::NotStarted {
            if delivery.revision != 0 {
                return Err(DirectLedgerError::EnvelopeConflict);
            }
        } else if delivery.revision == 0 || delivery.changed_at < decision.changed_at {
            return Err(DirectLedgerError::EnvelopeConflict);
        }
        Ok(())
    }
}

fn validate_retry(
    retry: &DirectRetryState,
    legacy_forwarding_allowed: bool,
) -> Result<(), DirectLedgerError> {
    if retry.relay_outcome == Some(DirectRelayOutcome::LegacyForwarded)
        && !legacy_forwarding_allowed
    {
        return Err(DirectLedgerError::EnvelopeConflict);
    }
    if retry.last_attempt_at.is_some_and(|at| at < 0)
        || retry.relay_changed_at.is_some_and(|at| at < 0)
        || (retry.attempt_count == 0 && retry.last_attempt_at.is_some())
        || (retry.attempt_count > 0 && retry.last_attempt_at.is_none())
        || retry.relay_outcome.is_some() != retry.relay_changed_at.is_some()
    {
        Err(DirectLedgerError::InvalidAttempt)
    } else {
        Ok(())
    }
}
