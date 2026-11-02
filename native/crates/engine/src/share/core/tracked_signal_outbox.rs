use super::direct_ledger::{DirectEnvelopeKind, DirectRequestEntry, DirectRetryState};
use super::direct_protocol::{
    DirectRequestId, SignedDirectDecision, SignedDirectDecisionReceipt, SignedDirectRequest,
    SignedDirectRequestReceipt,
};
use super::wire::TrackedDirectClientMsg;

const MAX_RETRY_DELAY_SECS: i64 = 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum TrackedOutboxEnvelope {
    Request(SignedDirectRequest),
    RequestReceipt(SignedDirectRequestReceipt),
    Decision(SignedDirectDecision),
    DecisionReceipt(SignedDirectDecisionReceipt),
}

impl TrackedOutboxEnvelope {
    pub(super) fn request_id(&self) -> &DirectRequestId {
        match self {
            Self::Request(value) => &value.request_id,
            Self::RequestReceipt(value) => &value.request_id,
            Self::Decision(value) => &value.request_id,
            Self::DecisionReceipt(value) => &value.request_id,
        }
    }

    pub(super) fn kind(&self) -> DirectEnvelopeKind {
        match self {
            Self::Request(_) => DirectEnvelopeKind::Request,
            Self::RequestReceipt(_) => DirectEnvelopeKind::RequestReceipt,
            Self::Decision(_) => DirectEnvelopeKind::Decision,
            Self::DecisionReceipt(_) => DirectEnvelopeKind::DecisionReceipt,
        }
    }

    pub(super) fn wire_message(
        &self,
        legacy_presence: Option<super::types::PeerPresence>,
    ) -> TrackedDirectClientMsg {
        match self {
            Self::Request(request) => TrackedDirectClientMsg::Request {
                request: Box::new(request.clone()),
                legacy_presence,
            },
            Self::RequestReceipt(receipt) => TrackedDirectClientMsg::RequestReceipt {
                receipt: receipt.clone(),
            },
            Self::Decision(decision) => TrackedDirectClientMsg::Decision {
                decision: decision.clone(),
            },
            Self::DecisionReceipt(receipt) => TrackedDirectClientMsg::DecisionReceipt {
                receipt: receipt.clone(),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PendingTrackedEnvelope {
    pub(super) envelope: TrackedOutboxEnvelope,
    pub(super) persisted_attempt_count: u32,
}

pub(super) fn pending_envelopes(
    entries: &[DirectRequestEntry],
    now: i64,
) -> Vec<PendingTrackedEnvelope> {
    let mut pending = Vec::new();
    for entry in entries {
        for kind in entry.pending_outboxes(now) {
            let retry = entry.retry(kind);
            if retry_due(retry, now) {
                if let Some(envelope) = envelope(entry, kind) {
                    push(&mut pending, envelope, retry);
                }
            }
        }
    }
    pending
}

fn envelope(entry: &DirectRequestEntry, kind: DirectEnvelopeKind) -> Option<TrackedOutboxEnvelope> {
    Some(match kind {
        DirectEnvelopeKind::Request => TrackedOutboxEnvelope::Request(entry.record.request.clone()),
        DirectEnvelopeKind::RequestReceipt => {
            TrackedOutboxEnvelope::RequestReceipt(entry.request_receipt.clone()?)
        }
        DirectEnvelopeKind::Decision => TrackedOutboxEnvelope::Decision(entry.decision.clone()?),
        DirectEnvelopeKind::DecisionReceipt => {
            TrackedOutboxEnvelope::DecisionReceipt(entry.decision_receipt.clone()?)
        }
    })
}

fn push(
    pending: &mut Vec<PendingTrackedEnvelope>,
    envelope: TrackedOutboxEnvelope,
    retry: &DirectRetryState,
) {
    pending.push(PendingTrackedEnvelope {
        envelope,
        persisted_attempt_count: retry.attempt_count,
    });
}

fn retry_due(retry: &DirectRetryState, now: i64) -> bool {
    let Some(last_attempt) = retry.last_attempt_at else {
        return true;
    };
    let exponent = retry.attempt_count.saturating_sub(1).min(6);
    let delay = (1_i64 << exponent).min(MAX_RETRY_DELAY_SECS);
    now.saturating_sub(last_attempt) >= delay
}
