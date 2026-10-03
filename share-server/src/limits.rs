use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};

use super::PeerPresence;

pub(super) const MAX_CONNECTION_WORKERS: usize = 256;
pub(super) const MAX_CONNECTIONS_PER_SOURCE: usize = 16;
pub(super) const MAX_REGISTERED_CLIENTS: usize = 128;
pub(super) const MAX_REGISTERED_CLIENTS_PER_SOURCE: usize = 8;
/// Registrations of one proven device key (app window, background worker,
/// command line, a reconnect that overlaps the old connection).
pub(super) const MAX_REGISTERED_CLIENTS_PER_KEY: usize = 4;
/// An IPv6 end site usually holds a /56 or /48, so one host can rotate
/// through many /64 sources; their sum per /56 is capped at this multiple of
/// the per-source caps (S47).
pub(super) const NETWORK_CAP_FACTOR: usize = 4;
pub(super) const WRITER_QUEUE_CAPACITY: usize = 32;
pub(super) const MAX_WRITER_QUEUED_BYTES: usize = 2 * 1024 * 1024;
pub(super) const MAX_PUBLISHED_DIRECTS_PER_CLIENT: usize = 64;
pub(super) const MAX_WATCHES_PER_CLIENT: usize = 256;
pub(super) const MAX_ROOMS_PER_CLIENT: usize = 64;
pub(super) const MAX_ROOM_MEMBERS: usize = 64;

const MAX_ID_BYTES: usize = 256;
const MAX_KIND_BYTES: usize = 32;
const MAX_NAME_BYTES: usize = 1024;
const MAX_URL_BYTES: usize = 2048;
const MAX_CANDIDATES: usize = 32;
const MAX_CANDIDATE_BYTES: usize = 256;
const MAX_NONCE_BYTES: usize = 256;
const MAX_PROOF_BYTES: usize = 1024;
const MAX_RETAINED_PRESENCE_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum SourceKey {
    Ipv4([u8; 4]),
    Ipv6Prefix64([u8; 8]),
    ExternallyLimitedProxy,
}

impl SourceKey {
    #[cfg(test)]
    pub(super) fn from_socket(address: SocketAddr) -> Self {
        Self::from_ip(address.ip())
    }

    fn from_ip(address: IpAddr) -> Self {
        match address {
            IpAddr::V4(address) => Self::Ipv4(address.octets()),
            IpAddr::V6(address) => {
                if let Some(address) = address.to_ipv4_mapped() {
                    return Self::Ipv4(address.octets());
                }
                let octets = address.octets();
                let mut prefix = [0_u8; 8];
                prefix.copy_from_slice(&octets[..8]);
                Self::Ipv6Prefix64(prefix)
            }
        }
    }

    pub(super) fn has_internal_source_limit(self) -> bool {
        !matches!(self, Self::ExternallyLimitedProxy)
    }

    /// The IPv6 /56 around a /64 source; IPv4 and proxies have none.
    pub(super) fn network(self) -> Option<NetworkKey> {
        match self {
            Self::Ipv6Prefix64(prefix) => {
                let mut network = [0_u8; 7];
                network.copy_from_slice(&prefix[..7]);
                Some(NetworkKey(network))
            }
            Self::Ipv4(_) | Self::ExternallyLimitedProxy => None,
        }
    }
}

/// An IPv6 /56 that groups the /64 sources of one end site (S47).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct NetworkKey([u8; 7]);

#[derive(Clone, Default)]
pub(super) struct SourceClassifier {
    externally_limited_proxies: HashSet<IpAddr>,
}

impl SourceClassifier {
    pub(super) fn parse_proxy_ips(value: &str) -> Result<Self, String> {
        let mut externally_limited_proxies = HashSet::new();
        for token in value
            .split([',', ';'])
            .map(str::trim)
            .filter(|token| !token.is_empty())
        {
            let address = token
                .parse::<IpAddr>()
                .map_err(|error| format!("invalid trusted proxy IP {token}: {error}"))?;
            externally_limited_proxies.insert(canonical_ip(address));
        }
        Ok(Self {
            externally_limited_proxies,
        })
    }

    pub(super) fn is_trusted_ip(&self, address: IpAddr) -> bool {
        self.externally_limited_proxies
            .contains(&canonical_ip(address))
    }

    pub(super) fn classify(&self, address: SocketAddr) -> SourceKey {
        let address = canonical_ip(address.ip());
        if self.externally_limited_proxies.contains(&address) {
            SourceKey::ExternallyLimitedProxy
        } else {
            SourceKey::from_ip(address)
        }
    }
}

fn canonical_ip(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(address) => address
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(address)),
        address => address,
    }
}

#[derive(Default)]
struct ConnectionCounts {
    total: usize,
    by_source: HashMap<SourceKey, usize>,
    by_network: HashMap<NetworkKey, usize>,
}

#[derive(Clone)]
pub(super) struct ConnectionLimiter {
    counts: Arc<Mutex<ConnectionCounts>>,
    max_total: usize,
    max_per_source: usize,
    max_per_network: usize,
}

impl ConnectionLimiter {
    pub(super) fn new(max_total: usize, max_per_source: usize) -> Self {
        Self {
            counts: Arc::new(Mutex::new(ConnectionCounts::default())),
            max_total,
            max_per_source,
            max_per_network: max_per_source.saturating_mul(NETWORK_CAP_FACTOR),
        }
    }

    pub(super) fn from_limits(limits: &ServerLimits) -> Self {
        let mut limiter = Self::new(limits.max_connections, limits.max_connections_per_source);
        limiter.max_per_network = limits.max_connections_per_network;
        limiter
    }

    pub(super) fn try_acquire(&self, source: SourceKey) -> Option<ConnectionPermit> {
        let mut counts = self
            .counts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let source_count = counts.by_source.get(&source).copied().unwrap_or_default();
        let network = source.network();
        let network_count = network
            .and_then(|network| counts.by_network.get(&network).copied())
            .unwrap_or_default();
        if counts.total >= self.max_total
            || (source.has_internal_source_limit() && source_count >= self.max_per_source)
            || (network.is_some() && network_count >= self.max_per_network)
        {
            return None;
        }
        counts.total += 1;
        let limited_source = source.has_internal_source_limit().then_some(source);
        if let Some(source) = limited_source {
            counts.by_source.insert(source, source_count + 1);
        }
        if let Some(network) = network {
            counts.by_network.insert(network, network_count + 1);
        }
        Some(ConnectionPermit {
            counts: self.counts.clone(),
            source: limited_source,
        })
    }

    #[cfg(test)]
    pub(super) fn active(&self) -> usize {
        self.counts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .total
    }
}

pub(super) struct ConnectionPermit {
    counts: Arc<Mutex<ConnectionCounts>>,
    source: Option<SourceKey>,
}

impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        let mut counts = self
            .counts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        counts.total = counts.total.saturating_sub(1);
        let Some(source) = self.source else {
            return;
        };
        if let Some(source_count) = counts.by_source.get_mut(&source) {
            *source_count = source_count.saturating_sub(1);
            if *source_count == 0 {
                counts.by_source.remove(&source);
            }
        }
        let Some(network) = source.network() else {
            return;
        };
        if let Some(network_count) = counts.by_network.get_mut(&network) {
            *network_count = network_count.saturating_sub(1);
            if *network_count == 0 {
                counts.by_network.remove(&network);
            }
        }
    }
}

/// Connection and registration ceilings; `config.rs` reads overrides from the
/// environment (FC4: limits are configurable and sized to the host).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ServerLimits {
    pub(super) max_connections: usize,
    pub(super) max_connections_per_source: usize,
    pub(super) max_connections_per_network: usize,
    pub(super) max_clients: usize,
    pub(super) max_clients_per_source: usize,
    pub(super) max_clients_per_network: usize,
    pub(super) max_clients_per_key: usize,
}

impl Default for ServerLimits {
    fn default() -> Self {
        Self {
            max_connections: MAX_CONNECTION_WORKERS,
            max_connections_per_source: MAX_CONNECTIONS_PER_SOURCE,
            max_connections_per_network: MAX_CONNECTIONS_PER_SOURCE * NETWORK_CAP_FACTOR,
            max_clients: MAX_REGISTERED_CLIENTS,
            max_clients_per_source: MAX_REGISTERED_CLIENTS_PER_SOURCE,
            max_clients_per_network: MAX_REGISTERED_CLIENTS_PER_SOURCE * NETWORK_CAP_FACTOR,
            max_clients_per_key: MAX_REGISTERED_CLIENTS_PER_KEY,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RetainError {
    InvalidField(&'static str),
    Limit(&'static str),
    /// The entry belongs to another proven device key (FC4).
    Bound(&'static str),
    /// A required relation access proof is missing or wrong (FC4).
    Denied(&'static str),
}

impl RetainError {
    pub(super) fn message(self) -> String {
        match self {
            Self::InvalidField(field) => format!("invalid or oversized {field}"),
            Self::Limit(resource) => format!("too many {resource}"),
            Self::Bound(entry) => format!("{entry} is bound to another device key"),
            Self::Denied(proof) => format!("{proof} is missing or does not match"),
        }
    }
}

pub(super) fn validate_identifier(field: &'static str, value: &str) -> Result<(), RetainError> {
    validate_text(field, value, MAX_ID_BYTES, false)
}

pub(super) fn validate_presence(presence: &PeerPresence) -> Result<(), RetainError> {
    validate_text("presence kind", &presence.kind, MAX_KIND_BYTES, false)?;
    validate_identifier("relation id", &presence.relation_id)?;
    validate_identifier("device id", &presence.device_id)?;
    validate_text("device name", &presence.device_name, MAX_NAME_BYTES, true)?;
    validate_text("public key", &presence.public_key, MAX_ID_BYTES, true)?;
    validate_text("fingerprint", &presence.fingerprint, MAX_ID_BYTES, true)?;
    validate_text("node id", &presence.node_id, MAX_ID_BYTES, true)?;
    validate_text("relay URL", &presence.relay_url, MAX_URL_BYTES, true)?;
    validate_text("nonce", &presence.nonce, MAX_NONCE_BYTES, true)?;
    validate_text("proof", &presence.proof, MAX_PROOF_BYTES, true)?;
    if presence.candidates.len() > MAX_CANDIDATES {
        return Err(RetainError::InvalidField("presence candidates"));
    }
    for candidate in &presence.candidates {
        validate_text("presence candidate", candidate, MAX_CANDIDATE_BYTES, false)?;
    }

    let retained_bytes = presence.kind.len()
        + presence.relation_id.len()
        + presence.device_id.len()
        + presence.device_name.len()
        + presence.public_key.len()
        + presence.fingerprint.len()
        + presence.node_id.len()
        + presence.relay_url.len()
        + presence.nonce.len()
        + presence.proof.len()
        + presence.candidates.iter().map(String::len).sum::<usize>();
    if retained_bytes > MAX_RETAINED_PRESENCE_BYTES {
        return Err(RetainError::InvalidField("presence"));
    }
    Ok(())
}

fn validate_text(
    field: &'static str,
    value: &str,
    max_bytes: usize,
    allow_empty: bool,
) -> Result<(), RetainError> {
    if (!allow_empty && value.is_empty())
        || value.len() > max_bytes
        || value.chars().any(char::is_control)
    {
        Err(RetainError::InvalidField(field))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_permits_are_hard_capped_and_reusable() {
        let limiter = ConnectionLimiter::new(2, 2);
        let source = SourceKey::Ipv4([127, 0, 0, 1]);
        let first = limiter.try_acquire(source).unwrap();
        let second = limiter.try_acquire(source).unwrap();
        assert_eq!(limiter.active(), 2);
        assert!(limiter.try_acquire(source).is_none());

        drop(first);
        assert_eq!(limiter.active(), 1);
        let replacement = limiter.try_acquire(source).unwrap();
        assert_eq!(limiter.active(), 2);

        drop((second, replacement));
        assert_eq!(limiter.active(), 0);
    }

    #[test]
    fn per_source_cap_preserves_capacity_for_other_sources() {
        let limiter = ConnectionLimiter::new(3, 1);
        let first_source = SourceKey::Ipv4([192, 0, 2, 1]);
        let second_source = SourceKey::Ipv4([192, 0, 2, 2]);
        let first = limiter.try_acquire(first_source).unwrap();
        assert!(limiter.try_acquire(first_source).is_none());
        let second = limiter.try_acquire(second_source).unwrap();
        assert_eq!(limiter.active(), 2);
        drop((first, second));
    }

    #[test]
    fn ipv6_sources_are_normalized_to_prefix_64() {
        let first = "[2001:db8:1:2::1]:1234".parse::<SocketAddr>().unwrap();
        let same_prefix = "[2001:db8:1:2::ffff]:9".parse::<SocketAddr>().unwrap();
        let other_prefix = "[2001:db8:1:3::1]:9".parse::<SocketAddr>().unwrap();
        assert_eq!(
            SourceKey::from_socket(first),
            SourceKey::from_socket(same_prefix)
        );
        assert_ne!(
            SourceKey::from_socket(first),
            SourceKey::from_socket(other_prefix)
        );

        let mapped = "[::ffff:203.0.113.9]:1234".parse::<SocketAddr>().unwrap();
        let ipv4 = "203.0.113.9:9".parse::<SocketAddr>().unwrap();
        assert_eq!(SourceKey::from_socket(mapped), SourceKey::from_socket(ipv4));
    }

    #[test]
    fn configured_proxy_delegates_only_its_source_bucket() {
        let classifier = SourceClassifier::parse_proxy_ips("127.0.0.1, ::1").unwrap();
        assert_eq!(
            classifier.classify("127.0.0.1:1".parse().unwrap()),
            SourceKey::ExternallyLimitedProxy
        );
        assert_eq!(
            classifier.classify("[::1]:1".parse().unwrap()),
            SourceKey::ExternallyLimitedProxy
        );
        assert_eq!(
            classifier.classify("192.0.2.1:1".parse().unwrap()),
            SourceKey::Ipv4([192, 0, 2, 1])
        );

        let limiter = ConnectionLimiter::new(3, 1);
        let first = limiter
            .try_acquire(SourceKey::ExternallyLimitedProxy)
            .unwrap();
        let second = limiter
            .try_acquire(SourceKey::ExternallyLimitedProxy)
            .unwrap();
        let third = limiter
            .try_acquire(SourceKey::ExternallyLimitedProxy)
            .unwrap();
        assert!(
            limiter
                .try_acquire(SourceKey::ExternallyLimitedProxy)
                .is_none(),
            "global connection cap must still apply to trusted proxies"
        );
        drop((first, second, third));
        assert_eq!(limiter.active(), 0);
    }

    #[test]
    fn invalid_proxy_ip_is_rejected() {
        assert!(SourceClassifier::parse_proxy_ips("127.0.0.1, nope").is_err());
    }

    #[test]
    fn review_task_ipv6_sources_of_one_end_site_share_a_network_cap() {
        let site = |index: u8| {
            SourceKey::from_socket(SocketAddr::new(
                IpAddr::V6(std::net::Ipv6Addr::new(
                    0x2001,
                    0xdb8,
                    0x1,
                    u16::from(index),
                    0,
                    0,
                    0,
                    1,
                )),
                1,
            ))
        };
        assert_ne!(site(1), site(2), "distinct /64 sources");
        assert_eq!(site(1).network(), site(2).network(), "one /56");
        let next_site = SourceKey::from_socket("[2001:db8:1:100::1]:1".parse().unwrap());
        assert_ne!(site(1).network(), next_site.network(), "the next /56");
        assert_eq!(SourceKey::Ipv4([192, 0, 2, 1]).network(), None);

        let limits = ServerLimits {
            max_connections: 100,
            max_connections_per_source: 2,
            max_connections_per_network: 3,
            ..ServerLimits::default()
        };
        let limiter = ConnectionLimiter::from_limits(&limits);
        let first = limiter.try_acquire(site(1)).unwrap();
        let second = limiter.try_acquire(site(2)).unwrap();
        let third = limiter.try_acquire(site(3)).unwrap();
        assert!(
            limiter.try_acquire(site(4)).is_none(),
            "a fourth /64 of the same /56 is refused"
        );
        assert!(limiter
            .try_acquire(SourceKey::Ipv4([192, 0, 2, 1]))
            .is_some());
        drop(first);
        assert!(limiter.try_acquire(site(4)).is_some());
        drop((second, third));
    }
}
