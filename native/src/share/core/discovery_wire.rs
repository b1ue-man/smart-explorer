use std::fmt;

use super::discovery_domain::{DiscoveryCryptoError, MAX_DISCOVERY_BUNDLE_BYTES};
use super::discovery_signal_types::DISCOVERY_PAIRING_VERSION;

const KE3_BUNDLE_MAGIC: &[u8; 4] = b"SEK3";
const KE3_BUNDLE_HEADER_BYTES: usize = 16;
const MAX_OPAQUE_MESSAGE_BYTES: usize = 4096;
const AEAD_TAG_BYTES: usize = 16;
const MAX_ENCRYPTED_FRAME_BYTES: usize = MAX_DISCOVERY_BUNDLE_BYTES + 64;
pub(crate) const MAX_KE3_BUNDLE_PACKET_BYTES: usize =
    KE3_BUNDLE_HEADER_BYTES + MAX_OPAQUE_MESSAGE_BYTES + MAX_ENCRYPTED_FRAME_BYTES;

macro_rules! opaque_message {
    ($name:ident) => {
        #[derive(Clone, Eq, PartialEq)]
        pub struct $name(Vec<u8>);

        impl $name {
            pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, DiscoveryCryptoError> {
                validate_opaque_message(&bytes)?;
                Ok(Self(bytes))
            }

            pub fn as_bytes(&self) -> &[u8] {
                &self.0
            }

            pub fn into_bytes(self) -> Vec<u8> {
                self.0
            }

            pub(crate) fn from_validated(bytes: Vec<u8>) -> Self {
                Self(bytes)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .debug_struct(stringify!($name))
                    .field("bytes", &"[OPAQUE MESSAGE]")
                    .field("len", &self.0.len())
                    .finish()
            }
        }
    };
}

opaque_message!(OpaqueKe1);
opaque_message!(OpaqueKe2);

#[derive(Clone, Eq, PartialEq)]
pub struct OpaqueKe3ConnectorBundle {
    ke3: Vec<u8>,
    connector_bundle: EncryptedFrame,
}

impl OpaqueKe3ConnectorBundle {
    pub fn from_bytes(mut bytes: Vec<u8>) -> Result<Self, DiscoveryCryptoError> {
        if bytes.len() < KE3_BUNDLE_HEADER_BYTES || &bytes[..4] != KE3_BUNDLE_MAGIC {
            return Err(DiscoveryCryptoError::InvalidMessage);
        }
        let version = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        if version != DISCOVERY_PAIRING_VERSION
            || bytes[10..12].iter().any(|byte| *byte != 0)
        {
            return Err(DiscoveryCryptoError::InvalidMessage);
        }
        let ke3_len = u16::from_be_bytes([bytes[8], bytes[9]]) as usize;
        let encrypted_len =
            u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
        if bytes.len() > MAX_KE3_BUNDLE_PACKET_BYTES
            || ke3_len == 0
            || ke3_len > MAX_OPAQUE_MESSAGE_BYTES
            || !(AEAD_TAG_BYTES..=MAX_ENCRYPTED_FRAME_BYTES).contains(&encrypted_len)
        {
            return Err(DiscoveryCryptoError::InvalidMessage);
        }
        let ke3_end = KE3_BUNDLE_HEADER_BYTES
            .checked_add(ke3_len)
            .ok_or(DiscoveryCryptoError::InvalidMessage)?;
        let message_end = ke3_end
            .checked_add(encrypted_len)
            .ok_or(DiscoveryCryptoError::InvalidMessage)?;
        if message_end != bytes.len() {
            return Err(DiscoveryCryptoError::InvalidMessage);
        }

        // Preserve ownership of the potentially large encrypted payload. Only
        // the small, bounded KE3 is copied after every advertised/actual length
        // has passed validation; shifting the payload reuses the input buffer.
        let ke3 = bytes[KE3_BUNDLE_HEADER_BYTES..ke3_end].to_vec();
        bytes.copy_within(ke3_end..message_end, 0);
        bytes.truncate(encrypted_len);
        let connector_bundle = EncryptedFrame::new(bytes)?;
        Ok(Self {
            ke3,
            connector_bundle,
        })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(
            KE3_BUNDLE_HEADER_BYTES + self.ke3.len() + self.connector_bundle.len(),
        );
        bytes.extend_from_slice(KE3_BUNDLE_MAGIC);
        bytes.extend_from_slice(&DISCOVERY_PAIRING_VERSION.to_be_bytes());
        bytes.extend_from_slice(&(self.ke3.len() as u16).to_be_bytes());
        bytes.extend_from_slice(&[0; 2]);
        bytes.extend_from_slice(&(self.connector_bundle.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&self.ke3);
        bytes.extend_from_slice(self.connector_bundle.as_bytes());
        bytes
    }

    pub(crate) fn new(ke3: Vec<u8>, connector_bundle: EncryptedFrame) -> Self {
        Self {
            ke3,
            connector_bundle,
        }
    }

    pub(crate) fn ke3(&self) -> &[u8] {
        &self.ke3
    }

    pub(crate) fn connector_bundle(&self) -> &EncryptedFrame {
        &self.connector_bundle
    }
}

impl fmt::Debug for OpaqueKe3ConnectorBundle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpaqueKe3ConnectorBundle")
            .field("ke3", &"[OPAQUE MESSAGE]")
            .field("ke3_len", &self.ke3.len())
            .field("connector_bundle", &"[ENCRYPTED]")
            .field("connector_bundle_len", &self.connector_bundle.len())
            .finish()
    }
}

macro_rules! encrypted_message {
    ($name:ident) => {
        #[derive(Clone, Eq, PartialEq)]
        pub struct $name(EncryptedFrame);

        impl $name {
            pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, DiscoveryCryptoError> {
                Ok(Self(EncryptedFrame::new(bytes)?))
            }

            pub fn as_bytes(&self) -> &[u8] {
                self.0.as_bytes()
            }

            pub fn into_bytes(self) -> Vec<u8> {
                self.0.into_bytes()
            }

            pub(crate) fn new(frame: EncryptedFrame) -> Self {
                Self(frame)
            }

            pub(crate) fn frame(&self) -> &EncryptedFrame {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .debug_struct(stringify!($name))
                    .field("payload", &"[ENCRYPTED]")
                    .field("len", &self.0.len())
                    .finish()
            }
        }
    };
}

encrypted_message!(PublisherBundle);
encrypted_message!(ConnectorCommit);
encrypted_message!(PublisherCommit);

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct EncryptedFrame(Vec<u8>);

impl EncryptedFrame {
    pub(crate) fn new(bytes: Vec<u8>) -> Result<Self, DiscoveryCryptoError> {
        if bytes.len() < AEAD_TAG_BYTES || bytes.len() > MAX_ENCRYPTED_FRAME_BYTES {
            return Err(DiscoveryCryptoError::InvalidMessage);
        }
        Ok(Self(bytes))
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }
}

fn validate_opaque_message(bytes: &[u8]) -> Result<(), DiscoveryCryptoError> {
    if bytes.is_empty() || bytes.len() > MAX_OPAQUE_MESSAGE_BYTES {
        return Err(DiscoveryCryptoError::InvalidMessage);
    }
    Ok(())
}
