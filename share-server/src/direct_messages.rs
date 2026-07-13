use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct DirectPeerIdentity {
    pub(super) device_id: String,
    pub(super) device_name: String,
    pub(super) node_id: String,
    pub(super) public_key: String,
    pub(super) fingerprint: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct SignedDirectRequest {
    pub(super) request_id: String,
    pub(super) lookup_id: String,
    pub(super) requester: DirectPeerIdentity,
    pub(super) target: DirectPeerIdentity,
    pub(super) created_at: i64,
    pub(super) expires_at: i64,
    pub(super) nonce: String,
    pub(super) message: Option<String>,
    pub(super) hmac_proof: String,
    pub(super) identity_signature: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct SignedDirectRequestReceipt {
    pub(super) request_id: String,
    pub(super) lookup_id: String,
    pub(super) requester: DirectPeerIdentity,
    pub(super) target: DirectPeerIdentity,
    pub(super) request_digest: String,
    pub(super) received_at: i64,
    pub(super) expires_at: i64,
    pub(super) nonce: String,
    pub(super) message: Option<String>,
    pub(super) hmac_proof: String,
    pub(super) identity_signature: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum DirectDecisionKind {
    Accepted,
    Rejected,
    Revoked,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct SignedDirectDecision {
    pub(super) request_id: String,
    pub(super) lookup_id: String,
    pub(super) requester: DirectPeerIdentity,
    pub(super) target: DirectPeerIdentity,
    pub(super) request_digest: String,
    pub(super) decision: DirectDecisionKind,
    pub(super) decision_revision: u64,
    pub(super) decided_at: i64,
    pub(super) expires_at: i64,
    pub(super) nonce: String,
    pub(super) message: Option<String>,
    pub(super) hmac_proof: String,
    pub(super) identity_signature: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct SignedDirectDecisionReceipt {
    pub(super) request_id: String,
    pub(super) lookup_id: String,
    pub(super) requester: DirectPeerIdentity,
    pub(super) target: DirectPeerIdentity,
    pub(super) decision_digest: String,
    pub(super) decision: DirectDecisionKind,
    pub(super) decision_revision: u64,
    pub(super) received_at: i64,
    pub(super) expires_at: i64,
    pub(super) nonce: String,
    pub(super) message: Option<String>,
    pub(super) hmac_proof: String,
    pub(super) identity_signature: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum DirectRoute {
    Request,
    RequestReceipt,
    Decision,
    DecisionReceipt,
}

/// A `forwarded` ACK means only that this relay accepted the message and
/// enqueued it to at least one currently connected, compatible client writer.
/// It is not proof that the peer socket received or persisted the payload.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum DirectRouteOutcome {
    Forwarded,
    LegacyForwarded,
    TargetOffline,
}
