//! QUIC address discovery (QAD) beside the relay: a device behind NAT learns
//! the public address this server observes and publishes it as a direct
//! candidate. QAD improves the route; it guards nothing, so no QAD failure
//! ends the server (plan E4, E5).

use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket},
    time::Duration,
};

use iroh_relay::{
    defaults::DEFAULT_RELAY_QUIC_PORT,
    quic::QadServerLimits,
    server::{QuicConfig, SpawnError},
};

use super::{
    explicitly_disabled, RELAY_ACCEPTS_PER_SECOND, RELAY_ACCEPTS_PER_SECOND_PER_SOURCE,
    RELAY_ACCEPT_BURST, RELAY_ACCEPT_BURST_PER_SOURCE, RELAY_MAX_ACTIVE_CONNECTIONS,
    RELAY_MAX_TCP_CONNECTIONS_PER_SOURCE,
};

/// Every device holds up to one QAD connection per address family.
const QAD_MAX_CONNECTIONS: usize = 2 * RELAY_MAX_ACTIVE_CONNECTIONS;
/// Carrier-grade NAT puts many devices behind one source address.
const QAD_MAX_CONNECTIONS_PER_SOURCE: usize = 2 * RELAY_MAX_TCP_CONNECTIONS_PER_SOURCE;
/// The client keeps the connection with a 25 s keepalive and a 35 s timeout.
const QAD_IDLE_TIMEOUT: Duration = Duration::from_secs(35);
/// A relay restart without QAD waits for the first attempt's listener to close.
const RESTART_ATTEMPTS: u32 = 40;
const RESTART_PAUSE: Duration = Duration::from_millis(50);

/// Whether and where QAD listens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum QadPlan {
    /// UDP address of the QAD listener.
    Listen(SocketAddr),
    /// QAD stays off; the text says why.
    Off(String),
}

/// The QAD plan from `SE_IROH_QAD_DISABLE` and `SE_IROH_QAD_BIND`.
pub(super) fn plan_from_env(relay: SocketAddr, tls: bool) -> QadPlan {
    plan(
        relay,
        tls,
        std::env::var("SE_IROH_QAD_DISABLE").ok().as_deref(),
        std::env::var("SE_IROH_QAD_BIND").ok().as_deref(),
    )
}

/// QAD listens on UDP at the relay's IP and port 7842, the port every client
/// probes without configuration; a relay on `0.0.0.0` gets the dual-stack
/// `[::]` so phones on IPv6 networks are answered too (R-SERVER-08: noq
/// clears `IPV6_V6ONLY`, and the iroh client canonicalizes an IPv4-mapped
/// observed address, `vendor/iroh-1.3.0/src/net_report.rs` `run_probe_v4`).
/// `bind` overrides it, `disable` (`1`/`true`) turns it off. QAD needs
/// TLS 1.3, so a plain-HTTP relay has none.
pub(super) fn plan(
    relay: SocketAddr,
    tls: bool,
    disable: Option<&str>,
    bind: Option<&str>,
) -> QadPlan {
    if explicitly_disabled(disable) {
        return QadPlan::Off("QAD disabled via SE_IROH_QAD_DISABLE".to_string());
    }
    if !tls {
        return QadPlan::Off(
            "QAD needs TLS (--tls-cert/--tls-key); the relay serves plain HTTP without QUIC \
             address discovery"
                .to_string(),
        );
    }
    match bind.map(str::trim).filter(|bind| !bind.is_empty()) {
        Some(bind) => match bind.parse::<SocketAddr>() {
            Ok(address) => QadPlan::Listen(address),
            Err(error) => {
                QadPlan::Off(format!("QAD off: invalid SE_IROH_QAD_BIND {bind}: {error}"))
            }
        },
        None => QadPlan::Listen(default_bind(relay.ip())),
    }
}

fn default_bind(relay: IpAddr) -> SocketAddr {
    let ip = match relay {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::UNSPECIFIED),
        ip => ip,
    };
    SocketAddr::new(ip, DEFAULT_RELAY_QUIC_PORT)
}

/// [`config`] for `address`; a host without IPv6 (dual-stack `[::]` cannot
/// bind) falls back to IPv4 `0.0.0.0` on the same port.
pub(super) fn config_or_ipv4(address: SocketAddr) -> Result<(SocketAddr, QuicConfig), String> {
    match config(address) {
        Ok(quic) => Ok((address, quic)),
        Err(reason) if address.ip() == IpAddr::V6(Ipv6Addr::UNSPECIFIED) => {
            let ipv4 = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), address.port());
            eprintln!("se-share-server: {reason}; trying {ipv4}");
            config(ipv4).map(|quic| (ipv4, quic))
        }
        Err(reason) => Err(reason),
    }
}

/// Limits of the QAD service, derived from the relay limits (plan E3).
pub(super) fn limits() -> QadServerLimits {
    let mut limits = QadServerLimits::default();
    limits.max_connections = Some(QAD_MAX_CONNECTIONS);
    limits.max_connections_per_source = Some(QAD_MAX_CONNECTIONS_PER_SOURCE);
    limits.accepts_per_second = Some(RELAY_ACCEPTS_PER_SECOND);
    limits.accept_burst = Some(RELAY_ACCEPT_BURST);
    limits.accepts_per_second_per_source = Some(RELAY_ACCEPTS_PER_SECOND_PER_SOURCE);
    limits.accept_burst_per_source = Some(RELAY_ACCEPT_BURST_PER_SOURCE);
    limits.idle_timeout = Some(QAD_IDLE_TIMEOUT);
    limits
}

/// The QAD configuration for `address`, or the reason it stays off: an
/// occupied UDP port is found before the relay starts.
pub(super) fn config(address: SocketAddr) -> Result<QuicConfig, String> {
    UdpSocket::bind(address)
        .map(drop)
        .map_err(|error| format!("QAD off: cannot bind UDP {address}: {error}"))?;
    iroh_relay::quic::set_server_limits(limits());
    // The TLS configuration is inherited from the relay.
    Ok(QuicConfig::new(address))
}

/// The reason when `error` is a failed QAD start; `None` for every other
/// relay start error, which stays fatal.
pub(super) fn failure(error: &SpawnError, address: SocketAddr) -> Option<String> {
    match error {
        SpawnError::QuicSpawn { source, .. } => {
            Some(format!("QAD off: cannot start on UDP {address}: {source}"))
        }
        _ => None,
    }
}

/// Starts the relay without QAD after a failed QAD start. The first attempt's
/// TCP listener closes asynchronously, so a bind error is retried briefly.
pub(super) fn restart_without_qad<T>(
    mut spawn: impl FnMut() -> Result<T, SpawnError>,
    mut pause: impl FnMut(Duration),
) -> Result<T, SpawnError> {
    let mut attempt = 1;
    loop {
        match spawn() {
            Err(SpawnError::BindTcpListener { .. }) if attempt < RESTART_ATTEMPTS => {
                attempt += 1;
                pause(RESTART_PAUSE);
            }
            result => return result,
        }
    }
}

/// The log line of a running QAD listener. A listener on `0.0.0.0` (set
/// explicitly, or the fallback of a host without IPv6) answers IPv4 only;
/// the note names the IPv6 bind (R-SERVER-08).
pub(super) fn listening_line(address: SocketAddr) -> String {
    let note = match address.ip() {
        std::net::IpAddr::V4(ip) if ip.is_unspecified() => {
            "; IPv4 only, set SE_IROH_QAD_BIND=[::]:7842 for IPv6"
        }
        _ => "",
    };
    format!("se-share-server QAD listening on {address} (UDP){note}")
}

#[cfg(test)]
#[path = "relay_qad_tests.rs"]
mod relay_qad_tests;
