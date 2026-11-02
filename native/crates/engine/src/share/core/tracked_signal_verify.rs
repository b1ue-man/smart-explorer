use super::direct_protocol::{
    DirectPeerIdentity, DirectProtocolError, SignedDirectDecision, SignedDirectDecisionReceipt,
    SignedDirectRequest, SignedDirectRequestReceipt,
};

pub(super) fn verify_request_for_target(
    request: &SignedDirectRequest,
    local_lookup_id: &str,
    local: &DirectPeerIdentity,
    relation_secret: &[u8],
    now: i64,
) -> Result<(), DirectProtocolError> {
    if request.lookup_id != local_lookup_id || !matches_local(&request.target, local, true) {
        return Err(DirectProtocolError::IdentityKeyMismatch);
    }
    request.verify_at(relation_secret, now)
}

pub(super) fn verify_request_receipt_for_requester(
    receipt: &SignedDirectRequestReceipt,
    request: &SignedDirectRequest,
    local: &DirectPeerIdentity,
    relation_secret: &[u8],
    now: i64,
) -> Result<(), DirectProtocolError> {
    require_full_local(&request.requester, local)?;
    receipt.verify_for(request, relation_secret, now)
}

pub(super) fn verify_decision_for_requester(
    decision: &SignedDirectDecision,
    request: &SignedDirectRequest,
    local: &DirectPeerIdentity,
    relation_secret: &[u8],
    now: i64,
) -> Result<(), DirectProtocolError> {
    require_full_local(&request.requester, local)?;
    decision.verify_for(request, relation_secret, now)
}

pub(super) fn verify_decision_receipt_for_target(
    receipt: &SignedDirectDecisionReceipt,
    decision: &SignedDirectDecision,
    local: &DirectPeerIdentity,
    relation_secret: &[u8],
    now: i64,
) -> Result<(), DirectProtocolError> {
    require_full_local(&decision.target, local)?;
    receipt.verify_for(decision, relation_secret, now)
}

fn require_full_local(
    peer: &DirectPeerIdentity,
    local: &DirectPeerIdentity,
) -> Result<(), DirectProtocolError> {
    if matches_local(peer, local, false) {
        Ok(())
    } else {
        Err(DirectProtocolError::IdentityKeyMismatch)
    }
}

fn matches_local(
    peer: &DirectPeerIdentity,
    local: &DirectPeerIdentity,
    allow_missing_id: bool,
) -> bool {
    peer.node_id == local.node_id
        && peer.public_key == local.public_key
        && peer.fingerprint == local.fingerprint
        && ((allow_missing_id && peer.device_id.is_empty()) || peer.device_id == local.device_id)
}
