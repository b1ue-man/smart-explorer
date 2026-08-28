use std::fmt;

use chacha20poly1305::{
    aead::{Aead, Payload},
    ChaCha20Poly1305, Key, KeyInit, Nonce,
};
use hkdf::Hkdf;
use sha2::{Digest, Sha512};
use zeroize::Zeroizing;

use super::discovery_domain::{
    DiscoveryCryptoError, DiscoveryExchangeBinding, PairingBundle, PairingRole, PairingStage,
};
use super::discovery_wire::{
    ConnectorCommit, EncryptedFrame, OpaqueKe3ConnectorBundle, PublisherBundle, PublisherCommit,
};

const SESSION_KEY_BYTES: usize = 64;
const AEAD_KEY_BYTES: usize = 32;
const COMMIT_MAGIC: &[u8] = b"smart-explorer/discovery/commit/v1";
// Every typestate permits one encryption for one transcript/direction/stage.
// HKDF therefore creates a single-use key for this nonce; retries resend the
// same ciphertext instead of encrypting again under that key.
const ZERO_NONCE: [u8; 12] = [0; 12];

#[must_use]
pub struct ConnectorAwaitingPublisherBundle {
    session: SessionCipher,
}

impl ConnectorAwaitingPublisherBundle {
    pub(crate) fn from_pake(
        binding: DiscoveryExchangeBinding,
        session_key: [u8; SESSION_KEY_BYTES],
        ke1: &[u8],
        ke2: &[u8],
        ke3: Vec<u8>,
        connector_bundle: &PairingBundle,
    ) -> Result<(Self, OpaqueKe3ConnectorBundle), DiscoveryCryptoError> {
        let mut session = SessionCipher::new(binding, session_key, ke1, ke2, &ke3);
        ensure_bundle_kind(session.binding(), connector_bundle)?;
        let encrypted = session.encrypt_bundle(
            PairingRole::Connector,
            PairingRole::Publisher,
            PairingStage::ConnectorBundle,
            connector_bundle,
        )?;
        session.include_frame(PairingStage::ConnectorBundle, &encrypted);
        let packet = OpaqueKe3ConnectorBundle::new(ke3, encrypted);
        Ok((Self { session }, packet))
    }

    pub fn accept_publisher_bundle(
        mut self,
        message: PublisherBundle,
    ) -> Result<(ConnectorReceivedPublisherBundle, PairingBundle), DiscoveryCryptoError> {
        let bundle = self.session.decrypt_bundle(
            PairingRole::Publisher,
            PairingRole::Connector,
            PairingStage::PublisherBundle,
            message.frame(),
        )?;
        self.session
            .include_frame(PairingStage::PublisherBundle, message.frame());
        Ok((
            ConnectorReceivedPublisherBundle {
                session: self.session,
            },
            bundle,
        ))
    }

    pub fn binding(&self) -> &DiscoveryExchangeBinding {
        self.session.binding()
    }
}

#[must_use]
pub struct ConnectorReceivedPublisherBundle {
    session: SessionCipher,
}

impl ConnectorReceivedPublisherBundle {
    /// Prepares the connector commit. The caller must not release the returned
    /// packet until the publisher bundle has been durably accepted locally.
    pub fn commit(
        mut self,
    ) -> Result<(ConnectorAwaitingPublisherCommit, ConnectorCommit), DiscoveryCryptoError> {
        let encrypted = self.session.encrypt_commit(
            PairingRole::Connector,
            PairingRole::Publisher,
            PairingStage::ConnectorCommit,
        )?;
        self.session
            .include_frame(PairingStage::ConnectorCommit, &encrypted);
        Ok((
            ConnectorAwaitingPublisherCommit {
                session: self.session,
            },
            ConnectorCommit::new(encrypted),
        ))
    }

    pub fn binding(&self) -> &DiscoveryExchangeBinding {
        self.session.binding()
    }
}

#[must_use]
pub struct ConnectorAwaitingPublisherCommit {
    session: SessionCipher,
}

impl ConnectorAwaitingPublisherCommit {
    pub fn finish(
        mut self,
        message: PublisherCommit,
    ) -> Result<ConnectorPairingComplete, DiscoveryCryptoError> {
        self.session.decrypt_commit(
            PairingRole::Publisher,
            PairingRole::Connector,
            PairingStage::PublisherCommit,
            message.frame(),
        )?;
        self.session
            .include_frame(PairingStage::PublisherCommit, message.frame());
        Ok(ConnectorPairingComplete {
            binding: self.session.binding,
        })
    }

    pub fn binding(&self) -> &DiscoveryExchangeBinding {
        self.session.binding()
    }
}

#[must_use]
pub struct PublisherReceivedConnectorBundle {
    session: SessionCipher,
}

impl PublisherReceivedConnectorBundle {
    pub(crate) fn from_pake(
        binding: DiscoveryExchangeBinding,
        session_key: [u8; SESSION_KEY_BYTES],
        ke1: &[u8],
        ke2: &[u8],
        message: &OpaqueKe3ConnectorBundle,
    ) -> Result<(Self, PairingBundle), DiscoveryCryptoError> {
        let mut session = SessionCipher::new(binding, session_key, ke1, ke2, message.ke3());
        let bundle = session.decrypt_bundle(
            PairingRole::Connector,
            PairingRole::Publisher,
            PairingStage::ConnectorBundle,
            message.connector_bundle(),
        )?;
        session.include_frame(PairingStage::ConnectorBundle, message.connector_bundle());
        Ok((Self { session }, bundle))
    }

    /// Prepares the publisher bundle. The caller must not release the returned
    /// packet until the connector bundle has been durably accepted locally.
    pub fn publisher_bundle(
        mut self,
        bundle: PairingBundle,
    ) -> Result<(PublisherAwaitingConnectorCommit, PublisherBundle), DiscoveryCryptoError> {
        ensure_bundle_kind(self.session.binding(), &bundle)?;
        let encrypted = self.session.encrypt_bundle(
            PairingRole::Publisher,
            PairingRole::Connector,
            PairingStage::PublisherBundle,
            &bundle,
        )?;
        self.session
            .include_frame(PairingStage::PublisherBundle, &encrypted);
        Ok((
            PublisherAwaitingConnectorCommit {
                session: self.session,
            },
            PublisherBundle::new(encrypted),
        ))
    }

    pub fn binding(&self) -> &DiscoveryExchangeBinding {
        self.session.binding()
    }
}

#[must_use]
pub struct PublisherAwaitingConnectorCommit {
    session: SessionCipher,
}

impl PublisherAwaitingConnectorCommit {
    pub fn accept_connector_commit(
        mut self,
        message: ConnectorCommit,
    ) -> Result<PublisherReadyToCommit, DiscoveryCryptoError> {
        self.session.decrypt_commit(
            PairingRole::Connector,
            PairingRole::Publisher,
            PairingStage::ConnectorCommit,
            message.frame(),
        )?;
        self.session
            .include_frame(PairingStage::ConnectorCommit, message.frame());
        Ok(PublisherReadyToCommit {
            session: self.session,
        })
    }

    pub fn binding(&self) -> &DiscoveryExchangeBinding {
        self.session.binding()
    }
}

#[must_use]
pub struct PublisherReadyToCommit {
    session: SessionCipher,
}

impl PublisherReadyToCommit {
    pub fn commit(
        mut self,
    ) -> Result<(PublisherPairingComplete, PublisherCommit), DiscoveryCryptoError> {
        let encrypted = self.session.encrypt_commit(
            PairingRole::Publisher,
            PairingRole::Connector,
            PairingStage::PublisherCommit,
        )?;
        self.session
            .include_frame(PairingStage::PublisherCommit, &encrypted);
        Ok((
            PublisherPairingComplete {
                binding: self.session.binding,
            },
            PublisherCommit::new(encrypted),
        ))
    }

    pub fn binding(&self) -> &DiscoveryExchangeBinding {
        self.session.binding()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectorPairingComplete {
    binding: DiscoveryExchangeBinding,
}

impl ConnectorPairingComplete {
    pub fn binding(&self) -> &DiscoveryExchangeBinding {
        &self.binding
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublisherPairingComplete {
    binding: DiscoveryExchangeBinding,
}

impl PublisherPairingComplete {
    pub fn binding(&self) -> &DiscoveryExchangeBinding {
        &self.binding
    }
}

struct SessionCipher {
    binding: DiscoveryExchangeBinding,
    session_key: Zeroizing<[u8; SESSION_KEY_BYTES]>,
    transcript: [u8; SESSION_KEY_BYTES],
}

impl SessionCipher {
    fn new(
        binding: DiscoveryExchangeBinding,
        session_key: [u8; SESSION_KEY_BYTES],
        ke1: &[u8],
        ke2: &[u8],
        ke3: &[u8],
    ) -> Self {
        let mut hash = Sha512::new();
        hash.update(b"smart-explorer/discovery/handshake-transcript/v1");
        hash_field(&mut hash, b"binding", &binding.exchange_context());
        hash_field(&mut hash, b"opaque-ke1", ke1);
        hash_field(&mut hash, b"opaque-ke2", ke2);
        hash_field(&mut hash, b"opaque-ke3", ke3);
        Self {
            binding,
            session_key: Zeroizing::new(session_key),
            transcript: hash.finalize().into(),
        }
    }

    fn binding(&self) -> &DiscoveryExchangeBinding {
        &self.binding
    }

    fn encrypt_bundle(
        &self,
        sender: PairingRole,
        receiver: PairingRole,
        stage: PairingStage,
        bundle: &PairingBundle,
    ) -> Result<EncryptedFrame, DiscoveryCryptoError> {
        let plaintext = bundle.encode_plaintext();
        self.encrypt(sender, receiver, stage, plaintext.as_slice())
    }

    fn decrypt_bundle(
        &self,
        sender: PairingRole,
        receiver: PairingRole,
        stage: PairingStage,
        frame: &EncryptedFrame,
    ) -> Result<PairingBundle, DiscoveryCryptoError> {
        let plaintext = self.decrypt(sender, receiver, stage, frame)?;
        PairingBundle::decode_plaintext(plaintext, self.binding.offer().kind())
    }

    fn encrypt_commit(
        &self,
        sender: PairingRole,
        receiver: PairingRole,
        stage: PairingStage,
    ) -> Result<EncryptedFrame, DiscoveryCryptoError> {
        let plaintext = self.commit_plaintext(sender);
        self.encrypt(sender, receiver, stage, plaintext.as_slice())
    }

    fn decrypt_commit(
        &self,
        sender: PairingRole,
        receiver: PairingRole,
        stage: PairingStage,
        frame: &EncryptedFrame,
    ) -> Result<(), DiscoveryCryptoError> {
        let received = Zeroizing::new(self.decrypt(sender, receiver, stage, frame)?);
        let expected = self.commit_plaintext(sender);
        if received.as_slice() != expected.as_slice() {
            return Err(DiscoveryCryptoError::AuthenticationFailed);
        }
        Ok(())
    }

    fn commit_plaintext(&self, sender: PairingRole) -> Zeroizing<Vec<u8>> {
        let mut plaintext = Zeroizing::new(Vec::with_capacity(
            COMMIT_MAGIC.len() + 1 + self.transcript.len(),
        ));
        plaintext.extend_from_slice(COMMIT_MAGIC);
        plaintext.push(sender.tag());
        plaintext.extend_from_slice(&self.transcript);
        plaintext
    }

    fn encrypt(
        &self,
        sender: PairingRole,
        receiver: PairingRole,
        stage: PairingStage,
        plaintext: &[u8],
    ) -> Result<EncryptedFrame, DiscoveryCryptoError> {
        let aad = self.binding.aad(sender, receiver, stage);
        let key = self.derive_key(&aad)?;
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&key[..]));
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&ZERO_NONCE),
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| DiscoveryCryptoError::EncryptionFailed)?;
        EncryptedFrame::new(ciphertext)
    }

    fn decrypt(
        &self,
        sender: PairingRole,
        receiver: PairingRole,
        stage: PairingStage,
        frame: &EncryptedFrame,
    ) -> Result<Vec<u8>, DiscoveryCryptoError> {
        let aad = self.binding.aad(sender, receiver, stage);
        let key = self.derive_key(&aad)?;
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&key[..]));
        cipher
            .decrypt(
                Nonce::from_slice(&ZERO_NONCE),
                Payload {
                    msg: frame.as_bytes(),
                    aad: &aad,
                },
            )
            .map_err(|_| DiscoveryCryptoError::AuthenticationFailed)
    }

    fn derive_key(
        &self,
        aad: &[u8],
    ) -> Result<Zeroizing<[u8; AEAD_KEY_BYTES]>, DiscoveryCryptoError> {
        let hkdf = Hkdf::<Sha512>::new(Some(&self.transcript), &self.session_key[..]);
        let mut info = Vec::with_capacity(48 + aad.len());
        info.extend_from_slice(b"smart-explorer/discovery/aead-key/v1");
        info.extend_from_slice(aad);
        let mut key = Zeroizing::new([0u8; AEAD_KEY_BYTES]);
        hkdf.expand(&info, &mut key[..])
            .map_err(|_| DiscoveryCryptoError::EncryptionFailed)?;
        Ok(key)
    }

    fn include_frame(&mut self, stage: PairingStage, frame: &EncryptedFrame) {
        let mut hash = Sha512::new();
        hash.update(b"smart-explorer/discovery/transcript-next/v1");
        hash_field(&mut hash, b"previous", &self.transcript);
        hash_field(&mut hash, b"stage", &[stage.tag()]);
        hash_field(&mut hash, b"frame", frame.as_bytes());
        self.transcript = hash.finalize().into();
    }
}

impl fmt::Debug for SessionCipher {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionCipher")
            .field("binding", &self.binding)
            .field("session_key", &"[REDACTED]")
            .field("transcript", &"[REDACTED]")
            .finish()
    }
}

macro_rules! redacted_state_debug {
    ($($name:ident),+ $(,)?) => {
        $(
            impl fmt::Debug for $name {
                fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter
                        .debug_struct(stringify!($name))
                        .field("binding", self.binding())
                        .field("cryptographic_state", &"[REDACTED]")
                        .finish()
                }
            }
        )+
    };
}

redacted_state_debug!(
    ConnectorAwaitingPublisherBundle,
    ConnectorReceivedPublisherBundle,
    ConnectorAwaitingPublisherCommit,
    PublisherReceivedConnectorBundle,
    PublisherAwaitingConnectorCommit,
    PublisherReadyToCommit,
);

fn ensure_bundle_kind(
    binding: &DiscoveryExchangeBinding,
    bundle: &PairingBundle,
) -> Result<(), DiscoveryCryptoError> {
    if binding.offer().kind() != bundle.kind() {
        return Err(DiscoveryCryptoError::BundleKindMismatch);
    }
    Ok(())
}

fn hash_field(hash: &mut Sha512, name: &[u8], value: &[u8]) {
    hash.update((name.len() as u16).to_be_bytes());
    hash.update(name);
    hash.update((value.len() as u32).to_be_bytes());
    hash.update(value);
}
