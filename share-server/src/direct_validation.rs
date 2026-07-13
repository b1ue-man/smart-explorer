use super::limits::{validate_identifier, validate_presence, RetainError};
use super::tracked_direct::{
    DirectPeerIdentity, SignedDirectDecision, SignedDirectDecisionReceipt, SignedDirectRequest,
    SignedDirectRequestReceipt,
};
use super::PeerPresence;

const MAX_NAME_BYTES: usize = 1024;
const MAX_CRYPTO_FIELD_BYTES: usize = 1024;
const MAX_DIGEST_BYTES: usize = 256;
const MAX_MESSAGE_BYTES: usize = 4096;
const MAX_SIGNED_ENVELOPE_TEXT_BYTES: usize = 32 * 1024;

pub(super) fn validate_request(request: &SignedDirectRequest) -> Result<(), RetainError> {
    validate_identifier("request id", &request.request_id)?;
    validate_identifier("lookup id", &request.lookup_id)?;
    validate_identity(&request.requester)?;
    validate_identity(&request.target)?;
    validate_identifier("nonce", &request.nonce)?;
    validate_optional_message(request.message.as_deref())?;
    validate_crypto("HMAC proof", &request.hmac_proof)?;
    validate_crypto("identity signature", &request.identity_signature)?;
    validate_total([
        request.request_id.as_str(),
        request.lookup_id.as_str(),
        request.requester.device_id.as_str(),
        request.requester.device_name.as_str(),
        request.requester.node_id.as_str(),
        request.requester.public_key.as_str(),
        request.requester.fingerprint.as_str(),
        request.target.device_id.as_str(),
        request.target.device_name.as_str(),
        request.target.node_id.as_str(),
        request.target.public_key.as_str(),
        request.target.fingerprint.as_str(),
        request.nonce.as_str(),
        request.message.as_deref().unwrap_or_default(),
        request.hmac_proof.as_str(),
        request.identity_signature.as_str(),
    ])
}

pub(super) fn validate_legacy_bridge(
    request: &SignedDirectRequest,
    presence: &PeerPresence,
) -> Result<(), RetainError> {
    validate_presence(presence)?;
    if presence.kind != "direct"
        || presence.relation_id != request.lookup_id
        || presence.device_id != request.requester.device_id
        || presence.device_name != request.requester.device_name
        || presence.node_id != request.requester.node_id
        || presence.public_key != request.requester.public_key
        || presence.fingerprint != request.requester.fingerprint
    {
        return Err(RetainError::InvalidField("legacy request bridge"));
    }
    Ok(())
}

pub(super) fn validate_request_receipt(
    receipt: &SignedDirectRequestReceipt,
) -> Result<(), RetainError> {
    validate_identifier("request id", &receipt.request_id)?;
    validate_identifier("lookup id", &receipt.lookup_id)?;
    validate_identity(&receipt.requester)?;
    validate_identity(&receipt.target)?;
    validate_digest("request digest", &receipt.request_digest)?;
    validate_identifier("nonce", &receipt.nonce)?;
    validate_optional_message(receipt.message.as_deref())?;
    validate_crypto("HMAC proof", &receipt.hmac_proof)?;
    validate_crypto("identity signature", &receipt.identity_signature)?;
    validate_common_total(
        &receipt.request_id,
        &receipt.lookup_id,
        &receipt.requester,
        &receipt.target,
        &receipt.request_digest,
        &receipt.nonce,
        receipt.message.as_deref(),
        &receipt.hmac_proof,
        &receipt.identity_signature,
    )
}

pub(super) fn validate_decision(decision: &SignedDirectDecision) -> Result<(), RetainError> {
    validate_identifier("request id", &decision.request_id)?;
    validate_identifier("lookup id", &decision.lookup_id)?;
    validate_identity(&decision.requester)?;
    validate_identity(&decision.target)?;
    validate_digest("request digest", &decision.request_digest)?;
    validate_identifier("nonce", &decision.nonce)?;
    validate_optional_message(decision.message.as_deref())?;
    validate_crypto("HMAC proof", &decision.hmac_proof)?;
    validate_crypto("identity signature", &decision.identity_signature)?;
    validate_common_total(
        &decision.request_id,
        &decision.lookup_id,
        &decision.requester,
        &decision.target,
        &decision.request_digest,
        &decision.nonce,
        decision.message.as_deref(),
        &decision.hmac_proof,
        &decision.identity_signature,
    )
}

pub(super) fn validate_decision_receipt(
    receipt: &SignedDirectDecisionReceipt,
) -> Result<(), RetainError> {
    validate_identifier("request id", &receipt.request_id)?;
    validate_identifier("lookup id", &receipt.lookup_id)?;
    validate_identity(&receipt.requester)?;
    validate_identity(&receipt.target)?;
    validate_digest("decision digest", &receipt.decision_digest)?;
    validate_identifier("nonce", &receipt.nonce)?;
    validate_optional_message(receipt.message.as_deref())?;
    validate_crypto("HMAC proof", &receipt.hmac_proof)?;
    validate_crypto("identity signature", &receipt.identity_signature)?;
    validate_common_total(
        &receipt.request_id,
        &receipt.lookup_id,
        &receipt.requester,
        &receipt.target,
        &receipt.decision_digest,
        &receipt.nonce,
        receipt.message.as_deref(),
        &receipt.hmac_proof,
        &receipt.identity_signature,
    )
}

pub(super) fn validate_legacy_request(
    lookup_id: &str,
    presence: &PeerPresence,
) -> Result<(), RetainError> {
    validate_identifier("lookup id", lookup_id)?;
    validate_presence(presence)
}

pub(super) fn validate_legacy_decision(
    lookup_id: &str,
    requester_device_id: &str,
    presence: Option<&PeerPresence>,
    message: Option<&str>,
) -> Result<(), RetainError> {
    validate_identifier("lookup id", lookup_id)?;
    validate_identifier("requester device id", requester_device_id)?;
    if let Some(presence) = presence {
        validate_presence(presence)?;
    }
    validate_optional_message(message)
}

fn validate_identity(identity: &DirectPeerIdentity) -> Result<(), RetainError> {
    validate_optional_id("device id", &identity.device_id)?;
    validate_text("device name", &identity.device_name, MAX_NAME_BYTES, true)?;
    validate_optional_id("node id", &identity.node_id)?;
    validate_optional_id("public key", &identity.public_key)?;
    validate_optional_id("fingerprint", &identity.fingerprint)
}

fn validate_optional_id(field: &'static str, value: &str) -> Result<(), RetainError> {
    validate_text(field, value, 256, true)
}

fn validate_crypto(field: &'static str, value: &str) -> Result<(), RetainError> {
    validate_text(field, value, MAX_CRYPTO_FIELD_BYTES, false)
}

fn validate_digest(field: &'static str, value: &str) -> Result<(), RetainError> {
    validate_text(field, value, MAX_DIGEST_BYTES, false)
}

fn validate_optional_message(message: Option<&str>) -> Result<(), RetainError> {
    validate_text(
        "message",
        message.unwrap_or_default(),
        MAX_MESSAGE_BYTES,
        true,
    )
}

fn validate_text(
    field: &'static str,
    value: &str,
    max: usize,
    allow_empty: bool,
) -> Result<(), RetainError> {
    if (!allow_empty && value.is_empty())
        || value.len() > max
        || value.chars().any(char::is_control)
    {
        Err(RetainError::InvalidField(field))
    } else {
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_common_total(
    request_id: &str,
    lookup_id: &str,
    requester: &DirectPeerIdentity,
    target: &DirectPeerIdentity,
    digest: &str,
    nonce: &str,
    message: Option<&str>,
    hmac_proof: &str,
    identity_signature: &str,
) -> Result<(), RetainError> {
    validate_total([
        request_id,
        lookup_id,
        &requester.device_id,
        &requester.device_name,
        &requester.node_id,
        &requester.public_key,
        &requester.fingerprint,
        &target.device_id,
        &target.device_name,
        &target.node_id,
        &target.public_key,
        &target.fingerprint,
        digest,
        nonce,
        message.unwrap_or_default(),
        hmac_proof,
        identity_signature,
    ])
}

fn validate_total<const N: usize>(fields: [&str; N]) -> Result<(), RetainError> {
    let total = fields
        .into_iter()
        .fold(0_usize, |total, field| total.saturating_add(field.len()));
    if total > MAX_SIGNED_ENVELOPE_TEXT_BYTES {
        Err(RetainError::InvalidField("signed envelope"))
    } else {
        Ok(())
    }
}
