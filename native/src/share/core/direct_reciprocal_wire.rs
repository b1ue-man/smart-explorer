use std::fmt;
use std::mem;

use sha2::{Digest, Sha256};

use super::core::random_bytes;
use super::direct_protocol::{validate_direct_lookup_id, DirectPeerIdentity};
use super::direct_reciprocal::{DirectReciprocalError, DirectRelationMaterial};

pub(crate) const DIRECT_RECIPROCAL_CAPABILITY: &str = "direct_reciprocal_v1";
pub(crate) const MAX_DIRECT_REPAIR_FRAME: usize = 8 * 1024;

const MAGIC: &[u8; 4] = b"SEDR";
const VERSION: u8 = 1;
const TRANSCRIPT_DOMAIN: &[u8] = b"smart-explorer/share/direct-reciprocal/v1/transcript\0";
const MAX_ID_BYTES: usize = 256;
const MAX_NAME_BYTES: usize = 1024;
const RELATION_SECRET_BYTES: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct DirectRepairId([u8; 16]);

impl DirectRepairId {
    pub(crate) fn generate() -> Result<Self, DirectRepairWireError> {
        let mut bytes = random_bytes::<16>().map_err(|_| DirectRepairWireError::EntropyUnavailable)?;
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Ok(Self(bytes))
    }

    pub(crate) fn from_bytes(bytes: [u8; 16]) -> Result<Self, DirectRepairWireError> {
        if bytes[6] >> 4 != 4 || bytes[8] >> 6 != 2 {
            return Err(DirectRepairWireError::InvalidRepairId);
        }
        Ok(Self(bytes))
    }

    pub(crate) fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Debug for DirectRepairId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl fmt::Display for DirectRepairId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, byte) in self.0.iter().enumerate() {
            if matches!(index, 4 | 6 | 8 | 10) {
                formatter.write_str("-")?;
            }
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct DirectRepairDigest([u8; 32]);

impl DirectRepairDigest {
    pub(crate) fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for DirectRepairDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DirectRepairDigest([REDACTED])")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DirectRepairPersisted {
    Changed,
    AlreadyComplete,
}

pub(crate) struct DirectRepairMaterial {
    lookup_id: String,
    secret: [u8; RELATION_SECRET_BYTES],
}

impl DirectRepairMaterial {
    pub(crate) fn from_domain(material: &DirectRelationMaterial) -> Self {
        let mut secret = [0; RELATION_SECRET_BYTES];
        secret.copy_from_slice(material.secret());
        Self {
            lookup_id: material.lookup_id().to_string(),
            secret,
        }
    }

    pub(crate) fn into_domain(mut self) -> Result<DirectRelationMaterial, DirectReciprocalError> {
        let lookup_id = mem::take(&mut self.lookup_id);
        let secret = self.secret.to_vec();
        self.secret.fill(0);
        DirectRelationMaterial::new(lookup_id, secret)
    }

    pub(crate) fn lookup_id(&self) -> &str {
        &self.lookup_id
    }
}

impl fmt::Debug for DirectRepairMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DirectRepairMaterial")
            .field("lookup_id", &self.lookup_id)
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

impl Drop for DirectRepairMaterial {
    fn drop(&mut self) {
        self.secret.fill(0);
    }
}

#[derive(Debug)]
pub(crate) struct DirectRepairHello {
    pub(super) repair_id: DirectRepairId,
    pub(super) identity: DirectPeerIdentity,
    pub(super) material: DirectRepairMaterial,
}

impl DirectRepairHello {
    pub(crate) fn digest(&self) -> DirectRepairDigest {
        digest_payload(|payload| encode_hello(self, payload))
    }
}

#[derive(Debug)]
pub(crate) struct DirectRepairOffer {
    pub(super) repair_id: DirectRepairId,
    pub(super) hello_digest: DirectRepairDigest,
    pub(super) identity: DirectPeerIdentity,
    pub(super) material: DirectRepairMaterial,
    pub(super) persisted: DirectRepairPersisted,
}

impl DirectRepairOffer {
    pub(crate) fn digest(&self) -> DirectRepairDigest {
        digest_payload(|payload| encode_offer(self, payload))
    }
}

#[derive(Debug)]
pub(crate) struct DirectRepairCommit {
    pub(super) repair_id: DirectRepairId,
    pub(super) offer_digest: DirectRepairDigest,
    pub(super) persisted: DirectRepairPersisted,
}

impl DirectRepairCommit {
    pub(crate) fn digest(&self) -> DirectRepairDigest {
        digest_payload(|payload| encode_commit(self, payload))
    }
}

#[derive(Debug)]
pub(crate) struct DirectRepairComplete {
    pub(super) repair_id: DirectRepairId,
    pub(super) commit_digest: DirectRepairDigest,
}

#[derive(Debug)]
pub(crate) enum DirectRepairMessage {
    Hello(DirectRepairHello),
    Offer(DirectRepairOffer),
    Commit(DirectRepairCommit),
    Complete(DirectRepairComplete),
}

pub(crate) struct EncodedDirectRepairFrame {
    bytes: Vec<u8>,
}

impl EncodedDirectRepairFrame {
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl fmt::Debug for EncodedDirectRepairFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EncodedDirectRepairFrame")
            .field("len", &self.bytes.len())
            .field("payload", &"[REDACTED]")
            .finish()
    }
}

impl Drop for EncodedDirectRepairFrame {
    fn drop(&mut self) {
        self.bytes.fill(0);
    }
}

pub(crate) fn encode_direct_repair_frame(
    message: &DirectRepairMessage,
) -> Result<EncodedDirectRepairFrame, DirectRepairWireError> {
    let mut bytes = vec![0; 4];
    match message {
        DirectRepairMessage::Hello(message) => encode_hello(message, &mut bytes),
        DirectRepairMessage::Offer(message) => encode_offer(message, &mut bytes),
        DirectRepairMessage::Commit(message) => encode_commit(message, &mut bytes),
        DirectRepairMessage::Complete(message) => encode_complete(message, &mut bytes),
    }
    let payload_len = bytes.len().saturating_sub(4);
    if payload_len == 0 || payload_len > MAX_DIRECT_REPAIR_FRAME {
        bytes.fill(0);
        return Err(DirectRepairWireError::FrameTooLarge);
    }
    bytes[..4].copy_from_slice(&(payload_len as u32).to_be_bytes());
    Ok(EncodedDirectRepairFrame { bytes })
}

pub(crate) fn decode_direct_repair_frame(
    mut bytes: Vec<u8>,
) -> Result<DirectRepairMessage, DirectRepairWireError> {
    let result = decode_frame_inner(&bytes);
    bytes.fill(0);
    result
}

fn decode_frame_inner(bytes: &[u8]) -> Result<DirectRepairMessage, DirectRepairWireError> {
    if bytes.len() < 4 {
        return Err(DirectRepairWireError::Truncated);
    }
    let declared = u32::from_be_bytes(bytes[..4].try_into().map_err(|_| DirectRepairWireError::Truncated)?) as usize;
    if declared == 0 || declared > MAX_DIRECT_REPAIR_FRAME {
        return Err(DirectRepairWireError::FrameTooLarge);
    }
    if bytes.len() != declared.saturating_add(4) {
        return Err(DirectRepairWireError::InvalidLength);
    }
    let mut reader = Reader::new(&bytes[4..]);
    if reader.array::<4>()? != *MAGIC || reader.byte()? != VERSION {
        return Err(DirectRepairWireError::UnsupportedVersion);
    }
    let message = match reader.byte()? {
        1 => DirectRepairMessage::Hello(decode_hello(&mut reader)?),
        2 => DirectRepairMessage::Offer(decode_offer(&mut reader)?),
        3 => DirectRepairMessage::Commit(decode_commit(&mut reader)?),
        4 => DirectRepairMessage::Complete(decode_complete(&mut reader)?),
        _ => return Err(DirectRepairWireError::UnknownMessage),
    };
    if !reader.is_done() {
        return Err(DirectRepairWireError::TrailingData);
    }
    Ok(message)
}

fn encode_header(kind: u8, out: &mut Vec<u8>) {
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.push(kind);
}

fn encode_hello(message: &DirectRepairHello, out: &mut Vec<u8>) {
    encode_header(1, out);
    out.extend_from_slice(message.repair_id.as_bytes());
    encode_identity(&message.identity, out);
    encode_material(&message.material, out);
}

fn encode_offer(message: &DirectRepairOffer, out: &mut Vec<u8>) {
    encode_header(2, out);
    out.extend_from_slice(message.repair_id.as_bytes());
    out.extend_from_slice(message.hello_digest.as_bytes());
    encode_identity(&message.identity, out);
    encode_material(&message.material, out);
    out.push(encode_outcome(message.persisted));
}

fn encode_commit(message: &DirectRepairCommit, out: &mut Vec<u8>) {
    encode_header(3, out);
    out.extend_from_slice(message.repair_id.as_bytes());
    out.extend_from_slice(message.offer_digest.as_bytes());
    out.push(encode_outcome(message.persisted));
}

fn encode_complete(message: &DirectRepairComplete, out: &mut Vec<u8>) {
    encode_header(4, out);
    out.extend_from_slice(message.repair_id.as_bytes());
    out.extend_from_slice(message.commit_digest.as_bytes());
}

fn encode_identity(identity: &DirectPeerIdentity, out: &mut Vec<u8>) {
    encode_string(&identity.device_id, out);
    encode_string(&identity.device_name, out);
    encode_string(&identity.node_id, out);
    encode_string(&identity.public_key, out);
    encode_string(&identity.fingerprint, out);
}

fn encode_material(material: &DirectRepairMaterial, out: &mut Vec<u8>) {
    encode_string(material.lookup_id(), out);
    out.extend_from_slice(&material.secret);
}

fn encode_string(value: &str, out: &mut Vec<u8>) {
    let len = u16::try_from(value.len()).unwrap_or(u16::MAX);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(value.as_bytes());
}

fn encode_outcome(outcome: DirectRepairPersisted) -> u8 {
    match outcome {
        DirectRepairPersisted::Changed => 1,
        DirectRepairPersisted::AlreadyComplete => 2,
    }
}

fn decode_hello(reader: &mut Reader<'_>) -> Result<DirectRepairHello, DirectRepairWireError> {
    Ok(DirectRepairHello {
        repair_id: DirectRepairId::from_bytes(reader.array()?)?,
        identity: decode_identity(reader)?,
        material: decode_material(reader)?,
    })
}

fn decode_offer(reader: &mut Reader<'_>) -> Result<DirectRepairOffer, DirectRepairWireError> {
    Ok(DirectRepairOffer {
        repair_id: DirectRepairId::from_bytes(reader.array()?)?,
        hello_digest: DirectRepairDigest(reader.array()?),
        identity: decode_identity(reader)?,
        material: decode_material(reader)?,
        persisted: decode_outcome(reader.byte()?)?,
    })
}

fn decode_commit(reader: &mut Reader<'_>) -> Result<DirectRepairCommit, DirectRepairWireError> {
    Ok(DirectRepairCommit {
        repair_id: DirectRepairId::from_bytes(reader.array()?)?,
        offer_digest: DirectRepairDigest(reader.array()?),
        persisted: decode_outcome(reader.byte()?)?,
    })
}

fn decode_complete(reader: &mut Reader<'_>) -> Result<DirectRepairComplete, DirectRepairWireError> {
    Ok(DirectRepairComplete {
        repair_id: DirectRepairId::from_bytes(reader.array()?)?,
        commit_digest: DirectRepairDigest(reader.array()?),
    })
}

fn decode_identity(reader: &mut Reader<'_>) -> Result<DirectPeerIdentity, DirectRepairWireError> {
    let identity = DirectPeerIdentity {
        device_id: reader.string(MAX_ID_BYTES, false)?,
        device_name: reader.string(MAX_NAME_BYTES, true)?,
        node_id: reader.string(MAX_ID_BYTES, false)?,
        public_key: reader.string(MAX_ID_BYTES, false)?,
        fingerprint: reader.string(MAX_ID_BYTES, false)?,
    };
    identity
        .validate()
        .map_err(|_| DirectRepairWireError::InvalidIdentity)?;
    Ok(identity)
}

fn decode_material(reader: &mut Reader<'_>) -> Result<DirectRepairMaterial, DirectRepairWireError> {
    let lookup_id = reader.string(MAX_ID_BYTES, false)?;
    let mut secret = reader.array::<RELATION_SECRET_BYTES>()?;
    if validate_direct_lookup_id(&lookup_id).is_err() {
        secret.fill(0);
        return Err(DirectRepairWireError::InvalidMaterial);
    }
    Ok(DirectRepairMaterial { lookup_id, secret })
}

fn decode_outcome(value: u8) -> Result<DirectRepairPersisted, DirectRepairWireError> {
    match value {
        1 => Ok(DirectRepairPersisted::Changed),
        2 => Ok(DirectRepairPersisted::AlreadyComplete),
        _ => Err(DirectRepairWireError::InvalidOutcome),
    }
}

fn digest_payload(encode: impl FnOnce(&mut Vec<u8>)) -> DirectRepairDigest {
    let mut payload = Vec::with_capacity(512);
    encode(&mut payload);
    let mut digest = Sha256::new();
    digest.update(TRANSCRIPT_DOMAIN);
    digest.update((payload.len() as u32).to_be_bytes());
    digest.update(&payload);
    let result = DirectRepairDigest(digest.finalize().into());
    payload.fill(0);
    result
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn byte(&mut self) -> Result<u8, DirectRepairWireError> {
        Ok(self.array::<1>()?[0])
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], DirectRepairWireError> {
        let end = self
            .offset
            .checked_add(N)
            .ok_or(DirectRepairWireError::InvalidLength)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(DirectRepairWireError::Truncated)?;
        self.offset = end;
        value.try_into().map_err(|_| DirectRepairWireError::Truncated)
    }

    fn string(
        &mut self,
        max: usize,
        allow_empty: bool,
    ) -> Result<String, DirectRepairWireError> {
        let len = u16::from_be_bytes(self.array()?) as usize;
        if len > max || (!allow_empty && len == 0) {
            return Err(DirectRepairWireError::InvalidField);
        }
        let end = self
            .offset
            .checked_add(len)
            .ok_or(DirectRepairWireError::InvalidLength)?;
        let raw = self
            .bytes
            .get(self.offset..end)
            .ok_or(DirectRepairWireError::Truncated)?;
        self.offset = end;
        std::str::from_utf8(raw)
            .map(str::to_string)
            .map_err(|_| DirectRepairWireError::InvalidField)
    }

    fn is_done(&self) -> bool {
        self.offset == self.bytes.len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DirectRepairWireError {
    EntropyUnavailable,
    InvalidRepairId,
    FrameTooLarge,
    InvalidLength,
    Truncated,
    UnsupportedVersion,
    UnknownMessage,
    InvalidIdentity,
    InvalidMaterial,
    InvalidOutcome,
    InvalidField,
    TrailingData,
}

impl fmt::Display for DirectRepairWireError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EntropyUnavailable => "secure repair id generation failed",
            Self::InvalidRepairId => "invalid repair id",
            Self::FrameTooLarge => "repair frame exceeds its fixed bound",
            Self::InvalidLength => "invalid repair frame length",
            Self::Truncated => "truncated repair frame",
            Self::UnsupportedVersion => "unsupported repair wire version",
            Self::UnknownMessage => "unknown repair message",
            Self::InvalidIdentity => "invalid repair identity",
            Self::InvalidMaterial => "invalid repair relation material",
            Self::InvalidOutcome => "invalid repair persistence outcome",
            Self::InvalidField => "invalid repair field",
            Self::TrailingData => "repair frame contains trailing data",
        })
    }
}

impl std::error::Error for DirectRepairWireError {}
