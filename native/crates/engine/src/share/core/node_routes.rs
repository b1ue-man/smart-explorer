//! Common admission for FS, Exec/terminal, Screen and the existing LAN-link actor.
use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iroh::endpoint::{Connection, VarInt};

use super::node::ShareIrohNode;
use super::route_cancellation::RouteCancellation;
use super::types::PeerEndpoint;

#[path = "node_routes_exec.rs"]
mod exec;
#[path = "exec_client_pool.rs"]
pub(in crate::share) mod pool;
#[cfg(test)]
#[path = "exec_pool_task_tests.rs"]
mod pool_tests;
#[cfg(test)]
#[path = "exec_loopback_fixture.rs"]
pub(in crate::share) mod test_fixture;

pub(super) fn exec_reuse_disabled() -> bool {
    #[cfg(any(test, feature = "test-support"))]
    {
        super::keepalive::transport_purpose::overrides::current().exec_reuse == Some(false)
    }
    #[cfg(not(any(test, feature = "test-support")))]
    {
        false
    }
}

pub(super) fn remaining_timeout(deadline: Option<Instant>) -> io::Result<Duration> {
    Ok(deadline
        .map(|limit| super::io_deadline::remaining(limit, "Exec-Verbindungsaufbau"))
        .transpose()?
        .unwrap_or(super::io_deadline::PEER_OP_TIMEOUT)
        .min(super::io_deadline::PEER_OP_TIMEOUT))
}

pub(super) async fn wait_cancelled(cancellation: Option<&Arc<RouteCancellation>>) {
    match cancellation {
        Some(rc) => rc.cancelled().await,
        None => std::future::pending::<()>().await,
    }
}

pub(super) fn interrupted_connect() -> io::Error {
    io::Error::new(
        io::ErrorKind::Interrupted,
        "Exec-Verbindungsaufbau abgebrochen",
    )
}

impl ShareIrohNode {
    pub(super) fn block_on_route<F: std::future::Future>(
        &self,
        generation: usize,
        future: F,
    ) -> F::Output {
        if let Some(connection) = super::route_monitor::connection(generation) {
            self.block_on(super::route_protocol::scope(connection, future))
        } else {
            self.block_on(future)
        }
    }

    pub(super) fn connect_exec_cancellable(
        &self,
        endpoint: &PeerEndpoint,
        cancellation: Option<&Arc<RouteCancellation>>,
        deadline: Option<Instant>,
    ) -> io::Result<Connection> {
        let offer_v3 =
            !exec_reuse_disabled() && !self.exec_pool.is_legacy_v2(&endpoint.presence.node_id);
        self.block_on(self.connect_exec_async(endpoint, cancellation, deadline, offer_v3, true))
    }

    pub(super) async fn connect_exec_async(
        &self,
        endpoint: &PeerEndpoint,
        cancellation: Option<&Arc<RouteCancellation>>,
        deadline: Option<Instant>,
        offer_v3: bool,
        bind_cancellation: bool,
    ) -> io::Result<Connection> {
        self.require_sharing_active()?;
        if let Some(cancellation) = cancellation {
            cancellation.check()?;
        }
        if endpoint
            .expected_node_id
            .as_deref()
            .is_some_and(|expected| {
                !expected.trim().is_empty() && expected != endpoint.presence.node_id
            })
        {
            return Err(io::Error::other(
                "Iroh NodeId passt nicht zur gepinnten Identitaet",
            ));
        }
        let addr = self.dial_addr(endpoint).await?;
        let generation = self
            .policy
            .snapshot(super::session::PeerPrincipal::from_endpoint(endpoint))?;
        let timeout = remaining_timeout(deadline)?;
        let connection = tokio::select! {
            biased;
            () = wait_cancelled(cancellation) => Err(interrupted_connect()),
            result = super::io_deadline::run_for("peer exec connection", timeout, async {
                super::keepalive::transport_purpose::connect_exec(&self.endpoint, addr, offer_v3)
                    .await
                    .map_err(super::io_deadline::disconnected)
            }) => result,
        }?;
        if bind_cancellation {
            if let Some(cancellation) = cancellation {
                cancellation.bind(&connection)?;
            }
        }
        if let Err(error) = self.policy.bind(&connection, generation) {
            connection.close(
                VarInt::from_u32(0x5345),
                b"authorization changed during handshake",
            );
            return Err(error);
        }
        let routed = tokio::select! {
            biased;
            () = wait_cancelled(cancellation) => Err(interrupted_connect()),
            result = self.await_route(&connection, Some(endpoint), deadline) => result,
        };
        if let Err(error) = routed {
            connection.close(VarInt::from_u32(0x4558), b"Exec route admission failed");
            return Err(error);
        }
        if let Some(cancellation) = cancellation {
            if let Err(error) = cancellation.check() {
                connection.close(VarInt::from_u32(0x4558), b"Exec setup cancelled");
                return Err(error);
            }
        }
        Ok(connection)
    }

    pub(super) fn monitor_route(
        &self,
        connection: &Connection,
        endpoint: Option<&PeerEndpoint>,
    ) -> io::Result<()> {
        self.require_sharing_active()?;
        let remote = connection.remote_id().to_string();
        let known = self.known_endpoint(&remote);
        let auth = self
            .auth
            .lock()
            .map_err(|_| io::Error::other("Share-Auth gesperrt"))?;
        let support = endpoint.is_some_and(|endpoint| endpoint.presence.supports_route_policy())
            || auth
                .direct_contacts
                .iter()
                .filter_map(|contact| contact.presence.as_ref())
                .chain(auth.rooms.iter().flat_map(|room| {
                    room.members
                        .iter()
                        .filter_map(|member| member.presence.as_ref())
                }))
                .any(|presence| {
                    presence.node_id == remote
                        && presence.is_current_at(super::core::now_secs())
                        && presence.supports_route_policy()
                });
        let hints = endpoint
            .map(|endpoint| {
                endpoint
                    .presence
                    .candidates
                    .iter()
                    .take(32)
                    .filter_map(|hint| crate::net::parse_candidate(hint))
                    .collect()
            })
            .unwrap_or_default();
        let local = auth.identity.node_id.clone();
        drop(auth);
        // Keeps trying the direct path while the relay is the fallback.
        let retry = Some(self.direct_retry_port());
        super::route_monitor::start_with_retry(connection, local, known, support, hints, retry)
    }

    pub(super) async fn await_route(
        &self,
        connection: &Connection,
        endpoint: Option<&PeerEndpoint>,
        deadline: Option<Instant>,
    ) -> io::Result<()> {
        self.monitor_route(connection, endpoint)?;
        super::route_monitor::wait_application_route(connection, deadline).await?;
        self.require_sharing_active()
    }
}
