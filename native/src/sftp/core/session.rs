use super::config::SftpConfig;
use super::host_key::{HostKeyVerdict, UnknownHostKey};
use super::io_err;
use super::known_hosts;
use russh::client;
use russh_sftp::client::SftpSession;
use std::borrow::Cow;
use std::collections::VecDeque;
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::task::JoinSet;
use tokio::time::Instant;

#[path = "session_auth.rs"]
mod auth;
#[path = "host_key_handler.rs"]
mod handler;

const SSH_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);
const SSH_KEEPALIVE_MAX_MISSED: usize = 3;
const TCP_ATTEMPT_DELAY: Duration = Duration::from_millis(250);
/// Receive window of every channel (SFTP and agent exec alike). One channel
/// moves at most one window per round trip; 16 MiB carries 1 Gbit/s up to a
/// 134 ms round trip. russh refills it on receipt, so it never bounds memory
/// (docs/refs/quic-sftp-throughput.md B.7).
pub(super) const SSH_WINDOW: u32 = 16 * 1024 * 1024;
/// OpenSSH's own session packet size (`CHAN_SES_PACKET_DEFAULT`, 32 KiB) and
/// russh's default; russh warns above 65535.
const SSH_MAX_PACKET: u32 = 32 * 1024;
/// Messages a channel buffers before the session loop waits for its reader
/// (and with it every other channel): one full window of full-size packets.
const SSH_CHANNEL_BUFFER: usize = (SSH_WINDOW / SSH_MAX_PACKET) as usize;

/// How the client treats the server's host key.
#[derive(Clone, Debug)]
pub(super) enum HostKeyMode {
    /// Pinned or OpenSSH-known keys only; an unknown key ends the handshake
    /// with its fingerprint (`UnknownHostKey::message`).
    Verify,
    /// Pins the presented key when its fingerprint is `expected`; a different
    /// recorded key is replaced only with `replace`.
    Trust { expected: String, replace: bool },
    /// Records the key and its verdict, then ends the handshake.
    Probe,
}

/// What the handshake learned about the host key.
#[derive(Clone, Debug, Default)]
pub(super) struct HostKeyOutcome {
    pub(super) presented: Option<UnknownHostKey>,
    pub(super) verdict: Option<HostKeyVerdict>,
    pub(super) error: Option<String>,
}

pub(super) struct Client {
    host: String,
    port: u16,
    mode: HostKeyMode,
    outcome: Arc<Mutex<HostKeyOutcome>>,
}

/// Why the SSH handshake did not produce a session.
enum HandshakeError {
    /// The host-key decision refused the server (detail for the person).
    HostKey(String),
    /// No host-key algorithm in common: the server only offers `ssh-rsa`
    /// with SHA-1 signatures, which new connections no longer accept.
    NoCommonHostKeyAlgorithm(russh::Error),
    Other(io::Error),
}

impl From<HandshakeError> for io::Error {
    fn from(error: HandshakeError) -> Self {
        match error {
            HandshakeError::HostKey(detail) => {
                io::Error::new(io::ErrorKind::PermissionDenied, detail)
            }
            HandshakeError::NoCommonHostKeyAlgorithm(error) => io_err(error),
            HandshakeError::Other(error) => error,
        }
    }
}

/// One SSH handshake; the host-key outcome is returned with the result,
/// because a probe learns the key exactly when the handshake is refused.
async fn handshake(
    host: &str,
    port: u16,
    mode: HostKeyMode,
    legacy_sha1: bool,
) -> (
    Result<client::Handle<Client>, HandshakeError>,
    HostKeyOutcome,
) {
    let config = Arc::new(client_config_with(legacy_sha1));
    let outcome = Arc::new(Mutex::new(HostKeyOutcome::default()));
    let handler = Client {
        host: host.to_string(),
        port,
        mode,
        outcome: outcome.clone(),
    };
    let result = match connect_tcp(host, port).await {
        Err(error) => Err(HandshakeError::Other(error)),
        Ok(socket) => {
            if config.nodelay {
                let _ = socket.set_nodelay(true);
            }
            client::connect_stream(config, socket, handler)
                .await
                .map_err(|error| classify(error, &outcome))
        }
    };
    let snapshot = outcome
        .lock()
        .map(|outcome| outcome.clone())
        .unwrap_or_default();
    (result, snapshot)
}

fn classify(error: russh::Error, outcome: &Mutex<HostKeyOutcome>) -> HandshakeError {
    if let Ok(detail) = outcome.lock() {
        if let Some(detail) = detail.error.clone() {
            return HandshakeError::HostKey(detail);
        }
    }
    if matches!(
        error,
        russh::Error::NoCommonAlgo {
            kind: russh::AlgorithmKind::Key,
            ..
        }
    ) {
        return HandshakeError::NoCommonHostKeyAlgorithm(error);
    }
    HandshakeError::Other(io_err(error))
}

pub(super) async fn connect_async(
    cfg: &SftpConfig,
) -> io::Result<(client::Handle<Client>, SftpSession)> {
    let mut session = match handshake(&cfg.host, cfg.port, HostKeyMode::Verify, false)
        .await
        .0
    {
        Ok(session) => session,
        // A server pinned before SHA-1 signatures were dropped keeps working;
        // its pinned key still has to match. New servers never get SHA-1.
        Err(HandshakeError::NoCommonHostKeyAlgorithm(_))
            if known_hosts::has_pin(&cfg.host, cfg.port) =>
        {
            handshake(&cfg.host, cfg.port, HostKeyMode::Verify, true)
                .await
                .0?
        }
        Err(error) => return Err(error.into()),
    };
    if !auth::authenticate(&mut session, cfg).await? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "SFTP-Authentifizierung fehlgeschlagen",
        ));
    }

    let channel = session.channel_open_session().await.map_err(io_err)?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(io_err)?;
    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(io_err)?;
    Ok((session, sftp))
}

/// Host-key handshake without login: `Probe` reports the presented key and
/// its verdict, `Trust` pins a confirmed key.
pub(super) async fn host_key_async(
    host: &str,
    port: u16,
    mode: HostKeyMode,
) -> io::Result<HostKeyOutcome> {
    let probing = matches!(mode, HostKeyMode::Probe);
    let mut legacy_sha1 = false;
    loop {
        let (result, outcome) = handshake(host, port, mode.clone(), legacy_sha1).await;
        match result {
            Ok(session) => {
                let _ = session
                    .disconnect(russh::Disconnect::ByApplication, "", "")
                    .await;
                return Ok(outcome);
            }
            // A pinned legacy server is checked the way `connect_async` does.
            Err(HandshakeError::NoCommonHostKeyAlgorithm(_))
                if !legacy_sha1 && known_hosts::has_pin(host, port) =>
            {
                legacy_sha1 = true;
            }
            // A probe ends the handshake itself once it saw the key.
            Err(_) if probing && outcome.presented.is_some() && outcome.error.is_none() => {
                return Ok(outcome);
            }
            Err(error) => return Err(error.into()),
        }
    }
}

async fn connect_tcp(host: &str, port: u16) -> io::Result<TcpStream> {
    let resolved = tokio::net::lookup_host((host, port)).await?;
    let addresses = interleave_addresses(resolved.collect());
    staggered_connect(addresses).await
}

pub(super) fn interleave_addresses(addresses: Vec<SocketAddr>) -> Vec<SocketAddr> {
    let mut unique = Vec::with_capacity(addresses.len());
    for address in addresses {
        if !unique.contains(&address) {
            unique.push(address);
        }
    }
    let Some(first) = unique.first() else {
        return unique;
    };

    let mut ipv4: VecDeque<_> = unique.iter().copied().filter(SocketAddr::is_ipv4).collect();
    let mut ipv6: VecDeque<_> = unique.iter().copied().filter(SocketAddr::is_ipv6).collect();
    let mut take_ipv6 = first.is_ipv6();
    let mut interleaved = Vec::with_capacity(unique.len());
    while !ipv4.is_empty() || !ipv6.is_empty() {
        let next = if take_ipv6 {
            ipv6.pop_front().or_else(|| ipv4.pop_front())
        } else {
            ipv4.pop_front().or_else(|| ipv6.pop_front())
        };
        if let Some(address) = next {
            interleaved.push(address);
        }
        take_ipv6 = !take_ipv6;
    }
    interleaved
}

async fn staggered_connect(addresses: Vec<SocketAddr>) -> io::Result<TcpStream> {
    let mut addresses = addresses.into_iter();
    let first = addresses.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "SFTP host did not resolve to an IP address",
        )
    })?;
    let mut attempts = JoinSet::new();
    spawn_connect(&mut attempts, first);
    let mut next = addresses.next();
    let mut next_launch = Instant::now() + TCP_ATTEMPT_DELAY;
    let mut last_error = None;

    loop {
        let completed = if next.is_some() {
            match tokio::time::timeout_at(next_launch, attempts.join_next()).await {
                Ok(completed) => completed,
                Err(_) => {
                    let Some(address) = next.take() else {
                        continue;
                    };
                    spawn_connect(&mut attempts, address);
                    next = addresses.next();
                    next_launch = Instant::now() + TCP_ATTEMPT_DELAY;
                    continue;
                }
            }
        } else {
            attempts.join_next().await
        };

        match completed {
            Some(Ok(Ok(stream))) => {
                attempts.abort_all();
                return Ok(stream);
            }
            Some(Ok(Err(error))) => last_error = Some(error),
            Some(Err(error)) => last_error = Some(io::Error::other(error.to_string())),
            None => {}
        }

        if attempts.is_empty() {
            if let Some(address) = next.take() {
                spawn_connect(&mut attempts, address);
                next = addresses.next();
                next_launch = Instant::now() + TCP_ATTEMPT_DELAY;
            } else {
                return Err(last_error.unwrap_or_else(|| {
                    io::Error::new(io::ErrorKind::NotConnected, "SFTP TCP connection failed")
                }));
            }
        }
    }
}

fn spawn_connect(attempts: &mut JoinSet<io::Result<TcpStream>>, address: SocketAddr) {
    attempts.spawn(async move { TcpStream::connect(address).await });
}

pub(super) fn client_config() -> client::Config {
    client_config_with(false)
}

/// `legacy_sha1` also offers `ssh-rsa` host keys with SHA-1 signatures
/// (OpenSSH turned them off by default in 8.8); only a server Smart Explorer
/// pinned earlier gets it.
fn client_config_with(legacy_sha1: bool) -> client::Config {
    let mut preferred = russh::Preferred::default();
    if !legacy_sha1 {
        preferred.key = Cow::Owned(
            preferred
                .key
                .iter()
                .filter(|algorithm| {
                    !matches!(algorithm, russh::keys::Algorithm::Rsa { hash: None })
                })
                .cloned()
                .collect(),
        );
    }
    preferred.cipher = Cow::Owned(cipher_order(hardware_aes()));
    client::Config {
        keepalive_interval: Some(SSH_KEEPALIVE_INTERVAL),
        keepalive_max: SSH_KEEPALIVE_MAX_MISSED,
        window_size: SSH_WINDOW,
        maximum_packet_size: SSH_MAX_PACKET,
        channel_buffer_size: SSH_CHANNEL_BUFFER,
        preferred,
        ..client::Config::default()
    }
}

/// Ciphers offered to the server, best first. With AES instructions in the
/// CPU `aes256-gcm@openssh.com` (ring) is faster than ChaCha20-Poly1305 and
/// goes first; without them ChaCha20 stays first, as in russh
/// (`negotiation.rs` `CIPHER_ORDER`, 0.64.1 lines 185-191). The server picks
/// the first of this list that it also supports.
pub(super) fn cipher_order(hardware_aes: bool) -> Vec<russh::cipher::Name> {
    use russh::cipher::{AES_128_CTR, AES_192_CTR, AES_256_CTR, AES_256_GCM, CHACHA20_POLY1305};
    if hardware_aes {
        vec![
            AES_256_GCM,
            CHACHA20_POLY1305,
            AES_256_CTR,
            AES_192_CTR,
            AES_128_CTR,
        ]
    } else {
        vec![
            CHACHA20_POLY1305,
            AES_256_GCM,
            AES_256_CTR,
            AES_192_CTR,
            AES_128_CTR,
        ]
    }
}

/// Whether this CPU has AES (and carry-less multiply for GCM) instructions,
/// the ones `ring` uses for AES-GCM.
pub(super) fn hardware_aes() -> bool {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        std::is_x86_feature_detected!("aes") && std::is_x86_feature_detected!("pclmulqdq")
    }
    #[cfg(target_arch = "aarch64")]
    {
        std::arch::is_aarch64_feature_detected!("aes")
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_task_sftp_cipher_order_prefers_aes_gcm_only_with_hardware_aes() {
        use russh::cipher::{AES_256_GCM, CHACHA20_POLY1305};
        let with = cipher_order(true);
        let without = cipher_order(false);
        assert_eq!(with.first(), Some(&AES_256_GCM));
        assert_eq!(with.get(1), Some(&CHACHA20_POLY1305));
        assert_eq!(without.first(), Some(&CHACHA20_POLY1305));
        assert_eq!(without.get(1), Some(&AES_256_GCM));
        assert_eq!(with.len(), 5, "same five ciphers either way");
        let mut a: Vec<&str> = with.iter().map(AsRef::as_ref).collect();
        let mut b: Vec<&str> = without.iter().map(AsRef::as_ref).collect();
        a.sort_unstable();
        b.sort_unstable();
        assert_eq!(a, b);
        // The order russh itself uses stays the order without AES.
        assert_eq!(
            without,
            russh::Preferred::DEFAULT.cipher.to_vec(),
            "no AES instructions: russh's own order"
        );
        // The configuration carries the order for this CPU.
        assert_eq!(
            client_config().preferred.cipher.to_vec(),
            cipher_order(hardware_aes())
        );
    }

    #[test]
    fn remote_drive_task_sftp_client_config_enables_bounded_keepalives() {
        let config = client_config();

        assert_eq!(config.keepalive_interval, Some(Duration::from_secs(15)));
        assert_eq!(config.keepalive_max, 3);
        assert_eq!(config.inactivity_timeout, None);
    }

    /// Pins of 0.5.180 hold the key type russh 0.61.2 negotiated: the first
    /// entry of its own `Preferred::DEFAULT.key` that the server supports
    /// (russh 0.61.2 negotiation.rs lines 155-173 and 378-402). russh 0.64.1
    /// lists the same algorithms in the same order (negotiation.rs lines
    /// 213-231 and 484-508), so the type a server presents, and with it the
    /// fingerprint an old pin holds, stays the same. A russh update that
    /// reorders the list must fail here before it breaks stored pins.
    #[test]
    fn luecken_task_host_key_algorithm_order_matches_0_5_180() {
        use russh::keys::{Algorithm, EcdsaCurve, HashAlg};

        let expected = vec![
            Algorithm::Ed25519,
            Algorithm::Ecdsa {
                curve: EcdsaCurve::NistP256,
            },
            Algorithm::Ecdsa {
                curve: EcdsaCurve::NistP384,
            },
            Algorithm::Ecdsa {
                curve: EcdsaCurve::NistP521,
            },
            Algorithm::Rsa {
                hash: Some(HashAlg::Sha512),
            },
            Algorithm::Rsa {
                hash: Some(HashAlg::Sha256),
            },
        ];
        assert_eq!(client_config().preferred.key.to_vec(), expected);
        let mut with_sha1 = expected;
        with_sha1.push(Algorithm::Rsa { hash: None });
        assert_eq!(client_config_with(true).preferred.key.to_vec(), with_sha1);
        // No host-key certificate algorithm is offered, so no server can
        // present a certificate instead of the pinned bare key.
        assert!(client_config().preferred.host_key_certificates.is_empty());
    }

    #[test]
    fn luecken_task_new_connections_never_offer_sha1_host_key_signatures() {
        let sha1 = russh::keys::Algorithm::Rsa { hash: None };
        assert!(!client_config().preferred.key.contains(&sha1));
        assert!(client_config()
            .preferred
            .key
            .contains(&russh::keys::Algorithm::Ed25519));
        assert!(client_config_with(true).preferred.key.contains(&sha1));
        assert_eq!(
            client_config().preferred.kex.first(),
            Some(&russh::kex::MLKEM768X25519_SHA256)
        );
    }
}
