use serde::{Deserialize, Serialize};
use std::fmt;

use super::direct_lifecycle::{
    DirectDecisionState, DirectDeliveryState, DirectFailure, DirectLifecycleEvent,
    DirectRequestRecord,
};
use super::direct_lifecycle_error::DirectLifecycleError;
use super::direct_protocol::{
    DirectProtocolError, SignedDirectDecision, SignedDirectDecisionReceipt, SignedDirectRequest,
    SignedDirectRequestReceipt,
};

// A fully materialized tracked entry can retain four signed envelopes and
// bounded messages. Keeping at most 24 leaves headroom under the 1 MiB profile
// file limit even for adversarial-but-valid invite-secret traffic.
pub const MAX_DIRECT_REQUEST_ENTRIES: usize = 24;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DirectRequestDirection {
    Outgoing,
    Incoming,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DirectEnvelopeKind {
    Request,
    RequestReceipt,
    Decision,
    DecisionReceipt,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DirectRelayOutcome {
    Forwarded,
    LegacyForwarded,
    TargetOffline,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectRetryState {
    #[serde(default)]
    pub attempt_count: u32,
    #[serde(default)]
    pub last_attempt_at: Option<i64>,
    #[serde(default)]
    pub relay_outcome: Option<DirectRelayOutcome>,
    #[serde(default)]
    pub relay_changed_at: Option<i64>,
    #[serde(default)]
    pub last_error: Option<DirectFailure>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectRequestRetries {
    #[serde(default)]
    pub request: DirectRetryState,
    #[serde(default)]
    pub request_receipt: DirectRetryState,
    #[serde(default)]
    pub decision: DirectRetryState,
    #[serde(default)]
    pub decision_receipt: DirectRetryState,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectRequestEntry {
    pub direction: DirectRequestDirection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_lookup_id: Option<String>,
    pub record: DirectRequestRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_receipt: Option<SignedDirectRequestReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<SignedDirectDecision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_receipt: Option<SignedDirectDecisionReceipt>,
    #[serde(default)]
    pub retries: DirectRequestRetries,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectLedgerError {
    UnknownRequest,
    RequestIdConflict,
    EnvelopeConflict,
    InvalidRelation,
    WrongDirection,
    MissingEnvelope,
    InvalidAttempt,
    InvalidTimestamp,
    LedgerFull,
    TombstoneFull,
    ActiveGrantRequiresRevoke,
    PendingPeerDelivery,
    IdentityConflict,
    Protocol(DirectProtocolError),
    Lifecycle(DirectLifecycleError),
}

impl fmt::Display for DirectLedgerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Protocol(error) => write!(formatter, "direct protocol: {error}"),
            Self::Lifecycle(error) => write!(formatter, "direct lifecycle: {error}"),
            other => formatter.write_str(match other {
                Self::UnknownRequest => "unknown direct request",
                Self::RequestIdConflict => "direct request ID is already bound to other data",
                Self::EnvelopeConflict => "direct request envelope conflicts with stored data",
                Self::InvalidRelation => "invalid direct request relation",
                Self::WrongDirection => "direct request has the wrong direction",
                Self::MissingEnvelope => "direct request envelope is not available",
                Self::InvalidAttempt => "invalid direct request attempt",
                Self::InvalidTimestamp => "invalid direct request timestamp",
                Self::LedgerFull => "direct request ledger is full",
                Self::TombstoneFull => "direct request deletion tombstone ledger is full",
                Self::ActiveGrantRequiresRevoke => {
                    "accepted incoming request cannot be deleted while its authorization is active; revoke the grant first"
                }
                Self::PendingPeerDelivery => {
                    "direct request still has pending peer delivery; wait for the signed receipt before deleting it"
                }
                Self::IdentityConflict => {
                    "direct request has an identity conflict and cannot be accepted"
                }
                Self::Protocol(_) | Self::Lifecycle(_) => unreachable!(),
            }),
        }
    }
}

impl std::error::Error for DirectLedgerError {}

impl From<DirectProtocolError> for DirectLedgerError {
    fn from(error: DirectProtocolError) -> Self {
        Self::Protocol(error)
    }
}

impl From<DirectLifecycleError> for DirectLedgerError {
    fn from(error: DirectLifecycleError) -> Self {
        Self::Lifecycle(error)
    }
}

impl DirectRequestEntry {
    pub(super) fn outgoing(contact_id: String, request: SignedDirectRequest) -> Self {
        Self {
            direction: DirectRequestDirection::Outgoing,
            contact_id: Some(contact_id),
            local_lookup_id: None,
            record: DirectRequestRecord::new(request),
            request_receipt: None,
            decision: None,
            decision_receipt: None,
            retries: DirectRequestRetries::default(),
        }
    }

    pub(super) fn incoming(
        local_lookup_id: String,
        request: SignedDirectRequest,
        received_at: i64,
    ) -> Result<Self, DirectLedgerError> {
        let request_id = request.request_id.clone();
        let mut entry = Self {
            direction: DirectRequestDirection::Incoming,
            contact_id: None,
            local_lookup_id: Some(local_lookup_id),
            record: DirectRequestRecord::new(request),
            request_receipt: None,
            decision: None,
            decision_receipt: None,
            retries: DirectRequestRetries::default(),
        };
        entry.record.apply(DirectLifecycleEvent::Delivery {
            request_id,
            state: DirectDeliveryState::Received,
            at: received_at,
            failure: None,
        })?;
        Ok(entry)
    }

    pub fn retry(&self, kind: DirectEnvelopeKind) -> &DirectRetryState {
        match kind {
            DirectEnvelopeKind::Request => &self.retries.request,
            DirectEnvelopeKind::RequestReceipt => &self.retries.request_receipt,
            DirectEnvelopeKind::Decision => &self.retries.decision,
            DirectEnvelopeKind::DecisionReceipt => &self.retries.decision_receipt,
        }
    }

    /// Returns the durable envelopes that still need transport work.
    /// Requests and decisions require signed peer receipts. Receipt envelopes
    /// stop after relay forwarding because there is no ACK-of-ACK; a duplicate
    /// source envelope requeues the retained signed receipt for self-healing.
    pub fn pending_outboxes(&self, now: i64) -> Vec<DirectEnvelopeKind> {
        let mut pending = Vec::with_capacity(2);
        match self.direction {
            DirectRequestDirection::Outgoing => {
                let request_pending = now <= self.record.request.expires_at
                    && self.request_receipt.is_none()
                    && self.decision.is_none()
                    && self.retries.request.relay_outcome
                        != Some(DirectRelayOutcome::LegacyForwarded);
                if request_pending {
                    pending.push(DirectEnvelopeKind::Request);
                }
                if let Some(receipt) = &self.decision_receipt {
                    let retry = self.retry(DirectEnvelopeKind::DecisionReceipt);
                    if now <= receipt.expires_at
                        && retry.relay_outcome != Some(DirectRelayOutcome::Forwarded)
                    {
                        pending.push(DirectEnvelopeKind::DecisionReceipt);
                    }
                }
            }
            DirectRequestDirection::Incoming => {
                if let Some(receipt) = &self.request_receipt {
                    let retry = self.retry(DirectEnvelopeKind::RequestReceipt);
                    if now <= receipt.expires_at
                        && self.decision.is_none()
                        && retry.relay_outcome != Some(DirectRelayOutcome::Forwarded)
                    {
                        pending.push(DirectEnvelopeKind::RequestReceipt);
                    }
                }
                if let Some(decision) = &self.decision {
                    if now <= decision.expires_at && self.decision_receipt.is_none() {
                        pending.push(DirectEnvelopeKind::Decision);
                    }
                }
            }
        }
        pending
    }

    /// Returns envelopes a user may explicitly requeue. A successful legacy
    /// bridge stops automatic retries because that protocol has no receipt, but
    /// it must remain manually retryable when the user knows delivery was lost.
    pub fn manually_retryable_outboxes(&self, now: i64) -> Vec<DirectEnvelopeKind> {
        let mut retryable = self.pending_outboxes(now);
        if self.direction == DirectRequestDirection::Outgoing
            && now <= self.record.request.expires_at
            && self.request_receipt.is_none()
            && self.decision.is_none()
            && self.retries.request.relay_outcome == Some(DirectRelayOutcome::LegacyForwarded)
        {
            retryable.push(DirectEnvelopeKind::Request);
        }
        retryable
    }

    /// History is removable only after every required peer-facing envelope is
    /// finished and the decision can no longer represent active authorization.
    pub fn removable_from_history(&self, now: i64) -> bool {
        self.pending_outboxes(now).is_empty()
            && (matches!(
                self.record.decision.state,
                DirectDecisionState::Rejected
                    | DirectDecisionState::Revoked
                    | DirectDecisionState::Failed
                    | DirectDecisionState::Expired
            ) || (self.record.decision.state == DirectDecisionState::Pending
                && self.record.request.expires_at < now))
    }

    pub(super) fn retry_mut(&mut self, kind: DirectEnvelopeKind) -> &mut DirectRetryState {
        match kind {
            DirectEnvelopeKind::Request => &mut self.retries.request,
            DirectEnvelopeKind::RequestReceipt => &mut self.retries.request_receipt,
            DirectEnvelopeKind::Decision => &mut self.retries.decision,
            DirectEnvelopeKind::DecisionReceipt => &mut self.retries.decision_receipt,
        }
    }

    pub(super) fn has_outbox(&self, kind: DirectEnvelopeKind) -> bool {
        match (self.direction, kind) {
            (DirectRequestDirection::Outgoing, DirectEnvelopeKind::Request) => true,
            (DirectRequestDirection::Incoming, DirectEnvelopeKind::RequestReceipt) => {
                self.request_receipt.is_some()
            }
            (DirectRequestDirection::Incoming, DirectEnvelopeKind::Decision) => {
                self.decision.is_some()
            }
            (DirectRequestDirection::Outgoing, DirectEnvelopeKind::DecisionReceipt) => {
                self.decision_receipt.is_some()
            }
            _ => false,
        }
    }

    pub(super) fn requeue_forwarded_receipt(&mut self, kind: DirectEnvelopeKind) -> bool {
        if self.retry(kind).relay_outcome != Some(DirectRelayOutcome::Forwarded) {
            return false;
        }
        *self.retry_mut(kind) = DirectRetryState::default();
        true
    }
}
