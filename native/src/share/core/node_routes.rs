//! Common admission for FS, Exec/terminal, Screen and the existing LAN-link actor.
use std::{io, sync::Arc, time::Instant};
use iroh::endpoint::Connection;
use super::node::ShareIrohNode;
use super::types::PeerEndpoint;

impl ShareIrohNode {
    pub(super) fn block_on_route<F: std::future::Future>(&self, generation: usize, future: F) -> F::Output {
        if let Some(connection) = super::route_monitor::connection(generation) {
            self.block_on(super::route_protocol::scope(connection, future))
        } else { self.block_on(future) }
    }

    pub(super) fn connect_exec_cancellable(&self, endpoint: &PeerEndpoint,
        cancellation: Option<&Arc<super::route_cancellation::RouteCancellation>>, deadline: Option<Instant>) -> io::Result<Connection> {
        self.require_sharing_active()?;
        if let Some(cancellation) = cancellation { cancellation.check()?; }
        if endpoint.expected_node_id.as_deref().is_some_and(|expected|
            !expected.trim().is_empty() && expected != endpoint.presence.node_id) {
            return Err(io::Error::other("Iroh NodeId passt nicht zur gepinnten Identitaet"));
        }
        let local = self.routes.current(&self.endpoint);
        let addr = super::session::endpoint_addr(&endpoint.presence, &local, &self.transport_options)?;
        let generation = self.policy.snapshot(super::session::PeerPrincipal::from_endpoint(endpoint))?;
        let timeout = deadline.map(|deadline| super::io_deadline::remaining(deadline, "Exec-Verbindungsaufbau"))
            .transpose()?.unwrap_or(super::io_deadline::PEER_OP_TIMEOUT).min(super::io_deadline::PEER_OP_TIMEOUT);
        let connection = self.block_on(async {
            let cancelled = async {
                if let Some(cancellation) = cancellation { cancellation.cancelled().await; }
                else { std::future::pending::<()>().await; }
            };
            tokio::select! {
                biased;
                _ = cancelled => Err(io::Error::new(io::ErrorKind::Interrupted, "Exec-Verbindungsaufbau abgebrochen")),
                result = super::io_deadline::run_for("peer exec connection", timeout, async {
                    self.endpoint.connect(addr, super::exec_protocol::EXEC_ALPN).await.map_err(super::io_deadline::disconnected)
                }) => result,
            }
        })?;
        if let Some(cancellation) = cancellation { cancellation.bind(&connection)?; }
        if let Err(error) = self.policy.bind(&connection, generation) {
            connection.close(iroh::endpoint::VarInt::from_u32(0x5345), b"authorization changed during handshake");
            return Err(error);
        }
        let routed = self.block_on(async {
            let cancelled = async {
                if let Some(cancellation) = cancellation { cancellation.cancelled().await; }
                else { std::future::pending::<()>().await; }
            };
            tokio::select! {
                biased;
                _ = cancelled => Err(io::Error::new(io::ErrorKind::Interrupted, "Exec-Verbindungsaufbau abgebrochen")),
                result = self.await_route(&connection, Some(endpoint), deadline) => result,
            }
        });
        if let Err(error) = routed {
            connection.close(iroh::endpoint::VarInt::from_u32(0x4558), b"Exec route admission failed");
            return Err(error);
        }
        if let Some(cancellation) = cancellation { cancellation.check()?; }
        Ok(connection)
    }

    pub(super) fn start_exec_cancellable(self: &Arc<Self>, endpoint: PeerEndpoint,
        identity: super::identity::ShareIdentity, start: super::exec_types::ExecStart,
        retainer: Option<Arc<dyn Send + Sync>>,
        cancellation: Option<Arc<super::route_cancellation::RouteCancellation>>, deadline: Option<Instant>)
        -> io::Result<super::exec_session::ShareExecSession> {
        let connection = self.connect_exec_cancellable(&endpoint, cancellation.as_ref(), deadline)?;
        if let Some(deadline) = deadline {
            if let Err(error) = super::io_deadline::remaining(deadline, "Exec-Verbindungsaufbau") {
                connection.close(iroh::endpoint::VarInt::from_u32(0x4558), b"Exec setup deadline expired");
                return Err(error);
            }
        }
        if let Some(cancellation) = &cancellation { cancellation.check()?; }
        let _runtime = self.rt.enter();
        let client = super::exec_client::spawn_connected_cancellable(connection, endpoint, identity, start, retainer, cancellation);
        Ok(super::exec_session::ShareExecSession::new(self.clone(), client))
    }

    pub(super) fn monitor_route(&self, connection: &Connection, endpoint: Option<&PeerEndpoint>) -> io::Result<()> {
        self.require_sharing_active()?;
        let remote = connection.remote_id().to_string();
        let known = self.known_endpoint(&remote);
        let auth = self.auth.lock().map_err(|_| io::Error::other("Share-Auth gesperrt"))?;
        let support = endpoint.is_some_and(|endpoint| endpoint.presence.supports_route_policy()) ||
            auth.direct_contacts.iter().filter_map(|contact| contact.presence.as_ref())
                .chain(auth.rooms.iter().flat_map(|room| room.members.iter().filter_map(|member| member.presence.as_ref())))
                .any(|presence| presence.node_id == remote && presence.is_current_at(super::core::now_secs()) && presence.supports_route_policy());
        let hints = endpoint.map(|endpoint| endpoint.presence.candidates.iter()
            .take(32).filter_map(|hint| crate::net::parse_candidate(hint)).collect()).unwrap_or_default();
        super::route_monitor::start(connection, auth.identity.node_id.clone(), known, support, hints)
    }

    pub(super) async fn await_route(&self, connection: &Connection, endpoint: Option<&PeerEndpoint>,
        deadline: Option<Instant>) -> io::Result<()> {
        self.monitor_route(connection, endpoint)?;
        super::route_monitor::wait_application_route(connection, deadline).await?;
        self.require_sharing_active()
    }
}
