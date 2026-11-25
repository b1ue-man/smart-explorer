//! Client side of `share-exec/3` multiplexed Exec connections (plan
//! 2026-11-14 E10, D4 B3): Stream 0 connection handshake and per-job session
//! spawning with at-most-once retry before the first job byte.
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;

use iroh::endpoint::{Connection, RecvStream, SendStream, VarInt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, watch};

use super::{
    run_connected, until, ExecClientEvent, ExecClientFailure, ExecClientFailureKind,
    ExecClientInput, ExecClientSession, ExecInputSender, EVENT_CAPACITY, FAILURE_EVENT_TIMEOUT,
    HANDSHAKE_TIMEOUT, INPUT_CAPACITY,
};
use crate::share::exec_access_view::{ExecClientNotice, ReceivedHelloOk};
use crate::share::exec_auth::build_client_hello_for;
use crate::share::exec_client_messages::{
    authentication, classify_io, client_failure_message, disconnected, failure, peer_name, protocol,
};
use crate::share::exec_protocol::{recv_hello_result, recv_server_hello, send_client_hello};
use crate::share::exec_types::{
    ExecId, ExecProviderStatus, ExecStart, ExecTerminal, EXEC_PROTOCOL_VERSION_V3,
};
use crate::share::identity::ShareIdentity;
use crate::share::node_routes::pool::PoolLeaseGuard;
use crate::share::route_cancellation::RouteCancellation;
use crate::share::types::PeerEndpoint;

#[path = "exec_client_v3_stream.rs"]
mod stream;

#[derive(Clone, Debug)]
pub(in crate::share) struct V3ConnectionInfo {
    pub(in crate::share) provider: ExecProviderStatus,
    pub(in crate::share) features: Vec<String>,
}

pub(in crate::share) struct V3StreamAttempt {
    pub(in crate::share) lease: PoolLeaseGuard,
    pub(in crate::share) send: SendStream,
    pub(in crate::share) recv: RecvStream,
    pub(in crate::share) was_reused: bool,
}

pub(in crate::share) enum FallbackOrV3 {
    V3(V3StreamAttempt),
    LegacyV2(Connection),
}

pub(in crate::share) type ReconnectFuture =
    Pin<Box<dyn Future<Output = Result<FallbackOrV3, ExecClientFailure>> + Send>>;
pub(in crate::share) type ReconnectFn = Box<dyn FnOnce() -> ReconnectFuture + Send>;

pub(super) async fn handshake_version<W: AsyncWrite + Unpin, R: AsyncRead + Unpin>(
    version: u16,
    send: &mut W,
    recv: &mut R,
    endpoint: &PeerEndpoint,
    identity: &ShareIdentity,
) -> Result<ReceivedHelloOk, ExecClientFailure> {
    let deadline = crate::share::route_protocol::active_now() + HANDSHAKE_TIMEOUT;
    let server = until(deadline, recv_server_hello(recv))
        .await
        .map_err(|error| classify_io(error, false, "server_hello"))?;
    let mut client = build_client_hello_for(version, &server, endpoint, identity)
        .map_err(|error| authentication(error, "server_identity", false))?;
    client.client_features.push("route_policy_v1".into());
    until(deadline, send_client_hello(send, &client))
        .await
        .map_err(|error| classify_io(error, false, "client_hello"))?;
    match until(deadline, recv_hello_result::<_, ReceivedHelloOk>(recv))
        .await
        .map_err(|error| classify_io(error, false, "hello_result"))?
    {
        Ok(hello) if !hello.hello.authorization.session_id.trim().is_empty() => Ok(hello),
        Ok(_) => Err(protocol("empty Exec session id", false, "invalid_hello_ok")),
        Err(error) => Err(failure(
            ExecClientFailureKind::Authentication,
            &error.code,
            &client_failure_message(&error.code, &error.message, &peer_name(endpoint)),
            false,
        )),
    }
}

/// Runs the one-time Stream 0 connection authentication of `share-exec/3`.
pub(in crate::share) async fn handshake_v3_connection(
    connection: &Connection,
    endpoint: &PeerEndpoint,
    identity: &ShareIdentity,
) -> Result<V3ConnectionInfo, ExecClientFailure> {
    if connection.remote_id().to_string() != endpoint.presence.node_id {
        connection.close(VarInt::from_u32(0x4558), b"Exec peer mismatch");
        return Err(failure(
            ExecClientFailureKind::Authentication,
            "peer_identity_mismatch",
            "connected Iroh identity does not match the pinned Exec target",
            false,
        ));
    }
    let (mut send, mut recv) =
        crate::share::route_protocol::run_with_route(connection, HANDSHAKE_TIMEOUT, None, async {
            connection.accept_bi().await.map_err(io::Error::other)
        })
        .await
        .map_err(|error| {
            connection.close(VarInt::from_u32(0x4558), b"Exec stream 0 failed");
            disconnected(error, false)
        })?;
    let ReceivedHelloOk { hello, .. } = match crate::share::route_protocol::scope(
        connection.clone(),
        handshake_version(
            EXEC_PROTOCOL_VERSION_V3,
            &mut send,
            &mut recv,
            endpoint,
            identity,
        ),
    )
    .await
    {
        Ok(received) => received,
        Err(error) => {
            connection.close(VarInt::from_u32(0x4558), b"Exec handshake rejected");
            return Err(error);
        }
    };
    let _ = send.finish();
    crate::share::route_monitor::authenticated_remote_capability(
        connection,
        hello
            .features
            .iter()
            .any(|feature| feature == "route_policy_v1"),
    );
    if !hello.provider.available {
        connection.close(VarInt::from_u32(0x4558), b"Exec provider unavailable");
        return Err(failure(
            ExecClientFailureKind::Remote,
            "provider_unavailable",
            &hello.provider.detail,
            false,
        ));
    }
    Ok(V3ConnectionInfo {
        provider: hello.provider,
        features: hello.features,
    })
}

pub(in crate::share) fn spawn_failed_session(
    exec_id: ExecId,
    error: ExecClientFailure,
    retainer: Option<Arc<dyn Send + Sync>>,
) -> ExecClientSession {
    let (input_tx, _input_rx) = mpsc::channel(INPUT_CAPACITY);
    let (cancel_tx, _cancel_rx) = watch::channel(false);
    let (event_tx, event_rx) = mpsc::channel(EVENT_CAPACITY);
    let (_notice_tx, notice_rx) = watch::channel(ExecClientNotice::default());
    let task = tokio::spawn(async move {
        let _retainer = retainer;
        let _ = tokio::time::timeout(
            FAILURE_EVENT_TIMEOUT,
            event_tx.send(ExecClientEvent::Failed(error.clone())),
        )
        .await;
        Err(error)
    });
    ExecClientSession {
        exec_id,
        input: ExecInputSender::new(input_tx, cancel_tx),
        events: event_rx,
        task,
        notice: notice_rx,
    }
}

pub(in crate::share) fn spawn_v3_job(
    attempt: V3StreamAttempt,
    endpoint: PeerEndpoint,
    identity: ShareIdentity,
    start: ExecStart,
    retainer: Option<Arc<dyn Send + Sync>>,
    cancellation: Option<Arc<RouteCancellation>>,
    reconnect: ReconnectFn,
) -> ExecClientSession {
    let exec_id = start.exec_id.clone();
    let (input_tx, input_rx) = mpsc::channel(INPUT_CAPACITY);
    let (cancel_tx, mut cancel_rx) = watch::channel(false);
    let (event_tx, event_rx) = mpsc::channel(EVENT_CAPACITY);
    let (notice_tx, notice_rx) = watch::channel(ExecClientNotice::default());
    let task = tokio::spawn(async move {
        let _retainer = retainer;
        let outcome = run_v3_job(
            attempt,
            endpoint,
            identity,
            start,
            input_rx,
            &mut cancel_rx,
            cancellation,
            event_tx.clone(),
            notice_tx,
            reconnect,
        )
        .await;
        if let Err(error) = &outcome {
            let _ = tokio::time::timeout(
                FAILURE_EVENT_TIMEOUT,
                event_tx.send(ExecClientEvent::Failed(error.clone())),
            )
            .await;
        }
        outcome
    });
    ExecClientSession {
        exec_id,
        input: ExecInputSender::new(input_tx, cancel_tx),
        events: event_rx,
        task,
        notice: notice_rx,
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_v3_job(
    attempt: V3StreamAttempt,
    endpoint: PeerEndpoint,
    identity: ShareIdentity,
    start: ExecStart,
    input_rx: mpsc::Receiver<ExecClientInput>,
    cancel_rx: &mut watch::Receiver<bool>,
    cancellation: Option<Arc<RouteCancellation>>,
    events: mpsc::Sender<ExecClientEvent>,
    notice: watch::Sender<ExecClientNotice>,
    reconnect: ReconnectFn,
) -> Result<ExecTerminal, ExecClientFailure> {
    // A host before shell selection gets the start without the choice; a
    // retry on a new connection to the same host keeps this start.
    let start = crate::share::exec_shell::for_host(start, &attempt.lease.info().features);
    let digest = start
        .digest()
        .map_err(|error| protocol(error, false, "invalid_start"))?;
    if events.is_closed() {
        return Err(failure(
            ExecClientFailureKind::Local,
            "session_closed",
            "local Exec session closed before Start",
            false,
        ));
    }
    let sent = match stream::send_start(
        attempt,
        &start,
        &digest,
        cancel_rx,
        cancellation.as_deref(),
    )
    .await
    {
        Ok(sent) => sent,
        Err((error, false)) => return Err(error),
        Err((_stale_error, true)) => {
            let fresh = tokio::select! {
                biased;
                () = stream::any_cancelled(cancel_rx, cancellation.as_deref()) => {
                    return Err(failure(
                        ExecClientFailureKind::Local,
                        "cancelled",
                        "Exec session cancelled before retry",
                        false,
                    ));
                }
                res = reconnect() => res?,
            };
            match fresh {
                FallbackOrV3::LegacyV2(connection) => {
                    return run_connected(
                        connection,
                        endpoint,
                        identity,
                        start,
                        input_rx,
                        cancel_rx.clone(),
                        events,
                        notice,
                    )
                    .await;
                }
                FallbackOrV3::V3(mut fresh_attempt) => {
                    fresh_attempt.was_reused = false;
                    stream::send_start(
                        fresh_attempt,
                        &start,
                        &digest,
                        cancel_rx,
                        cancellation.as_deref(),
                    )
                    .await
                    .map_err(|(err, _)| err)?
                }
            }
        }
    };
    stream::run_after_start(
        sent,
        endpoint,
        start,
        input_rx,
        cancel_rx.clone(),
        cancellation,
        events,
        notice,
    )
    .await
}
