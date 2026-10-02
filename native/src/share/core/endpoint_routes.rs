use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use iroh::{Endpoint, EndpointAddr, RelayUrl, Watcher as _};

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
        relay_configured: bool,
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

        if relay_configured {
            let online_endpoint = endpoint.clone();
            let online_revision = revision.clone();
            runtime.spawn(async move {
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
                        break;
                    }
                    // Each wait is bounded, but a relay which becomes reachable
                    // later must still trigger the first usable presence publish.
                    // Runtime shutdown cancels this task and its Endpoint clone.
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
