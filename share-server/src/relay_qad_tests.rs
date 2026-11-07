//! QUIC address discovery beside the relay: the plan, the pre-check, the
//! fallback without QAD and the start through the relay (plan M1, M2).
//! The limits of the service are in `relay_qad_limits_tests.rs`.
//!
//! `config` and `start_at` set the process-wide QAD limits and `start_at`
//! reads `SE_IROH_QAD_BIND`; a test that reaches either takes [`Settings`],
//! which serializes them and restores the process on drop.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, UdpSocket};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use iroh_relay::defaults::DEFAULT_RELAY_QUIC_PORT;
use iroh_relay::quic::qad_testing::{probe, reset_server_limits};
use iroh_relay::server::{QuicConfig, RelayConfig, Server, ServerConfig, SpawnError};
use rustls::pki_types::PrivatePkcs8KeyDer;

use super::{
    config, config_or_ipv4, failure, listening_line, plan, restart_without_qad, QadPlan,
    RESTART_ATTEMPTS, RESTART_PAUSE,
};
use crate::idle::Keepalive;
use crate::relay::{start_at, RelayOptions};
use crate::relay_access::RelayAdmissions;

#[path = "relay_qad_limits_tests.rs"]
mod limits;

/// Every test ends within this time.
const TEST_LIMIT: Duration = Duration::from_secs(60);

static PROCESS: Mutex<()> = Mutex::new(());

/// Holds the lock of the process-wide QAD settings, sets the given
/// environment variables and restores everything when dropped.
struct Settings {
    names: Vec<&'static str>,
    _lock: MutexGuard<'static, ()>,
}

impl Settings {
    fn new(variables: &[(&'static str, &str)]) -> Self {
        let lock = PROCESS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::env::remove_var("SE_IROH_QAD_DISABLE");
        std::env::remove_var("SE_IROH_QAD_BIND");
        for (name, value) in variables {
            std::env::set_var(name, value);
        }
        Self {
            names: variables.iter().map(|(name, _)| *name).collect(),
            _lock: lock,
        }
    }
}

impl Drop for Settings {
    fn drop(&mut self) {
        for name in &self.names {
            std::env::remove_var(name);
        }
        reset_server_limits();
    }
}

/// A self-signed certificate for `localhost` with the server and client
/// configuration that trust each other.
struct TestTls {
    certificate: rcgen::CertifiedKey<rcgen::KeyPair>,
}

impl TestTls {
    fn new() -> Self {
        Self {
            certificate: rcgen::generate_simple_self_signed(vec!["localhost".into()])
                .expect("self-signed certificate"),
        }
    }

    fn provider() -> Arc<rustls::crypto::CryptoProvider> {
        Arc::new(rustls::crypto::ring::default_provider())
    }

    /// The server side. With `tls12_only` QAD cannot use it: QUIC takes its
    /// initial cipher suite from the provider, and this provider has TLS 1.2
    /// suites only (limiting the protocol versions alone would leave the
    /// TLS 1.3 suites in the provider).
    fn server(&self, tls12_only: bool) -> rustls::ServerConfig {
        let mut provider = rustls::crypto::ring::default_provider();
        if tls12_only {
            provider
                .cipher_suites
                .retain(|suite| suite.tls13().is_none());
        }
        let builder = rustls::ServerConfig::builder_with_provider(Arc::new(provider));
        let versions = if tls12_only {
            builder.with_protocol_versions(&[&rustls::version::TLS12])
        } else {
            builder.with_safe_default_protocol_versions()
        };
        let key = PrivatePkcs8KeyDer::from(self.certificate.signing_key.serialize_der());
        versions
            .expect("protocol versions")
            .with_no_client_auth()
            .with_single_cert(vec![self.certificate.cert.der().clone()], key.into())
            .expect("certificate")
    }

    fn client(&self) -> rustls::ClientConfig {
        let mut roots = rustls::RootCertStore::empty();
        roots
            .add(self.certificate.cert.der().clone())
            .expect("root certificate");
        rustls::ClientConfig::builder_with_provider(Self::provider())
            .with_safe_default_protocol_versions()
            .expect("protocol versions")
            .with_root_certificates(roots)
            .with_no_client_auth()
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime")
}

fn block_on_within<F: std::future::Future>(
    runtime: &tokio::runtime::Runtime,
    future: F,
) -> F::Output {
    runtime
        .block_on(tokio::time::timeout(TEST_LIMIT, future))
        .expect("the test exceeded its time limit")
}

fn address(text: &str) -> SocketAddr {
    text.parse().expect("socket address")
}

fn options(tls: rustls::ServerConfig) -> RelayOptions {
    RelayOptions {
        keepalive: Keepalive::default(),
        tls: Some(tls),
        plaintext_fallback: false,
        trusted_proxy: false,
        admissions: Arc::new(RelayAdmissions::default()),
    }
}

#[test]
fn qad_plan_selects_the_bind_address() {
    assert_eq!(DEFAULT_RELAY_QUIC_PORT, 7842);
    let listens = |relay: &str, bind: Option<&str>| plan(address(relay), true, None, bind);
    let on = |text: &str| QadPlan::Listen(address(text));

    // The relay's address with the QAD port, unless a bind overrides it.
    assert_eq!(listens("127.0.0.1:51821", None), on("127.0.0.1:7842"));
    assert_eq!(listens("[::1]:51821", None), on("[::1]:7842"));
    assert_eq!(
        listens("0.0.0.0:444", Some("127.0.0.1:0")),
        on("127.0.0.1:0")
    );
    assert_eq!(
        listens("0.0.0.0:51821", Some("0.0.0.0:7842")),
        on("0.0.0.0:7842"),
        "an explicit bind keeps its family"
    );
    // A blank bind counts as not set.
    assert_eq!(listens("127.0.0.1:51821", Some("  ")), on("127.0.0.1:7842"));
    assert_eq!(listens("127.0.0.1:51821", Some("")), on("127.0.0.1:7842"));
    // A relay on 0.0.0.0 gets the dual-stack address (R-SERVER-08).
    assert_eq!(listens("0.0.0.0:51821", None), on("[::]:7842"));
    assert_eq!(listens("0.0.0.0:51821", Some("  ")), on("[::]:7842"));
}

#[test]
fn qad_plan_stays_off_and_names_the_reason() {
    let relay = address("127.0.0.1:51821");
    let off = |tls: bool, disable: Option<&str>, bind: Option<&str>| match plan(
        relay, tls, disable, bind,
    ) {
        QadPlan::Off(reason) => Some(reason),
        QadPlan::Listen(_) => None,
    };

    // SE_IROH_QAD_DISABLE reads like SE_IROH_RELAY_DISABLE: 1 or true.
    for value in ["1", "true", "TRUE", "True"] {
        assert_eq!(
            off(true, Some(value), None).as_deref(),
            Some("QAD disabled via SE_IROH_QAD_DISABLE"),
            "{value}"
        );
    }
    for value in ["0", "false", "", "yes"] {
        assert_eq!(off(true, Some(value), None), None, "{value:?}");
    }
    // Switched off beats every other reason.
    assert!(off(false, Some("1"), Some("not-an-address"))
        .expect("off")
        .contains("disabled"));

    // QAD inherits the relay's TLS; without it the relay serves plain HTTP.
    let reason = off(false, None, None).expect("no TLS");
    assert!(reason.contains("QAD needs TLS"), "{reason}");
    assert!(off(false, None, Some("127.0.0.1:0")).is_some());

    // A bind that is no address ends QAD, not the server (decision D3).
    let reason = off(true, None, Some("not-an-address")).expect("invalid bind");
    assert!(
        reason.starts_with("QAD off: invalid SE_IROH_QAD_BIND not-an-address"),
        "{reason}"
    );
}

#[test]
fn qad_precheck_reports_a_busy_udp_port() {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("udp socket");
    let busy = socket.local_addr().expect("address");
    let reason = config(busy).err().expect("a busy UDP port");
    assert!(reason.starts_with("QAD off: "), "{reason}");
    assert!(
        reason.contains(&format!("cannot bind UDP {busy}")),
        "{reason}"
    );
}

#[test]
fn qad_listening_line_and_dual_stack_bind() {
    assert_eq!(
        listening_line(address("127.0.0.1:7842")),
        "se-share-server QAD listening on 127.0.0.1:7842 (UDP)"
    );
    assert_eq!(
        listening_line(address("[::]:7842")),
        "se-share-server QAD listening on [::]:7842 (UDP)"
    );
    let ipv4_only = listening_line(address("0.0.0.0:7842"));
    assert!(ipv4_only.contains("IPv4 only"), "{ipv4_only}");
    assert!(
        ipv4_only.contains("SE_IROH_QAD_BIND=[::]:7842"),
        "{ipv4_only}"
    );

    // [::] binds both families; a host without IPv6 falls back to 0.0.0.0.
    let _settings = Settings::new(&[]);
    let dual_stack = address("[::]:0");
    let has_ipv6 = UdpSocket::bind(dual_stack).is_ok();
    let (bound, _quic) = config_or_ipv4(dual_stack).expect("dual-stack or IPv4 fallback");
    let expected = if has_ipv6 {
        dual_stack
    } else {
        address("0.0.0.0:0")
    };
    assert_eq!(bound, expected);
}

/// A real `BindTcpListener` error: the relay cannot take an address that
/// another listener holds.
fn bind_failure(runtime: &tokio::runtime::Runtime, busy: SocketAddr) -> Result<i32, SpawnError> {
    block_on_within(runtime, async {
        let mut config = ServerConfig::default();
        config.relay = Some(RelayConfig::new(busy));
        Server::spawn(config).await.map(|_| 7)
    })
}

/// A real `QuicSpawn` error: QAD without any TLS configuration.
fn quic_failure(runtime: &tokio::runtime::Runtime) -> Result<i32, SpawnError> {
    block_on_within(runtime, async {
        let mut config = ServerConfig::default();
        config.quic = Some(QuicConfig::new((Ipv4Addr::LOCALHOST, 0)));
        Server::spawn(config).await.map(|_| 7)
    })
}

#[test]
fn qad_restart_retries_only_bind_errors() {
    let runtime = runtime();
    let holder = TcpListener::bind("127.0.0.1:0").expect("listener");
    let busy = holder.local_addr().expect("address");
    assert_eq!(RESTART_PAUSE, Duration::from_millis(50));

    // Two bind errors, then the start works.
    let mut calls = 0;
    let mut pauses = Vec::new();
    let result = restart_without_qad(
        || {
            calls += 1;
            if calls <= 2 {
                bind_failure(&runtime, busy)
            } else {
                Ok(7)
            }
        },
        |pause| pauses.push(pause),
    );
    assert!(matches!(result, Ok(7)));
    assert_eq!(calls, 3);
    assert_eq!(pauses, vec![RESTART_PAUSE; 2]);

    // Bind errors without end: exactly RESTART_ATTEMPTS attempts.
    let mut calls = 0;
    let mut pauses = 0;
    let result = restart_without_qad(
        || {
            calls += 1;
            bind_failure(&runtime, busy)
        },
        |_| pauses += 1,
    );
    assert!(matches!(result, Err(SpawnError::BindTcpListener { .. })));
    assert_eq!(calls, RESTART_ATTEMPTS);
    assert_eq!(pauses, RESTART_ATTEMPTS - 1);
    let bind_error = result.err().expect("the last bind error");
    assert_eq!(failure(&bind_error, address("127.0.0.1:7842")), None);

    // Any other start error is final at once, and it is a failed QAD start.
    let mut calls = 0;
    let mut pauses = 0;
    let result = restart_without_qad(
        || {
            calls += 1;
            quic_failure(&runtime)
        },
        |_| pauses += 1,
    );
    assert_eq!((calls, pauses), (1, 0));
    let quic_error = result.err().expect("the QAD error");
    assert!(matches!(quic_error, SpawnError::QuicSpawn { .. }));
    let reason = failure(&quic_error, address("127.0.0.1:7842")).expect("a QAD failure");
    assert!(
        reason.starts_with("QAD off: cannot start on UDP 127.0.0.1:7842: "),
        "{reason}"
    );
}

#[test]
fn qad_tls12_only_starts_without_qad() {
    let _settings = Settings::new(&[("SE_IROH_QAD_BIND", "127.0.0.1:0")]);
    let tls = TestTls::new();
    let guard = start_at(address("127.0.0.1:0"), options(tls.server(true)))
        .unwrap_or_else(|error| panic!("the relay must start without QAD: {error}"));
    assert!(guard._server.quic_addr().is_none());
    assert!(guard._server.https_addr().is_some());
}

#[test]
fn qad_reports_the_clients_own_socket_address() {
    let _settings = Settings::new(&[("SE_IROH_QAD_BIND", "127.0.0.1:0")]);
    let tls = TestTls::new();
    let guard = start_at(address("127.0.0.1:0"), options(tls.server(false)))
        .unwrap_or_else(|error| panic!("the relay must start with QAD: {error}"));
    let quic = guard._server.quic_addr().expect("QAD address");
    assert_ne!(quic.port(), 0, "a bind to port 0 reports the real port");
    assert_eq!(quic.ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));

    let runtime = runtime();
    block_on_within(&runtime, async {
        let client = probe(quic, "localhost", tls.client())
            .await
            .expect("QAD connection");
        let local = client.local_addr().expect("client socket address");
        assert_eq!(
            client.observed(Duration::from_secs(5)).await,
            Some(local),
            "the server reports the address it sees"
        );
        client.close().await;
    });
}
