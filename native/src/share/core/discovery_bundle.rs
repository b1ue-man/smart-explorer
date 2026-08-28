use std::fmt;

use zeroize::Zeroizing;

use super::direct_protocol::DirectPeerIdentity;
use super::direct_reciprocal::{DirectReciprocalPeer, DirectRelationMaterial};
use super::discovery_signal_types::DiscoveryKind;
use super::room_relation::{RoomJoinIntent, RoomRelationMaterial, RoomRelationOffer};

pub const MAX_APPLICATION_BUNDLE_BYTES: usize = 16 * 1024;

const BUNDLE_MAGIC: &[u8; 4] = b"SEAB";
const BUNDLE_VERSION: u16 = 1;
const BUNDLE_HEADER_BYTES: usize = 8;
const MAX_TEXT_FIELD_BYTES: usize = 1024;
const CONNECTOR_DIRECTION: u8 = 1;
const PUBLISHER_DIRECTION: u8 = 2;
const DIRECT_KIND: u8 = 1;
const ROOM_KIND: u8 = 2;

#[derive(Clone, PartialEq, Eq)]
pub enum ConnectorApplicationBundle {
    Direct(DirectReciprocalPeer),
    Room(RoomJoinIntent),
}

impl ConnectorApplicationBundle {
    pub fn kind(&self) -> DiscoveryKind {
        match self {
            Self::Direct(_) => DiscoveryKind::Direct,
            Self::Room(_) => DiscoveryKind::Room,
        }
    }

    pub(crate) fn encode_plaintext(
        &self,
    ) -> Result<Zeroizing<Vec<u8>>, ApplicationBundleError> {
        let mut output = bundle_header(CONNECTOR_DIRECTION, self.kind());
        match self {
            Self::Direct(peer) => encode_direct_peer(&mut output, peer)?,
            Self::Room(_) => {}
        }
        finish_encoding(output)
    }

    pub(crate) fn decode_plaintext(
        plaintext: Vec<u8>,
    ) -> Result<Self, ApplicationBundleError> {
        let mut reader = BundleReader::new(plaintext, CONNECTOR_DIRECTION)?;
        let bundle = match reader.kind {
            DiscoveryKind::Direct => Self::Direct(decode_direct_peer(&mut reader)?),
            DiscoveryKind::Room => Self::Room(RoomJoinIntent),
        };
        reader.finish()?;
        Ok(bundle)
    }
}

impl fmt::Debug for ConnectorApplicationBundle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Direct(_) => formatter.write_str("ConnectorApplicationBundle::Direct([REDACTED])"),
            Self::Room(_) => formatter.write_str("ConnectorApplicationBundle::RoomJoinIntent"),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum PublisherApplicationBundle {
    Direct(DirectReciprocalPeer),
    Room(RoomRelationOffer),
}

impl PublisherApplicationBundle {
    pub fn kind(&self) -> DiscoveryKind {
        match self {
            Self::Direct(_) => DiscoveryKind::Direct,
            Self::Room(_) => DiscoveryKind::Room,
        }
    }

    pub(crate) fn encode_plaintext(
        &self,
    ) -> Result<Zeroizing<Vec<u8>>, ApplicationBundleError> {
        let mut output = bundle_header(PUBLISHER_DIRECTION, self.kind());
        match self {
            Self::Direct(peer) => encode_direct_peer(&mut output, peer)?,
            Self::Room(offer) => {
                push_text(&mut output, offer.display_name())?;
                push_text(&mut output, offer.material().room_id())?;
                output.extend_from_slice(offer.material().secret());
            }
        }
        finish_encoding(output)
    }

    pub(crate) fn decode_plaintext(
        plaintext: Vec<u8>,
    ) -> Result<Self, ApplicationBundleError> {
        let mut reader = BundleReader::new(plaintext, PUBLISHER_DIRECTION)?;
        let bundle = match reader.kind {
            DiscoveryKind::Direct => Self::Direct(decode_direct_peer(&mut reader)?),
            DiscoveryKind::Room => {
                let display_name = reader.read_text()?;
                let room_id = reader.read_text()?;
                let secret = reader.read_secret()?;
                let material = RoomRelationMaterial::new(room_id, secret)
                    .map_err(|_| ApplicationBundleError::InvalidRoomMaterial)?;
                let offer = RoomRelationOffer::new(material, display_name.clone())
                    .map_err(|_| ApplicationBundleError::InvalidRoomMaterial)?;
                if offer.display_name() != display_name {
                    return Err(ApplicationBundleError::InvalidRoomMaterial);
                }
                Self::Room(offer)
            }
        };
        reader.finish()?;
        Ok(bundle)
    }
}

impl fmt::Debug for PublisherApplicationBundle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Direct(_) => formatter.write_str("PublisherApplicationBundle::Direct([REDACTED])"),
            Self::Room(_) => formatter.write_str("PublisherApplicationBundle::Room([REDACTED])"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationBundleError {
    PayloadTooLarge,
    InvalidHeader,
    UnsupportedVersion,
    DirectionMismatch,
    InvalidKind,
    InvalidField,
    InvalidDirectPeer,
    InvalidRoomMaterial,
    TrailingBytes,
}

impl fmt::Display for ApplicationBundleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PayloadTooLarge => "application bundle exceeds its byte budget",
            Self::InvalidHeader => "invalid application bundle header",
            Self::UnsupportedVersion => "unsupported application bundle version",
            Self::DirectionMismatch => "application bundle has the wrong direction",
            Self::InvalidKind => "invalid application bundle kind",
            Self::InvalidField => "invalid application bundle field",
            Self::InvalidDirectPeer => "invalid authenticated Direct application bundle",
            Self::InvalidRoomMaterial => "invalid Room application bundle",
            Self::TrailingBytes => "application bundle contains trailing bytes",
        })
    }
}

impl std::error::Error for ApplicationBundleError {}

fn bundle_header(direction: u8, kind: DiscoveryKind) -> Zeroizing<Vec<u8>> {
    let mut output = Zeroizing::new(Vec::with_capacity(512));
    output.extend_from_slice(BUNDLE_MAGIC);
    output.extend_from_slice(&BUNDLE_VERSION.to_be_bytes());
    output.push(direction);
    output.push(match kind {
        DiscoveryKind::Direct => DIRECT_KIND,
        DiscoveryKind::Room => ROOM_KIND,
    });
    output
}

fn encode_direct_peer(
    output: &mut Zeroizing<Vec<u8>>,
    peer: &DirectReciprocalPeer,
) -> Result<(), ApplicationBundleError> {
    let identity = peer.identity();
    push_text(output, &identity.device_id)?;
    push_text(output, &identity.device_name)?;
    push_text(output, &identity.node_id)?;
    push_text(output, &identity.public_key)?;
    push_text(output, &identity.fingerprint)?;
    push_text(output, peer.material().lookup_id())?;
    output.extend_from_slice(peer.material().secret());
    Ok(())
}

fn decode_direct_peer(
    reader: &mut BundleReader,
) -> Result<DirectReciprocalPeer, ApplicationBundleError> {
    let identity = DirectPeerIdentity {
        device_id: reader.read_text()?,
        device_name: reader.read_text()?,
        node_id: reader.read_text()?,
        public_key: reader.read_text()?,
        fingerprint: reader.read_text()?,
    };
    let lookup_id = reader.read_text()?;
    let secret = reader.read_secret()?;
    let material = DirectRelationMaterial::new(lookup_id, secret)
        .map_err(|_| ApplicationBundleError::InvalidDirectPeer)?;
    DirectReciprocalPeer::authenticated(identity, material)
        .map_err(|_| ApplicationBundleError::InvalidDirectPeer)
}

fn push_text(
    output: &mut Zeroizing<Vec<u8>>,
    value: &str,
) -> Result<(), ApplicationBundleError> {
    if value.len() > MAX_TEXT_FIELD_BYTES || value.chars().any(char::is_control) {
        return Err(ApplicationBundleError::InvalidField);
    }
    let length = u16::try_from(value.len()).map_err(|_| ApplicationBundleError::InvalidField)?;
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

fn finish_encoding(
    output: Zeroizing<Vec<u8>>,
) -> Result<Zeroizing<Vec<u8>>, ApplicationBundleError> {
    if output.len() > MAX_APPLICATION_BUNDLE_BYTES {
        Err(ApplicationBundleError::PayloadTooLarge)
    } else {
        Ok(output)
    }
}

struct BundleReader {
    input: Zeroizing<Vec<u8>>,
    offset: usize,
    kind: DiscoveryKind,
}

impl BundleReader {
    fn new(input: Vec<u8>, expected_direction: u8) -> Result<Self, ApplicationBundleError> {
        let input = Zeroizing::new(input);
        if input.len() > MAX_APPLICATION_BUNDLE_BYTES {
            return Err(ApplicationBundleError::PayloadTooLarge);
        }
        if input.len() < BUNDLE_HEADER_BYTES || &input[..4] != BUNDLE_MAGIC {
            return Err(ApplicationBundleError::InvalidHeader);
        }
        if u16::from_be_bytes([input[4], input[5]]) != BUNDLE_VERSION {
            return Err(ApplicationBundleError::UnsupportedVersion);
        }
        if input[6] != expected_direction {
            return Err(ApplicationBundleError::DirectionMismatch);
        }
        let kind = match input[7] {
            DIRECT_KIND => DiscoveryKind::Direct,
            ROOM_KIND => DiscoveryKind::Room,
            _ => return Err(ApplicationBundleError::InvalidKind),
        };
        Ok(Self {
            input,
            offset: BUNDLE_HEADER_BYTES,
            kind,
        })
    }

    fn read_text(&mut self) -> Result<String, ApplicationBundleError> {
        let length_bytes = self.take(2)?;
        let length = usize::from(u16::from_be_bytes([length_bytes[0], length_bytes[1]]));
        if length > MAX_TEXT_FIELD_BYTES {
            return Err(ApplicationBundleError::InvalidField);
        }
        let value = self.take(length)?;
        let value = std::str::from_utf8(value).map_err(|_| ApplicationBundleError::InvalidField)?;
        if value.chars().any(char::is_control) {
            return Err(ApplicationBundleError::InvalidField);
        }
        Ok(value.to_string())
    }

    fn read_secret(&mut self) -> Result<Vec<u8>, ApplicationBundleError> {
        Ok(self
            .take(super::room_relation::ROOM_RELATION_SECRET_BYTES)?
            .to_vec())
    }

    fn take(&mut self, length: usize) -> Result<&[u8], ApplicationBundleError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(ApplicationBundleError::InvalidField)?;
        if end > self.input.len() {
            return Err(ApplicationBundleError::InvalidField);
        }
        let start = self.offset;
        self.offset = end;
        Ok(&self.input[start..end])
    }

    fn finish(self) -> Result<(), ApplicationBundleError> {
        if self.offset == self.input.len() {
            Ok(())
        } else {
            Err(ApplicationBundleError::TrailingBytes)
        }
    }
}
