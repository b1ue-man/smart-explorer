//! TLS and WebSocket handshake of the signaling connection (FC3): Mozilla
//! roots or a certificate pin (`#sha256=`), signaling-sized frame and message
//! limits, IP literals as TLS server names, and the relay trust that follows
//! a pinned server.

use std::io;
use std::net::{IpAddr, TcpStream};
use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::crypto::{
    verify_tls12_signature, verify_tls13_signature, CryptoProvider, WebPkiSupportedAlgorithms,
};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, ClientConnection, DigitallySignedStruct, RootCertStore,
    SignatureScheme, StreamOwned,
};
use sha2::{Digest, Sha256};
use tungstenite::handshake::HandshakeError;
use tungstenite::protocol::WebSocketConfig;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::WebSocket;

use crate::share::core::eio;
use crate::share::line::MAX_SIGNAL_LINE;
use crate::share::server_address::SignalEndpoint;

/// The signaling WebSocket never carries more than one signaling line.
pub(super) fn websocket_config() -> WebSocketConfig {
    WebSocketConfig {
        max_message_size: Some(MAX_SIGNAL_LINE),
        max_frame_size: Some(MAX_SIGNAL_LINE),
        ..WebSocketConfig::default()
    }
}

/// Upgrades a connected socket to the signaling WebSocket, through TLS for
/// `wss`. The socket's read and write timeouts bound both handshakes.
pub(super) fn handshake(
    endpoint: &SignalEndpoint,
    tcp: TcpStream,
) -> io::Result<WebSocket<MaybeTlsStream<TcpStream>>> {
    let url = endpoint
        .websocket_url()
        .ok_or_else(|| eio("Share-Server-Endpunkt ist kein WebSocket"))?;
    let stream = if endpoint.is_encrypted() {
        let config = client_config(endpoint.pin())?;
        let connection =
            ClientConnection::new(config, server_name(endpoint.host())?).map_err(eio)?;
        MaybeTlsStream::Rustls(StreamOwned::new(connection, tcp))
    } else {
        MaybeTlsStream::Plain(tcp)
    };
    match tungstenite::client::client_with_config(url.as_str(), stream, Some(websocket_config())) {
        Ok((socket, _response)) => Ok(socket),
        Err(HandshakeError::Failure(tungstenite::Error::Io(error))) => Err(certificate_hint(error)),
        Err(HandshakeError::Failure(error)) => Err(eio(error)),
        Err(HandshakeError::Interrupted(_)) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "Share-Server-Handshake hat das Zeitlimit ueberschritten",
        )),
    }
}

fn client_config(pin: Option<[u8; 32]>) -> io::Result<Arc<ClientConfig>> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(eio)?;
    let config = match pin {
        Some(pin) => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinnedServerCert::new(
                vec![pin],
                None,
                &provider,
            )))
            .with_no_client_auth(),
        None => builder
            .with_root_certificates(mozilla_roots())
            .with_no_client_auth(),
    };
    Ok(Arc::new(config))
}

fn mozilla_roots() -> RootCertStore {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    roots
}

/// TLS names carry IPv6 literals without brackets.
fn server_name(host: &str) -> io::Result<ServerName<'static>> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(ServerName::from(ip));
    }
    ServerName::try_from(host.to_string())
        .map_err(|_| eio(format!("ungueltiger TLS-Servername: {host}")))
}

fn certificate_hint(error: io::Error) -> io::Error {
    let rejected = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
        .is_some_and(|inner| matches!(inner, rustls::Error::InvalidCertificate(_)));
    if !rejected {
        return error;
    }
    io::Error::new(
        error.kind(),
        format!(
            "TLS-Zertifikat des Share-Servers abgelehnt ({error}); einen selbst signierten \
             Server mit #sha256=<Fingerabdruck> an der Adresse anheften"
        ),
    )
}

/// A pin is the SHA-256 of the DER certificate (as `openssl x509 -fingerprint
/// -sha256` prints it) or of its public key, which survives renewals.
pub(crate) fn certificate_matches_pin(cert: &CertificateDer<'_>, pin: &[u8; 32]) -> bool {
    if Sha256::digest(cert.as_ref()).as_slice() == pin.as_slice() {
        return true;
    }
    rustls::server::ParsedCertificate::try_from(cert).is_ok_and(|parsed| {
        Sha256::digest(parsed.subject_public_key_info().as_ref()).as_slice() == pin.as_slice()
    })
}

/// Accepts the pinned certificates; with `fallback`, other certificates
/// still need a publicly valid chain (relays of peers on other servers). The
/// handshake signature is always verified, so the server proves its key.
#[derive(Debug)]
pub(crate) struct PinnedServerCert {
    pins: Vec<[u8; 32]>,
    fallback: Option<Arc<WebPkiServerVerifier>>,
    algorithms: WebPkiSupportedAlgorithms,
}

impl PinnedServerCert {
    fn new(
        pins: Vec<[u8; 32]>,
        fallback: Option<Arc<WebPkiServerVerifier>>,
        provider: &CryptoProvider,
    ) -> Self {
        Self {
            pins,
            fallback,
            algorithms: provider.signature_verification_algorithms,
        }
    }
}

impl ServerCertVerifier for PinnedServerCert {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if self
            .pins
            .iter()
            .any(|pin| certificate_matches_pin(end_entity, pin))
        {
            return Ok(ServerCertVerified::assertion());
        }
        match &self.fallback {
            Some(webpki) => webpki.verify_server_cert(
                end_entity,
                intermediates,
                server_name,
                ocsp_response,
                now,
            ),
            None => Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            )),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// Relay trust of a pinned server: the pinned certificates plus publicly
/// valid ones. `None` keeps Iroh's default (embedded Mozilla roots).
pub(crate) fn relay_ca_tls_config(pins: &[[u8; 32]]) -> Option<iroh::tls::CaTlsConfig> {
    if pins.is_empty() {
        return None;
    }
    let pins = pins.to_vec();
    let verifier = move |provider: Arc<CryptoProvider>| -> io::Result<Arc<dyn ServerCertVerifier>> {
        let roots = Arc::new(mozilla_roots());
        let webpki = WebPkiServerVerifier::builder_with_provider(roots, provider.clone())
            .build()
            .map_err(io::Error::other)?;
        Ok(Arc::new(PinnedServerCert::new(
            pins.clone(),
            Some(webpki),
            &provider,
        )))
    };
    Some(iroh::tls::CaTlsConfig::custom_server_cert_verifier(
        Arc::new(verifier),
    ))
}

#[cfg(test)]
#[path = "signal_connection_tls_tests.rs"]
mod review_task_tests;
