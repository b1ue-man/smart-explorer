//! The login protocol reuses Share's endpoint, TLS and P28 route admission.
//! No profile or credential store is read. The user signer comes from the
//! existing ShareService; the machine key comes from the private gateway loader.
pub use super::route_cancellation::RouteCancellation;
pub use super::route_state::RouteStatus;
#[path = "login_transport_admission.rs"]
mod admission;
use crate::login::core::gateway_types::{GatewayNetwork, GatewayTarget, LOGIN_ALPN};
pub use admission::{GatewayAdmission, GatewayTlsSlot};
use iroh::{
    endpoint::{ApplicationPathUpdate, Connection, VarInt},
    Endpoint, EndpointAddr, RelayUrl, SecretKey, TransportAddr,
};
use std::{
    future::Future,
    io,
    sync::Arc,
    time::{Duration, Instant},
};

/// Opaque lifetime retainer for the existing user endpoint and its runtime.
/// Constructed only from ShareService; never creates a second user endpoint.
pub struct LoginClientContext {
    pub endpoint: Endpoint,
    pub runtime: tokio::runtime::Handle,
    pub peer_id: String,
    pub key: SecretKey,
    /// Relays of the Share server configured on this device (E14): a target
    /// that asks for "default relays" is reached through these, never n0.
    pub relay_urls: Vec<RelayUrl>,
    pub _retainer: Arc<dyn Send + Sync>,
}
impl super::service::ShareService {
    pub fn login_client_context(&self) -> io::Result<LoginClientContext> {
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
            relay_urls: self.iroh.transport_options.relay_urls.clone(),
            _retainer: self.iroh.clone(),
        })
    }
}
/// Relay URLs for login options (E14). `use_default_relays` means "the
/// relays of the Share server configured in the program": `configured` holds
/// them (empty without a configured server, so only direct paths remain).
/// The number0 relays are never used.
fn relay_urls(options: &GatewayNetwork, configured: &[RelayUrl]) -> io::Result<Vec<RelayUrl>> {
    options.validate()?;
    if options.use_default_relays {
        return Ok(configured.to_vec());
    }
    options
        .relay_urls
        .iter()
        .map(|url| url.parse().map_err(io::Error::other))
        .collect()
}
/// The relays the machine gateway binds and publishes. The gateway reads no
/// user profile, so its configured server is the one provisioning wrote into
/// `gateway-network.json` ([`configured_server_network`]); a stored
/// `use_default_relays: true` without such relays leaves only direct paths.
fn gateway_relays(options: &GatewayNetwork) -> io::Result<Vec<RelayUrl>> {
    relay_urls(options, &[])
}
/// Network options for "the relays of the configured Share server" (E14):
/// relays, certificate pins and plaintext rule of the stored server address
/// `server`, resolved exactly like the Share endpoint does. Without a server
/// the result has no relays (direct paths only). The field names of
/// `gateway-network.json` stay; the result carries explicit relays.
pub fn configured_server_network(server: &str) -> GatewayNetwork {
    let options = super::transport_options::load(server);
    GatewayNetwork {
        format: crate::login::core::model::VERSION,
        use_default_relays: false,
        relay_urls: options
            .relay_urls
            .iter()
            .take(16)
            .map(ToString::to_string)
            .collect(),
        relay_pins: options.relay_pins.iter().take(16).copied().collect(),
        allow_plaintext_relays: options.plaintext_relays,
    }
}
pub async fn bind_machine(key: SecretKey, options: &GatewayNetwork) -> io::Result<Endpoint> {
    let urls = gateway_relays(options)?;
    let mode = super::endpoint_routes::relay_mode(&urls);
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
/// The distributed target names the relays the gateway itself uses (E14),
/// so a caller never substitutes other relays for an explicit gateway.
pub fn address(machine: &str, endpoint: &Endpoint, options: &GatewayNetwork) -> GatewayTarget {
    let address = endpoint.addr();
    let mut direct_addresses: Vec<_> = address.ip_addrs().copied().take(32).collect();
    direct_addresses.sort();
    direct_addresses.dedup();
    // `use_default_relays: true` binds no relay at the gateway
    // (`gateway_relays`); the target then says so instead of sending callers
    // to relays where the gateway is not registered.
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
        use_default_relays: false,
        allow_plaintext_relays: options.allow_plaintext_relays,
    }
}
/// A target with `use_default_relays: true` (also an older one) is reached
/// through the relays of the caller's configured server (E14).
fn target_address(target: &GatewayTarget, configured: &[RelayUrl]) -> io::Result<EndpointAddr> {
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
        .chain(
            relay_urls(&options, configured)?
                .into_iter()
                .map(TransportAddr::Relay),
        );
    Ok(EndpointAddr::from_parts(id, addresses))
}
pub async fn connect(
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
        result = tokio::time::timeout_at(deadline.into(), context.endpoint.connect(target_address(target, &context.relay_urls)?, LOGIN_ALPN)) =>
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
pub fn monitor(
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
pub fn authenticated(connection: &Connection) {
    super::route_monitor::authenticated_remote_capability(connection, true);
}
pub fn status(connection: &Connection) -> Option<RouteStatus> {
    super::route_monitor::snapshot(connection)
}
pub async fn routed<T>(
    connection: &Connection,
    active: Duration,
    deadline: Instant,
    future: impl Future<Output = io::Result<T>>,
) -> io::Result<T> {
    super::route_protocol::run_with_route(connection, active, Some(deadline), future).await
}
pub struct Close(pub Connection);
impl Drop for Close {
    fn drop(&mut self) {
        close(&self.0);
    }
}
pub fn close(connection: &Connection) {
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
pub async fn cancelled(cancel: &Arc<RouteCancellation>) -> io::Error {
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

#[cfg(test)]
#[path = "login_transport_tests.rs"]
mod tests;
