//! The login protocol reuses Share's endpoint, TLS and P28 route admission.
//! No profile or credential store is read. The user signer comes from the
//! existing ShareService; the machine key comes from the private gateway loader.
pub use super::route_cancellation::RouteCancellation;
pub use super::route_state::RouteStatus;
#[path = "login_transport_admission.rs"]
mod admission;
#[path = "node_sessions_login_target.rs"]
mod session_fetch;
#[path = "login_target_store.rs"]
pub mod target_store;
#[path = "login_target_wire.rs"]
pub mod target_wire;
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
pub use target_store::{
    choose_open, login_state, LoginSwitch, LoginTargetBook, LookupError, OpenChoice,
    PairedLoginTarget, SwitchAction, ViewerSurface,
};
pub use target_wire::{LoginTargetMessage, SessionPeer, WithdrawReason, LOGIN_TARGET_FEATURE};

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
    let endpoint = builder.bind().await.map_err(io::Error::other)?;
    // Two candidates on one host start without QAD (`relay_mode`).
    let deferred = super::endpoint_routes::deferred_qad(&urls);
    if !deferred.is_empty() {
        tokio::spawn(super::endpoint_routes::grant_qad_to_home_relay(
            endpoint.clone(),
            deferred,
        ));
    }
    Ok(endpoint)
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

/// Host side of `login_target_v1` (E22) for the device the Share session
/// authenticated: `broker` is `crate::login::self_setup::broker_view()` of
/// this host. Steering is checked here again, at the moment of the answer.
pub fn host_login_target(
    host_device_id: &str,
    device_id: &str,
    public_key: &str,
    broker: io::Result<crate::login::core::self_grants::SelfReply>,
) -> LoginTargetMessage {
    let steering = crate::share::access_steering::steering_device(device_id, public_key);
    let broker = broker.ok().filter(|reply| reply.state == "ok");
    target_wire::host_message(&target_wire::HostFacts {
        host_device_id,
        device_id,
        steering,
        account: broker.as_ref().map(|reply| reply.account.as_str()),
        own_grants: broker
            .as_ref()
            .map_or(&[][..], |reply| reply.own_grants.as_slice()),
        target: broker.as_ref().and_then(|reply| reply.target.as_ref()),
    })
}

/// Host side of `FsRequest::LoginTarget` (E43) for the device the asking
/// session authenticated. Every authenticated peer may ask; only a device
/// that steers this screen now makes the host ask its login broker, and
/// every other device gets a withdrawal without target. The answer stays
/// within `MAX_NETWORK` bytes (S2).
pub(in crate::share) fn host_login_answer(
    auth: &std::sync::Mutex<super::types::ShareAuthState>,
    device_id: &str,
    public_key: &str,
) -> io::Result<LoginTargetMessage> {
    let host = auth
        .lock()
        .map_err(|_| io::Error::other("Share-State gesperrt"))?
        .identity
        .device_id
        .clone();
    let broker = if crate::share::access_steering::steering_device(device_id, public_key) {
        crate::login::self_setup::broker_view()
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "no steering right",
        ))
    };
    let message = host_login_target(&host, device_id, public_key, broker);
    message.encode()?;
    Ok(message)
}

/// Viewer side: whether a host's features let the viewer ask for its
/// login target; an older host never gets the request.
pub(in crate::share) fn asks_login_target(features: &super::wire::FsHostFeatures) -> bool {
    features.login_target_v1
}

/// Viewer side: the host's message in its answer to `LoginTarget`, checked
/// for format and size before it takes effect.
pub(in crate::share) fn login_answer(
    response: super::wire::FsResponse,
) -> io::Result<LoginTargetMessage> {
    match response {
        super::wire::FsResponse::LoginTarget { message } => {
            message.encode()?;
            Ok(message)
        }
        super::wire::FsResponse::Err { msg, .. } => Err(io::Error::other(msg)),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unerwartete Antwort auf die Anmeldeziel-Abfrage",
        )),
    }
}

/// Viewer side: the device whose paired login target goes with a removed
/// Direct contact (its recorded device, else its last presence).
pub(in crate::share) fn removed_contact_device(
    remote_device_id: Option<&str>,
    presence: Option<&super::types::PeerPresence>,
) -> Option<String> {
    remote_device_id
        .filter(|device| !device.is_empty())
        .or_else(|| presence.map(|presence| presence.device_id.as_str()))
        .filter(|device| !device.is_empty())
        .map(str::to_string)
}

/// Viewer side: applies the host's answer from the pinned session `peer`.
/// A refused message changes nothing and fails with `PermissionDenied`.
pub fn viewer_login_target(
    peer: &SessionPeer,
    message: &LoginTargetMessage,
) -> io::Result<target_store::Change> {
    let now = crate::share::core_now_secs().max(0) as u64;
    target_store::update(|book| match book.apply(peer, message, now) {
        Ok(change) => (Ok(change), change != target_store::Change::Unchanged),
        Err(refusal) => (Err(refusal), false),
    })?
    .map_err(|refusal| io::Error::new(io::ErrorKind::PermissionDenied, refusal.key()))
}

/// Viewer side: the pinned session's host does not offer `login_target_v1`.
pub fn viewer_login_target_unsupported(peer: &SessionPeer) -> io::Result<bool> {
    let now = crate::share::core_now_secs().max(0) as u64;
    target_store::update(|book| {
        let changed = book.note_legacy(peer, now);
        (changed, changed)
    })
}

/// Viewer side: the contact `device_id` was removed or re-paired.
pub fn viewer_forget_login_target(device_id: &str) -> io::Result<bool> {
    target_store::update(|book| {
        let changed = book.forget(device_id);
        (changed, changed)
    })
}

/// The paired target `selector` names (CLI, MCP, Bridge).
pub fn paired_login_target(selector: &str) -> Result<PairedLoginTarget, LookupError> {
    target_store::load().map_err(store_error)?.resolve(selector)
}

/// The paired target of exactly this device and Share key (device pages).
pub fn paired_login_target_bound(device_id: &str, public_key: &str) -> Option<PairedLoginTarget> {
    target_store::load()
        .ok()?
        .bound(device_id, public_key)
        .cloned()
}

/// The form of „Anmeldung öffnen“ for `selector` (see [`choose_open`]).
pub fn login_open_choice(
    selector: &str,
    acknowledged: bool,
    file_exists: bool,
) -> Result<OpenChoice, LookupError> {
    if acknowledged {
        return Ok(OpenChoice::File);
    }
    let book = target_store::load().map_err(store_error)?;
    choose_open(&book, selector, acknowledged, file_exists)
}

fn store_error(error: io::Error) -> LookupError {
    LookupError {
        code: "login_target_store",
        message: format!("Anmeldeziele nicht lesbar: {error}"),
    }
}

#[cfg(test)]
#[path = "login_transport_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "login_target_task_tests.rs"]
mod target_task_tests;

#[cfg(test)]
#[path = "login_ziel_weg_task_tests.rs"]
mod ziel_weg_task_tests;

#[cfg(test)]
#[path = "login_ziel_knoten_task_tests.rs"]
mod ziel_knoten_task_tests;
