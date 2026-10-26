//! Local, explicitly acknowledged loopback listener. Reversing the endpoint
//! pair requires a separately started tunnel and that target's own grant.
use std::{io, sync::Arc, time::Duration};
use iroh::endpoint::Connection;
use tokio::{io::AsyncReadExt, sync::Semaphore, task::JoinSet};
use super::core::{eio, hmac_proof, now_secs, random_token};
use super::node::ShareIrohNode;
use super::node_tcp::{cancelled_stop, require_policy, CloseTcp};
use super::profiles::fingerprint_matches;
use super::service::ShareService;
use super::session::{endpoint_addr, relation_kind_id, PeerPrincipal};
use super::tcp_forward::{TcpStart, TunnelId, TunnelRole, TunnelView, MAX_TCP_STREAMS, TCP_ALPN, TCP_PROTOCOL_VERSION};
use super::tcp_registry::TcpRegistration;
use super::tcp_wire::{self, TcpClientHello, TcpDataOpen, TcpDataReply, TcpHelloReply, TcpServerHello};
use super::types::{PeerEndpoint, PeerOpenTarget};

impl ShareIrohNode {
    async fn connect_tcp(&self, endpoint: &PeerEndpoint) -> io::Result<CloseTcp> {
        self.require_sharing_active()?;
        if endpoint.expected_node_id.as_deref().is_some_and(|pin|
            !pin.is_empty() && pin != endpoint.presence.node_id) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "TCP peer NodeId pin mismatch"));
        }
        let local = self.routes.current(&self.endpoint);
        let address = endpoint_addr(&endpoint.presence, &local, &self.transport_options)?;
        let generation = self.policy.snapshot(PeerPrincipal::from_endpoint(endpoint))?;
        let connection = tokio::time::timeout(Duration::from_secs(30), self.endpoint.connect(address, TCP_ALPN))
            .await.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "TCP peer connect timed out"))?
            .map_err(|error| io::Error::new(io::ErrorKind::NotConnected, format!("TCP peer connection: {error}; the peer must support share-tcp/1")))?;
        let close = CloseTcp(connection.clone());
        require_policy(&connection)?;
        self.policy.bind(&connection, generation)?;
        self.await_route(&connection, Some(endpoint), None).await?;
        Ok(close)
    }
}

impl ShareService {
    pub(crate) fn tcp_start(&self, target: &PeerOpenTarget, request: TcpStart,
        pending: impl FnMut()) -> io::Result<TunnelView> {
        self.tcp_start_cancellable(target, request, pending, None)
    }
    pub(crate) fn tcp_start_cancellable(&self, target: &PeerOpenTarget, request: TcpStart,
        mut pending: impl FnMut(), cancellation: Option<Arc<super::route_cancellation::RouteCancellation>>)
        -> io::Result<TunnelView> {
        request.validate().map_err(eio)?;
        let endpoint = self.endpoint_for_target(target).map_err(eio)?;
        let node = self.iroh.clone();
        let identity = self.identity.clone();
        node.block_on(async {
          let cancelled = async {
            if let Some(cancel) = &cancellation {
                loop {
                    if let Err(error) = cancel.check() { return error; }
                    tokio::select! {
                        () = cancel.cancelled() => return io::Error::new(io::ErrorKind::Interrupted, "TCP setup cancelled"),
                        _ = tokio::time::sleep(Duration::from_millis(100)) => {},
                    }
                }
            } else { std::future::pending::<io::Error>().await }
          };
          tokio::select! {
            biased;
            error = cancelled => Err(error),
            result = async {
            let slot = node.tcp.slots.clone().try_acquire_owned()
                .map_err(|_| io::Error::new(io::ErrorKind::WouldBlock, "TCP tunnel admission exhausted"))?;
            let close = node.connect_tcp(&endpoint).await?;
            let connection = close.0.clone();
            if let Some(cancel) = &cancellation { cancel.bind(&connection)?; }
            let id = TunnelId::generate().map_err(eio)?;
            let (mut send, mut recv, bound) = super::route_protocol::scope(connection.clone(), async {
                let deadline = super::route_protocol::active_now() + Duration::from_secs(20);
                let (mut send, mut recv) = super::route_protocol::run_until_active(deadline,
                    "TCP hello timed out", async { connection.accept_bi().await.map_err(eio) }).await?;
                let server: TcpServerHello = super::route_protocol::run_until_active(deadline,
                    "TCP server hello timed out", tcp_wire::recv_json(&mut recv)).await?;
                check_server(&server, &endpoint, &connection)?;
                super::route_monitor::authenticated_remote_capability(&connection, server.challenge.starts_with("rp1."));
                let (kind, relation) = relation_kind_id(&endpoint);
                let mut client = TcpClientHello {
                    version: TCP_PROTOCOL_VERSION, relation_kind: kind.into(), relation_id: relation,
                    device_id: identity.device_id.clone(), device_name: identity.device_name.clone(),
                    public_key: identity.public_key.clone(), fingerprint: identity.fingerprint.clone(),
                    node_id: identity.node_id.clone(), nonce: format!("rp1.{}", random_token(24).map_err(eio)?),
                    proof: String::new(), tunnel: id.clone(), request: request.clone(),
                };
                client.proof = hmac_proof(&endpoint.relation_secret,
                    &super::relation_proof::relation_transcript(&server.relation(&client)?, &client.relation()));
                super::route_protocol::run_until_active(deadline, "TCP client hello timed out",
                    tcp_wire::send_json(&mut send, &client)).await?;
                let mut reply: TcpHelloReply = super::route_protocol::timeout_for(Duration::from_secs(30),
                    tcp_wire::recv_json(&mut recv)).await??;
                let mut asked = false;
                loop {
                    match reply {
                        TcpHelloReply::Opened { target } => {
                            check_bound_target(&request, target)?;
                            return Ok::<_, io::Error>((send, recv, target));
                        }
                        TcpHelloReply::Refused { detail } => return Err(io::Error::new(io::ErrorKind::PermissionDenied,
                            super::tcp_forward::shown_error(&detail))),
                        TcpHelloReply::Pending if !asked => { asked = true; pending(); },
                        TcpHelloReply::Pending => return Err(io::Error::new(io::ErrorKind::InvalidData, "Duplicate TCP pending reply")),
                    }
                    // Human confirmation has no implicit timeout. Closing the
                    // authenticated daemon request drops this transport.
                    reply = tcp_wire::recv_json(&mut recv).await?;
                }
            }).await?;
            let listener = super::tcp_sockets::listen(&request).await?;
            let now = now_secs();
            let (kind, relation_id) = relation_kind_id(&endpoint);
            let view = TunnelView {
                id, role: TunnelRole::Listener, remote_node: endpoint.presence.node_id.clone(),
                initiator_node: identity.node_id.clone(), target_node: endpoint.presence.node_id.clone(),
                direction: super::access_constraints::TcpDirection::PeerToHost,
                relation_kind: kind.into(), relation_id, destination: request.destination.clone(),
                bound_target: Some(bound), listener: Some(listener.local_addr()?), started_at: now,
                expires_at: now.saturating_add(i64::from(request.lifetime_secs)), active_connections: 0,
                socket_read_bytes: 0, socket_written_bytes: 0, closing: false, last_error: None,
            };
            let registration = node.tcp.registry.insert(view.clone(), connection.clone())?;
            let weak_node = Arc::downgrade(&node);
            node.rt.spawn(async move {
                let _slot = slot;
                let _close = close;
                let _control_send = &mut send;
                run_listener(weak_node, connection, listener, &mut recv, registration, request.lifetime_secs).await;
            });
            Ok(view)
            } => result,
          }
        })
    }
    pub(crate) fn tcp_list(&self) -> Vec<TunnelView> { self.iroh.tcp.registry.list() }
    pub(crate) fn tcp_status(&self, id: &TunnelId) -> Option<TunnelView> { self.iroh.tcp.registry.status(id) }
    pub(crate) fn tcp_close(&self, id: &TunnelId) -> bool { self.iroh.tcp.registry.close(id) }
}

fn check_server(server: &TcpServerHello, endpoint: &PeerEndpoint, connection: &Connection) -> io::Result<()> {
    let presence = &endpoint.presence;
    if server.version != TCP_PROTOCOL_VERSION || server.device_id != presence.device_id
        || server.public_key != presence.public_key || server.fingerprint != presence.fingerprint
        || server.node_id != presence.node_id || server.node_id != connection.remote_id().to_string()
        || !fingerprint_matches(&server.public_key, &server.fingerprint) || server.challenge.len() < 24 {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "TCP host identity or protocol does not match pinned device"));
    }
    Ok(())
}
fn check_bound_target(request: &TcpStart, target: std::net::SocketAddr) -> io::Result<()> {
    use super::access_constraints::TcpHost;
    let matches = match &request.destination.host {
        TcpHost::Loopback => target.ip().is_loopback(),
        TcpHost::Ip { address } => *address == target.ip(),
        TcpHost::Dns { .. } => !target.ip().is_unspecified() && !target.ip().is_multicast(),
    };
    if matches && target.port() == request.destination.port.get() { Ok(()) }
    else { Err(io::Error::new(io::ErrorKind::PermissionDenied, "TCP host acknowledged an unexpected target")) }
}

async fn run_listener(node: std::sync::Weak<ShareIrohNode>, connection: Connection, listener: tokio::net::TcpListener,
    control_recv: &mut iroh::endpoint::RecvStream, registration: TcpRegistration, lifetime: u32) {
    let mut stop = registration.cancelled.clone();
    let streams = Arc::new(Semaphore::new(MAX_TCP_STREAMS));
    let activity = Arc::new(super::node::IncomingActivity::default());
    let mut children = JoinSet::new();
    let ttl = tokio::time::sleep(Duration::from_secs(u64::from(lifetime)));
    tokio::pin!(ttl);
    let mut close_byte = [0];
    loop {
        tokio::select! {
            biased;
            () = cancelled_stop(&mut stop) => break,
            _ = &mut ttl => break,
            _ = connection.closed() => break,
            _ = AsyncReadExt::read(control_recv, &mut close_byte) => break,
            Some(_) = children.join_next(), if !children.is_empty() => {},
            incoming = async {
                let slot = streams.clone().acquire_owned().await.map_err(eio)?;
                let (socket, _) = listener.accept().await?;
                Ok::<_, io::Error>((slot, socket))
            } => {
                let Ok((slot, mut socket)) = incoming else { break; };
                let connection = connection.clone();
                let id = registration.id.clone();
                let counters = registration.counters.clone();
                let registry = registration.registry();
                let Some(current) = node.upgrade() else { break; };
                let hold = current.incoming_stream_started(&activity);
                drop(current);
                children.spawn(async move {
                    let _slot = slot;
                    let _hold = hold;
                    let result = super::route_protocol::scope(connection.clone(), async {
                        socket.set_nodelay(true)?;
                        let (mut send, mut recv) = super::route_protocol::timeout_for(Duration::from_secs(20),
                            connection.open_bi()).await?.map_err(eio)?;
                        super::route_protocol::timeout_for(Duration::from_secs(20),
                            tcp_wire::send_json(&mut send, &TcpDataOpen { tunnel: id.clone() })).await??;
                        let reply: TcpDataReply = super::route_protocol::timeout_for(Duration::from_secs(20),
                            tcp_wire::recv_json(&mut recv)).await??;
                        match reply {
                            TcpDataReply::Connected => super::tcp_pump::pump(&connection,
                                &mut socket, &mut send, &mut recv, counters).await,
                            TcpDataReply::Refused { detail } => Err(io::Error::other(super::tcp_forward::shown_error(&detail))),
                        }
                    }).await;
                    if let Err(error) = &result { registry.record_error(&id, error); }
                    result
                });
            }
        }
    }
    super::node_tcp::close_connection(&connection);
    children.shutdown().await;
    drop(listener);
    drop(registration);
}
