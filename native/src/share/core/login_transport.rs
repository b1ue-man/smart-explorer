//! The login protocol reuses Share's endpoint, TLS and P28 route admission.
//! No profile or credential store is read. The user signer comes from the
//! existing ShareService; the machine key comes from the private gateway loader.
pub(crate) use super::route_cancellation::RouteCancellation;
pub(crate) use super::route_state::RouteStatus;
use crate::login::core::gateway_types::{GatewayNetwork, GatewayTarget, LOGIN_ALPN};
use iroh::{
    endpoint::{ApplicationPathUpdate, Connection, VarInt},
    Endpoint, EndpointAddr, RelayMode, RelayUrl, SecretKey, TransportAddr,
};
use std::{
    future::Future,
    io,
    sync::Arc,
    time::{Duration, Instant},
};

/// Opaque lifetime retainer for the existing user endpoint and its runtime.
/// Constructed only from ShareService; never creates a second user endpoint.
pub(crate) struct LoginClientContext {
    pub(crate) endpoint: Endpoint,
    pub(crate) runtime: tokio::runtime::Handle,
    pub(crate) peer_id: String,
    pub(crate) key: SecretKey,
    pub(crate) _retainer: Arc<dyn Send + Sync>,
}
impl super::service::ShareService {
    pub(crate) fn login_client_context(&self) -> io::Result<LoginClientContext> {
        self.iroh.require_sharing_active()?;
        if self.iroh.endpoint.id() != self.identity.iroh_secret.public() {
            return Err(io::Error::other(
                "user endpoint and signing identity differ",
            ));
        }
        Ok(LoginClientContext {
            endpoint: self.iroh.endpoint.clone(),
            runtime: self.iroh.rt.handle().clone(),
            peer_id: self.identity.device_id.clone(),
            key: self.identity.iroh_secret.clone(),
            _retainer: self.iroh.clone(),
        })
    }
}
fn relay_urls(options: &GatewayNetwork) -> io::Result<Vec<RelayUrl>> {
    options.validate()?;
    if options.use_default_relays {
        return Ok(RelayMode::Default.relay_map().urls::<Vec<RelayUrl>>());
    }
    options
        .relay_urls
        .iter()
        .map(|url| url.parse().map_err(io::Error::other))
        .collect()
}
pub(crate) async fn bind_machine(key: SecretKey, options: &GatewayNetwork) -> io::Result<Endpoint> {
    let urls = relay_urls(options)?;
    let mode = if urls.is_empty() {
        RelayMode::Disabled
    } else {
        RelayMode::custom(urls)
    };
    let trust = super::endpoint_routes::NodeTransportOptions::new(Vec::new(), false)
        .with_server_trust(options.relay_pins.clone(), options.allow_plaintext_relays);
    let mut builder = Endpoint::builder(iroh::endpoint::presets::Minimal)
        .secret_key(key)
        .alpns(vec![LOGIN_ALPN.to_vec()])
        .crypto_provider(super::node::pq_kx::share_crypto_provider())
        .relay_mode(mode)
        .dns_resolver(super::transport_options::dns_resolver())
        .path_selector(Arc::new(super::route_adapter::RouteSelector::default()))
        .transport_config(gateway_transport_config());
    if let Some(config) = trust.ca_tls_config() {
        builder = builder.ca_tls_config(config);
    }
    builder.bind().await.map_err(io::Error::other)
}
fn gateway_transport_config() -> iroh::endpoint::QuicTransportConfig {
    use super::keepalive;
    use crate::login::core::model::MAX_CONTROL;
    let windows = keepalive::transport_windows(crate::transfer::memory_budget());
    iroh::endpoint::QuicTransportConfig::builder()
        .application_path_policy(true)
        .congestion_controller_factory(keepalive::congestion_factory(
            std::env::var(keepalive::QUIC_CC_VARIABLE)
                .as_deref()
                .unwrap_or(""),
        ))
        .max_idle_timeout(Some(
            VarInt::from_u32(keepalive::IROH_CONNECTION_IDLE_TIMEOUT.as_millis() as u32).into(),
        ))
        .keep_alive_interval(keepalive::IROH_CONNECTION_KEEPALIVE_INTERVAL)
        .default_path_keep_alive_interval(keepalive::IROH_PATH_KEEPALIVE_INTERVAL)
        // The host only receives a hello plus signed control; raw/PNG frames
        // flow in the other direction. Unauthenticated peers cannot fill a
        // filesystem-sized receive window on each pending TLS connection.
        .max_concurrent_bidi_streams(VarInt::from_u32(1))
        .max_concurrent_uni_streams(VarInt::from_u32(0))
        .stream_receive_window(VarInt::from_u32((MAX_CONTROL * 2) as u32))
        .receive_window(VarInt::from_u32((MAX_CONTROL * 2) as u32))
        .send_window(windows.send)
        .build()
}
pub(crate) fn address(
    machine: &str,
    endpoint: &Endpoint,
    options: &GatewayNetwork,
) -> GatewayTarget {
    let address = endpoint.addr();
    let mut direct_addresses: Vec<_> = address.ip_addrs().copied().take(32).collect();
    direct_addresses.sort();
    direct_addresses.dedup();
    let relay_urls = if options.use_default_relays {
        Vec::new()
    } else {
        options.relay_urls.clone()
    };
    GatewayTarget {
        format: 1,
        machine_id: machine.into(),
        public_key: endpoint.id().to_string(),
        direct_addresses,
        relay_urls,
        use_default_relays: options.use_default_relays,
        allow_plaintext_relays: options.allow_plaintext_relays,
    }
}
fn target_address(target: &GatewayTarget) -> io::Result<EndpointAddr> {
    target.validate()?;
    let id = target.public_key.parse().map_err(io::Error::other)?;
    let options = GatewayNetwork {
        format: 1,
        use_default_relays: target.use_default_relays,
        relay_urls: target.relay_urls.clone(),
        relay_pins: Vec::new(),
        allow_plaintext_relays: target.allow_plaintext_relays,
    };
    let addresses = target
        .direct_addresses
        .iter()
        .copied()
        .map(TransportAddr::Ip)
        .chain(relay_urls(&options)?.into_iter().map(TransportAddr::Relay));
    Ok(EndpointAddr::from_parts(id, addresses))
}
pub(crate) async fn connect(
    context: &LoginClientContext,
    target: &GatewayTarget,
    cancel: &Arc<RouteCancellation>,
    deadline: Instant,
) -> io::Result<Close> {
    cancel.check().map_err(|error| {
        if error.kind() == io::ErrorKind::Interrupted {
            cancelled_error()
        } else {
            error
        }
    })?;
    let connection = tokio::select! {
        biased;
        error = cancelled(cancel) => return Err(error),
        result = tokio::time::timeout_at(deadline.into(), context.endpoint.connect(target_address(target)?, LOGIN_ALPN)) =>
            result.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "login transport handshake timed out"))?.map_err(io::Error::other)?,
    };
    let close = Close(connection.clone());
    if connection.remote_id().to_string() != target.public_key {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "login transport identity differs from machine pin",
        ));
    }
    cancel.bind(&connection)?;
    monitor(
        &connection,
        context.endpoint.id().to_string(),
        true,
        target.direct_addresses.clone(),
    )?;
    tokio::select! {
        biased;
        error = cancelled(cancel) => return Err(error),
        result = super::route_monitor::wait_application_route(&connection, Some(deadline)) => { result?; }
    }
    Ok(close)
}
pub(crate) fn monitor(
    connection: &Connection,
    local: String,
    pinned: bool,
    hints: Vec<std::net::SocketAddr>,
) -> io::Result<()> {
    if !connection.application_path_policy().enabled {
        return Err(io::Error::other(
            "login connection has no serialized P28 payload policy",
        ));
    }
    super::route_monitor::start(connection, local, pinned, false, hints)
}
pub(crate) fn authenticated(connection: &Connection) {
    super::route_monitor::authenticated_remote_capability(connection, true);
}
pub(crate) fn status(connection: &Connection) -> Option<RouteStatus> {
    super::route_monitor::snapshot(connection)
}
pub(crate) async fn routed<T>(
    connection: &Connection,
    active: Duration,
    deadline: Instant,
    future: impl Future<Output = io::Result<T>>,
) -> io::Result<T> {
    super::route_protocol::run_with_route(connection, active, Some(deadline), future).await
}
pub(crate) struct Close(pub(crate) Connection);
impl Drop for Close {
    fn drop(&mut self) {
        close(&self.0);
    }
}
pub(crate) fn close(connection: &Connection) {
    let generation = connection.application_path_policy().generation;
    let _ = connection.update_application_path_policy(ApplicationPathUpdate::Block(generation));
    super::route_monitor::cancelled(connection);
    connection.close(VarInt::from_u32(0x4c47), b"login session ended");
}
fn cancelled_error() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "login connection cancelled")
}

/// Observe the existing non-consuming local EOF probe throughout setup and
/// paused I/O, rather than only when the first byte is queued.
pub(crate) async fn cancelled(cancel: &Arc<RouteCancellation>) -> io::Error {
    loop {
        if let Err(error) = cancel.check() {
            return if error.kind() == io::ErrorKind::Interrupted {
                cancelled_error()
            } else {
                error
            };
        }
        tokio::select! {
            () = cancel.cancelled() => return cancelled_error(),
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }
}
