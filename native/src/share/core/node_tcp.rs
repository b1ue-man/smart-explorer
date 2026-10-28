//! Authenticated TCP target on the shared policy-enabled Iroh endpoint.
//! One exact relationship and one immutable resolved IP own the whole tunnel.
use super::access_revocation::{AccessHandle, AccessRevocation};
use super::core::{eio, random_token};
use super::handshake_limits::ApplicationHandshakePermit;
use super::node::ShareIrohNode;
use super::relation_proof::prove_relation;
use super::session::PeerPrincipal;
use super::tcp_forward::{tcp_version_for_alpn, MAX_TCP_STREAMS, MAX_TCP_TUNNELS};
use super::tcp_registry::TcpRegistry;
use super::tcp_wire::{
    self, TcpClientHello, TcpDataOpen, TcpDataReply, TcpHelloReply, TcpServerHello,
};
use iroh::endpoint::{Connection, VarInt};
use std::{io, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    io::AsyncReadExt,
    sync::{watch, Semaphore},
    task::JoinSet,
};

#[path = "node_tcp_target.rs"]
mod target;

pub(crate) struct NodeTcp {
    pub(crate) registry: Arc<TcpRegistry>,
    pub(crate) slots: Arc<Semaphore>,
}
impl NodeTcp {
    pub(super) fn new() -> Self {
        Self {
            registry: Arc::new(TcpRegistry::default()),
            slots: Arc::new(Semaphore::new(MAX_TCP_TUNNELS)),
        }
    }
}
pub(super) fn stop_all(tcp: &NodeTcp) {
    tcp.registry.close_all();
}

/// Every exit closes the transport, including malformed setup, denied
/// approval, failed DNS, registry admission and dropped outer futures.
pub(crate) struct CloseTcp(pub(crate) Connection);
impl Drop for CloseTcp {
    fn drop(&mut self) {
        close_connection(&self.0);
    }
}
pub(crate) fn close_connection(connection: &Connection) {
    let generation = connection.application_path_policy().generation;
    let _ = connection
        .update_application_path_policy(iroh::endpoint::ApplicationPathUpdate::Block(generation));
    connection.close(VarInt::from_u32(0x5443), b"TCP tunnel ended");
}

impl ShareIrohNode {
    pub(super) async fn handle_tcp_connection(
        self: Arc<Self>,
        connection: Connection,
        permit: ApplicationHandshakePermit,
    ) -> io::Result<()> {
        let _close = CloseTcp(connection.clone());
        let _incoming = self.track_incoming(&connection)?;
        let _slot = self.tcp.slots.clone().try_acquire_owned().map_err(|_| {
            io::Error::new(io::ErrorKind::WouldBlock, "TCP tunnel admission exhausted")
        })?;
        self.require_sharing_active()?;
        require_policy(&connection)?;
        // `/2` and `/1` are both registered (design 4.6); the ALPN fixes the
        // protocol version of the whole tunnel.
        let version = tcp_version_for_alpn(connection.alpn())
            .ok_or_else(|| eio("unknown TCP tunnel protocol"))?;
        self.await_route(&connection, None, None).await?;
        super::route_protocol::scope(
            connection.clone(),
            self.serve_tcp(connection, permit, version),
        )
        .await
    }

    async fn serve_tcp(
        self: &Arc<Self>,
        connection: Connection,
        permit: ApplicationHandshakePermit,
        version: u16,
    ) -> io::Result<()> {
        let deadline = super::route_protocol::active_now() + Duration::from_secs(20);
        let (mut send, mut recv) =
            super::route_protocol::run_until_active(deadline, "TCP handshake timed out", async {
                connection.open_bi().await.map_err(eio)
            })
            .await?;
        let identity = self
            .auth
            .lock()
            .map_err(|_| eio("Share authorization locked"))?
            .identity
            .clone();
        let server = TcpServerHello {
            version,
            challenge: format!("rp1.{}", random_token(32).map_err(eio)?),
            device_id: identity.device_id,
            public_key: identity.public_key,
            fingerprint: identity.fingerprint,
            node_id: identity.node_id,
        };
        super::route_protocol::run_until_active(
            deadline,
            "TCP server hello timed out",
            tcp_wire::send_json(&mut send, &server),
        )
        .await?;
        let remote = connection.remote_id().to_string();
        let received = super::route_protocol::run_until_active(
            deadline,
            "TCP client hello timed out",
            tcp_wire::recv_json::<_, TcpClientHello>(&mut recv),
        )
        .await;
        let client = match received {
            Ok(client) => client,
            Err(error) => {
                self.auth_penalties().record(
                    &remote,
                    super::auth_penalty::PenaltyKind::NoAuthentication,
                    std::time::Instant::now(),
                );
                return Err(error);
            }
        };
        if let Err(error) = client.request.validate_for(version) {
            self.auth_penalties().record(
                &remote,
                super::auth_penalty::PenaltyKind::AuthFailure,
                std::time::Instant::now(),
            );
            return Err(eio(error));
        }
        let principal = {
            let state = self
                .auth
                .lock()
                .map_err(|_| eio("Share authorization locked"))?;
            prove_relation(
                &server.relation(&client)?,
                &client.relation(),
                &remote,
                &state,
            )
        }
        .map_err(|rejection| {
            if rejection.counts_as_failed_authentication() {
                self.auth_penalties().record(
                    &remote,
                    super::auth_penalty::PenaltyKind::AuthFailure,
                    std::time::Instant::now(),
                );
            }
            super::exec_audit::record(super::exec_audit::AuditEvent::AuthRejected {
                remote_node: &remote,
                claimed_device: &client.device_id,
                reason: "tcp: relation authentication failed or relation inactive",
            });
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "TCP relation authentication failed",
            )
        })?;
        self.bind_incoming_principal(
            &connection,
            PeerPrincipal::new(
                &principal.relation_kind,
                &principal.relation_id,
                &principal.device_id,
                &principal.public_key,
                &principal.node_id,
            ),
        )
        .await?;
        super::route_monitor::authenticated_remote_capability(
            &connection,
            client.nonce.starts_with("rp1."),
        );
        drop(permit);
        refresh_exposed_ports(&client.request.destination).await;
        // DNS is resolved by this target host; the peer supplies no resolved
        // IP authority. Selection consults the exact stored relation only.
        let mut setup_close = [0u8; 1];
        let admission = tokio::select! {
            biased;
            _ = connection.closed() => Err(io::Error::new(io::ErrorKind::Interrupted, "TCP requester left during target selection")),
            _ = AsyncReadExt::read(&mut recv, &mut setup_close) => Err(io::Error::new(io::ErrorKind::Interrupted, "TCP requester closed target setup")),
            result = self.admit_tcp_target(&connection, &principal, &client, &server, &mut send) => result,
        };
        let target::AdmittedTarget {
            registration,
            access,
            address: target,
            socket,
        } = match admission {
            Ok(admission) => admission,
            Err(error) => {
                let refusal = TcpHelloReply::Refused {
                    detail: error.to_string(),
                };
                let _ = tokio::time::timeout(
                    Duration::from_secs(2),
                    tcp_wire::send_json(&mut send, &refusal),
                )
                .await;
                let _ = send.finish();
                let _ = tokio::time::timeout(Duration::from_secs(2), connection.closed()).await;
                return Err(error);
            }
        };
        let mut initial_socket = Some(socket);
        let opened = TcpHelloReply::Opened { target };
        let accepted = tcp_wire::send_json(&mut send, &opened);
        tokio::select! {
            biased;
            () = revoked(&access) => return Err(io::Error::new(io::ErrorKind::PermissionDenied, "TCP access revoked")),
            result = super::route_protocol::timeout_for(Duration::from_secs(20), accepted) => result??,
        }
        let _control_send = send;
        let mut close_byte = [0];
        let mut cancelled = registration.cancelled.clone();
        // Only an explicitly chosen lifetime ends the tunnel by time; with
        // `lifetime_secs = 0` (version 2) it ends with the connection, the
        // grant or a close (design 2.6, user rule r7).
        let fixed = client.request.fixed_lifetime();
        let ttl = async move {
            match fixed {
                Some(lifetime) => tokio::time::sleep(lifetime).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::pin!(ttl);
        let streams = Arc::new(Semaphore::new(MAX_TCP_STREAMS));
        let activity = Arc::new(super::node::IncomingActivity::default());
        let mut children = JoinSet::new();
        loop {
            tokio::select! {
                biased;
                () = revoked(&access) => break,
                () = cancelled_stop(&mut cancelled) => break,
                _ = &mut ttl => break,
                _ = connection.closed() => break,
                _ = AsyncReadExt::read(&mut recv, &mut close_byte) => break,
                Some(_) = children.join_next(), if !children.is_empty() => {},
                opened = async {
                    let slot = streams.clone().acquire_owned().await.map_err(eio)?;
                    let (send, recv) = connection.accept_bi().await.map_err(eio)?;
                    Ok::<_, io::Error>((slot, send, recv))
                } => {
                    let Ok((slot, send, recv)) = opened else { break; };
                    let connection = connection.clone();
                    let destination = client.request.destination.clone();
                    let id = client.tunnel.clone();
                    let counters = registration.counters.clone();
                    let registry = self.tcp.registry.clone();
                    let mut revoked = access.revoked();
                    let authority = access.clone();
                    let initial_socket = initial_socket.take();
                    let hold = self.incoming_stream_started(&activity);
                    let (auth, leases) = (self.auth.clone(), self.access_registry().clone());
                    children.spawn(async move {
                        let _slot = slot;
                        let _hold = hold;
                        reverify_exposure(&destination, &auth, &leases).await;
                        let result = super::route_protocol::scope(connection.clone(), async {
                            tokio::select! {
                                biased;
                                () = revocation_stop(&mut revoked) => Err(io::Error::new(io::ErrorKind::PermissionDenied, "TCP access revoked")),
                                result = serve_socket(&connection, &id, &destination, target, send, recv, counters, authority, initial_socket) => result,
                            }
                        }).await;
                        if let Err(error) = &result { registry.record_error(&id, error); }
                        result
                    });
                }
            }
        }
        close_connection(&connection);
        children.shutdown().await;
        drop(initial_socket);
        // Authority stays registered until the real sockets were dropped.
        drop(registration);
        drop(access);
        Ok(())
    }
}

async fn serve_socket(
    connection: &Connection,
    id: &super::tcp_forward::TunnelId,
    destination: &super::tcp_forward::TcpDestination,
    target: SocketAddr,
    mut send: iroh::endpoint::SendStream,
    mut recv: iroh::endpoint::RecvStream,
    counters: Arc<super::tcp_registry::TcpCounters>,
    access: Arc<AccessHandle>,
    initial: Option<super::tcp_sockets::AuthorizedSocket>,
) -> io::Result<()> {
    let open: TcpDataOpen =
        super::route_protocol::timeout_for(Duration::from_secs(20), tcp_wire::recv_json(&mut recv))
            .await??;
    if &open.tunnel != id {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "TCP tunnel ID mismatch",
        ));
    }
    let socket =
        super::tcp_sockets::connect_bound_reusing(destination, target, access, initial).await;
    let mut socket = match socket {
        Ok(socket) => socket,
        Err(error) => {
            super::route_protocol::timeout_for(
                Duration::from_secs(20),
                tcp_wire::send_json(
                    &mut send,
                    &TcpDataReply::Refused {
                        detail: error.to_string(),
                    },
                ),
            )
            .await??;
            return Err(error);
        }
    };
    super::route_protocol::timeout_for(
        Duration::from_secs(20),
        tcp_wire::send_json(&mut send, &TcpDataReply::Connected),
    )
    .await??;
    super::tcp_pump::pump(connection, socket.stream(), &mut send, &mut recv, counters).await
}
/// Before every further data stream of a running tunnel to a loopback port
/// (review S5): the service listing is read again and every lease is
/// rechecked under the auth lock, so a port that changed owner or scope
/// since the tunnel opened is no longer reached through steering; a TCP
/// grant that names the port keeps it, as before.
async fn reverify_exposure(
    destination: &super::tcp_forward::TcpDestination,
    auth: &std::sync::Mutex<super::types::ShareAuthState>,
    leases: &super::access_revocation::AccessLeaseRegistry,
) {
    use super::access_constraints::TcpHost;
    let rdp = super::access_steering::own_rdp_ports().contains(&destination.port.get());
    if destination.host != TcpHost::Loopback || rdp {
        return;
    }
    refresh_exposed_ports(destination).await;
    if let Ok(state) = auth.lock() {
        leases.recheck(&state, super::core::now_secs());
    }
}

/// Steering exposes the local services of the last listing (design 4.2).
/// After a restart, or for a service started since then, that set is stale:
/// list once more before the decision. A failed listing exposes nothing
/// (`tcp_sockets::local_services`), and the admission below refuses with
/// its usual reason.
async fn refresh_exposed_ports(destination: &super::tcp_forward::TcpDestination) {
    use super::access_constraints::TcpHost;
    if destination.host == TcpHost::Loopback {
        let listing = tokio::task::spawn_blocking(super::tcp_sockets::local_services);
        let _ = listing.await;
    }
}

pub(crate) fn require_policy(connection: &Connection) -> io::Result<()> {
    if connection.application_path_policy().enabled {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "TCP requires the shared P28 scheduler policy",
        ))
    }
}
async fn revoked(access: &AccessHandle) {
    revocation_stop(&mut access.revoked()).await;
}
async fn revocation_stop(revoked: &mut watch::Receiver<Option<AccessRevocation>>) {
    loop {
        if revoked.borrow().is_some() {
            return;
        }
        if revoked.changed().await.is_err() {
            return;
        }
    }
}
pub(crate) async fn cancelled_stop(cancelled: &mut watch::Receiver<bool>) {
    loop {
        if *cancelled.borrow() {
            return;
        }
        if cancelled.changed().await.is_err() {
            return;
        }
    }
}
