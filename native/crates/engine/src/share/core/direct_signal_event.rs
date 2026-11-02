use serde::{Deserialize, Serialize};

use super::direct_ledger::{DirectEnvelopeKind, DirectRelayOutcome};
use super::direct_lifecycle::DirectFailure;
use super::direct_protocol::{
    DirectRequestId, SignedDirectDecision, SignedDirectDecisionReceipt, SignedDirectRequest,
    SignedDirectRequestReceipt,
};

/// Verified direct-request transport facts. The signal worker never mutates
/// the durable ledger itself; the daemon applies these events transactionally.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DirectSignalEvent {
    RequestReceived {
        request: SignedDirectRequest,
        received_at: i64,
    },
    RequestReceiptReceived {
        receipt: SignedDirectRequestReceipt,
        received_at: i64,
    },
    DecisionReceived {
        decision: SignedDirectDecision,
        received_at: i64,
    },
    DecisionReceiptReceived {
        receipt: SignedDirectDecisionReceipt,
        received_at: i64,
    },
    EnvelopeAttempted {
        request_id: DirectRequestId,
        envelope: DirectEnvelopeKind,
        attempt_count: u32,
        at: i64,
        failure: Option<DirectFailure>,
    },
    RelayAcknowledged {
        request_id: DirectRequestId,
        envelope: DirectEnvelopeKind,
        outcome: DirectRelayOutcome,
        at: i64,
    },
}
