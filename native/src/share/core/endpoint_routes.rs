use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use iroh::{Endpoint, EndpointAddr, RelayUrl, Watcher as _};

use super::types::ShareEvent;

/// Without a relay, devices outside this network cannot reach this one; a
/// normal start has its relay well before this.
const RELAY_WARNING_AFTER: Duration = Duration::from_secs(20);

#[derive(Clone, Debug, Default)]
pub(super) struct NodeTransportOptions {
    pub(super) relay_urls: Vec<RelayUrl>,
    pub(super) relay_only: bool,
    /// Certificate pins of the signaling server; its relay uses the same
    /// certificate (FC3).
    pub(super) relay_pins: Vec<[u8; 32]>,
    /// The user allowed a plaintext server: plain `http://` relays (also in
    /// peers' presences) are acceptable. Otherwise only `https://`.
    pub(super) plaintext_relays: bool,
}

impl NodeTransportOptions {
    pub(super) fn new(relay_urls: Vec<RelayUrl>, relay_only: bool) -> Self {
        Self {
            relay_urls,
            relay_only,
            relay_pins: Vec::new(),
            plaintext_relays: false,
        }
    }

    pub(super) fn with_server_trust(
        mut self,
        relay_pins: Vec<[u8; 32]>,
        plaintext_relays: bool,
    ) -> Self {
        self.relay_pins = relay_pins;
        self.plaintext_relays = plaintext_relays;
        self
    }

    /// TLS trust for the Iroh relays: the pinned server certificate plus
    /// publicly valid ones; `None` keeps Iroh's default roots.
    // Applied by the endpoint builder in `node.rs` (H-DISPATCH, see
    // anfragen/S-SIGNAL.md A2).
    #[allow(dead_code)]
    pub(super) fn ca_tls_config(&self) -> Option<iroh::tls::CaTlsConfig> {
        super::signal_connection::tls::relay_ca_tls_config(&self.relay_pins)
    }

    /// Whether a relay URL from a peer's presence may be dialed (S03).
    // Used by `session::endpoint_addr` (H-DISPATCH, see anfragen/S-SIGNAL.md A3).
    #[allow(dead_code)]
    pub(super) fn accepts_relay_url(&self, url: &str) -> bool {
        super::server_address::relay_url_acceptable(url, self.plaintext_relays)
    }
}

#[derive(Clone, Debug)]
pub(super) struct PublishedEndpointRoutes {
    pub(super) relay_url: String,
    pub(super) candidates: Vec<String>,
}

#[derive(Debug)]
pub(super) struct EndpointRoutes {
    revision: Arc<AtomicU64>,
}

impl EndpointRoutes {
    pub(super) fn start(
        runtime: &tokio::runtime::Runtime,
        endpoint: &Endpoint,
        relay_urls: &[RelayUrl],
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) -> Self {
        let revision = Arc::new(AtomicU64::new(0));

        let mut watcher = endpoint.watch_addr();
        let _ = watcher.get();
        let watcher_revision = revision.clone();
        runtime.spawn(async move {
            while watcher.updated().await.is_ok() {
                watcher_revision.fetch_add(1, Ordering::Release);
            }
        });

        if !relay_urls.is_empty() {
            let online_endpoint = endpoint.clone();
            let online_revision = revision.clone();
            let warning = relay_warning(relay_urls);
            let events = events.clone();
            runtime.spawn(async move {
                let started = Instant::now();
                let mut warned = false;
                loop {
                    if tokio::time::timeout(
                        Duration::from_secs(iroh::NET_REPORT_TIMEOUT),
                        online_endpoint.online(),
                    )
                    .await
                    .is_ok()
                    {
                        // A home-relay URL can be selected before its registration
                        // handshake completes. Publish once more when it is usable.
                        online_revision.fetch_add(1, Ordering::Release);
                        if warned {
                            let relay = online_endpoint
                                .addr()
                                .relay_urls()
                                .next()
                                .map(ToString::to_string)
                                .unwrap_or_default();
                            let _ = events
                                .try_send(ShareEvent::Status(format!("Relay erreichbar: {relay}")));
                        }
                        break;
                    }
                    // Each wait is bounded, but a relay which becomes reachable
                    // later must still trigger the first usable presence publish.
                    // Runtime shutdown cancels this task and its Endpoint clone.
                    if !warned && started.elapsed() >= RELAY_WARNING_AFTER {
                        warned = true;
                        if let Some(warning) = &warning {
                            let _ = events.try_send(ShareEvent::Error(warning.clone()));
                        }
                    }
                }
            });
        }

        Self { revision }
    }

    pub(super) fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    pub(super) fn current(&self, endpoint: &Endpoint) -> EndpointAddr {
        endpoint.addr()
    }

    pub(super) fn published(&self, endpoint: &Endpoint) -> PublishedEndpointRoutes {
        let current = self.current(endpoint);
        let relay_url = current
            .relay_urls()
            .next()
            .map(ToString::to_string)
            .unwrap_or_default();
        let candidates = current.ip_addrs().map(ToString::to_string).collect();
        PublishedEndpointRoutes {
            relay_url,
            candidates,
        }
    }
}

/// The log line for relays that stay unreachable. A relay on this machine
/// (loopback, as test servers configure it) is not reported.
fn relay_warning(relay_urls: &[RelayUrl]) -> Option<String> {
    let reported: Vec<String> = relay_urls
        .iter()
        .filter(|url| !is_loopback(url.host_str().unwrap_or_default()))
        .map(ToString::to_string)
        .collect();
    (!reported.is_empty()).then(|| {
        format!(
            "Relay nicht erreichbar ({}): Geräte außerhalb dieses Netzwerks erreichen dieses \
             Gerät nicht, bis das Relay antwortet",
            reported.join(", ")
        )
    })
}

fn is_loopback(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::relay_warning;

    #[test]
    fn windows_direct_task_unreachable_relays_are_named_except_loopback() {
        let relays = |urls: &[&str]| -> Vec<iroh::RelayUrl> {
            urls.iter().map(|url| url.parse().unwrap()).collect()
        };
        let warning = relay_warning(&relays(&[
            "https://silasweis.de:51821/",
            "http://127.0.0.1:1/",
        ]))
        .unwrap();
        assert!(warning.contains("Relay nicht erreichbar"), "{warning}");
        assert!(warning.contains("https://silasweis.de:51821/"), "{warning}");
        assert!(!warning.contains("127.0.0.1"), "{warning}");
        assert_eq!(
            relay_warning(&relays(&[
                "http://127.0.0.1:1/",
                "http://localhost:2/",
                "http://[::1]:3/"
            ])),
            None
        );
    }
}
