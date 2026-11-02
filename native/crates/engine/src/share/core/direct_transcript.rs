use super::core::{
    hmac_proof_bytes, iroh_signature, sha256_b64, verify_hmac_bytes, verify_iroh_signature,
};
use super::direct_protocol::{
    DirectPeerIdentity, DirectProtocolError, SignedDirectDecision, SignedDirectDecisionReceipt,
    SignedDirectRequest, SignedDirectRequestReceipt,
};

const TRANSCRIPT_PREFIX: &[u8] = b"smart-explorer/direct/tracked-v1";
const REQUEST_DOMAIN: &[u8] = b"request";
const REQUEST_RECEIPT_DOMAIN: &[u8] = b"request-receipt";
const DECISION_DOMAIN: &[u8] = b"decision";
const DECISION_RECEIPT_DOMAIN: &[u8] = b"decision-receipt";
const RELATION_SECRET_BYTES: usize = 32;

pub(super) fn seal_request(
    request: &mut SignedDirectRequest,
    relation_secret: &[u8],
    signer: &iroh::SecretKey,
) -> Result<(), DirectProtocolError> {
    seal(
        request_transcript(request),
        relation_secret,
        signer,
        &request.requester,
        &mut request.hmac_proof,
        &mut request.identity_signature,
    )
}

pub(super) fn verify_request(
    request: &SignedDirectRequest,
    relation_secret: &[u8],
) -> Result<(), DirectProtocolError> {
    verify(
        request_transcript(request),
        relation_secret,
        &request.requester,
        &request.hmac_proof,
        &request.identity_signature,
    )
}

pub(super) fn seal_request_receipt(
    receipt: &mut SignedDirectRequestReceipt,
    relation_secret: &[u8],
    signer: &iroh::SecretKey,
) -> Result<(), DirectProtocolError> {
    seal(
        request_receipt_transcript(receipt),
        relation_secret,
        signer,
        &receipt.target,
        &mut receipt.hmac_proof,
        &mut receipt.identity_signature,
    )
}

pub(super) fn verify_request_receipt(
    receipt: &SignedDirectRequestReceipt,
    relation_secret: &[u8],
) -> Result<(), DirectProtocolError> {
    verify(
        request_receipt_transcript(receipt),
        relation_secret,
        &receipt.target,
        &receipt.hmac_proof,
        &receipt.identity_signature,
    )
}

pub(super) fn seal_decision(
    decision: &mut SignedDirectDecision,
    relation_secret: &[u8],
    signer: &iroh::SecretKey,
) -> Result<(), DirectProtocolError> {
    seal(
        decision_transcript(decision),
        relation_secret,
        signer,
        &decision.target,
        &mut decision.hmac_proof,
        &mut decision.identity_signature,
    )
}

pub(super) fn verify_decision(
    decision: &SignedDirectDecision,
    relation_secret: &[u8],
) -> Result<(), DirectProtocolError> {
    verify(
        decision_transcript(decision),
        relation_secret,
        &decision.target,
        &decision.hmac_proof,
        &decision.identity_signature,
    )
}

pub(super) fn seal_decision_receipt(
    receipt: &mut SignedDirectDecisionReceipt,
    relation_secret: &[u8],
    signer: &iroh::SecretKey,
) -> Result<(), DirectProtocolError> {
    seal(
        decision_receipt_transcript(receipt),
        relation_secret,
        signer,
        &receipt.requester,
        &mut receipt.hmac_proof,
        &mut receipt.identity_signature,
    )
}

pub(super) fn verify_decision_receipt(
    receipt: &SignedDirectDecisionReceipt,
    relation_secret: &[u8],
) -> Result<(), DirectProtocolError> {
    verify(
        decision_receipt_transcript(receipt),
        relation_secret,
        &receipt.requester,
        &receipt.hmac_proof,
        &receipt.identity_signature,
    )
}

pub(super) fn request_digest(request: &SignedDirectRequest) -> String {
    sha256_b64(request_transcript(request).as_bytes())
}

pub(super) fn decision_digest(decision: &SignedDirectDecision) -> String {
    sha256_b64(decision_transcript(decision).as_bytes())
}

fn seal(
    transcript: Transcript,
    relation_secret: &[u8],
    signer: &iroh::SecretKey,
    identity: &DirectPeerIdentity,
    hmac: &mut String,
    signature: &mut String,
) -> Result<(), DirectProtocolError> {
    require_relation_secret(relation_secret)?;
    if signer.public().to_string() != identity.public_key {
        return Err(DirectProtocolError::SignerMismatch);
    }
    *hmac = hmac_proof_bytes(relation_secret, transcript.as_bytes());
    *signature = iroh_signature(signer, transcript.as_bytes());
    Ok(())
}

fn verify(
    transcript: Transcript,
    relation_secret: &[u8],
    identity: &DirectPeerIdentity,
    hmac: &str,
    signature: &str,
) -> Result<(), DirectProtocolError> {
    require_relation_secret(relation_secret)?;
    if !verify_hmac_bytes(relation_secret, transcript.as_bytes(), hmac) {
        return Err(DirectProtocolError::InvalidHmac);
    }
    if !verify_iroh_signature(&identity.public_key, transcript.as_bytes(), signature) {
        return Err(DirectProtocolError::InvalidSignature);
    }
    Ok(())
}

fn require_relation_secret(secret: &[u8]) -> Result<(), DirectProtocolError> {
    if secret.len() == RELATION_SECRET_BYTES {
        Ok(())
    } else {
        Err(DirectProtocolError::InvalidField("relation_secret"))
    }
}

fn request_transcript(request: &SignedDirectRequest) -> Transcript {
    let mut out = Transcript::new(REQUEST_DOMAIN);
    out.text("request_id", request.request_id.as_str());
    out.text("lookup_id", &request.lookup_id);
    out.identity("requester", &request.requester);
    out.identity("target", &request.target);
    out.i64("created_at", request.created_at);
    out.i64("expires_at", request.expires_at);
    out.text("nonce", &request.nonce);
    out.optional_text("message", request.message.as_deref());
    out
}

fn request_receipt_transcript(receipt: &SignedDirectRequestReceipt) -> Transcript {
    let mut out = Transcript::new(REQUEST_RECEIPT_DOMAIN);
    out.text("request_id", receipt.request_id.as_str());
    out.text("lookup_id", &receipt.lookup_id);
    out.identity("requester", &receipt.requester);
    out.identity("target", &receipt.target);
    out.text("request_digest", &receipt.request_digest);
    out.i64("received_at", receipt.received_at);
    out.i64("expires_at", receipt.expires_at);
    out.text("nonce", &receipt.nonce);
    out.optional_text("message", receipt.message.as_deref());
    out
}

fn decision_transcript(decision: &SignedDirectDecision) -> Transcript {
    let mut out = Transcript::new(DECISION_DOMAIN);
    out.text("request_id", decision.request_id.as_str());
    out.text("lookup_id", &decision.lookup_id);
    out.identity("requester", &decision.requester);
    out.identity("target", &decision.target);
    out.text("request_digest", &decision.request_digest);
    out.text("decision", decision.decision.code());
    out.u64("decision_revision", decision.decision_revision);
    out.i64("decided_at", decision.decided_at);
    out.i64("expires_at", decision.expires_at);
    out.text("nonce", &decision.nonce);
    out.optional_text("message", decision.message.as_deref());
    out
}

fn decision_receipt_transcript(receipt: &SignedDirectDecisionReceipt) -> Transcript {
    let mut out = Transcript::new(DECISION_RECEIPT_DOMAIN);
    out.text("request_id", receipt.request_id.as_str());
    out.text("lookup_id", &receipt.lookup_id);
    out.identity("requester", &receipt.requester);
    out.identity("target", &receipt.target);
    out.text("decision_digest", &receipt.decision_digest);
    out.text("decision", receipt.decision.code());
    out.u64("decision_revision", receipt.decision_revision);
    out.i64("received_at", receipt.received_at);
    out.i64("expires_at", receipt.expires_at);
    out.text("nonce", &receipt.nonce);
    out.optional_text("message", receipt.message.as_deref());
    out
}

struct Transcript {
    bytes: Vec<u8>,
}

impl Transcript {
    fn new(domain: &[u8]) -> Self {
        let mut transcript = Self { bytes: Vec::new() };
        transcript.bytes("protocol", TRANSCRIPT_PREFIX);
        transcript.bytes("domain", domain);
        transcript
    }

    fn identity(&mut self, role: &str, identity: &DirectPeerIdentity) {
        self.text(&format!("{role}.device_id"), &identity.device_id);
        self.text(&format!("{role}.device_name"), &identity.device_name);
        self.text(&format!("{role}.node_id"), &identity.node_id);
        self.text(&format!("{role}.public_key"), &identity.public_key);
        self.text(&format!("{role}.fingerprint"), &identity.fingerprint);
    }

    fn text(&mut self, label: &str, value: &str) {
        self.bytes(label, value.as_bytes());
    }

    fn optional_text(&mut self, label: &str, value: Option<&str>) {
        self.bytes(&format!("{label}.present"), &[u8::from(value.is_some())]);
        if let Some(value) = value {
            self.text(label, value);
        }
    }

    fn i64(&mut self, label: &str, value: i64) {
        self.bytes(label, &value.to_be_bytes());
    }

    fn u64(&mut self, label: &str, value: u64) {
        self.bytes(label, &value.to_be_bytes());
    }

    fn bytes(&mut self, label: &str, value: &[u8]) {
        put_len(&mut self.bytes, label.len());
        self.bytes.extend_from_slice(label.as_bytes());
        put_len(&mut self.bytes, value.len());
        self.bytes.extend_from_slice(value);
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

fn put_len(output: &mut Vec<u8>, len: usize) {
    let len = u64::try_from(len).expect("bounded direct transcript field length fits u64");
    output.extend_from_slice(&len.to_be_bytes());
}
