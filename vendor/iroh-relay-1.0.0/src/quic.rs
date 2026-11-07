// Smart Explorer patch (2026-11-07): address validation (Retry) and bounded
// QAD admission, server idle timeout and no panic on a failed connection task.
// See `vendor/SMART_EXPLORER_QAD_LIMITS.md`.
//! Create a QUIC server that accepts connections
//! for QUIC address discovery.
use std::{net::SocketAddr, sync::Arc};

use n0_error::stack_error;
use n0_future::time::Duration;
use noq::{VarInt, crypto::rustls::QuicClientConfig};

/// ALPN for our quic addr discovery
pub const ALPN_QUIC_ADDR_DISC: &[u8] = b"/iroh-qad/0";
/// Endpoint close error code
pub const QUIC_ADDR_DISC_CLOSE_CODE: VarInt = VarInt::from_u32(1);
/// Endpoint close reason
pub const QUIC_ADDR_DISC_CLOSE_REASON: &[u8] = b"finished";

#[cfg(feature = "server")]
pub use self::limits::{QadServerLimits, set_server_limits};

/// Smart Explorer patch: admission limits of the QUIC address discovery server.
#[cfg(feature = "server")]
mod limits {
    use std::{
        collections::HashMap,
        net::{IpAddr, Ipv4Addr},
        num::NonZeroUsize,
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };

    use lru::LruCache;

    /// Bounds memory even when an attacker rotates through many source prefixes.
    const SOURCE_BUCKET_CAPACITY: usize = 4096;

    static SERVER_LIMITS: Mutex<Option<QadServerLimits>> = Mutex::new(None);

    /// Resource limits of the QUIC address discovery server.
    ///
    /// Every field left at `None` keeps the upstream behaviour: no limit.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    #[non_exhaustive]
    pub struct QadServerLimits {
        /// Most connections held at the same time.
        pub max_connections: Option<usize>,
        /// Most connections held at the same time from one source (an IPv4
        /// address or an IPv6 /64 prefix).
        pub max_connections_per_source: Option<usize>,
        /// Accepted new connections per second over all sources.
        pub accepts_per_second: Option<f64>,
        /// Burst of the global accept rate (default: the rate rounded up).
        pub accept_burst: Option<usize>,
        /// Accepted new connections per second from one source.
        pub accepts_per_second_per_source: Option<f64>,
        /// Burst of the per-source accept rate (default: the rate rounded up).
        pub accept_burst_per_source: Option<usize>,
        /// Idle timeout the server offers; the connection uses the smaller of
        /// both sides.
        pub idle_timeout: Option<Duration>,
    }

    /// Sets the limits that every QUIC address discovery server spawned
    /// afterwards in this process applies. Servers already running keep the
    /// limits they started with.
    pub fn set_server_limits(limits: QadServerLimits) {
        *SERVER_LIMITS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(limits);
    }

    pub(super) fn current_limits() -> QadServerLimits {
        let limits: Option<QadServerLimits> = *SERVER_LIMITS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        limits.unwrap_or_default()
    }

    /// Normalized source: IPv4 address (also when IPv4-mapped) or IPv6 /64.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    enum Source {
        V4(Ipv4Addr),
        V6Prefix64(u64),
    }

    impl Source {
        fn from_ip(ip: IpAddr) -> Self {
            match ip {
                IpAddr::V4(ip) => Self::V4(ip),
                IpAddr::V6(ip) => match ip.to_ipv4_mapped() {
                    Some(ip) => Self::V4(ip),
                    None => Self::V6Prefix64((u128::from(ip) >> 64) as u64),
                },
            }
        }
    }

    #[derive(Clone, Copy, Debug)]
    struct RateSpec {
        tokens_per_second: f64,
        burst: f64,
    }

    impl RateSpec {
        fn from_options(
            scope: &str,
            rate: Option<f64>,
            burst: Option<usize>,
        ) -> Result<Option<Self>, String> {
            let Some(tokens_per_second) = rate else {
                return Ok(None);
            };
            if !tokens_per_second.is_finite() || tokens_per_second <= 0.0 {
                return Err(format!(
                    "{scope} QAD accept rate must be a positive finite number"
                ));
            }
            let burst = burst.unwrap_or_else(|| tokens_per_second.ceil() as usize);
            if burst == 0 {
                return Err(format!(
                    "{scope} QAD accept burst must be greater than zero"
                ));
            }
            Ok(Some(Self {
                tokens_per_second,
                burst: burst as f64,
            }))
        }
    }

    #[derive(Debug)]
    struct TokenBucket {
        spec: RateSpec,
        tokens: f64,
        updated_at: Instant,
    }

    impl TokenBucket {
        fn new(spec: RateSpec, now: Instant) -> Self {
            Self {
                spec,
                tokens: spec.burst,
                updated_at: now,
            }
        }

        fn refill(&mut self, now: Instant) {
            let elapsed = now.saturating_duration_since(self.updated_at);
            self.tokens = (self.tokens + elapsed.as_secs_f64() * self.spec.tokens_per_second)
                .min(self.spec.burst);
            self.updated_at = now;
        }

        fn has_token(&self) -> bool {
            self.tokens >= 1.0
        }
    }

    #[derive(Debug)]
    struct State {
        total: usize,
        by_source: HashMap<Source, usize>,
        global_rate: Option<TokenBucket>,
        source_rate: Option<(RateSpec, LruCache<Source, TokenBucket>)>,
    }

    #[derive(Debug)]
    struct Inner {
        max_total: Option<usize>,
        max_per_source: Option<usize>,
        state: Mutex<State>,
    }

    /// Why an incoming connection was refused.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum Refusal {
        Rate,
        Capacity,
    }

    /// Admission control of the accept loop: rate first, then held connections.
    #[derive(Debug)]
    pub(super) struct Admission {
        inner: Arc<Inner>,
    }

    impl Admission {
        pub(super) fn new(limits: &QadServerLimits) -> Result<Self, String> {
            Self::new_at(limits, Instant::now())
        }

        /// [`Self::new`] at a given time (tests of this patch inject the clock).
        pub(super) fn new_at(limits: &QadServerLimits, now: Instant) -> Result<Self, String> {
            let global =
                RateSpec::from_options("global", limits.accepts_per_second, limits.accept_burst)?;
            let source = RateSpec::from_options(
                "per-source",
                limits.accepts_per_second_per_source,
                limits.accept_burst_per_source,
            )?;
            let capacity = NonZeroUsize::new(SOURCE_BUCKET_CAPACITY)
                .ok_or_else(|| "QAD source bucket capacity must be non-zero".to_string())?;
            Ok(Self {
                inner: Arc::new(Inner {
                    max_total: limits.max_connections,
                    max_per_source: limits.max_connections_per_source,
                    state: Mutex::new(State {
                        total: 0,
                        by_source: HashMap::new(),
                        global_rate: global.map(|spec| TokenBucket::new(spec, now)),
                        source_rate: source.map(|spec| (spec, LruCache::new(capacity))),
                    }),
                }),
            })
        }

        /// Admits one incoming connection from `ip`; the permit holds its
        /// place until dropped.
        pub(super) fn try_admit(&self, ip: IpAddr) -> Result<Permit, Refusal> {
            self.try_admit_at(ip, Instant::now())
        }

        /// [`Self::try_admit`] at a given time.
        pub(super) fn try_admit_at(&self, ip: IpAddr, now: Instant) -> Result<Permit, Refusal> {
            let source = Source::from_ip(ip);
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let State {
                total,
                by_source,
                global_rate,
                source_rate,
            } = &mut *state;
            let held_by_source = by_source.get(&source).copied().unwrap_or(0);
            if let Some(global) = global_rate.as_mut() {
                global.refill(now);
                if !global.has_token() {
                    return Err(Refusal::Rate);
                }
            }
            let mut source_bucket = None;
            if let Some((spec, buckets)) = source_rate.as_mut() {
                let spec = *spec;
                let bucket = buckets.get_or_insert_mut(source, || TokenBucket::new(spec, now));
                bucket.refill(now);
                if !bucket.has_token() {
                    return Err(Refusal::Rate);
                }
                source_bucket = Some(bucket);
            }
            let full = self.inner.max_total.is_some_and(|max| *total >= max)
                || self
                    .inner
                    .max_per_source
                    .is_some_and(|max| held_by_source >= max);
            let (Some(next_total), Some(next_by_source), false) =
                (total.checked_add(1), held_by_source.checked_add(1), full)
            else {
                return Err(Refusal::Capacity);
            };
            if let Some(bucket) = source_bucket {
                bucket.tokens -= 1.0;
            }
            if let Some(global) = global_rate.as_mut() {
                global.tokens -= 1.0;
            }
            *total = next_total;
            by_source.insert(source, next_by_source);
            Ok(Permit {
                inner: self.inner.clone(),
                source,
            })
        }

        /// Connections held right now (read by `qad_testing`).
        pub(super) fn held(&self) -> usize {
            self.inner
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .total
        }
    }

    /// One admitted connection; dropping it frees its place.
    #[derive(Debug)]
    pub(super) struct Permit {
        inner: Arc<Inner>,
        source: Source,
    }

    impl Drop for Permit {
        fn drop(&mut self) {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.total = state.total.saturating_sub(1);
            if let Some(held) = state.by_source.get_mut(&self.source) {
                *held = held.saturating_sub(1);
                if *held == 0 {
                    state.by_source.remove(&self.source);
                }
            }
        }
    }
}

#[cfg(feature = "server")]
pub(crate) mod server {
    use n0_error::e;
    use noq::{
        ApplicationClose, ConnectionError,
        crypto::rustls::{NoInitialCipherSuite, QuicServerConfig},
    };
    use tokio::task::JoinSet;
    use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};
    use tracing::{Instrument, debug, info, info_span, warn};

    use super::{limits::Admission, *};
    use crate::server::Metrics;

    pub(crate) struct QuicServer {
        bind_addr: SocketAddr,
        cancel: CancellationToken,
        handle: AbortOnDropHandle<()>,
    }

    /// Server spawn errors
    #[allow(missing_docs)]
    #[stack_error(derive, add_meta)]
    #[non_exhaustive]
    pub enum QuicSpawnError {
        #[error("TLS not configured")]
        TlsNotConfigured {},
        #[error(transparent)]
        NoInitialCipherSuite {
            #[error(std_err, from)]
            source: NoInitialCipherSuite,
        },
        // Smart Explorer patch: the message names the socket error.
        #[error("Unable to spawn a QUIC endpoint server: {source}")]
        EndpointServer {
            #[error(std_err)]
            source: std::io::Error,
        },
        #[error("Unable to get the local address from the endpoint")]
        LocalAddr {
            #[error(std_err)]
            source: std::io::Error,
        },
        /// Smart Explorer patch: the limits set by `set_server_limits` are invalid.
        #[error("Invalid QAD limits: {details}")]
        InvalidLimits { details: String },
    }

    impl QuicServer {
        /// Returns a handle for this server.
        ///
        /// The server runs in the background as several async tasks.  This allows controlling
        /// the server, in particular it allows gracefully shutting down the server.
        pub(crate) fn handle(&self) -> ServerHandle {
            ServerHandle {
                cancel_token: self.cancel.clone(),
            }
        }

        /// Returns the [`AbortOnDropHandle`] for the supervisor task managing the endpoint.
        ///
        /// This is the root of all the tasks for the QUIC address discovery service.  Aborting it will abort all the
        /// other tasks for the service.  Awaiting it will complete when all the service tasks are
        /// completed.[]
        pub(crate) fn task_handle(&mut self) -> &mut AbortOnDropHandle<()> {
            &mut self.handle
        }

        /// Returns the socket address for this QUIC server.
        pub(crate) fn bind_addr(&self) -> SocketAddr {
            self.bind_addr
        }

        /// Spawns a QUIC server that creates and QUIC endpoint and listens
        /// for QUIC connections for address discovery
        ///
        /// # Errors
        /// If the given `quic_config` contains a [`rustls::ServerConfig`] that cannot
        /// be converted to a [`QuicServerConfig`], usually because it does not support
        /// TLS 1.3, a [`NoInitialCipherSuite`] will occur.
        ///
        /// A panic during a connection ends only that connection and is logged
        /// as a warning (Smart Explorer patch); the limits set by
        /// [`super::set_server_limits`] bound admission before the handshake,
        /// and only for senders whose address a stateless Retry validated.
        pub(crate) fn spawn(
            bind_addr: SocketAddr,
            mut server_config: rustls::ServerConfig,
            metrics: Arc<Metrics>,
        ) -> Result<Self, QuicSpawnError> {
            let limits = super::limits::current_limits();
            let admission = Admission::new(&limits)
                .map_err(|details| e!(QuicSpawnError::InvalidLimits { details }))?;
            let idle_timeout = match limits.idle_timeout {
                Some(timeout) => Some(noq::IdleTimeout::try_from(timeout).map_err(|_| {
                    e!(QuicSpawnError::InvalidLimits {
                        details: format!("QAD idle timeout {timeout:?} is out of range"),
                    })
                })?),
                None => None,
            };
            server_config.alpn_protocols = vec![crate::quic::ALPN_QUIC_ADDR_DISC.to_vec()];
            let server_config = QuicServerConfig::try_from(server_config)?;
            let mut server_config = noq::ServerConfig::with_crypto(Arc::new(server_config));
            let transport_config =
                Arc::get_mut(&mut server_config.transport).expect("not used yet");
            transport_config
                .max_concurrent_uni_streams(0_u8.into())
                .max_concurrent_bidi_streams(0_u8.into())
                // enable sending quic address discovery frames
                .send_observed_address_reports(true);
            if idle_timeout.is_some() {
                transport_config.max_idle_timeout(idle_timeout);
            }

            let endpoint = noq::Endpoint::server(server_config, bind_addr)
                .map_err(|err| e!(QuicSpawnError::EndpointServer, err))?;
            let bind_addr = endpoint
                .local_addr()
                .map_err(|err| e!(QuicSpawnError::LocalAddr, err))?;

            info!(?bind_addr, "QUIC server listening on");

            let cancel = CancellationToken::new();
            let cancel_accept_loop = cancel.clone();

            let task = tokio::task::spawn(
                async move {
                    let mut set = JoinSet::new();
                    debug!("waiting for connections...");
                    loop {
                        tokio::select! {
                            biased;
                            _ = cancel_accept_loop.cancelled() => {
                                break;
                            }
                            Some(res) = set.join_next() => {
                                if let Err(err) = res {
                                    if err.is_panic() {
                                        // Smart Explorer patch: one failed connection
                                        // must not end the whole service.
                                        warn!("quic task panicked: {err:#?}");
                                    } else {
                                        debug!("quic task cancelled: {err:#?}");
                                    }
                                }
                            }
                            res = endpoint.accept() => match res {
                                Some(incoming) if !incoming.remote_address_validated() => {
                                    // Smart Explorer patch: a stateless Retry first proves
                                    // that the sender receives at its source address, so
                                    // spoofed senders hold no place and no rate token.
                                    let remote_addr = incoming.remote_address();
                                    if let Err(error) = incoming.retry() {
                                        debug!(%remote_addr, "QAD retry not allowed; ignoring");
                                        error.into_incoming().ignore();
                                    }
                                }
                                Some(incoming) => {
                                    let remote_addr = incoming.remote_address();
                                    match admission.try_admit(remote_addr.ip()) {
                                        Ok(permit) => {
                                            let metrics = metrics.clone();
                                            set.spawn(
                                                async move {
                                                    let _permit = permit;
                                                    handle_connection(incoming, metrics).await
                                                }
                                                .instrument(info_span!("qad-conn", %remote_addr)),
                                            );
                                        }
                                        Err(refusal) => {
                                            debug!(%remote_addr, ?refusal, "QAD connection refused by limits");
                                            incoming.refuse();
                                        }
                                    }
                                }
                                None => {
                                    debug!("endpoint closed");
                                    break;
                                }
                            }
                        }
                    }
                    // close all connections and wait until they have all grace
                    // fully closed.
                    endpoint.close(QUIC_ADDR_DISC_CLOSE_CODE, QUIC_ADDR_DISC_CLOSE_REASON);
                    endpoint.wait_idle().await;

                    // all tasks should be closed, since the endpoint has shutdown
                    // all connections, but await to ensure they are finished.
                    set.abort_all();
                    while !set.is_empty() {
                        _ = set.join_next().await;
                    }

                    debug!("quic endpoint has been shutdown.");
                }
                .instrument(info_span!("quic-endpoint")),
            );
            Ok(Self {
                bind_addr,
                cancel,
                handle: AbortOnDropHandle::new(task),
            })
        }

        /// Closes the underlying QUIC endpoint and the tasks running the
        /// QUIC connections.
        pub(crate) async fn shutdown(mut self) {
            self.cancel.cancel();
            if !self.task_handle().is_finished() {
                // only possible error is a `JoinError`, no errors about what might
                // have happened during a connection are propagated.
                _ = self.task_handle().await;
            }
        }
    }

    /// A handle for the Server side of QUIC address discovery.
    ///
    /// This does not allow access to the task but can communicate with it.
    #[derive(Debug, Clone)]
    pub(crate) struct ServerHandle {
        cancel_token: CancellationToken,
    }

    impl ServerHandle {
        /// Gracefully shut down the quic endpoint.
        pub(crate) fn shutdown(&self) {
            self.cancel_token.cancel()
        }
    }

    /// Handle the connection from the client.
    async fn handle_connection(
        incoming: noq::Incoming,
        metrics: Arc<Metrics>,
    ) -> Result<(), ConnectionError> {
        metrics.qad_incoming.inc();
        debug!("incoming");
        let connection = match incoming.await {
            Ok(conn) => conn,
            Err(e) => {
                debug!("establishing failed: {e:#}");
                metrics.qad_incoming_error.inc();
                return Err(e);
            }
        };
        metrics.qad_connections.inc();
        debug!("established");
        // wait for the client to close the connection
        let connection_err = connection.closed().await;
        metrics.qad_connections_closed.inc();
        match connection_err {
            noq::ConnectionError::ApplicationClosed(ApplicationClose { error_code, .. })
                if error_code == QUIC_ADDR_DISC_CLOSE_CODE =>
            {
                debug!("peer disconnected");
                Ok(())
            }
            _ => {
                debug!("peer disconnected with {connection_err:#}");
                metrics.qad_connections_errored.inc();
                Err(connection_err)
            }
        }
    }
}

/// Quic client related errors.
#[allow(missing_docs)]
#[stack_error(derive, add_meta, from_sources, std_sources)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    Connect {
        #[error(std_err)]
        source: noq::ConnectError,
    },
    #[error(transparent)]
    Connection {
        #[error(std_err)]
        source: noq::ConnectionError,
    },
    NoObservedAddr,
}

/// Handles the client side of QUIC address discovery.
#[derive(Debug, Clone)]
pub struct QuicClient {
    /// A QUIC Endpoint.
    ep: noq::Endpoint,
    /// A client config.
    client_config: noq::ClientConfig,
}

impl QuicClient {
    /// Create a new QuicClient to handle the client side of QUIC
    /// address discovery.
    pub fn new(ep: noq::Endpoint, mut client_config: rustls::ClientConfig) -> Self {
        // add QAD alpn
        client_config.alpn_protocols = vec![ALPN_QUIC_ADDR_DISC.into()];
        // go from rustls client config to rustls QUIC specific client config to
        // a noq client config
        let mut client_config = noq::ClientConfig::new(Arc::new(
            QuicClientConfig::try_from(client_config).expect("known ciphersuite"),
        ));

        // enable the receive side of address discovery
        let mut transport = noq_proto::TransportConfig::default();
        // Setting the initial RTT estimate to a low value means
        // we're sacrificing initial throughput, which is fine for
        // QAD, which doesn't require us to have good initial throughput.
        // It also implies a 999ms probe timeout, which means that
        // if the packet gets lost (e.g. because we're probing ipv6, but
        // ipv6 packets always get lost in our network configuration) we
        // time out *closing the connection* after only 999ms.
        // Even if the round trip time is bigger than 999ms, this doesn't
        // prevent us from connecting, since that's dependent on the idle
        // timeout (set to 30s by default).
        transport.initial_rtt(Duration::from_millis(111));
        transport.receive_observed_address_reports(true);

        // keep it alive
        transport.keep_alive_interval(Some(Duration::from_secs(25)));
        transport.max_idle_timeout(Some(
            Duration::from_secs(35).try_into().expect("known value"),
        ));
        client_config.transport_config(Arc::new(transport));

        Self { ep, client_config }
    }

    /// Client side of QUIC address discovery.
    ///
    /// Creates a connection and returns the observed address
    /// and estimated latency of the connection.
    ///
    /// Consumes and gracefully closes the connection.
    #[cfg(all(test, feature = "server"))]
    async fn get_addr_and_latency(
        &self,
        server_addr: SocketAddr,
        host: &str,
    ) -> Result<(SocketAddr, std::time::Duration), Error> {
        use n0_future::StreamExt;
        use noq_proto::PathId;

        let connecting = self
            .ep
            .connect_with(self.client_config.clone(), server_addr, host);
        let conn = connecting?.await?;
        let mut external_addresses = conn.observed_external_addr();
        // TODO(ramfox): I'd like to be able to cancel this so we can close cleanly
        // if there the task that runs this function gets aborted.
        // tokio::select! {
        //     _ = cancel.cancelled() => {
        //         conn.close(QUIC_ADDR_DISC_CLOSE_CODE, QUIC_ADDR_DISC_CLOSE_REASON);
        //         bail_any!("QUIC address discovery canceled early");
        //     },
        //     res = external_addresses.wait_for(|addr| addr.is_some()) => {
        //         let addr = res?.expect("checked");
        //         let latency = conn.rtt() / 2;
        //         // gracefully close the connections
        //         conn.close(QUIC_ADDR_DISC_CLOSE_CODE, QUIC_ADDR_DISC_CLOSE_REASON);
        //         Ok((addr, latency))
        //     }

        let Some(mut observed_addr) = external_addresses.next().await else {
            n0_error::bail!(Error::NoObservedAddr);
        };
        // if we've sent to an ipv4 address, but received an observed address
        // that is ivp6 then the address is an [IPv4-Mapped IPv6 Addresses](https://doc.rust-lang.org/beta/std/net/struct.Ipv6Addr.html#ipv4-mapped-ipv6-addresses)
        observed_addr = SocketAddr::new(observed_addr.ip().to_canonical(), observed_addr.port());
        let latency = conn.rtt(PathId::ZERO).unwrap_or_default();
        // gracefully close the connections
        conn.close(QUIC_ADDR_DISC_CLOSE_CODE, QUIC_ADDR_DISC_CLOSE_REASON);
        Ok((observed_addr, latency))
    }

    /// Create a connection usable for qad
    pub async fn create_conn(
        &self,
        server_addr: SocketAddr,
        host: &str,
    ) -> Result<noq::Connection, Error> {
        let config = self.client_config.clone();
        let connecting = self.ep.connect_with(config, server_addr, host);
        let conn = connecting?.await?;
        Ok(conn)
    }
}

/// Smart Explorer patch: test support for the QAD admission limits and the QAD
/// client. Not part of the upstream API. It wraps the existing types and changes
/// no limit, no admission path, no retry and no server behaviour.
#[cfg(feature = "server")]
#[doc(hidden)]
pub mod qad_testing {
    use std::{
        net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
        time::{Duration, Instant},
    };

    use n0_future::StreamExt;

    use super::limits::{Admission, Permit, Refusal};

    /// Admission of the QAD accept loop, with the same checks as at server start.
    #[derive(Debug)]
    pub struct TestAdmission {
        inner: Admission,
    }

    impl TestAdmission {
        /// Builds the admission for `limits`; invalid limits are an error.
        pub fn new(limits: super::QadServerLimits) -> Result<Self, String> {
            Admission::new(&limits).map(|inner| Self { inner })
        }

        /// Admits one connection from `source` at `now`. The error text is
        /// `"rate"` or `"capacity"`.
        pub fn try_admit_at(
            &self,
            source: IpAddr,
            now: Instant,
        ) -> Result<TestPermit, &'static str> {
            self.inner
                .try_admit_at(source, now)
                .map(|permit| TestPermit { _permit: permit })
                .map_err(|refusal| match refusal {
                    Refusal::Rate => "rate",
                    Refusal::Capacity => "capacity",
                })
        }

        /// Connections held right now.
        pub fn held(&self) -> usize {
            self.inner.held()
        }
    }

    /// One admitted connection; dropping it frees its place.
    #[derive(Debug)]
    pub struct TestPermit {
        _permit: Permit,
    }

    /// A QAD client connection made by [`probe`].
    pub struct QadProbe {
        endpoint: noq::Endpoint,
        connection: noq::Connection,
    }

    impl QadProbe {
        /// The address the server observed for this connection, or `None` when
        /// none arrives within `timeout`.
        pub async fn observed(&self, timeout: Duration) -> Option<SocketAddr> {
            let mut addresses = self.connection.observed_external_addr();
            n0_future::time::timeout(timeout, addresses.next())
                .await
                .ok()
                .flatten()
        }

        /// The local address of the client socket.
        pub fn local_addr(&self) -> Option<SocketAddr> {
            self.endpoint.local_addr().ok()
        }

        /// The reason the connection closed, or `None` while it is open.
        pub fn close_reason(&self) -> Option<String> {
            self.connection
                .close_reason()
                .map(|reason| reason.to_string())
        }

        /// Closes the connection and waits until the endpoint is idle, so the
        /// server has received the close frame when this returns.
        pub async fn close(self) {
            self.connection.close(
                super::QUIC_ADDR_DISC_CLOSE_CODE,
                super::QUIC_ADDR_DISC_CLOSE_REASON,
            );
            self.endpoint.wait_idle().await;
        }
    }

    /// Connects a QAD client from the loopback address of the server's address
    /// family to `server`, verifying the certificate for `server_name`.
    pub async fn probe(
        server: SocketAddr,
        server_name: &str,
        client_config: rustls::ClientConfig,
    ) -> Result<QadProbe, String> {
        let local = match server {
            SocketAddr::V4(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
            SocketAddr::V6(_) => IpAddr::V6(Ipv6Addr::LOCALHOST),
        };
        let endpoint = noq::Endpoint::client(SocketAddr::new(local, 0))
            .map_err(|error| format!("cannot create the QAD client endpoint: {error}"))?;
        let client = super::QuicClient::new(endpoint.clone(), client_config);
        let connection = client
            .create_conn(server, server_name)
            .await
            .map_err(|error| error.to_string())?;
        Ok(QadProbe {
            endpoint,
            connection,
        })
    }

    /// Restores the process-wide QAD limits to none (`QadServerLimits::default()`).
    pub fn reset_server_limits() {
        super::set_server_limits(super::QadServerLimits::default());
    }
}

#[cfg(all(test, feature = "server", with_crypto_provider))]
mod tests {
    use std::{net::Ipv4Addr, sync::Arc};

    use n0_error::{Result, StdResultExt};
    use n0_future::{
        task::AbortOnDropHandle,
        time::{self, Instant},
    };
    use n0_tracing_test::traced_test;
    use noq::crypto::rustls::QuicServerConfig;
    use tracing::{Instrument, debug, info, info_span};
    use webpki_types::PrivatePkcs8KeyDer;

    use super::*;

    #[tokio::test]
    #[traced_test]
    #[cfg(feature = "test-utils")]
    async fn quic_endpoint_basic() -> Result {
        use super::server::QuicServer;

        let host: Ipv4Addr = "127.0.0.1".parse().unwrap();
        // create a server config with self signed certificates
        let (_, server_config) = super::super::server::testing::self_signed_tls_certs_and_config();
        let bind_addr = SocketAddr::new(host.into(), 0);
        let quic_server = QuicServer::spawn(bind_addr, server_config, Default::default())?;

        // create a client-side endpoint
        let client_endpoint =
            noq::Endpoint::client(SocketAddr::new(host.into(), 0)).std_context("client")?;
        let client_addr = client_endpoint.local_addr().std_context("local addr")?;

        // create the client configuration used for the client endpoint when they
        // initiate a connection with the server
        let client_config = crate::tls::make_dangerous_client_config();
        let quic_client = QuicClient::new(client_endpoint.clone(), client_config);

        let (addr, _latency) = quic_client
            .get_addr_and_latency(quic_server.bind_addr(), &host.to_string())
            .await?;

        // wait until the endpoint delivers the closing message to the server
        client_endpoint.wait_idle().await;
        // shut down the quic server
        quic_server.shutdown().await;

        assert_eq!(client_addr, addr);
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    #[traced_test]
    async fn test_qad_client_closes_unresponsive_fast() -> Result {
        // create a client-side endpoint
        let client_endpoint = noq::Endpoint::client(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0))
            .std_context("client")?;

        // create an socket that does not respond.
        let server_socket =
            tokio::net::UdpSocket::bind(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0))
                .await
                .std_context("bind")?;
        let server_addr = server_socket.local_addr().std_context("local addr")?;

        // create the client configuration used for the client endpoint when they
        // initiate a connection with the server
        let client_config = crate::tls::make_dangerous_client_config();
        let quic_client = QuicClient::new(client_endpoint.clone(), client_config);

        // Start a connection attempt with nirvana - this will fail
        let task = AbortOnDropHandle::new(tokio::spawn({
            async move {
                quic_client
                    .get_addr_and_latency(server_addr, "localhost")
                    .await
            }
        }));

        // Even if we wait longer than the probe timeout, we will still be attempting to connect:
        tokio::time::sleep(Duration::from_millis(1000)).await;
        assert!(!task.is_finished());

        // time the closing of the client endpoint
        let before = Instant::now();
        client_endpoint.close(0u32.into(), b"byeeeee");
        client_endpoint.wait_idle().await;
        let time = Instant::now().duration_since(before);

        assert_eq!(time, Duration::from_millis(999));

        Ok(())
    }

    /// Makes sure that, even though the RTT was set to some fairly low value,
    /// we *do* try to connect for longer than what the time out would be after closing
    /// the connection, when we *don't* close the connection.
    ///
    /// In this case we don't simulate it via synthetically high RTT, but by dropping
    /// all packets on the server-side for 2 seconds.
    #[tokio::test]
    #[traced_test]
    async fn test_qad_connect_delayed() -> Result {
        // Create a socket for our QAD server.  We need the socket separately because we
        // need to pop off messages before we attach it to the Noq Endpoint.
        let socket = tokio::net::UdpSocket::bind(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0))
            .await
            .std_context("bind")?;
        let server_addr = socket.local_addr().std_context("local addr")?;
        info!(addr = ?server_addr, "server socket bound");

        // Create a QAD server with a self-signed cert, all manually.
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()])
            .std_context("self signed")?;
        let key = PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der());
        let mut server_crypto = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .std_context("crypto provider")?
        .with_no_client_auth()
        .with_single_cert(vec![cert.cert.into()], key.into())
        .std_context("tls")?;
        server_crypto.key_log = Arc::new(rustls::KeyLogFile::new());
        server_crypto.alpn_protocols = vec![ALPN_QUIC_ADDR_DISC.to_vec()];
        let mut server_config = noq::ServerConfig::with_crypto(Arc::new(
            QuicServerConfig::try_from(server_crypto).std_context("config")?,
        ));
        let transport_config = Arc::get_mut(&mut server_config.transport).unwrap();
        transport_config.send_observed_address_reports(true);

        let start = Instant::now();
        let server_task = tokio::spawn(
            async move {
                info!("Dropping all packets");
                time::timeout(Duration::from_secs(2), async {
                    let mut buf = [0u8; 1500];
                    loop {
                        let (len, src) = socket.recv_from(&mut buf).await.unwrap();
                        debug!(%len, ?src, "Dropped a packet");
                    }
                })
                .await
                .ok();
                info!("starting server");
                let server = noq::Endpoint::new(
                    Default::default(),
                    Some(server_config),
                    socket.into_std().unwrap(),
                    Arc::new(noq::TokioRuntime),
                )
                .std_context("endpoint new")?;
                info!("accepting conn");
                let incoming = server.accept().await.expect("missing conn");
                info!("incoming!");
                let conn = incoming.await.std_context("incoming")?;
                conn.closed().await;
                server.wait_idle().await;
                n0_error::Ok(())
            }
            .instrument(info_span!("server")),
        );
        let server_task = AbortOnDropHandle::new(server_task);

        info!("starting client");
        let client_endpoint = noq::Endpoint::client(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0))
            .std_context("client")?;

        // create the client configuration used for the client endpoint when they
        // initiate a connection with the server
        let client_config = crate::tls::make_dangerous_client_config();
        let quic_client = QuicClient::new(client_endpoint.clone(), client_config);

        // Now we should still connect, but it should take more than 1s.
        info!("making QAD request");
        let (addr, latency) = time::timeout(
            Duration::from_secs(10),
            quic_client.get_addr_and_latency(server_addr, "localhost"),
        )
        .await
        .std_context("timeout")??;
        let duration = start.elapsed();
        info!(?duration, ?addr, ?latency, "QAD succeeded");
        assert!(duration >= Duration::from_secs(1));

        time::timeout(Duration::from_secs(10), server_task)
            .await
            .std_context("timeout")?
            .std_context("server task")??;

        Ok(())
    }
}
