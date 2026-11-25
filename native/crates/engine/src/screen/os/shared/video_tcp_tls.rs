//! TLS 1.3 of the direct TCP picture link (`video_tcp_link.rs`): keys and
//! configurations. The host is the TLS server whoever dialed (a
//! simultaneous open makes both sides TCP clients); it presents a fresh
//! Ed25519 raw public key (RFC 7250) whose SHA-256 the viewer pins from
//! `VIDEO_TCP_ANSWER`, which came over the already authenticated QUIC
//! control stream. A raw key instead of a self-signed certificate: the pin
//! covers the key either way, and the key needs no certificate generator on
//! Windows and Android (rcgen is a Linux dependency of the engine only). The
//! provider is the Share one with the X25519MLKEM768 hybrid first
//! (`share/core/pq_kx.rs`), as for every Share connection. After the
//! handshake the viewer writes the 32-byte session token, the host compares
//! it in constant time (`token_matches`) and answers `CONFIRM`
//! (`video_tcp_records.rs`).
use std::io;
use std::sync::Arc;

use ring::signature::{Ed25519KeyPair, KeyPair};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls13_signature_with_raw_key, WebPkiSupportedAlgorithms};
use rustls::pki_types::{
    CertificateDer, PrivatePkcs8KeyDer, ServerName, SubjectPublicKeyInfoDer, UnixTime,
};
use rustls::server::AlwaysResolvesServerRawPublicKeys;
use rustls::sign::CertifiedKey;
use rustls::{
    CertificateError, ClientConfig, ClientConnection, Connection, DigitallySignedStruct,
    ServerConfig, ServerConnection, SignatureScheme,
};

use crate::screen::core::video_tcp_wire::{TcpToken, TCP_ALPN, TOKEN_BYTES};

pub(super) fn tls_io(error: rustls::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let digest = ring::digest::digest(&ring::digest::SHA256, bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(digest.as_ref());
    out
}

/// The host's fresh TLS key of one link and the SHA-256 the viewer pins.
pub(super) struct HostKey {
    certified: Arc<CertifiedKey>,
    pub(super) cert_sha256: [u8; 32],
}

impl HostKey {
    pub(super) fn generate() -> io::Result<Self> {
        let failed = || io::Error::other("TLS-Schlüssel der TCP-Verbindung nicht erzeugbar");
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).map_err(|_| failed())?;
        let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).map_err(|_| failed())?;
        let spki = rustls::sign::public_key_to_spki(
            &rustls::pki_types::alg_id::ED25519,
            pair.public_key().as_ref(),
        );
        let signing = rustls::crypto::ring::sign::any_eddsa_type(&PrivatePkcs8KeyDer::from(
            pkcs8.as_ref().to_vec(),
        ))
        .map_err(tls_io)?;
        let cert = CertificateDer::from(spki.to_vec());
        Ok(Self {
            cert_sha256: sha256(cert.as_ref()),
            certified: Arc::new(CertifiedKey::new(vec![cert], signing)),
        })
    }

    pub(super) fn server_config(&self) -> io::Result<Arc<ServerConfig>> {
        let mut config = ServerConfig::builder_with_provider(crate::share::share_crypto_provider())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(tls_io)?
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(AlwaysResolvesServerRawPublicKeys::new(
                self.certified.clone(),
            )));
        config.alpn_protocols = vec![TCP_ALPN.to_vec()];
        config.send_tls13_tickets = 0;
        Ok(Arc::new(config))
    }
}

/// The viewer's check of the host key: exactly the pinned SHA-256, a
/// TLS 1.3 signature by that key.
#[derive(Debug)]
struct PinnedHostKey {
    cert_sha256: [u8; 32],
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for PinnedHostKey {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if !intermediates.is_empty() || sha256(end_entity.as_ref()) != self.cert_sha256 {
            return Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ));
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::PeerIncompatible(
            rustls::PeerIncompatible::Tls12NotOffered,
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature_with_raw_key(
            message,
            &SubjectPublicKeyInfoDer::from(cert.as_ref()),
            dss,
            &self.algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }

    fn requires_raw_public_keys(&self) -> bool {
        true
    }
}

/// The viewer's TLS configuration pinning `cert_sha256`; no SNI, no
/// resumption (every link has a fresh key).
pub(super) fn client_config(cert_sha256: [u8; 32]) -> io::Result<Arc<ClientConfig>> {
    let provider = crate::share::share_crypto_provider();
    let algorithms = provider.signature_verification_algorithms;
    let mut config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(tls_io)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedHostKey {
            cert_sha256,
            algorithms,
        }))
        .with_no_client_auth();
    config.alpn_protocols = vec![TCP_ALPN.to_vec()];
    config.enable_sni = false;
    config.resumption = rustls::client::Resumption::disabled();
    Ok(Arc::new(config))
}

pub(super) fn server_connection(config: &Arc<ServerConfig>) -> io::Result<Connection> {
    Ok(Connection::Server(
        ServerConnection::new(config.clone()).map_err(tls_io)?,
    ))
}

pub(super) fn client_connection(config: &Arc<ClientConfig>) -> io::Result<Connection> {
    // The name is never sent (no SNI) and never checked (the key is pinned).
    let name = ServerName::try_from("screen-tcp.invalid")
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    Ok(Connection::Client(
        ClientConnection::new(config.clone(), name).map_err(tls_io)?,
    ))
}

/// Constant-time comparison of the received token with the expected one.
pub(super) fn token_matches(expected: &TcpToken, received: &[u8; TOKEN_BYTES]) -> bool {
    let mut difference = 0u8;
    for (left, right) in expected.0.iter().zip(received) {
        difference |= left ^ right;
    }
    std::hint::black_box(difference) == 0
}
