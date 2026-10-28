//! Viewer side of one TCP tunnel (design 2.6, 4.3, 4.4, 4.6). The listener
//! binds loopback only (127.0.0.1 or the device address of design 4.3) and
//! checks the origin of every local connection by its `ListenerAdmission`
//! before a byte reaches the peer. Without an explicitly chosen lifetime a
//! tunnel ends with its QUIC connection, a close or a revocation; an
//! auto-forward tunnel also ends with its last local connection. Reversing
//! the endpoint pair requires a separately started tunnel and that target's
//! own grant.
use super::core::{eio, hmac_proof, now_secs, random_token};
use super::node::ShareIrohNode;
use super::node_tcp::{require_policy, CloseTcp};
use super::profiles::fingerprint_matches;
use super::route_cancellation::RouteCancellation;
use super::service::ShareService;
use super::session::{endpoint_addr, relation_kind_id, PeerPrincipal};
use super::tcp_forward::{
    tcp_version_for_alpn, ListenerAdmission, TcpStart, TunnelId, TunnelRole, TunnelView,
    MAX_TCP_STREAMS, TCP_ALPN, TCP_ALPN_V1, TCP_PROTOCOL_VERSION_V1, TCP_V1_MAX_LIFETIME_SECS,
};
use super::tcp_registry::{spawn_tunnel, ListenerOptions, Source, TunnelParts};
use super::tcp_wire::{self, TcpClientHello, TcpHelloReply, TcpServerHello};
use super::types::{PeerEndpoint, PeerOpenTarget};
use crate::share::ShareIdentity;
use iroh::endpoint::{ConnectOptions, Connection, RecvStream, SendStream};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::{io, sync::Arc, time::Duration};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

/// Shown on a tunnel to a host up to 0.5.188, which knows only protocol 1
/// and ends every tunnel after at most 24 hours (design 4.6).
pub(crate) const OLD_HOST_NOTICE: &str =
    "Gegenstelle älter als 0.5.189: Weiterleitung endet nach 24 h";

/// An auto-forward tunnel: its ID and the sender of further admitted local
/// connections. A send fails, returning the socket, once the tunnel ended.
pub(crate) type ChannelLink = (TunnelId, mpsc::Sender<TcpStream>);

/// One authenticated tunnel after the host's `Opened`.
pub(crate) struct OpenedLink {
    pub(crate) close: CloseTcp,
    send: SendStream,
    recv: RecvStream,
    target: SocketAddr,
    /// The request as sent (protocol 1: lifetime capped at 24 h).
    request: TcpStart,
    old_host: bool,
}
impl OpenedLink {
    /// What the running tunnel keeps (`tcp_registry::spawn_tunnel`).
    pub(crate) fn into_parts(self) -> TunnelParts {
        TunnelParts {
            lifetime: self.request.fixed_lifetime(),
            close: self.close,
            send: self.send,
            recv: self.recv,
        }
    }
}

impl ShareIrohNode {
    /// Offers protocol 2 and protocol 1; the host's ALPN choice fixes the
    /// version of the whole tunnel (design 4.6).
    async fn connect_tcp(&self, endpoint: &PeerEndpoint) -> io::Result<(CloseTcp, u16)> {
        self.require_sharing_active()?;
        if endpoint
            .expected_node_id
            .as_deref()
            .is_some_and(|pin| !pin.is_empty() && pin != endpoint.presence.node_id)
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "TCP peer NodeId pin mismatch",
            ));
        }
        let local = self.routes.current(&self.endpoint);
        let address = endpoint_addr(&endpoint.presence, &local, &self.transport_options)?;
        let generation = self
            .policy
            .snapshot(PeerPrincipal::from_endpoint(endpoint))?;
        let options = ConnectOptions::new().with_additional_alpns(vec![TCP_ALPN_V1.to_vec()]);
        let connect = async {
            let connecting = self
                .endpoint
                .connect_with_opts(address, TCP_ALPN, options)
                .await
                .map_err(|error| io::Error::new(io::ErrorKind::NotConnected, error.to_string()))?;
            connecting
                .await
                .map_err(|error| io::Error::new(io::ErrorKind::NotConnected, error.to_string()))
        };
        // Dial bound (design 7: kept): an unreachable device must not hang.
        let connection = tokio::time::timeout(Duration::from_secs(30), connect)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "TCP peer connect timed out"))?
            .map_err(|error| {
                io::Error::new(
                    io::ErrorKind::NotConnected,
                    format!("TCP peer connection: {error}; the peer must support share-tcp"),
                )
            })?;
        let close = CloseTcp(connection.clone());
        let version = tcp_version_for_alpn(connection.alpn()).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "TCP host chose an unknown protocol",
            )
        })?;
        require_policy(&connection)?;
        self.policy.bind(&connection, generation)?;
        self.await_route(&connection, Some(endpoint), None).await?;
        Ok((close, version))
    }

    /// Dials, authenticates and asks the host to open `request`; returns
    /// after the host's `Opened`. A human confirmation on the host has no
    /// time limit; closing the caller's request drops this transport.
    pub(crate) async fn open_tcp_link(
        &self,
        identity: &ShareIdentity,
        endpoint: &PeerEndpoint,
        request: TcpStart,
        id: &TunnelId,
        pending: &mut (dyn FnMut() + Send),
        cancellation: Option<&Arc<RouteCancellation>>,
    ) -> io::Result<OpenedLink> {
        let (close, version) = self.connect_tcp(endpoint).await?;
        let connection = close.0.clone();
        if let Some(cancel) = cancellation {
            cancel.bind(&connection)?;
        }
        let mut request = request;
        // Version 2 hosts treat it as „the listener checks the origin“;
        // version 1 hosts require it (design 4.4).
        request.acknowledge_local_access = true;
        let old_host = version == TCP_PROTOCOL_VERSION_V1 && request.lifetime_secs == 0;
        if old_host {
            request.lifetime_secs = TCP_V1_MAX_LIFETIME_SECS;
        }
        request.validate_for(version).map_err(eio)?;
        let (send, recv, target) = super::route_protocol::scope(connection.clone(), async {
            let deadline = super::route_protocol::active_now() + Duration::from_secs(20);
            let (mut send, mut recv) =
                super::route_protocol::run_until_active(deadline, "TCP hello timed out", async {
                    connection.accept_bi().await.map_err(eio)
                })
                .await?;
            let server: TcpServerHello = super::route_protocol::run_until_active(
                deadline,
                "TCP server hello timed out",
                tcp_wire::recv_json(&mut recv),
            )
            .await?;
            check_server(&server, endpoint, &connection, version)?;
            super::route_monitor::authenticated_remote_capability(
                &connection,
                server.challenge.starts_with("rp1."),
            );
            let (kind, relation) = relation_kind_id(endpoint);
            let mut client = TcpClientHello {
                version,
                relation_kind: kind.into(),
                relation_id: relation,
                device_id: identity.device_id.clone(),
                device_name: identity.device_name.clone(),
                public_key: identity.public_key.clone(),
                fingerprint: identity.fingerprint.clone(),
                node_id: identity.node_id.clone(),
                nonce: format!("rp1.{}", random_token(24).map_err(eio)?),
                proof: String::new(),
                tunnel: id.clone(),
                request: request.clone(),
            };
            client.proof = hmac_proof(
                &endpoint.relation_secret,
                &super::relation_proof::relation_transcript(
                    &server.relation(&client)?,
                    &client.relation(),
                ),
            );
            super::route_protocol::run_until_active(
                deadline,
                "TCP client hello timed out",
                tcp_wire::send_json(&mut send, &client),
            )
            .await?;
            let mut reply: TcpHelloReply = super::route_protocol::timeout_for(
                Duration::from_secs(30),
                tcp_wire::recv_json(&mut recv),
            )
            .await??;
            let mut asked = false;
            loop {
                match reply {
                    TcpHelloReply::Opened { target } => {
                        check_bound_target(&request, target)?;
                        return Ok::<_, io::Error>((send, recv, target));
                    }
                    TcpHelloReply::Refused { detail } => {
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            super::tcp_forward::shown_error(&detail),
                        ))
                    }
                    TcpHelloReply::Pending if !asked => {
                        asked = true;
                        pending();
                    }
                    TcpHelloReply::Pending => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "Duplicate TCP pending reply",
                        ))
                    }
                }
                reply = tcp_wire::recv_json(&mut recv).await?;
            }
        })
        .await?;
        Ok(OpenedLink {
            close,
            send,
            recv,
            target,
            request,
            old_host,
        })
    }
}

pub(crate) fn tunnel_view(
    id: TunnelId,
    identity: &ShareIdentity,
    endpoint: &PeerEndpoint,
    link: &OpenedLink,
    listener: SocketAddr,
    admission: ListenerAdmission,
) -> TunnelView {
    let now = now_secs();
    let (kind, relation_id) = relation_kind_id(endpoint);
    TunnelView {
        id,
        role: TunnelRole::Listener,
        remote_node: endpoint.presence.node_id.clone(),
        initiator_node: identity.node_id.clone(),
        target_node: endpoint.presence.node_id.clone(),
        direction: super::access_constraints::TcpDirection::PeerToHost,
        relation_kind: kind.into(),
        relation_id,
        destination: link.request.destination.clone(),
        bound_target: Some(link.target),
        listener: Some(listener),
        started_at: now,
        expires_at: link.request.expires_at(now),
        admission: Some(admission),
        active_connections: 0,
        socket_read_bytes: 0,
        socket_written_bytes: 0,
        closing: false,
        last_error: link.old_host.then(|| OLD_HOST_NOTICE.to_string()),
    }
}

/// Resolves when `cancellation` fires; never without one.
async fn cancelled(cancellation: Option<Arc<RouteCancellation>>) -> io::Error {
    let Some(cancel) = cancellation else {
        return std::future::pending::<io::Error>().await;
    };
    loop {
        if let Err(error) = cancel.check() {
            return error;
        }
        tokio::select! {
            () = cancel.cancelled() => {
                return io::Error::new(io::ErrorKind::Interrupted, "TCP setup cancelled");
            }
            // The probe observes the closed IPC request (route_cancellation).
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }
}

impl ShareService {
    /// Starts a listener tunnel to `request.destination` of `target`. The
    /// listener binds `options.bind` (default 127.0.0.1) at
    /// `request.local_port` (0 = any) and forwards only connections its
    /// `options.admission` admits.
    pub(crate) fn tcp_start_cancellable(
        &self,
        target: &PeerOpenTarget,
        request: TcpStart,
        options: ListenerOptions,
        mut pending: impl FnMut() + Send,
        cancellation: Option<Arc<RouteCancellation>>,
    ) -> io::Result<TunnelView> {
        request.destination.validate().map_err(eio)?;
        options.validate()?;
        let endpoint = self.endpoint_for_target(target).map_err(eio)?;
        let node = self.iroh.clone();
        let identity = self.identity.clone();
        node.block_on(async {
            tokio::select! {
                biased;
                error = cancelled(cancellation.clone()) => Err(error),
                result = start_listener(
                    &node,
                    &identity,
                    &endpoint,
                    request,
                    options,
                    &mut pending,
                    cancellation.as_ref(),
                ) => result,
            }
        })
    }

    /// Auto-forward (design 4.3): opens a tunnel for `first`, a connection
    /// the caller's listener at `listener` already admitted by `admission`.
    /// Further admitted sockets go through the returned sender; the tunnel
    /// ends with its last local connection. Resolved per tunnel, because the
    /// device's addresses may have changed since the listener started.
    pub(crate) fn tcp_open_channel(
        &self,
        target: &PeerOpenTarget,
        request: TcpStart,
        listener: SocketAddr,
        admission: ListenerAdmission,
        first: TcpStream,
    ) -> impl std::future::Future<Output = io::Result<ChannelLink>> + Send {
        let endpoint = self.endpoint_for_target(target).map_err(eio);
        let node = self.iroh.clone();
        let identity = self.identity.clone();
        async move {
            let endpoint = endpoint?;
            let slot =
                node.tcp.slots.clone().try_acquire_owned().map_err(|_| {
                    io::Error::new(io::ErrorKind::WouldBlock, "TCP-Grenze 64 erreicht")
                })?;
            let id = TunnelId::generate().map_err(eio)?;
            let mut no_pending = || {};
            let link = node
                .open_tcp_link(&identity, &endpoint, request, &id, &mut no_pending, None)
                .await?;
            let view = tunnel_view(id.clone(), &identity, &endpoint, &link, listener, admission);
            let registration = node.tcp.registry.insert(view, link.close.0.clone())?;
            let (sockets, receiver) = mpsc::channel(MAX_TCP_STREAMS);
            sockets
                .try_send(first)
                .map_err(|_| io::Error::other("TCP-Tunnel nahm die erste Verbindung nicht an"))?;
            let source = Source::Channel(receiver);
            spawn_tunnel(&node, slot, link.into_parts(), source, registration);
            Ok((id, sockets))
        }
    }

    pub(crate) fn tcp_list(&self) -> Vec<TunnelView> {
        self.iroh.tcp.registry.list()
    }
    pub(crate) fn tcp_status(&self, id: &TunnelId) -> Option<TunnelView> {
        self.iroh.tcp.registry.status(id)
    }
    pub(crate) fn tcp_close(&self, id: &TunnelId) -> bool {
        self.iroh.tcp.registry.close(id)
    }
    /// Design 9.3 `BindProcess`: the one client of an `AwaitProcess` tunnel.
    pub(crate) fn tcp_bind_process(
        &self,
        id: &TunnelId,
        bound: super::tcp_registry::BoundProcess,
    ) -> io::Result<()> {
        self.iroh.tcp.registry.bind_process(id, bound)
    }
}

async fn start_listener(
    node: &Arc<ShareIrohNode>,
    identity: &ShareIdentity,
    endpoint: &PeerEndpoint,
    request: TcpStart,
    options: ListenerOptions,
    pending: &mut (dyn FnMut() + Send),
    cancellation: Option<&Arc<RouteCancellation>>,
) -> io::Result<TunnelView> {
    let slot = node
        .tcp
        .slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| io::Error::new(io::ErrorKind::WouldBlock, "TCP-Grenze 64 erreicht"))?;
    let id = TunnelId::generate().map_err(eio)?;
    let port = request.local_port;
    let link = node
        .open_tcp_link(identity, endpoint, request, &id, pending, cancellation)
        .await?;
    let address = SocketAddr::new(
        IpAddr::V4(options.bind.unwrap_or(Ipv4Addr::LOCALHOST)),
        port,
    );
    let listener = TcpListener::bind(address).await.map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("Lokaler Port {address} ist nicht verfügbar: {error}"),
        )
    })?;
    let view = tunnel_view(
        id,
        identity,
        endpoint,
        &link,
        listener.local_addr()?,
        options.admission,
    );
    let connection = link.close.0.clone();
    let registration = node.tcp.registry.insert(view.clone(), connection)?;
    let gate = registration.gate(options.admission, options.check);
    let source = Source::Listener { listener, gate };
    spawn_tunnel(node, slot, link.into_parts(), source, registration);
    Ok(view)
}

fn check_server(
    server: &TcpServerHello,
    endpoint: &PeerEndpoint,
    connection: &Connection,
    version: u16,
) -> io::Result<()> {
    let presence = &endpoint.presence;
    if server.version != version
        || server.device_id != presence.device_id
        || server.public_key != presence.public_key
        || server.fingerprint != presence.fingerprint
        || server.node_id != presence.node_id
        || server.node_id != connection.remote_id().to_string()
        || !fingerprint_matches(&server.public_key, &server.fingerprint)
        || server.challenge.len() < 24
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "TCP host identity or protocol does not match pinned device",
        ));
    }
    Ok(())
}
fn check_bound_target(request: &TcpStart, target: SocketAddr) -> io::Result<()> {
    use super::access_constraints::TcpHost;
    let matches = match &request.destination.host {
        TcpHost::Loopback => target.ip().is_loopback(),
        TcpHost::Ip { address } => *address == target.ip(),
        TcpHost::Dns { .. } => !target.ip().is_unspecified() && !target.ip().is_multicast(),
    };
    if matches && target.port() == request.destination.port.get() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "TCP host acknowledged an unexpected target",
        ))
    }
}
