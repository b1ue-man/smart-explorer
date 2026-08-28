use serde::{Deserialize, Serialize};
use std::fmt;

use super::core::{public_fingerprint, random_token, random_uuid_v4};
use super::direct_transcript;

const MAX_ID_BYTES: usize = 256;
const MAX_NAME_BYTES: usize = 1024;
const MAX_NONCE_BYTES: usize = 256;
const MAX_MESSAGE_BYTES: usize = 4096;
pub(crate) const MAX_TRACKED_DIRECT_ENVELOPE_LIFETIME_SECS: i64 = 30 * 24 * 60 * 60;
pub(crate) const MAX_TRACKED_DIRECT_CLOCK_SKEW_SECS: i64 = 5 * 60;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DirectRequestId(String);

impl DirectRequestId {
    pub fn generate() -> Result<Self, DirectProtocolError> {
        random_uuid_v4()
            .map_err(|_| DirectProtocolError::EntropyUnavailable)
            .and_then(Self::parse)
    }

    pub fn parse(value: impl AsRef<str>) -> Result<Self, DirectProtocolError> {
        let value = value.as_ref();
        let bytes = value.as_bytes();
        let valid_shape = bytes.len() == 36
            && bytes.iter().enumerate().all(|(index, byte)| match index {
                8 | 13 | 18 | 23 => *byte == b'-',
                _ => byte.is_ascii_hexdigit(),
            });
        let valid_version = bytes.get(14) == Some(&b'4');
        let valid_variant = bytes
            .get(19)
            .is_some_and(|byte| matches!(byte.to_ascii_lowercase(), b'8' | b'9' | b'a' | b'b'));
        if !valid_shape || !valid_version || !valid_variant {
            return Err(DirectProtocolError::InvalidRequestId);
        }
        Ok(Self(value.to_ascii_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DirectRequestId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for DirectRequestId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for DirectRequestId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectPeerIdentity {
    pub device_id: String,
    pub device_name: String,
    pub node_id: String,
    pub public_key: String,
    pub fingerprint: String,
}

impl DirectPeerIdentity {
    pub fn from_secret(
        device_id: impl Into<String>,
        device_name: impl Into<String>,
        secret: &iroh::SecretKey,
    ) -> Self {
        let public_key = secret.public().to_string();
        Self {
            device_id: device_id.into(),
            device_name: device_name.into(),
            node_id: public_key.clone(),
            fingerprint: public_fingerprint(public_key.as_bytes()),
            public_key,
        }
    }

    pub fn pinned_target(node_id: impl Into<String>, fingerprint: impl Into<String>) -> Self {
        let node_id = node_id.into();
        Self {
            device_id: String::new(),
            device_name: String::new(),
            public_key: node_id.clone(),
            node_id,
            fingerprint: fingerprint.into(),
        }
    }

    pub fn validate(&self) -> Result<iroh::PublicKey, DirectProtocolError> {
        validate_text("device_id", &self.device_id, MAX_ID_BYTES, false)?;
        validate_text("device_name", &self.device_name, MAX_NAME_BYTES, true)?;
        self.validate_pin()
    }

    pub fn validate_pin(&self) -> Result<iroh::PublicKey, DirectProtocolError> {
        validate_text("device_id", &self.device_id, MAX_ID_BYTES, true)?;
        validate_text("device_name", &self.device_name, MAX_NAME_BYTES, true)?;
        validate_text("node_id", &self.node_id, MAX_ID_BYTES, false)?;
        validate_text("public_key", &self.public_key, MAX_ID_BYTES, false)?;
        let public = self
            .public_key
            .parse::<iroh::PublicKey>()
            .map_err(|_| DirectProtocolError::InvalidPublicKey)?;
        let node = self
            .node_id
            .parse::<iroh::PublicKey>()
            .map_err(|_| DirectProtocolError::InvalidNodeId)?;
        if public != node {
            return Err(DirectProtocolError::IdentityKeyMismatch);
        }
        if self.fingerprint != public_fingerprint(self.public_key.as_bytes()) {
            return Err(DirectProtocolError::InvalidFingerprint);
        }
        Ok(public)
    }
}

pub(crate) fn validate_direct_lookup_id(value: &str) -> Result<(), DirectProtocolError> {
    validate_text("lookup_id", value, MAX_ID_BYTES, false)
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DirectDecisionKind {
    Accepted,
    Rejected,
    Revoked,
}

impl DirectDecisionKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Revoked => "revoked",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedDirectRequest {
    pub request_id: DirectRequestId,
    pub lookup_id: String,
    pub requester: DirectPeerIdentity,
    pub target: DirectPeerIdentity,
    pub created_at: i64,
    pub expires_at: i64,
    pub nonce: String,
    pub message: Option<String>,
    pub hmac_proof: String,
    pub identity_signature: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedDirectRequestReceipt {
    pub request_id: DirectRequestId,
    pub lookup_id: String,
    pub requester: DirectPeerIdentity,
    pub target: DirectPeerIdentity,
    pub request_digest: String,
    pub received_at: i64,
    pub expires_at: i64,
    pub nonce: String,
    pub message: Option<String>,
    pub hmac_proof: String,
    pub identity_signature: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedDirectDecision {
    pub request_id: DirectRequestId,
    pub lookup_id: String,
    pub requester: DirectPeerIdentity,
    pub target: DirectPeerIdentity,
    pub request_digest: String,
    pub decision: DirectDecisionKind,
    pub decision_revision: u64,
    pub decided_at: i64,
    pub expires_at: i64,
    pub nonce: String,
    pub message: Option<String>,
    pub hmac_proof: String,
    pub identity_signature: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedDirectDecisionReceipt {
    pub request_id: DirectRequestId,
    pub lookup_id: String,
    pub requester: DirectPeerIdentity,
    pub target: DirectPeerIdentity,
    pub decision_digest: String,
    pub decision: DirectDecisionKind,
    pub decision_revision: u64,
    pub received_at: i64,
    pub expires_at: i64,
    pub nonce: String,
    pub message: Option<String>,
    pub hmac_proof: String,
    pub identity_signature: String,
}

impl SignedDirectRequest {
    #[allow(clippy::too_many_arguments)]
    pub fn sign(
        request_id: DirectRequestId,
        lookup_id: impl Into<String>,
        requester: DirectPeerIdentity,
        target: DirectPeerIdentity,
        created_at: i64,
        expires_at: i64,
        message: Option<String>,
        relation_secret: &[u8],
        signer: &iroh::SecretKey,
    ) -> Result<Self, DirectProtocolError> {
        let nonce = random_token(18).map_err(|_| DirectProtocolError::EntropyUnavailable)?;
        Self::sign_with_nonce(
            request_id,
            lookup_id,
            requester,
            target,
            created_at,
            expires_at,
            nonce,
            message,
            relation_secret,
            signer,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn sign_with_nonce(
        request_id: DirectRequestId,
        lookup_id: impl Into<String>,
        requester: DirectPeerIdentity,
        target: DirectPeerIdentity,
        created_at: i64,
        expires_at: i64,
        nonce: impl Into<String>,
        message: Option<String>,
        relation_secret: &[u8],
        signer: &iroh::SecretKey,
    ) -> Result<Self, DirectProtocolError> {
        let mut request = Self {
            request_id,
            lookup_id: lookup_id.into(),
            requester,
            target,
            created_at,
            expires_at,
            nonce: nonce.into(),
            message,
            hmac_proof: String::new(),
            identity_signature: String::new(),
        };
        request.validate_fields()?;
        direct_transcript::seal_request(&mut request, relation_secret, signer)?;
        Ok(request)
    }

    pub fn verify_at(&self, relation_secret: &[u8], now: i64) -> Result<(), DirectProtocolError> {
        self.validate_fields()?;
        validate_not_expired(now, self.created_at, self.expires_at)?;
        direct_transcript::verify_request(self, relation_secret)
    }

    pub fn digest(&self) -> Result<String, DirectProtocolError> {
        self.validate_fields()?;
        Ok(direct_transcript::request_digest(self))
    }

    pub(crate) fn validate_authenticity(
        &self,
        relation_secret: &[u8],
    ) -> Result<(), DirectProtocolError> {
        self.validate_fields()?;
        direct_transcript::verify_request(self, relation_secret)
    }

    fn validate_fields(&self) -> Result<(), DirectProtocolError> {
        validate_request_common(
            &self.lookup_id,
            &self.requester,
            &self.target,
            self.created_at,
            self.expires_at,
            &self.nonce,
            self.message.as_deref(),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectProtocolError {
    EntropyUnavailable,
    InvalidRequestId,
    InvalidField(&'static str),
    InvalidTimestamp,
    LifetimeExceeded,
    TimestampTooFarFuture,
    Expired,
    InvalidPublicKey,
    InvalidNodeId,
    IdentityKeyMismatch,
    InvalidFingerprint,
    SignerMismatch,
    InvalidHmac,
    InvalidSignature,
    DigestMismatch,
    InvalidDecisionRevision,
}

impl DirectProtocolError {
    pub fn code(self) -> &'static str {
        match self {
            Self::EntropyUnavailable => "entropy_unavailable",
            Self::InvalidRequestId => "invalid_request_id",
            Self::InvalidField(_) => "invalid_field",
            Self::InvalidTimestamp => "invalid_timestamp",
            Self::LifetimeExceeded => "lifetime_exceeded",
            Self::TimestampTooFarFuture => "timestamp_too_far_future",
            Self::Expired => "expired",
            Self::InvalidPublicKey => "invalid_public_key",
            Self::InvalidNodeId => "invalid_node_id",
            Self::IdentityKeyMismatch => "identity_key_mismatch",
            Self::InvalidFingerprint => "invalid_fingerprint",
            Self::SignerMismatch => "signer_mismatch",
            Self::InvalidHmac => "invalid_hmac",
            Self::InvalidSignature => "invalid_signature",
            Self::DigestMismatch => "digest_mismatch",
            Self::InvalidDecisionRevision => "invalid_decision_revision",
        }
    }
}

impl fmt::Display for DirectProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidField(field) => {
                write!(formatter, "invalid direct protocol field: {field}")
            }
            other => formatter.write_str(other.code()),
        }
    }
}

impl std::error::Error for DirectProtocolError {}

pub(super) fn validate_common(
    lookup_id: &str,
    requester: &DirectPeerIdentity,
    target: &DirectPeerIdentity,
    timestamp: i64,
    expires_at: i64,
    nonce: &str,
    message: Option<&str>,
) -> Result<(), DirectProtocolError> {
    validate_text("lookup_id", lookup_id, MAX_ID_BYTES, false)?;
    validate_text("nonce", nonce, MAX_NONCE_BYTES, false)?;
    validate_optional_message(message)?;
    requester.validate()?;
    target.validate()?;
    validate_timestamp_interval(timestamp, expires_at)
}

pub(super) fn validate_request_common(
    lookup_id: &str,
    requester: &DirectPeerIdentity,
    target: &DirectPeerIdentity,
    timestamp: i64,
    expires_at: i64,
    nonce: &str,
    message: Option<&str>,
) -> Result<(), DirectProtocolError> {
    validate_text("lookup_id", lookup_id, MAX_ID_BYTES, false)?;
    validate_text("nonce", nonce, MAX_NONCE_BYTES, false)?;
    validate_optional_message(message)?;
    requester.validate()?;
    target.validate_pin()?;
    validate_timestamp_interval(timestamp, expires_at)
}

pub(super) fn validate_not_expired(
    now: i64,
    created_at: i64,
    expires_at: i64,
) -> Result<(), DirectProtocolError> {
    validate_timestamp_interval(created_at, expires_at)?;
    if now < 0 {
        Err(DirectProtocolError::InvalidTimestamp)
    } else if created_at > now.saturating_add(MAX_TRACKED_DIRECT_CLOCK_SKEW_SECS) {
        Err(DirectProtocolError::TimestampTooFarFuture)
    } else if now > expires_at {
        Err(DirectProtocolError::Expired)
    } else {
        Ok(())
    }
}

pub(super) fn validate_persisted_timestamp(
    timestamp: i64,
    expires_at: i64,
) -> Result<(), DirectProtocolError> {
    validate_timestamp_interval(timestamp, expires_at)
}

fn validate_timestamp_interval(timestamp: i64, expires_at: i64) -> Result<(), DirectProtocolError> {
    if timestamp < 0 || expires_at <= timestamp {
        return Err(DirectProtocolError::InvalidTimestamp);
    }
    if expires_at.saturating_sub(timestamp) > MAX_TRACKED_DIRECT_ENVELOPE_LIFETIME_SECS {
        return Err(DirectProtocolError::LifetimeExceeded);
    }
    Ok(())
}

fn validate_text(
    field: &'static str,
    value: &str,
    max_bytes: usize,
    allow_empty: bool,
) -> Result<(), DirectProtocolError> {
    if (!allow_empty && value.is_empty())
        || value.len() > max_bytes
        || value.chars().any(char::is_control)
    {
        return Err(DirectProtocolError::InvalidField(field));
    }
    Ok(())
}

fn validate_optional_message(message: Option<&str>) -> Result<(), DirectProtocolError> {
    match message {
        Some(message) => validate_text("message", message, MAX_MESSAGE_BYTES, true),
        None => Ok(()),
    }
}

#[cfg(test)]
#[path = "direct_protocol_lifetime_tests.rs"]
mod lifetime_tests;
