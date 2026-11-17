//! Per-job stream execution for `share-exec/3`: zero-byte write accounting
//! (`CountingWriter`), `Start` send with retry eligibility, per-job `HELLO_OK`
//! verification, active I/O delegation and stream-isolated cancellation.
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use iroh::endpoint::{Connection, RecvStream, SendStream};
use tokio::io::AsyncWrite;
use tokio::sync::{mpsc, watch};

use super::super::{
    emit, input, until, write_frame_with_timeout, ExecClientEvent, ExecClientFailure,
    ExecClientFailureKind, ExecClientInput, HANDSHAKE_TIMEOUT, INPUT_CAPACITY,
};
use super::V3StreamAttempt;
use crate::share::exec_access_view::{ExecClientNotice, ReceivedHelloOk};
use crate::share::exec_client_messages::{
    classify_io, client_failure_message, disconnected, failure, peer_name, protocol,
};
use crate::share::exec_heartbeat::EXEC_HEARTBEAT_POLICY;
use crate::share::exec_protocol::{recv_hello_result, ClientFrame};
use crate::share::exec_terminal::require_host_terminal;
use crate::share::exec_types::{ExecStart, ExecTerminal};
use crate::share::node_routes::pool::PoolLeaseGuard;
use crate::share::route_cancellation::RouteCancellation;
use crate::share::types::PeerEndpoint;

pub(super) struct SentV3Start {
    pub(super) lease: PoolLeaseGuard,
    pub(super) writer: CountingWriter<SendStream>,
    pub(super) recv: RecvStream,
}

pub(super) async fn send_start(
    attempt: V3StreamAttempt,
    start: &ExecStart,
    digest: &str,
    cancel_rx: &mut watch::Receiver<bool>,
    route_cancel: Option<&RouteCancellation>,
) -> Result<SentV3Start, (ExecClientFailure, bool)> {
    require_host_terminal(start, &attempt.lease.info().features).map_err(|err| (err, false))?;
    if attempt.lease.connection().close_reason().is_some() {
        attempt.lease.evict();
        return Err((
            disconnected("pooled Exec connection closed", false),
            attempt.was_reused,
        ));
    }
    let mut writer = CountingWriter {
        inner: attempt.send,
        bytes_written: 0,
    };
    let conn = attempt.lease.connection().clone();
    let frame = ClientFrame::Start {
        start: start.clone(),
        digest: digest.to_string(),
    };
    let write = crate::share::route_protocol::scope(
        conn,
        write_frame_with_timeout(
            &mut writer,
            &frame,
            false,
            EXEC_HEARTBEAT_POLICY.write_timeout,
        ),
    );
    let outcome = tokio::select! {
        biased;
        () = any_cancelled(cancel_rx, route_cancel) => None,
        res = write => Some(res),
    };
    let Some(outcome) = outcome else {
        let sent = writer.bytes_written > 0;
        return Err((
            failure(
                ExecClientFailureKind::Local,
                "cancelled",
                "Exec session cancelled before Start",
                sent,
            ),
            false,
        ));
    };
    match outcome {
        Ok(()) => Ok(SentV3Start {
            lease: attempt.lease,
            writer,
            recv: attempt.recv,
        }),
        Err(mut error) => {
            let sent = writer.bytes_written > 0;
            error.start_may_have_been_sent = sent;
            attempt.lease.evict_and_close_if_sole();
            let retryable = attempt.was_reused
                && !sent
                && matches!(error.kind, ExecClientFailureKind::Disconnected);
            Err((error, retryable))
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_after_start(
    sent: SentV3Start,
    endpoint: PeerEndpoint,
    start: ExecStart,
    input_rx: mpsc::Receiver<ExecClientInput>,
    mut cancel_rx: watch::Receiver<bool>,
    cancellation: Option<Arc<RouteCancellation>>,
    events: mpsc::Sender<ExecClientEvent>,
    notice: watch::Sender<ExecClientNotice>,
) -> Result<ExecTerminal, ExecClientFailure> {
    let SentV3Start {
        lease,
        mut writer,
        recv,
    } = sent;
    let conn = lease.connection().clone();
    let (operation_tx, operation_rx) = mpsc::channel(INPUT_CAPACITY);
    let result = {
        let forwarding = forward_v3(
            &conn,
            input_rx,
            cancel_rx.clone(),
            cancellation.clone(),
            operation_tx,
        );
        let active = crate::share::route_protocol::scope(
            conn.clone(),
            run_v3_stream_active(
                &mut writer,
                recv,
                &conn,
                &endpoint,
                &start,
                operation_rx,
                events.clone(),
                &notice,
            ),
        );
        tokio::pin!(active, forwarding);
        let abandoned = abandon_during_pause_v3(&lease, events.closed());
        let cancelled = abandon_during_pause_v3(
            &lease,
            any_cancelled(&mut cancel_rx, cancellation.as_deref()),
        );
        tokio::pin!(abandoned, cancelled);
        enum Exit {
            Done(Result<ExecTerminal, ExecClientFailure>),
            WaitClosed,
            Detached(&'static str),
        }
        let mut forwarded = false;
        let exit = loop {
            tokio::select! {
                res = &mut active => break Exit::Done(res),
                () = &mut forwarding, if !forwarded => forwarded = true,
                sole = &mut abandoned => {
                    break if sole {
                        Exit::WaitClosed
                    } else {
                        Exit::Detached("Exec owner closed during route pause")
                    };
                }
                sole = &mut cancelled => {
                    break if sole {
                        Exit::WaitClosed
                    } else {
                        Exit::Detached("Exec cancelled during route pause")
                    };
                }
            }
        };
        match exit {
            Exit::Done(res) => res,
            Exit::WaitClosed => active.await,
            Exit::Detached(msg) => Err(disconnected(msg, true)),
        }
    };
    let _ = writer.inner.finish();
    if let Err(error) = &result {
        if matches!(
            error.kind,
            ExecClientFailureKind::Disconnected
                | ExecClientFailureKind::Protocol
                | ExecClientFailureKind::Authentication
        ) {
            lease.evict_and_close_if_sole();
        }
    }
    result
}

#[allow(clippy::too_many_arguments)]
async fn run_v3_stream_active<W: AsyncWrite + Unpin>(
    writer: &mut W,
    mut recv: RecvStream,
    conn: &Connection,
    endpoint: &PeerEndpoint,
    start: &ExecStart,
    operation_rx: mpsc::Receiver<ExecClientInput>,
    events: mpsc::Sender<ExecClientEvent>,
    notice: &watch::Sender<ExecClientNotice>,
) -> Result<ExecTerminal, ExecClientFailure> {
    let deadline = crate::share::route_protocol::active_now() + HANDSHAKE_TIMEOUT;
    let ReceivedHelloOk { hello, access } =
        match until(deadline, recv_hello_result::<_, ReceivedHelloOk>(&mut recv))
            .await
            .map_err(|error| classify_io(error, true, "hello_result"))?
        {
            Ok(hello) if !hello.hello.authorization.session_id.trim().is_empty() => hello,
            Ok(_) => return Err(protocol("empty Exec session id", true, "invalid_hello_ok")),
            Err(error) => {
                return Err(failure(
                    ExecClientFailureKind::Authentication,
                    &error.code,
                    &client_failure_message(&error.code, &error.message, &peer_name(endpoint)),
                    false,
                ));
            }
        };
    notice.send_modify(|current| current.access = access);
    emit(
        &events,
        ExecClientEvent::Authorized {
            authorization: hello.authorization,
            provider: hello.provider.clone(),
        },
        true,
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
    require_host_terminal(start, &hello.features)?;
    let close_watch = conn.clone();
    crate::share::exec_client_active::run_with_notice(
        writer,
        recv,
        operation_rx,
        events,
        start.exec_id.clone(),
        start.terminal.is_some(),
        EXEC_HEARTBEAT_POLICY,
        Box::pin(async move {
            let _ = close_watch.closed().await;
        }),
        notice,
    )
    .await
}

async fn forward_v3(
    connection: &Connection,
    mut input_rx: mpsc::Receiver<ExecClientInput>,
    mut cancel_rx: watch::Receiver<bool>,
    route_cancel: Option<Arc<RouteCancellation>>,
    output: mpsc::Sender<ExecClientInput>,
) {
    loop {
        let command = tokio::select! {
            biased;
            () = any_cancelled(&mut cancel_rx, route_cancel.as_deref()) => break,
            command = input_rx.recv() => match command { Some(cmd) => cmd, None => return },
            _ = connection.closed() => return,
        };
        tokio::select! {
            biased;
            () = any_cancelled(&mut cancel_rx, route_cancel.as_deref()) => break,
            sent = output.send(command) => if sent.is_err() { return; },
            _ = connection.closed() => return,
        }
    }
    if let Some(rc) = &route_cancel {
        rc.cancel();
    }
    tokio::select! {
        _ = output.send(ExecClientInput::Cancel) => {}
        _ = connection.closed() => {}
    }
}

async fn abandon_during_pause_v3(
    lease: &PoolLeaseGuard,
    abandoned: impl Future<Output = ()>,
) -> bool {
    abandoned.await;
    let conn = lease.connection();
    let Some(mut status) = crate::share::route_monitor::subscribe(conn) else {
        let _ = conn.closed().await;
        return true;
    };
    loop {
        if !conn.application_data_allowed() {
            if lease.is_sole_owner() {
                lease.evict_and_close_if_sole();
                return true;
            }
            return false;
        }
        if status.changed().await.is_err() {
            return true;
        }
    }
}

pub(super) async fn any_cancelled(
    cancel: &mut watch::Receiver<bool>,
    route: Option<&RouteCancellation>,
) {
    let route_wait = async {
        match route {
            Some(rc) => rc.cancelled().await,
            None => std::future::pending::<()>().await,
        }
    };
    tokio::select! {
        () = input::cancelled(cancel) => {}
        () = route_wait => {}
    }
}

pub(super) struct CountingWriter<W> {
    pub(super) inner: W,
    pub(super) bytes_written: u64,
}

impl<W: AsyncWrite + Unpin> AsyncWrite for CountingWriter<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let poll = Pin::new(&mut self.inner).poll_write(cx, buf);
        if let Poll::Ready(Ok(n)) = &poll {
            self.bytes_written = self.bytes_written.saturating_add(*n as u64);
        }
        poll
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
