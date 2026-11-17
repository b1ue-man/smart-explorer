//! Outgoing Exec session setup on `ShareIrohNode`: pooled `share-exec/3`
//! stream acquisition, singleflight connection establishment, transparent `/2`
//! fallback with per-peer memory, and stream-limit overflow.
use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::FutureExt;
use iroh::endpoint::{Connection, VarInt};

use super::pool::ExecPoolKey;
use super::{exec_reuse_disabled, interrupted_connect, remaining_timeout, wait_cancelled};
use crate::share::exec_client::v3::{
    handshake_v3_connection, spawn_failed_session, spawn_v3_job, FallbackOrV3, ReconnectFn,
    V3StreamAttempt,
};
use crate::share::exec_client::ExecClientFailure;
use crate::share::exec_protocol::EXEC_ALPN;
use crate::share::identity::ShareIdentity;
use crate::share::node::ShareIrohNode;
use crate::share::route_cancellation::RouteCancellation;
use crate::share::types::PeerEndpoint;

enum PooledReuse {
    Ready(V3StreamAttempt),
    None,
    Saturated,
}

enum PreparedExec {
    V3(V3StreamAttempt),
    LegacyV2(Connection),
    HandshakeFailed(ExecClientFailure),
}

impl ShareIrohNode {
    pub(in crate::share) fn start_exec_cancellable(
        self: &Arc<Self>,
        endpoint: PeerEndpoint,
        identity: ShareIdentity,
        start: crate::share::exec_types::ExecStart,
        retainer: Option<Arc<dyn Send + Sync>>,
        cancellation: Option<Arc<RouteCancellation>>,
        deadline: Option<Instant>,
    ) -> io::Result<crate::share::exec_session::ShareExecSession> {
        self.require_sharing_active()?;
        if let Some(cancellation) = &cancellation {
            cancellation.check()?;
        }
        if exec_reuse_disabled() || self.exec_pool.is_legacy_v2(&endpoint.presence.node_id) {
            let connection = self.block_on(self.connect_exec_async(
                &endpoint,
                cancellation.as_ref(),
                deadline,
                false,
                true,
            ))?;
            return self.finish_legacy_v2_start(
                connection,
                endpoint,
                identity,
                start,
                retainer,
                cancellation,
                deadline,
            );
        }
        let key = ExecPoolKey::of(&identity, &endpoint);
        let prepared = self.block_on(self.acquire_exec_async(
            &key,
            &endpoint,
            &identity,
            cancellation.as_ref(),
            deadline,
        ))?;
        match prepared {
            PreparedExec::LegacyV2(connection) => self.finish_legacy_v2_start(
                connection,
                endpoint,
                identity,
                start,
                retainer,
                cancellation,
                deadline,
            ),
            PreparedExec::HandshakeFailed(failure) => {
                let retainer = wrap_exec_retainer(&start, retainer);
                let _runtime = self.rt.enter();
                let client = spawn_failed_session(start.exec_id, failure, retainer);
                Ok(crate::share::exec_session::ShareExecSession::new(
                    self.clone(),
                    client,
                ))
            }
            PreparedExec::V3(attempt) => {
                if let Some(limit) = deadline {
                    crate::share::io_deadline::remaining(limit, "Exec-Verbindungsaufbau")?;
                }
                if let Some(cancellation) = &cancellation {
                    cancellation.check()?;
                }
                let retainer = wrap_exec_retainer(&start, retainer);
                let retry_node = Arc::clone(self);
                let retry_endpoint = endpoint.clone();
                let retry_identity = identity.clone();
                let retry_cancel = cancellation.clone();
                let reconnect: ReconnectFn = Box::new(move || {
                    Box::pin(async move {
                        match retry_node
                            .acquire_exec_async(
                                &key,
                                &retry_endpoint,
                                &retry_identity,
                                retry_cancel.as_ref(),
                                deadline,
                            )
                            .await
                        {
                            Ok(PreparedExec::V3(fresh)) => Ok(FallbackOrV3::V3(fresh)),
                            Ok(PreparedExec::LegacyV2(conn)) => Ok(FallbackOrV3::LegacyV2(conn)),
                            Ok(PreparedExec::HandshakeFailed(err)) => Err(err),
                            Err(err) => {
                                Err(crate::share::exec_client_messages::disconnected(err, false))
                            }
                        }
                    })
                });
                let _runtime = self.rt.enter();
                let client = spawn_v3_job(
                    attempt,
                    endpoint,
                    identity,
                    start,
                    retainer,
                    cancellation,
                    reconnect,
                );
                Ok(crate::share::exec_session::ShareExecSession::new(
                    self.clone(),
                    client,
                ))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_legacy_v2_start(
        self: &Arc<Self>,
        connection: Connection,
        endpoint: PeerEndpoint,
        identity: ShareIdentity,
        start: crate::share::exec_types::ExecStart,
        retainer: Option<Arc<dyn Send + Sync>>,
        cancellation: Option<Arc<RouteCancellation>>,
        deadline: Option<Instant>,
    ) -> io::Result<crate::share::exec_session::ShareExecSession> {
        if let Some(limit) = deadline {
            if let Err(error) =
                crate::share::io_deadline::remaining(limit, "Exec-Verbindungsaufbau")
            {
                connection.close(VarInt::from_u32(0x4558), b"Exec setup deadline expired");
                return Err(error);
            }
        }
        if let Some(cancellation) = &cancellation {
            cancellation.check()?;
        }
        let retainer = wrap_exec_retainer(&start, retainer);
        let _runtime = self.rt.enter();
        let client = crate::share::exec_client::spawn_connected_cancellable(
            connection,
            endpoint,
            identity,
            start,
            retainer,
            cancellation,
        );
        Ok(crate::share::exec_session::ShareExecSession::new(
            self.clone(),
            client,
        ))
    }

    async fn try_open_pooled_v3(
        &self,
        key: &ExecPoolKey,
        deadline: Option<Instant>,
    ) -> PooledReuse {
        let Some(lease) = self.exec_pool.try_lease_existing(key) else {
            return PooledReuse::None;
        };
        match lease.connection().open_bi().now_or_never() {
            Some(Ok((send, recv))) => PooledReuse::Ready(V3StreamAttempt {
                lease,
                send,
                recv,
                was_reused: true,
            }),
            Some(Err(_)) => {
                lease.evict();
                PooledReuse::None
            }
            None if lease.is_sole_owner() => {
                let wait = remaining_timeout(deadline).unwrap_or(Duration::from_secs(5));
                match tokio::time::timeout(wait, lease.connection().open_bi()).await {
                    Ok(Ok((send, recv))) => PooledReuse::Ready(V3StreamAttempt {
                        lease,
                        send,
                        recv,
                        was_reused: true,
                    }),
                    _ => {
                        lease.evict_and_close_if_sole();
                        PooledReuse::None
                    }
                }
            }
            None => PooledReuse::Saturated,
        }
    }

    async fn acquire_exec_async(
        &self,
        key: &ExecPoolKey,
        endpoint: &PeerEndpoint,
        identity: &ShareIdentity,
        cancellation: Option<&Arc<RouteCancellation>>,
        deadline: Option<Instant>,
    ) -> io::Result<PreparedExec> {
        match self.try_open_pooled_v3(key, deadline).await {
            PooledReuse::Ready(attempt) => return Ok(PreparedExec::V3(attempt)),
            PooledReuse::Saturated => {
                return self
                    .dial_exec_async(key, endpoint, identity, cancellation, deadline, false)
                    .await;
            }
            PooledReuse::None => {}
        }
        let gate = self.exec_pool.connect_gate(key);
        let timeout = remaining_timeout(deadline)?;
        let guard = tokio::select! {
            biased;
            () = wait_cancelled(cancellation) => return Err(interrupted_connect()),
            locked = tokio::time::timeout(timeout, gate.lock()) => {
                locked.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Exec-Verbindungsaufbau Timeout"))?
            }
        };
        if self.exec_pool.is_legacy_v2(&endpoint.presence.node_id) {
            drop(guard);
            let conn = self
                .connect_exec_async(endpoint, cancellation, deadline, false, true)
                .await?;
            return Ok(PreparedExec::LegacyV2(conn));
        }
        match self.try_open_pooled_v3(key, deadline).await {
            PooledReuse::Ready(attempt) => Ok(PreparedExec::V3(attempt)),
            PooledReuse::Saturated => {
                drop(guard);
                self.dial_exec_async(key, endpoint, identity, cancellation, deadline, false)
                    .await
            }
            PooledReuse::None => {
                self.dial_exec_async(key, endpoint, identity, cancellation, deadline, true)
                    .await
            }
        }
    }

    async fn dial_exec_async(
        &self,
        key: &ExecPoolKey,
        endpoint: &PeerEndpoint,
        identity: &ShareIdentity,
        cancellation: Option<&Arc<RouteCancellation>>,
        deadline: Option<Instant>,
        pool_entry: bool,
    ) -> io::Result<PreparedExec> {
        let connection = self
            .connect_exec_async(endpoint, cancellation, deadline, true, false)
            .await?;
        if connection.alpn() == EXEC_ALPN {
            self.exec_pool
                .mark_legacy_v2(endpoint.presence.node_id.clone());
            if let Some(cancellation) = cancellation {
                cancellation.bind(&connection)?;
            }
            return Ok(PreparedExec::LegacyV2(connection));
        }
        let handshook = tokio::select! {
            biased;
            () = wait_cancelled(cancellation) => {
                connection.close(VarInt::from_u32(0x4558), b"Exec setup cancelled");
                return Err(interrupted_connect());
            }
            res = handshake_v3_connection(&connection, endpoint, identity) => res,
        };
        let info = match handshook {
            Ok(info) => info,
            Err(failure) => return Ok(PreparedExec::HandshakeFailed(failure)),
        };
        let lease = self
            .exec_pool
            .insert_and_lease(key.clone(), connection, info, pool_entry);
        let timeout = remaining_timeout(deadline)?;
        let opened = tokio::select! {
            biased;
            () = wait_cancelled(cancellation) => {
                lease.evict_and_close_if_sole();
                return Err(interrupted_connect());
            }
            res = tokio::time::timeout(timeout, lease.connection().open_bi()) => res,
        };
        match opened {
            Ok(Ok((send, recv))) => Ok(PreparedExec::V3(V3StreamAttempt {
                lease,
                send,
                recv,
                was_reused: false,
            })),
            Ok(Err(error)) => {
                lease.evict_and_close_if_sole();
                Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    error.to_string(),
                ))
            }
            Err(_) => {
                lease.evict_and_close_if_sole();
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Exec-Stromaufbau Timeout",
                ))
            }
        }
    }
}

fn wrap_exec_retainer(
    start: &crate::share::exec_types::ExecStart,
    retainer: Option<Arc<dyn Send + Sync>>,
) -> Option<Arc<dyn Send + Sync>> {
    if start.terminal.is_some() {
        use crate::share::keepalive::interactive_presence::{retain_with, InteractiveKind};
        Some(retain_with(InteractiveKind::ExecTerminal, retainer))
    } else {
        retainer
    }
}
