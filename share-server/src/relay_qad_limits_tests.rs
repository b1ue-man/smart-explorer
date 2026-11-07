//! The limits of the QAD service (plan M3): the admission rules through the
//! vendored test support, and the service end to end over loopback.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use iroh_relay::quic::qad_testing::{probe, reset_server_limits, QadProbe, TestAdmission};
use iroh_relay::quic::{set_server_limits, QadServerLimits};
use iroh_relay::server::{QuicConfig, Server, ServerConfig, SpawnError};

use super::{block_on_within, runtime, Settings, TestTls};

const WAIT: Duration = Duration::from_secs(5);
const TIMED_OUT: &str = "QAD probe timed out";

fn limits(configure: impl FnOnce(&mut QadServerLimits)) -> QadServerLimits {
    let mut limits = QadServerLimits::default();
    configure(&mut limits);
    limits
}

fn admission(configure: impl FnOnce(&mut QadServerLimits)) -> TestAdmission {
    TestAdmission::new(limits(configure)).expect("valid limits")
}

fn ip(text: &str) -> IpAddr {
    text.parse().expect("IP address")
}

#[test]
fn qad_admission_caps_the_total_connections() {
    let admission = admission(|limits| limits.max_connections = Some(2));
    let now = Instant::now();
    let first = admission
        .try_admit_at(ip("203.0.113.1"), now)
        .expect("first");
    let _second = admission
        .try_admit_at(ip("203.0.113.2"), now)
        .expect("second");
    assert_eq!(admission.held(), 2);

    assert_eq!(
        admission.try_admit_at(ip("203.0.113.3"), now).err(),
        Some("capacity")
    );
    assert_eq!(admission.held(), 2, "a refusal holds no place");

    drop(first);
    assert_eq!(admission.held(), 1, "a closed connection frees its place");
    let _third = admission
        .try_admit_at(ip("203.0.113.3"), now)
        .expect("third after a place was freed");
    assert_eq!(admission.held(), 2);
}

#[test]
fn qad_admission_caps_connections_per_source() {
    let admission = admission(|limits| limits.max_connections_per_source = Some(1));
    let now = Instant::now();
    let refused = |text: &str| admission.try_admit_at(ip(text), now).err();

    let ipv4 = admission.try_admit_at(ip("203.0.113.9"), now).expect("v4");
    assert_eq!(refused("203.0.113.9"), Some("capacity"));
    // An IPv4-mapped IPv6 address is the same source.
    assert_eq!(refused("::ffff:203.0.113.9"), Some("capacity"));

    // An IPv6 source is its /64 prefix.
    let ipv6 = admission
        .try_admit_at(ip("2001:db8:1:2::1"), now)
        .expect("v6");
    assert_eq!(refused("2001:db8:1:2::ffff"), Some("capacity"));
    let other_prefix = admission
        .try_admit_at(ip("2001:db8:1:3::1"), now)
        .expect("another /64");
    assert_eq!(admission.held(), 3);

    // The place of the IPv4 source frees for its mapped form as well.
    drop(ipv4);
    let mapped = admission
        .try_admit_at(ip("::ffff:203.0.113.9"), now)
        .expect("mapped form after the place was freed");
    drop((ipv6, other_prefix, mapped));
    assert_eq!(admission.held(), 0);
}

#[test]
fn qad_admission_rates_per_source_and_in_total() {
    // Per source: one accept per second, burst one.
    let per_source = admission(|limits| {
        limits.accepts_per_second_per_source = Some(1.0);
        limits.accept_burst_per_source = Some(1);
    });
    let started = Instant::now();
    let source = ip("203.0.113.9");
    let _first = per_source.try_admit_at(source, started).expect("burst");
    assert_eq!(per_source.try_admit_at(source, started).err(), Some("rate"));
    assert!(
        per_source.try_admit_at(ip("203.0.113.10"), started).is_ok(),
        "another source has its own bucket"
    );
    assert!(
        per_source
            .try_admit_at(source, started + Duration::from_secs(1))
            .is_ok(),
        "one second refills one token"
    );

    // In total: the same rate over all sources.
    let total = admission(|limits| {
        limits.accepts_per_second = Some(1.0);
        limits.accept_burst = Some(1);
    });
    let started = Instant::now();
    let _first = total
        .try_admit_at(ip("203.0.113.1"), started)
        .expect("burst");
    assert_eq!(
        total.try_admit_at(ip("203.0.113.2"), started).err(),
        Some("rate")
    );
    assert!(total
        .try_admit_at(ip("203.0.113.2"), started + Duration::from_secs(1))
        .is_ok());
}

#[test]
fn qad_invalid_limits_are_rejected() {
    let invalid = [
        limits(|limits| limits.accepts_per_second = Some(f64::NAN)),
        limits(|limits| limits.accepts_per_second = Some(f64::INFINITY)),
        limits(|limits| limits.accepts_per_second = Some(0.0)),
        limits(|limits| {
            limits.accepts_per_second = Some(1.0);
            limits.accept_burst = Some(0);
        }),
        limits(|limits| limits.accepts_per_second_per_source = Some(f64::NAN)),
        limits(|limits| {
            limits.accepts_per_second_per_source = Some(1.0);
            limits.accept_burst_per_source = Some(0);
        }),
    ];
    for (index, invalid) in invalid.into_iter().enumerate() {
        assert!(
            TestAdmission::new(invalid).is_err(),
            "invalid limits {index} were accepted"
        );
    }
    assert!(TestAdmission::new(QadServerLimits::default()).is_ok());
}

fn quic_config(tls: &TestTls) -> QuicConfig {
    let mut quic = QuicConfig::new((Ipv4Addr::LOCALHOST, 0));
    quic.server_config = Some(tls.server(false));
    quic
}

/// A QAD server on a free loopback port, without a relay.
async fn start(tls: &TestTls) -> Server {
    let mut config = ServerConfig::default();
    config.quic = Some(quic_config(tls));
    Server::spawn(config)
        .await
        .unwrap_or_else(|error| panic!("the QAD server did not start: {error}"))
}

async fn stop(server: Server) {
    let _ = tokio::time::timeout(WAIT, server.shutdown()).await;
}

async fn connect(address: SocketAddr, tls: &TestTls, limit: Duration) -> Result<QadProbe, String> {
    tokio::time::timeout(limit, probe(address, "localhost", tls.client()))
        .await
        .unwrap_or_else(|_| Err(TIMED_OUT.to_string()))
}

/// The server frees a place once it has seen the close; connect until then.
async fn connect_when_free(address: SocketAddr, tls: &TestTls) -> QadProbe {
    let deadline = Instant::now() + WAIT;
    loop {
        match connect(address, tls, Duration::from_secs(2)).await {
            Ok(client) => return client,
            Err(error) => assert!(
                Instant::now() < deadline,
                "no connection was admitted after the first one closed: {error}"
            ),
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn incoming(server: &Server) -> u64 {
    server.metrics().server.qad_incoming.get()
}

async fn wait_for_incoming(server: &Server, expected: u64) {
    let deadline = Instant::now() + WAIT;
    while incoming(server) != expected {
        assert!(
            Instant::now() < deadline,
            "qad_incoming is {}, expected {expected}",
            incoming(server)
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn assert_reports_own_address(client: &QadProbe) {
    let local = client.local_addr().expect("client socket address");
    assert_eq!(
        client.observed(WAIT).await,
        Some(local),
        "the server reports the address it sees"
    );
}

/// Without limits (the upstream behaviour the vendored patch keeps): every
/// connection is admitted and told its own address.
async fn without_limits(tls: &TestTls) {
    reset_server_limits();
    let server = start(tls).await;
    let address = server.quic_addr().expect("QAD address");
    let first = connect(address, tls, WAIT).await.expect("first");
    let second = connect(address, tls, WAIT).await.expect("second");
    assert_reports_own_address(&first).await;
    assert_reports_own_address(&second).await;
    assert!(first.close_reason().is_none() && second.close_reason().is_none());
    wait_for_incoming(&server, 2).await;
    first.close().await;
    second.close().await;
    stop(server).await;
}

/// One connection at a time: the second is refused while the first is open,
/// and a new one is admitted after the first closed.
async fn with_one_connection(tls: &TestTls) {
    set_server_limits(limits(|limits| limits.max_connections = Some(1)));
    let server = start(tls).await;
    let address = server.quic_addr().expect("QAD address");

    let first = connect(address, tls, WAIT).await.expect("first");
    // The address check (Retry) ran, and the server saw the true source.
    assert_reports_own_address(&first).await;
    // Only the validated connection reached the accept path.
    wait_for_incoming(&server, 1).await;

    let refusal = match connect(address, tls, WAIT).await {
        Ok(_) => panic!("the second connection was admitted"),
        Err(error) => error,
    };
    assert_ne!(
        refusal, TIMED_OUT,
        "the server must refuse, not stay silent"
    );
    assert!(first.close_reason().is_none(), "the first stays open");
    assert_eq!(incoming(&server), 1, "a refusal never reaches the handler");

    first.close().await;
    let third = connect_when_free(address, tls).await;
    assert!(third.close_reason().is_none());
    wait_for_incoming(&server, 2).await;
    third.close().await;
    stop(server).await;
}

/// Limits the service cannot use end the QAD start, not the process.
async fn with_invalid_limits(tls: &TestTls) {
    set_server_limits(limits(|limits| limits.accepts_per_second = Some(f64::NAN)));
    let mut config = ServerConfig::default();
    config.quic = Some(quic_config(tls));
    match Server::spawn(config).await {
        Err(SpawnError::QuicSpawn { .. }) => {}
        Err(error) => panic!("unexpected start error: {error}"),
        Ok(_) => panic!("the server started with invalid limits"),
    }
}

#[test]
fn qad_limits_end_to_end() {
    let _settings = Settings::new(&[]);
    let tls = TestTls::new();
    let runtime = runtime();
    block_on_within(&runtime, async {
        without_limits(&tls).await;
        with_one_connection(&tls).await;
        with_invalid_limits(&tls).await;
    });
}
