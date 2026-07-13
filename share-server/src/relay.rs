use std::{
    collections::HashMap,
    fmt,
    hash::Hash,
    net::{AddrParseError, SocketAddr},
    num::NonZeroU32,
    sync::{Arc, Mutex},
};

use iroh_base::EndpointId;
use iroh_relay::server::{
    Access, AccessControl, ClientRateLimit, ClientRequest, ConnectionId, RelayConfig,
};

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
const RELAY_CAPACITY_DENIAL: &str = "relay connection capacity reached";

pub(super) struct RelayGuard {
    _runtime: tokio::runtime::Runtime,
    _server: iroh_relay::server::Server,
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

pub(super) fn start(signal_bind: &str) -> Result<Option<RelayGuard>, RelayStartError> {
    if explicitly_disabled(std::env::var("SE_IROH_RELAY_DISABLE").ok().as_deref()) {
        eprintln!("iroh relay disabled via SE_IROH_RELAY_DISABLE");
        return Ok(None);
    }
    let bind = std::env::var("SE_IROH_RELAY_BIND")
        .ok()
        .unwrap_or_else(|| default_bind(signal_bind));
    let address = bind
        .parse::<SocketAddr>()
        .map_err(|source| RelayStartError::InvalidBind { bind, source })?;
    start_at(address).map(Some)
}

fn explicitly_disabled(value: Option<&str>) -> bool {
    value
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn start_at(address: SocketAddr) -> Result<RelayGuard, RelayStartError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("se-iroh-relay")
        .build()
        .map_err(RelayStartError::Runtime)?;
    let server = runtime
        .block_on(async {
            let mut config = iroh_relay::server::ServerConfig::default();
            config.relay = Some(relay_config(address));
            iroh_relay::server::Server::spawn(config).await
        })
        .map_err(|error| RelayStartError::Server {
            address,
            details: error.to_string(),
        })?;
    let relay_url = server
        .http_addr()
        .map(|address| format!("http://{address}"))
        .unwrap_or_else(|| format!("http://{address}"));
    eprintln!("se-share-server iroh relay listening on {relay_url}");
    Ok(RelayGuard {
        _runtime: runtime,
        _server: server,
    })
}

fn relay_config(address: std::net::SocketAddr) -> RelayConfig {
    let bytes_per_second = NonZeroU32::new(RELAY_RX_BYTES_PER_SECOND)
        .expect("RELAY_RX_BYTES_PER_SECOND is a non-zero constant");
    let max_burst_bytes =
        NonZeroU32::new(RELAY_RX_BURST_BYTES).expect("RELAY_RX_BURST_BYTES is a non-zero constant");
    let mut rate_limit = ClientRateLimit::new(bytes_per_second);
    rate_limit.max_burst_bytes = Some(max_burst_bytes);

    let mut config = RelayConfig::new(address);
    config.limits.client_rx = Some(rate_limit);
    config.limits.max_concurrent_tcp_connections = Some(RELAY_MAX_TCP_CONNECTIONS);
    config.limits.max_concurrent_tcp_connections_per_source =
        Some(RELAY_MAX_TCP_CONNECTIONS_PER_SOURCE);
    config.limits.accept_conn_limit = Some(RELAY_ACCEPTS_PER_SECOND);
    config.limits.accept_conn_burst = Some(RELAY_ACCEPT_BURST);
    config.limits.accept_conn_limit_per_source = Some(RELAY_ACCEPTS_PER_SECOND_PER_SOURCE);
    config.limits.accept_conn_burst_per_source = Some(RELAY_ACCEPT_BURST_PER_SOURCE);
    config.key_cache_capacity = Some(RELAY_KEY_CACHE_CAPACITY);
    config.access = Arc::new(RelayAccess::new(
        RELAY_MAX_ACTIVE_CONNECTIONS,
        RELAY_MAX_CONNECTIONS_PER_ENDPOINT,
    ));
    config
}

#[derive(Debug)]
struct RelayAccess {
    counts: Mutex<AdmissionCounts<EndpointId, ConnectionId>>,
    max_total: usize,
    max_per_endpoint: usize,
}

impl RelayAccess {
    fn new(max_total: usize, max_per_endpoint: usize) -> Self {
        Self {
            counts: Mutex::new(AdmissionCounts::default()),
            max_total,
            max_per_endpoint,
        }
    }

    fn with_counts<R>(
        &self,
        callback: impl FnOnce(&mut AdmissionCounts<EndpointId, ConnectionId>) -> R,
    ) -> R {
        let mut counts = match self.counts.lock() {
            Ok(counts) => counts,
            Err(poisoned) => poisoned.into_inner(),
        };
        callback(&mut counts)
    }
}

impl AccessControl for RelayAccess {
    async fn on_connect(&self, request: &ClientRequest) -> Access {
        // Iroh authenticates this EndpointId during its challenge/proof handshake before
        // invoking the access hook, so an outside client cannot choose another key's bucket.
        let admitted = self.with_counts(|counts| {
            counts.try_admit(
                request.endpoint_id(),
                request.connection_id(),
                self.max_total,
                self.max_per_endpoint,
            )
        });
        if admitted {
            Access::Allow
        } else {
            Access::Deny {
                reason: Some(RELAY_CAPACITY_DENIAL.to_string()),
            }
        }
    }

    fn on_disconnect(&self, _endpoint_id: EndpointId, connection_id: ConnectionId) {
        self.with_counts(|counts| counts.release(&connection_id));
    }
}

#[derive(Debug)]
struct AdmissionCounts<K, C> {
    by_endpoint: HashMap<K, usize>,
    admitted: HashMap<C, K>,
}

impl<K, C> Default for AdmissionCounts<K, C> {
    fn default() -> Self {
        Self {
            by_endpoint: HashMap::new(),
            admitted: HashMap::new(),
        }
    }
}

impl<K: Clone + Eq + Hash, C: Eq + Hash> AdmissionCounts<K, C> {
    fn try_admit(
        &mut self,
        endpoint_id: K,
        connection_id: C,
        max_total: usize,
        max_per_endpoint: usize,
    ) -> bool {
        let endpoint_count = self.by_endpoint.get(&endpoint_id).copied().unwrap_or(0);
        if self.admitted.len() >= max_total
            || endpoint_count >= max_per_endpoint
            || self.admitted.contains_key(&connection_id)
        {
            return false;
        }
        self.by_endpoint
            .insert(endpoint_id.clone(), endpoint_count + 1);
        self.admitted.insert(connection_id, endpoint_id);
        true
    }

    fn release(&mut self, connection_id: &C) {
        let Some(endpoint_id) = self.admitted.remove(connection_id) else {
            return;
        };
        let Some(endpoint_count) = self.by_endpoint.get_mut(&endpoint_id) else {
            return;
        };
        *endpoint_count = endpoint_count.saturating_sub(1);
        if *endpoint_count == 0 {
            self.by_endpoint.remove(&endpoint_id);
        }
    }
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
        let error = match start_at(address) {
            Ok(_) => panic!("relay unexpectedly bound an occupied address"),
            Err(error) => error,
        };
        assert!(matches!(error, RelayStartError::Server { .. }));
        assert!(error.to_string().contains(&address.to_string()));
    }

    #[test]
    fn relay_config_applies_resource_limits() {
        let config = relay_config("127.0.0.1:51821".parse().unwrap());
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
    }

    #[test]
    fn relay_admission_enforces_per_endpoint_cap_and_reuses_capacity() {
        let mut counts = AdmissionCounts::default();
        assert!(counts.try_admit(7_u8, 10_u8, 8, 2));
        assert!(counts.try_admit(7_u8, 11_u8, 8, 2));
        assert!(!counts.try_admit(7_u8, 12_u8, 8, 2));

        counts.release(&10);
        assert!(counts.try_admit(7_u8, 12_u8, 8, 2));
        counts.release(&10);
        counts.release(&11);
        counts.release(&12);

        assert!(counts.admitted.is_empty());
        assert!(!counts.by_endpoint.contains_key(&7));
    }

    #[test]
    fn relay_admission_enforces_global_cap_and_reuses_capacity() {
        let mut counts = AdmissionCounts::default();
        assert!(counts.try_admit(1_u8, 10_u8, 3, 2));
        assert!(counts.try_admit(1_u8, 11_u8, 3, 2));
        assert!(counts.try_admit(2_u8, 12_u8, 3, 2));
        assert!(!counts.try_admit(3_u8, 13_u8, 3, 2));

        counts.release(&10);
        assert!(counts.try_admit(3_u8, 13_u8, 3, 2));
        assert_eq!(counts.admitted.len(), 3);
    }
}
