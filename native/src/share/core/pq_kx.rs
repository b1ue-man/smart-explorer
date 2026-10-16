//! Post-quantum hybrid key exchange for the Share TLS handshake.
//!
//! `X25519MLKEM768` (code point 0x11ec, RFC 10024, August 2026, formerly
//! draft-ietf-tls-ecdhe-mlkem) combines ML-KEM-768 (FIPS 203, RustCrypto
//! `ml-kem`) with X25519 from rustls' ring provider. Both shares and the
//! shared secret put the ML-KEM part first (client share 1184 + 32 bytes,
//! server share 1088 + 32 bytes, secret 32 + 32 bytes). The layout and the
//! split into `start`/`start_and_complete` follow rustls' own aws-lc-rs hybrid
//! (`crypto/aws_lc_rs/pq/hybrid.rs`, rustls 0.23.46); the ML-KEM calls follow
//! russh's `kex/hybrid_mlkem.rs` (read in russh 0.61.2, same `ml-kem` 0.3.2).
//!
//! Input checks required by RFC 10024 sections 4.2 and 4.3: the server runs
//! the FIPS 203 section 7.2 encapsulation key check (length and modulus) inside
//! `EncapsulationKey::new_from_slice`; the client checks the ciphertext length;
//! ring rejects an all-zero X25519 shared secret (RFC 7748 section 6.1). Each
//! failure becomes `PeerMisbehaved::InvalidKeyShare`, which rustls sends as an
//! `illegal_parameter` alert.
//!
//! The provider keeps every ring default and only puts the hybrid group in
//! front. A client offers the hybrid share plus its X25519 part
//! (`hybrid_component`), so a peer that predates this module completes plain
//! X25519 without an extra round trip, and two updated peers agree on the
//! hybrid group. Recorded traffic of an updated pair can then not be
//! decrypted later by breaking X25519 alone.
use std::sync::Arc;

use ml_kem::kem::{Decapsulate, DecapsulationKey, Encapsulate, EncapsulationKey};
use ml_kem::{Kem, KeyExport, MlKem768, TryKeyInit};
use rustls::crypto::{
    ActiveKeyExchange, CompletedKeyExchange, CryptoProvider, SharedSecret, SupportedKxGroup,
};
use rustls::ffdhe_groups::FfdheGroup;
use rustls::{Error, NamedGroup, PeerMisbehaved, ProtocolVersion};

/// ML-KEM-768 encapsulation key (client share part), FIPS 203 table 3.
const MLKEM768_ENCAPSULATION_KEY_LEN: usize = 1184;
/// ML-KEM-768 ciphertext (server share part), FIPS 203 table 3.
const MLKEM768_CIPHERTEXT_LEN: usize = 1088;
/// X25519 public value, RFC 7748.
const X25519_LEN: usize = 32;
/// Both component secrets are 32 bytes.
const HYBRID_SECRET_LEN: usize = 64;

/// The hybrid group, usable only with TLS 1.3 (all Share connections).
pub(crate) static X25519MLKEM768: &dyn SupportedKxGroup = &HybridX25519MlKem768;

/// rustls' ring provider with `X25519MLKEM768` as the preferred group,
/// followed by the unchanged ring defaults (X25519, P-256, P-384).
pub(crate) fn share_crypto_provider() -> Arc<CryptoProvider> {
    let mut provider = rustls::crypto::ring::default_provider();
    let mut groups: Vec<&'static dyn SupportedKxGroup> = Vec::with_capacity(4);
    groups.push(X25519MLKEM768);
    groups.extend(
        provider
            .kx_groups
            .iter()
            .copied()
            .filter(|group| group.name() != NamedGroup::X25519MLKEM768),
    );
    provider.kx_groups = groups;
    Arc::new(provider)
}

#[derive(Debug)]
struct HybridX25519MlKem768;

impl SupportedKxGroup for HybridX25519MlKem768 {
    fn start(&self) -> Result<Box<dyn ActiveKeyExchange>, Error> {
        let classical = rustls::crypto::ring::kx_group::X25519.start()?;
        let (decapsulation, encapsulation) = MlKem768::generate_keypair_from_rng(&mut rand::rng());
        let encapsulation = encapsulation.to_bytes();
        let mut combined =
            Vec::with_capacity(MLKEM768_ENCAPSULATION_KEY_LEN.saturating_add(X25519_LEN));
        combined.extend_from_slice(encapsulation.as_slice());
        combined.extend_from_slice(classical.pub_key());
        if combined.len() != MLKEM768_ENCAPSULATION_KEY_LEN + X25519_LEN {
            return Err(Error::General(
                "hybrid key share has an unexpected length".into(),
            ));
        }
        Ok(Box::new(ActiveHybrid {
            classical,
            decapsulation: Box::new(decapsulation),
            combined,
        }))
    }

    fn start_and_complete(&self, client_share: &[u8]) -> Result<CompletedKeyExchange, Error> {
        if client_share.len() != MLKEM768_ENCAPSULATION_KEY_LEN + X25519_LEN {
            return Err(invalid_share());
        }
        let (pq_share, classical_share) = client_share.split_at(MLKEM768_ENCAPSULATION_KEY_LEN);
        let encapsulation =
            EncapsulationKey::<MlKem768>::new_from_slice(pq_share).map_err(|_| invalid_share())?;
        let classical =
            rustls::crypto::ring::kx_group::X25519.start_and_complete(classical_share)?;
        let (ciphertext, pq_secret) = encapsulation.encapsulate_with_rng(&mut rand::rng());
        let mut pub_key = Vec::with_capacity(MLKEM768_CIPHERTEXT_LEN + X25519_LEN);
        pub_key.extend_from_slice(ciphertext.as_slice());
        pub_key.extend_from_slice(&classical.pub_key);
        let mut secret = Vec::with_capacity(HYBRID_SECRET_LEN);
        secret.extend_from_slice(pq_secret.as_slice());
        secret.extend_from_slice(classical.secret.secret_bytes());
        Ok(CompletedKeyExchange {
            group: NamedGroup::X25519MLKEM768,
            pub_key,
            secret: SharedSecret::from(secret),
        })
    }

    fn ffdhe_group(&self) -> Option<FfdheGroup<'static>> {
        None
    }

    fn name(&self) -> NamedGroup {
        NamedGroup::X25519MLKEM768
    }

    fn usable_for_version(&self, version: ProtocolVersion) -> bool {
        version == ProtocolVersion::TLSv1_3
    }
}

struct ActiveHybrid {
    classical: Box<dyn ActiveKeyExchange>,
    decapsulation: Box<DecapsulationKey<MlKem768>>,
    combined: Vec<u8>,
}

impl ActiveKeyExchange for ActiveHybrid {
    /// `peer_pub_key` is the server's ML-KEM ciphertext followed by its
    /// X25519 public value.
    fn complete(self: Box<Self>, peer_pub_key: &[u8]) -> Result<SharedSecret, Error> {
        if peer_pub_key.len() != MLKEM768_CIPHERTEXT_LEN + X25519_LEN {
            return Err(invalid_share());
        }
        let (pq_share, classical_share) = peer_pub_key.split_at(MLKEM768_CIPHERTEXT_LEN);
        let ciphertext =
            ml_kem::Ciphertext::<MlKem768>::try_from(pq_share).map_err(|_| invalid_share())?;
        let pq_secret = self.decapsulation.decapsulate(&ciphertext);
        let classical = self.classical.complete(classical_share)?;
        let mut secret = Vec::with_capacity(HYBRID_SECRET_LEN);
        secret.extend_from_slice(pq_secret.as_slice());
        secret.extend_from_slice(classical.secret_bytes());
        Ok(SharedSecret::from(secret))
    }

    /// The X25519 part is offered as a separate key share as well, so a peer
    /// without the hybrid group completes the handshake without a retry.
    fn hybrid_component(&self) -> Option<(NamedGroup, &[u8])> {
        Some((self.classical.group(), self.classical.pub_key()))
    }

    fn complete_hybrid_component(
        self: Box<Self>,
        peer_pub_key: &[u8],
    ) -> Result<SharedSecret, Error> {
        self.classical.complete(peer_pub_key)
    }

    fn pub_key(&self) -> &[u8] {
        &self.combined
    }

    fn ffdhe_group(&self) -> Option<FfdheGroup<'static>> {
        None
    }

    fn group(&self) -> NamedGroup {
        NamedGroup::X25519MLKEM768
    }
}

fn invalid_share() -> Error {
    Error::PeerMisbehaved(PeerMisbehaved::InvalidKeyShare)
}

#[cfg(test)]
#[path = "pq_kx_tests.rs"]
mod tests;
