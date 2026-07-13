use std::fmt;
use std::future::Future;
use std::io;
use std::time::Duration;

use iroh::endpoint::{Connection, VarInt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;

use super::exec_auth::build_client_hello;
use super::exec_client_active::TransportClosed;
use super::exec_heartbeat::{ExecHeartbeatPolicy, EXEC_HEARTBEAT_POLICY};
use super::exec_protocol::{
    recv_hello_result, recv_server_hello, send_client_frame, send_client_hello, ClientFrame,
    ExecHelloOk,
};
use super::exec_types::{ExecAuthorization, ExecId, ExecProviderStatus, ExecStart, ExecTerminal};
use super::identity::ShareIdentity;
use super::types::PeerEndpoint;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
const EVENT_TIMEOUT: Duration = Duration::from_secs(30);
const FAILURE_EVENT_TIMEOUT: Duration = Duration::from_secs(1);
const INPUT_CAPACITY: usize = 16;
const EVENT_CAPACITY: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ExecClientInput {
    Stdin(Vec<u8>),
    StdinEof,
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ExecClientEvent {
    Authorized {
        authorization: ExecAuthorization,
        provider: ExecProviderStatus,
    },
    Started,
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    Terminal(ExecTerminal),
    Failed(ExecClientFailure),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExecClientFailureKind {
    Authentication,
    Disconnected,
    Protocol,
    Remote,
    Local,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExecClientFailure {
    pub(crate) kind: ExecClientFailureKind,
    pub(crate) code: String,
    pub(crate) message: String,
    /// True once any part of Start may have reached the peer. Callers must
    /// never retry automatically when this is set.
    pub(crate) start_may_have_been_sent: bool,
}

impl fmt::Display for ExecClientFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ExecClientFailure {}

pub(crate) struct ExecClientSession {
    pub(crate) exec_id: ExecId,
    pub(crate) input: mpsc::Sender<ExecClientInput>,
    pub(crate) events: mpsc::Receiver<ExecClientEvent>,
    pub(crate) task: tokio::task::JoinHandle<Result<ExecTerminal, ExecClientFailure>>,
}

pub(crate) fn spawn_connected(
    connection: Connection,
    endpoint: PeerEndpoint,
    identity: ShareIdentity,
    start: ExecStart,
) -> ExecClientSession {
    let exec_id = start.exec_id.clone();
    let (input_tx, input_rx) = mpsc::channel(INPUT_CAPACITY);
    let (event_tx, event_rx) = mpsc::channel(EVENT_CAPACITY);
    let task = tokio::spawn(run_connected(
        connection, endpoint, identity, start, input_rx, event_tx,
    ));
    ExecClientSession {
        exec_id,
        input: input_tx,
        events: event_rx,
        task,
    }
}

pub(crate) async fn run_connected(
    connection: Connection,
    endpoint: PeerEndpoint,
    identity: ShareIdentity,
    start: ExecStart,
    input: mpsc::Receiver<ExecClientInput>,
    events: mpsc::Sender<ExecClientEvent>,
) -> Result<ExecTerminal, ExecClientFailure> {
    let result =
        run_connected_inner(connection, endpoint, identity, start, input, events.clone()).await;
    if let Err(error) = &result {
        let _ = tokio::time::timeout(
            FAILURE_EVENT_TIMEOUT,
            events.send(ExecClientEvent::Failed(error.clone())),
        )
        .await;
    }
    result
}

async fn run_connected_inner(
    connection: Connection,
    endpoint: PeerEndpoint,
    identity: ShareIdentity,
    start: ExecStart,
    input: mpsc::Receiver<ExecClientInput>,
    events: mpsc::Sender<ExecClientEvent>,
) -> Result<ExecTerminal, ExecClientFailure> {
    let remote = connection.remote_id().to_string();
    if remote != endpoint.presence.node_id {
        return Err(failure(
            ExecClientFailureKind::Authentication,
            "peer_identity_mismatch",
            "connected Iroh identity does not match the pinned Exec target",
            false,
        ));
    }
    // The server owns the challenge and therefore opens the single handshake
    // stream. If both sides call open_bi() and wait for the peer's first frame,
    // QUIC never exposes either stream because an empty local stream is not
    // announced on the wire.
    let (send, recv) = tokio::time::timeout(HANDSHAKE_TIMEOUT, connection.accept_bi())
        .await
        .map_err(|_| handshake_timeout(false))?
        .map_err(|error| disconnected(error, false))?;
    let close_watch = connection.clone();
    let result = run_stream_with_transport(
        send,
        recv,
        endpoint,
        identity,
        start,
        input,
        events,
        EXEC_HEARTBEAT_POLICY,
        Box::pin(async move {
            let _ = close_watch.closed().await;
        }),
    )
    .await;
    connection.close(VarInt::from_u32(0x4558), b"Exec session completed");
    result
}

#[cfg(test)]
async fn run_stream<W, R>(
    send: W,
    recv: R,
    endpoint: PeerEndpoint,
    identity: ShareIdentity,
    start: ExecStart,
    input: mpsc::Receiver<ExecClientInput>,
    events: mpsc::Sender<ExecClientEvent>,
) -> Result<ExecTerminal, ExecClientFailure>
where
    W: AsyncWrite + Unpin,
    R: AsyncRead + Unpin + Send + 'static,
{
    run_stream_with_policy(
        send,
        recv,
        endpoint,
        identity,
        start,
        input,
        events,
        EXEC_HEARTBEAT_POLICY,
    )
    .await
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
async fn run_stream_with_policy<W, R>(
    send: W,
    recv: R,
    endpoint: PeerEndpoint,
    identity: ShareIdentity,
    start: ExecStart,
    input: mpsc::Receiver<ExecClientInput>,
    events: mpsc::Sender<ExecClientEvent>,
    heartbeat_policy: ExecHeartbeatPolicy,
) -> Result<ExecTerminal, ExecClientFailure>
where
    W: AsyncWrite + Unpin,
    R: AsyncRead + Unpin + Send + 'static,
{
    run_stream_with_transport(
        send,
        recv,
        endpoint,
        identity,
        start,
        input,
        events,
        heartbeat_policy,
        Box::pin(std::future::pending()),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn run_stream_with_transport<W, R>(
    mut send: W,
    mut recv: R,
    endpoint: PeerEndpoint,
    identity: ShareIdentity,
    start: ExecStart,
    input: mpsc::Receiver<ExecClientInput>,
    events: mpsc::Sender<ExecClientEvent>,
    heartbeat_policy: ExecHeartbeatPolicy,
    transport_closed: TransportClosed,
) -> Result<ExecTerminal, ExecClientFailure>
where
    W: AsyncWrite + Unpin,
    R: AsyncRead + Unpin + Send + 'static,
{
    let hello = handshake(&mut send, &mut recv, &endpoint, &identity).await?;
    emit(
        &events,
        ExecClientEvent::Authorized {
            authorization: hello.authorization,
            provider: hello.provider.clone(),
        },
        false,
    )
    .await?;
    if !hello.provider.available {
        return Err(failure(
            ExecClientFailureKind::Remote,
            "provider_unavailable",
            &hello.provider.detail,
            false,
        ));
    }

    let exec_id = start.exec_id.clone();
    let digest = start
        .digest()
        .map_err(|error| protocol(error, false, "invalid_start"))?;
    // This flag is set before the only Start write. Even a partial write is an
    // ambiguous handoff and must never be retried by this client.
    let start_sent = true;
    write_frame_with_timeout(
        &mut send,
        &ClientFrame::Start { start, digest },
        start_sent,
        heartbeat_policy.write_timeout,
    )
    .await?;
    super::exec_client_active::run(
        send,
        recv,
        input,
        events,
        exec_id,
        heartbeat_policy,
        transport_closed,
    )
    .await
}

async fn handshake<W: AsyncWrite + Unpin, R: AsyncRead + Unpin>(
    send: &mut W,
    recv: &mut R,
    endpoint: &PeerEndpoint,
    identity: &ShareIdentity,
) -> Result<ExecHelloOk, ExecClientFailure> {
    let deadline = tokio::time::Instant::now() + HANDSHAKE_TIMEOUT;
    let server = until(deadline, recv_server_hello(recv))
        .await
        .map_err(|error| classify_io(error, false, "server_hello"))?;
    let client = build_client_hello(&server, endpoint, identity)
        .map_err(|error| authentication(error, "server_identity", false))?;
    until(deadline, send_client_hello(send, &client))
        .await
        .map_err(|error| classify_io(error, false, "client_hello"))?;
    match until(deadline, recv_hello_result(recv))
        .await
        .map_err(|error| classify_io(error, false, "hello_result"))?
    {
        Ok(hello) if !hello.authorization.session_id.trim().is_empty() => Ok(hello),
        Ok(_) => Err(protocol("empty Exec session id", false, "invalid_hello_ok")),
        Err(error) => Err(failure(
            ExecClientFailureKind::Authentication,
            &error.code,
            &error.message,
            false,
        )),
    }
}

pub(super) async fn write_frame_with_timeout<W: AsyncWrite + Unpin>(
    send: &mut W,
    frame: &ClientFrame,
    start_sent: bool,
    timeout: Duration,
) -> Result<(), ExecClientFailure> {
    tokio::time::timeout(timeout, send_client_frame(send, frame))
        .await
        .map_err(|_| {
            failure(
                ExecClientFailureKind::Disconnected,
                "write_timeout",
                "Exec peer stopped accepting input",
                start_sent,
            )
        })?
        .map_err(|error| classify_io(error, start_sent, "client_frame"))
}

pub(super) async fn emit(
    events: &mpsc::Sender<ExecClientEvent>,
    event: ExecClientEvent,
    start_sent: bool,
) -> Result<(), ExecClientFailure> {
    emit_with_timeout(events, event, start_sent, EVENT_TIMEOUT).await
}

pub(super) async fn emit_with_timeout(
    events: &mpsc::Sender<ExecClientEvent>,
    event: ExecClientEvent,
    start_sent: bool,
    timeout: Duration,
) -> Result<(), ExecClientFailure> {
    tokio::time::timeout(timeout, events.send(event))
        .await
        .map_err(|_| {
            failure(
                ExecClientFailureKind::Local,
                "event_backpressure",
                "local Exec event consumer stalled",
                start_sent,
            )
        })?
        .map_err(|_| {
            failure(
                ExecClientFailureKind::Local,
                "event_closed",
                "local Exec event consumer closed",
                start_sent,
            )
        })
}

pub(super) async fn emit_or_cancel<W: AsyncWrite + Unpin>(
    events: &mpsc::Sender<ExecClientEvent>,
    event: ExecClientEvent,
    send: &mut W,
    exec_id: &ExecId,
    start_sent: bool,
    peer_deadline: tokio::time::Instant,
) -> Result<(), ExecClientFailure> {
    let remaining = peer_deadline.saturating_duration_since(tokio::time::Instant::now());
    if remaining.is_zero() {
        return Err(failure(
            ExecClientFailureKind::Disconnected,
            "peer_unresponsive",
            "Exec peer stopped answering authenticated heartbeats",
            start_sent,
        ));
    }
    if let Err(error) =
        emit_with_timeout(events, event, start_sent, remaining.min(EVENT_TIMEOUT)).await
    {
        let cancel_timeout = peer_deadline
            .saturating_duration_since(tokio::time::Instant::now())
            .min(EXEC_HEARTBEAT_POLICY.write_timeout);
        if !cancel_timeout.is_zero() {
            let _ = write_frame_with_timeout(
                send,
                &ClientFrame::Cancel {
                    exec_id: exec_id.clone(),
                },
                start_sent,
                cancel_timeout,
            )
            .await;
        }
        return Err(error);
    }
    Ok(())
}

async fn until<T>(
    deadline: tokio::time::Instant,
    future: impl Future<Output = io::Result<T>>,
) -> io::Result<T> {
    tokio::time::timeout_at(deadline, future)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Exec handshake timed out"))?
}

fn authentication(error: impl fmt::Display, code: &str, start_sent: bool) -> ExecClientFailure {
    failure(
        ExecClientFailureKind::Authentication,
        code,
        &error.to_string(),
        start_sent,
    )
}

fn disconnected(error: impl fmt::Display, start_sent: bool) -> ExecClientFailure {
    failure(
        ExecClientFailureKind::Disconnected,
        "disconnected",
        &error.to_string(),
        start_sent,
    )
}

fn handshake_timeout(start_sent: bool) -> ExecClientFailure {
    failure(
        ExecClientFailureKind::Disconnected,
        "handshake_timeout",
        "Exec peer handshake timed out",
        start_sent,
    )
}

pub(super) fn protocol(
    error: impl fmt::Display,
    start_sent: bool,
    code: &str,
) -> ExecClientFailure {
    failure(
        ExecClientFailureKind::Protocol,
        code,
        &error.to_string(),
        start_sent,
    )
}

pub(super) fn classify_io(error: io::Error, start_sent: bool, code: &str) -> ExecClientFailure {
    match error.kind() {
        io::ErrorKind::InvalidData | io::ErrorKind::InvalidInput => {
            protocol(error, start_sent, code)
        }
        io::ErrorKind::PermissionDenied => authentication(error, code, start_sent),
        _ => disconnected(error, start_sent),
    }
}

pub(super) fn failure(
    kind: ExecClientFailureKind,
    code: &str,
    message: &str,
    start_may_have_been_sent: bool,
) -> ExecClientFailure {
    ExecClientFailure {
        kind,
        code: code.to_string(),
        message: message.to_string(),
        start_may_have_been_sent,
    }
}

#[cfg(test)]
#[path = "exec_client_tests.rs"]
mod tests;
