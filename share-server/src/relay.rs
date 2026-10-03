use std::{
    fmt,
    net::{AddrParseError, Ipv4Addr, SocketAddr},
    num::NonZeroU32,
    sync::Arc,
    time::Duration,
};

use iroh_relay::server::{CertConfig, ClientPingSchedule, ClientRateLimit, RelayConfig, TlsConfig};

use super::idle::Keepalive;
use super::relay_access::{RelayAccess, RelayAdmissions};

const RELAY_MAX_ACTIVE_CONNECTIONS: usize = 512;
const RELAY_MAX_CONNECTIONS_PER_ENDPOINT: usize = 4;
const RELAY_MAX_TCP_CONNECTIONS: usize = 512;
const RELAY_MAX_TCP_CONNECTIONS_PER_SOURCE: usize = 64;
const RELAY_ACCEPTS_PER_SECOND: f64 = 256.0;
const RELAY_ACCEPT_BURST: usize = 512;
const RELAY_ACCEPTS_PER_SECOND_PER_SOURCE: f64 = 32.0;
const RELAY_ACCEPT_BURST_PER_SOURCE: usize = 64;
const RELAY_KEY_CACHE_CAPACITY: usize = 4_096;
const RELAY_RX_BYTES_PER_SECOND: u32 = 64 * 1024 * 1024;
const RELAY_RX_BURST_BYTES: u32 = 8 * 1024 * 1024;
/// Base wait before the first server ping of a connection (iroh's cadence),
/// so a dead new connection is still noticed quickly.
const RELAY_FIRST_PING_INTERVAL: Duration = Duration::from_secs(15);
/// Fixed answer window: a phone woken by the ping needs longer than iroh's
/// RTT-based default of at most five seconds.
const RELAY_PONG_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) struct RelayGuard {
    _runtime: tokio::runtime::Runtime,
    _server: iroh_relay::server::Server,
}

/// How the relay runs (FC4, B20).
pub(super) struct RelayOptions {
    pub(super) keepalive: Keepalive,
    /// HTTPS with the signaling certificate; `None` serves plain HTTP.
    pub(super) tls: Option<rustls::ServerConfig>,
    /// With TLS: connections without a TLS record are served as plain HTTP
    /// (older clients while plaintext is still allowed).
    pub(super) plaintext_fallback: bool,
    /// A reverse proxy collapses sources; the proxy limits per client.
    pub(super) trusted_proxy: bool,
    /// Endpoints registered at the signaling (the only ones admitted).
    pub(super) admissions: Arc<RelayAdmissions>,
}

#[derive(Debug)]
pub(super) enum RelayStartError {
    InvalidBind {
        bind: String,
        source: AddrParseError,
    },
    Runtime(std::io::Error),
    Server {
        address: SocketAddr,
        details: String,
    },
}

impl fmt::Display for RelayStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBind { bind, source } => {
                write!(formatter, "invalid Iroh relay bind {bind}: {source}")
            }
            Self::Runtime(source) => {
                write!(formatter, "cannot create Iroh relay runtime: {source}")
            }
            Self::Server { address, details } => {
                write!(formatter, "cannot start Iroh relay on {address}: {details}")
            }
        }
    }
}

impl std::error::Error for RelayStartError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidBind { source, .. } => Some(source),
            Self::Runtime(source) => Some(source),
            Self::Server { .. } => None,
        }
    }
}

/// The relay bind: `SE_IROH_RELAY_BIND`, else the signaling port plus one.
/// `None` when `SE_IROH_RELAY_DISABLE` is set.
pub(super) fn bind_address(signal_bind: &str) -> Result<Option<SocketAddr>, RelayStartError> {
    if explicitly_disabled(std::env::var("SE_IROH_RELAY_DISABLE").ok().as_deref()) {
        return Ok(None);
    }
    let bind = std::env::var("SE_IROH_RELAY_BIND")
        .ok()
        .unwrap_or_else(|| default_bind(signal_bind));
    bind.parse::<SocketAddr>()
        .map(Some)
        .map_err(|source| RelayStartError::InvalidBind { bind, source })
}

pub(super) fn start(
    address: Option<SocketAddr>,
    options: RelayOptions,
) -> Result<Option<RelayGuard>, RelayStartError> {
    let Some(address) = address else {
        eprintln!("iroh relay disabled via SE_IROH_RELAY_DISABLE");
        return Ok(None);
    };
    start_at(address, options).map(Some)
}

fn explicitly_disabled(value: Option<&str>) -> bool {
    value
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn start_at(address: SocketAddr, options: RelayOptions) -> Result<RelayGuard, RelayStartError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("se-iroh-relay")
        .build()
        .map_err(RelayStartError::Runtime)?;
    let tls = options.tls.is_some();
    let server = runtime
        .block_on(async {
            let mut config = iroh_relay::server::ServerConfig::default();
            config.relay = Some(relay_config(address, options));
            iroh_relay::server::Server::spawn(config).await
        })
        .map_err(|error| RelayStartError::Server {
            address,
            details: error.to_string(),
        })?;
    let relay_url = match (tls, server.https_addr(), server.http_addr()) {
        (true, Some(https), _) => format!("https://{https}"),
        (_, _, Some(http)) => format!("http://{http}"),
        _ => format!("http://{address}"),
    };
    eprintln!("se-share-server iroh relay listening on {relay_url}");
    Ok(RelayGuard {
        _runtime: runtime,
        _server: server,
    })
}

/// Server pings: the first after 15 s, then every K s (each plus 1-5 s
/// jitter) once the client answered. Clients that ping on their own (desktop,
/// every 15 s) keep resetting this timer and are not pinged at all; an idle
/// phone is woken only once per keepalive interval.
fn relay_ping_schedule(keepalive: Keepalive) -> ClientPingSchedule {
    ClientPingSchedule::new(
        RELAY_FIRST_PING_INTERVAL,
        keepalive.duration(),
        Some(RELAY_PONG_TIMEOUT),
    )
}

fn relay_config(address: SocketAddr, options: RelayOptions) -> RelayConfig {
    let bytes_per_second = NonZeroU32::new(RELAY_RX_BYTES_PER_SECOND)
        .expect("RELAY_RX_BYTES_PER_SECOND is a non-zero constant");
    let max_burst_bytes =
        NonZeroU32::new(RELAY_RX_BURST_BYTES).expect("RELAY_RX_BURST_BYTES is a non-zero constant");
    let mut rate_limit = ClientRateLimit::new(bytes_per_second);
    rate_limit.max_burst_bytes = Some(max_burst_bytes);

    // With TLS the relay serves HTTPS on `address`; the captive-portal probe
    // listener the relay always adds then stays on loopback.
    let mut config = match options.tls {
        Some(server_config) => {
            let mut config = RelayConfig::new(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)));
            let mut tls = TlsConfig::new(address, CertConfig::Manual { server_config });
            tls.plaintext_fallback = options.plaintext_fallback;
            config.tls = Some(tls);
            config
        }
        None => RelayConfig::new(address),
    };
    config.limits.client_rx = Some(rate_limit);
    config.limits.max_concurrent_tcp_connections = Some(RELAY_MAX_TCP_CONNECTIONS);
    config.limits.accept_conn_limit = Some(RELAY_ACCEPTS_PER_SECOND);
    config.limits.accept_conn_burst = Some(RELAY_ACCEPT_BURST);
    if options.trusted_proxy {
        // Every connection comes from the proxy, which limits per client.
        config.limits.max_concurrent_tcp_connections_per_source = Some(RELAY_MAX_TCP_CONNECTIONS);
    } else {
        config.limits.max_concurrent_tcp_connections_per_source =
            Some(RELAY_MAX_TCP_CONNECTIONS_PER_SOURCE);
        config.limits.accept_conn_limit_per_source = Some(RELAY_ACCEPTS_PER_SECOND_PER_SOURCE);
        config.limits.accept_conn_burst_per_source = Some(RELAY_ACCEPT_BURST_PER_SOURCE);
    }
    config.key_cache_capacity = Some(RELAY_KEY_CACHE_CAPACITY);
    config.access = Arc::new(RelayAccess::new(
        options.admissions,
        RELAY_MAX_ACTIVE_CONNECTIONS,
        RELAY_MAX_CONNECTIONS_PER_ENDPOINT,
        relay_ping_schedule(options.keepalive),
    ));
    config
}

pub(super) fn default_bind(signal_bind: &str) -> String {
    let host_port = signal_bind
        .trim()
        .trim_start_matches("tcp://")
        .trim_end_matches('/');
    if let Ok(address) = host_port.parse::<std::net::SocketAddr>() {
        let mut next = address;
        next.set_port(address.port().saturating_add(1));
        return next.to_string();
    }
    "0.0.0.0:51821".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(keepalive: Keepalive) -> RelayOptions {
        RelayOptions {
            keepalive,
            tls: None,
            plaintext_fallback: false,
            trusted_proxy: false,
            admissions: Arc::new(RelayAdmissions::default()),
        }
    }

    #[test]
    fn relay_bind_defaults_to_next_port() {
        assert_eq!(default_bind("127.0.0.1:51820"), "127.0.0.1:51821");
        assert_eq!(default_bind("0.0.0.0:443"), "0.0.0.0:444");
    }

    #[test]
    fn only_explicit_true_values_disable_the_relay() {
        assert!(explicitly_disabled(Some("1")));
        assert!(explicitly_disabled(Some("true")));
        assert!(explicitly_disabled(Some("TRUE")));
        assert!(!explicitly_disabled(None));
        assert!(!explicitly_disabled(Some("0")));
        assert!(!explicitly_disabled(Some("false")));
    }

    #[test]
    fn occupied_relay_bind_is_a_startup_error() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let error = match start_at(address, options(Keepalive::default())) {
            Ok(_) => panic!("relay unexpectedly bound an occupied address"),
            Err(error) => error,
        };
        assert!(matches!(error, RelayStartError::Server { .. }));
        assert!(error.to_string().contains(&address.to_string()));
    }

    #[test]
    fn relay_config_applies_resource_limits() {
        let config = relay_config(
            "127.0.0.1:51821".parse().unwrap(),
            options(Keepalive::default()),
        );
        let rate_limit = config.limits.client_rx.unwrap();
        assert_eq!(rate_limit.bytes_per_second.get(), RELAY_RX_BYTES_PER_SECOND);
        assert_eq!(
            rate_limit.max_burst_bytes.map(NonZeroU32::get),
            Some(RELAY_RX_BURST_BYTES)
        );
        assert_eq!(
            config.limits.max_concurrent_tcp_connections,
            Some(RELAY_MAX_TCP_CONNECTIONS)
        );
        assert_eq!(
            config.limits.max_concurrent_tcp_connections_per_source,
            Some(RELAY_MAX_TCP_CONNECTIONS_PER_SOURCE)
        );
        assert_eq!(
            config.limits.accept_conn_limit,
            Some(RELAY_ACCEPTS_PER_SECOND)
        );
        assert_eq!(config.limits.accept_conn_burst, Some(RELAY_ACCEPT_BURST));
        assert_eq!(
            config.limits.accept_conn_limit_per_source,
            Some(RELAY_ACCEPTS_PER_SECOND_PER_SOURCE)
        );
        assert_eq!(
            config.limits.accept_conn_burst_per_source,
            Some(RELAY_ACCEPT_BURST_PER_SOURCE)
        );
        assert_eq!(config.key_cache_capacity, Some(RELAY_KEY_CACHE_CAPACITY));
        assert!(config.tls.is_none());
    }

    #[test]
    fn review_task_trusted_proxy_lifts_only_the_per_source_relay_caps() {
        let mut proxied = options(Keepalive::default());
        proxied.trusted_proxy = true;
        let config = relay_config("127.0.0.1:51821".parse().unwrap(), proxied);
        assert_eq!(
            config.limits.max_concurrent_tcp_connections_per_source,
            Some(RELAY_MAX_TCP_CONNECTIONS)
        );
        assert_eq!(config.limits.accept_conn_limit_per_source, None);
        assert_eq!(
            config.limits.accept_conn_limit,
            Some(RELAY_ACCEPTS_PER_SECOND)
        );
    }

    #[test]
    fn android_background_task_relay_pings_at_keepalive_interval() {
        let endpoint = iroh_base::SecretKey::from_bytes(&[7; 32]).public();
        for (keepalive, expected) in [(Keepalive::default(), 180), (Keepalive::clamped(45), 45)] {
            let config = relay_config("127.0.0.1:51821".parse().unwrap(), options(keepalive));
            let schedule = config.access.ping_schedule(endpoint);
            assert_eq!(schedule.first_interval, Duration::from_secs(15));
            assert_eq!(schedule.interval, Duration::from_secs(expected));
            assert_eq!(schedule.pong_timeout, Some(Duration::from_secs(30)));
        }
        // The vendored relay keeps iroh's cadence without an explicit schedule.
        let upstream = ClientPingSchedule::default();
        assert_eq!(upstream.interval, Duration::from_secs(15));
        assert_eq!(upstream.pong_timeout, None);
    }
}
