use std::fmt;

use zeroize::Zeroizing;

use super::discovery_signal_types::{
    DiscoveryKind, DISCOVERY_PAIRING_SUITE, DISCOVERY_PAIRING_VERSION,
};
pub const MAX_DISCOVERY_ID_BYTES: usize = 128;
pub const MAX_DISCOVERY_BUNDLE_BYTES: usize = 128 * 1024;

const BUNDLE_MAGIC: &[u8; 4] = b"SEDB";
const BUNDLE_HEADER_BYTES: usize = 16;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
/// Client-generated random identifier scoped to one discoverability offer.
pub struct OfferId(String);

impl OfferId {
    pub fn new(value: impl Into<String>) -> Result<Self, DiscoveryCryptoError> {
        let value = value.into();
        validate_public_id(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
/// Signaling-server identifier scoped to one ephemeral published offer.
pub struct DiscoveryId(String);

impl DiscoveryId {
    pub fn new(value: impl Into<String>) -> Result<Self, DiscoveryCryptoError> {
        let value = value.into();
        validate_public_id(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
/// Signaling-server identifier scoped to one online pairing attempt.
pub struct ExchangeId(String);

impl ExchangeId {
    pub fn new(value: impl Into<String>) -> Result<Self, DiscoveryCryptoError> {
        let value = value.into();
        validate_public_id(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveryOfferBinding {
    kind: DiscoveryKind,
    offer_id: OfferId,
}

impl DiscoveryOfferBinding {
    pub fn new(kind: DiscoveryKind, offer_id: OfferId) -> Self {
        Self { kind, offer_id }
    }

    pub fn kind(&self) -> DiscoveryKind {
        self.kind
    }

    pub fn offer_id(&self) -> &OfferId {
        &self.offer_id
    }

    pub fn for_exchange(
        &self,
        discovery_id: DiscoveryId,
        exchange_id: ExchangeId,
    ) -> DiscoveryExchangeBinding {
        DiscoveryExchangeBinding {
            offer: self.clone(),
            discovery_id,
            exchange_id,
        }
    }

    pub(crate) fn registration_context(&self) -> Vec<u8> {
        let mut context = Vec::with_capacity(192);
        context.extend_from_slice(b"smart-explorer/discovery/offer");
        push_field(&mut context, b"suite", DISCOVERY_PAIRING_SUITE.as_bytes());
        push_field(
            &mut context,
            b"version",
            &DISCOVERY_PAIRING_VERSION.to_be_bytes(),
        );
        push_field(&mut context, b"kind", &[kind_tag(self.kind)]);
        push_field(
            &mut context,
            b"offer-id",
            self.offer_id.as_str().as_bytes(),
        );
        context
    }

    pub(crate) fn connector_identifier(&self) -> Vec<u8> {
        let mut identifier = self.registration_context();
        push_field(&mut identifier, b"role", b"connector");
        identifier
    }

    pub(crate) fn publisher_identifier(&self) -> Vec<u8> {
        let mut identifier = self.registration_context();
        push_field(&mut identifier, b"role", b"publisher");
        identifier
    }

    pub(crate) fn credential_identifier(&self) -> Vec<u8> {
        let mut identifier = self.registration_context();
        push_field(&mut identifier, b"purpose", b"opaque-credential");
        identifier
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveryExchangeBinding {
    offer: DiscoveryOfferBinding,
    discovery_id: DiscoveryId,
    exchange_id: ExchangeId,
}

impl DiscoveryExchangeBinding {
    pub fn offer(&self) -> &DiscoveryOfferBinding {
        &self.offer
    }

    pub fn exchange_id(&self) -> &ExchangeId {
        &self.exchange_id
    }

    pub fn discovery_id(&self) -> &DiscoveryId {
        &self.discovery_id
    }

    pub(crate) fn exchange_context(&self) -> Vec<u8> {
        let mut context = self.offer.registration_context();
        push_field(
            &mut context,
            b"discovery-id",
            self.discovery_id.as_str().as_bytes(),
        );
        push_field(
            &mut context,
            b"exchange-id",
            self.exchange_id.as_str().as_bytes(),
        );
        context
    }

    pub(crate) fn aad(
        &self,
        sender: PairingRole,
        receiver: PairingRole,
        stage: PairingStage,
    ) -> Vec<u8> {
        let mut aad = self.exchange_context();
        push_field(&mut aad, b"sender", &[sender.tag()]);
        push_field(&mut aad, b"receiver", &[receiver.tag()]);
        push_field(&mut aad, b"stage", &[stage.tag()]);
        aad
    }
}

#[derive(Clone)]
pub struct PairingBundle {
    kind: DiscoveryKind,
    payload: Zeroizing<Vec<u8>>,
}

impl PairingBundle {
    /// Wraps an existing DirectCode/RoomCode payload for authenticated transport.
    /// Long-term relation material must remain independently random and is never
    /// derived from the PIN or from this exchange's OPAQUE session key.
    pub fn new(kind: DiscoveryKind, payload: Vec<u8>) -> Result<Self, DiscoveryCryptoError> {
        let payload = Zeroizing::new(payload);
        if payload.len() > MAX_DISCOVERY_BUNDLE_BYTES {
            return Err(DiscoveryCryptoError::PayloadTooLarge);
        }
        Ok(Self { kind, payload })
    }

    pub fn kind(&self) -> DiscoveryKind {
        self.kind
    }

    pub fn payload(&self) -> &[u8] {
        self.payload.as_slice()
    }

    pub(crate) fn encode_plaintext(&self) -> Zeroizing<Vec<u8>> {
        let mut output = Zeroizing::new(Vec::with_capacity(BUNDLE_HEADER_BYTES + self.payload.len()));
        output.extend_from_slice(BUNDLE_MAGIC);
        output.extend_from_slice(&DISCOVERY_PAIRING_VERSION.to_be_bytes());
        output.push(kind_tag(self.kind));
        output.extend_from_slice(&[0; 3]);
        output.extend_from_slice(&(self.payload.len() as u32).to_be_bytes());
        output.extend_from_slice(self.payload.as_slice());
        output
    }

    pub(crate) fn decode_plaintext(
        plaintext: Vec<u8>,
        expected_kind: DiscoveryKind,
    ) -> Result<Self, DiscoveryCryptoError> {
        let plaintext = Zeroizing::new(plaintext);
        if plaintext.len() < BUNDLE_HEADER_BYTES || &plaintext[..4] != BUNDLE_MAGIC {
            return Err(DiscoveryCryptoError::InvalidBundle);
        }
        let version = u32::from_be_bytes([plaintext[4], plaintext[5], plaintext[6], plaintext[7]]);
        if version != DISCOVERY_PAIRING_VERSION
            || plaintext[9..12].iter().any(|byte| *byte != 0)
        {
            return Err(DiscoveryCryptoError::InvalidBundle);
        }
        let kind = kind_from_tag(plaintext[8])?;
        if kind != expected_kind {
            return Err(DiscoveryCryptoError::BundleKindMismatch);
        }
        let payload_len = u32::from_be_bytes([
            plaintext[12],
            plaintext[13],
            plaintext[14],
            plaintext[15],
        ]) as usize;
        if payload_len > MAX_DISCOVERY_BUNDLE_BYTES
            || BUNDLE_HEADER_BYTES.checked_add(payload_len) != Some(plaintext.len())
        {
            return Err(DiscoveryCryptoError::InvalidBundle);
        }
        Ok(Self {
            kind,
            payload: Zeroizing::new(plaintext[BUNDLE_HEADER_BYTES..].to_vec()),
        })
    }
}

impl fmt::Debug for PairingBundle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairingBundle")
            .field("kind", &self.kind)
            .field("payload", &"[REDACTED]")
            .field("payload_len", &self.payload.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PairingRole {
    Connector,
    Publisher,
}

impl PairingRole {
    pub(crate) fn tag(self) -> u8 {
        match self {
            Self::Connector => 1,
            Self::Publisher => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PairingStage {
    ConnectorBundle,
    PublisherBundle,
    ConnectorCommit,
    PublisherCommit,
}

impl PairingStage {
    pub(crate) fn tag(self) -> u8 {
        match self {
            Self::ConnectorBundle => 1,
            Self::PublisherBundle => 2,
            Self::ConnectorCommit => 3,
            Self::PublisherCommit => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscoveryCryptoError {
    PinTooLong,
    InvalidIdentifier,
    BindingMismatch,
    InvalidMessage,
    CryptographicFailure,
    AuthenticationFailed,
    EncryptionFailed,
    PayloadTooLarge,
    InvalidBundle,
    BundleKindMismatch,
}

impl fmt::Display for DiscoveryCryptoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PinTooLong => "PIN exceeds the supported byte limit",
            Self::InvalidIdentifier => "invalid ephemeral discovery identifier",
            Self::BindingMismatch => "pairing exchange does not match the published offer",
            Self::InvalidMessage => "invalid pairing message",
            Self::CryptographicFailure => "pairing cryptographic operation failed",
            Self::AuthenticationFailed => "pairing authentication failed",
            Self::EncryptionFailed => "pairing message authentication failed",
            Self::PayloadTooLarge => "pairing payload exceeds the supported byte limit",
            Self::InvalidBundle => "invalid pairing bundle",
            Self::BundleKindMismatch => "pairing bundle kind does not match the offer",
        })
    }
}

impl std::error::Error for DiscoveryCryptoError {}

fn validate_public_id(value: &str) -> Result<(), DiscoveryCryptoError> {
    if value.is_empty()
        || value.len() > MAX_DISCOVERY_ID_BYTES
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
        })
    {
        return Err(DiscoveryCryptoError::InvalidIdentifier);
    }
    Ok(())
}

fn kind_tag(kind: DiscoveryKind) -> u8 {
    match kind {
        DiscoveryKind::Direct => 1,
        DiscoveryKind::Room => 2,
    }
}

fn kind_from_tag(tag: u8) -> Result<DiscoveryKind, DiscoveryCryptoError> {
    match tag {
        1 => Ok(DiscoveryKind::Direct),
        2 => Ok(DiscoveryKind::Room),
        _ => Err(DiscoveryCryptoError::InvalidBundle),
    }
}

fn push_field(output: &mut Vec<u8>, name: &[u8], value: &[u8]) {
    output.extend_from_slice(&(name.len() as u16).to_be_bytes());
    output.extend_from_slice(name);
    output.extend_from_slice(&(value.len() as u32).to_be_bytes());
    output.extend_from_slice(value);
}
