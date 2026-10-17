use std::collections::VecDeque;
use std::future::Future;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::Connection;
use tokio::io::AsyncWrite;
use tokio::sync::mpsc;

use super::access_grant_runtime::GrantEndWatch;
use super::core::{eio, now_secs, random_token};
use super::exec_access_view::ExecApprovalPending;
use super::exec_auth::authorize_client_hello;
use super::exec_frame_reader::{self, FrameReader};
use super::exec_heartbeat::{ExecHeartbeatPolicy, EXEC_HEARTBEAT_POLICY};
use super::exec_job::{run_contained_job, JobInput, JobLaunch};
use super::exec_protocol::{
    recv_client_frame, recv_client_hello, send_hello_error, send_hello_ok, send_server_frame,
    send_server_hello, ClientFrame, ExecHelloOk, ExecServerHello, ExecWireError, ServerFrame,
    ServerProtocolState,
};
use super::exec_registry::{ExecAdmission, ExecCancelReason, ExecRegistry};
use super::exec_types::ExecId;
use super::handshake_limits::ApplicationHandshakePermit;
use super::io_deadline;
use super::node::ShareIrohNode;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);
/// Inputs a client may send ahead of its job (the job's own queue holds 16).
const MAX_QUEUED_INPUT: usize = 16;

#[path = "exec_admission.rs"]
mod admission;
pub(super) use admission::handle_connection;
#[cfg(test)]
pub(super) use admission::start_grant;

/// The client half after the start frame: its frame reader, the inputs it
/// sent ahead of the job, and its heartbeat so far.
pub(super) struct ClientStream {
    pub(super) frames: FrameReader<ClientFrame>,
    pub(super) queued: VecDeque<JobInput>,
    pub(super) last_ping: tokio::time::Instant,
    pub(super) last_sequence: u64,
}

impl ClientStream {
    pub(super) fn new(frames: FrameReader<ClientFrame>) -> Self {
        Self {
            frames,
            queued: VecDeque::new(),
            last_ping: tokio::time::Instant::now(),
            last_sequence: 0,
        }
    }
}

/// How waiting for the host user's decision on a start ended.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) enum ApprovalWait<T> {
    Decided(T),
    /// The client cancelled the start before a decision.
    Cancelled,
}

/// Waits for `decision` on a start that needs the host user (PRM-M3 calls it
/// with the approval service). The client stays connected meanwhile: every
/// heartbeat is answered (`PING` 20, `PONG` 37), so clients up to 0.5.182
/// wait as long as the decision takes; inputs sent ahead are kept for the
/// job (reading pauses at 16 of them); a cancel ends the wait. `pending` is
/// sent first, only to clients that announced `exec_approval_v1`.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) async fn wait_for_approval<W, D, T>(
    send: &mut W,
    stream: &mut ClientStream,
    protocol: &mut ServerProtocolState,
    exec_id: &ExecId,
    pending: Option<ExecApprovalPending>,
    decision: D,
    policy: ExecHeartbeatPolicy,
) -> io::Result<ApprovalWait<T>>
where
    W: AsyncWrite + Unpin,
    D: Future<Output = T>,
{
    if let Some(pending) = pending {
        let deadline = stream.last_ping + policy.peer_timeout;
        send_frame_bounded(send, &ServerFrame::ApprovalPending(pending), deadline).await?;
    }
    tokio::pin!(decision);
    loop {
        let heartbeat_deadline = stream.last_ping + policy.peer_timeout;
        tokio::select! {
            outcome = &mut decision => return Ok(ApprovalWait::Decided(outcome)),
            incoming = stream.frames.next(), if stream.queued.len() < MAX_QUEUED_INPUT => {
                let frame = incoming.ok_or_else(|| eio("Exec-Frame-Reader endete unerwartet"))??;
                if frame_exec_id(&frame) != exec_id || protocol.accept(&frame).is_err() {
                    return Err(eio("ungueltiger Exec-Frame-Ablauf"));
                }
                match frame {
                    ClientFrame::Ping { sequence, .. } => {
                        if sequence <= stream.last_sequence {
                            return Err(eio("ungueltige Exec-Heartbeat-Sequenz"));
                        }
                        stream.last_sequence = sequence;
                        stream.last_ping = tokio::time::Instant::now();
                        let pong = ServerFrame::Pong { exec_id: exec_id.clone(), sequence };
                        let deadline = stream.last_ping + policy.peer_timeout;
                        send_frame_bounded(send, &pong, deadline).await?;
                    }
                    ClientFrame::Cancel { .. } => return Ok(ApprovalWait::Cancelled),
                    ClientFrame::Stdin { data, .. } => {
                        stream.queued.push_back(JobInput::Stdin(data));
                    }
                    ClientFrame::StdinEof { .. } => stream.queued.push_back(JobInput::StdinEof),
                    ClientFrame::Resize { size, .. } => {
                        stream.queued.push_back(JobInput::Resize(size));
                    }
                    ClientFrame::Start { .. } | ClientFrame::ResultAck { .. } => {
                        return Err(eio("unerwarteter Exec-Frame vor dem Start"));
                    }
                }
            }
            _ = tokio::time::sleep_until(heartbeat_deadline) => {
                return Err(timed_out("Exec-Peer beantwortet keine authentifizierten Heartbeats"));
            }
        }
    }
}

/// `grant_end` ends the job as revoked when the grant's end comes (C5).
async fn serve_job(
    registry: Arc<ExecRegistry>,
    launch: JobLaunch,
    mut send: iroh::endpoint::SendStream,
    stream: ClientStream,
    mut protocol: ServerProtocolState,
    connection: Connection,
    mut grant_end: Option<GrantEndWatch>,
) -> io::Result<()> {
    let exec_id = launch.start.exec_id.clone();
    let (input_tx, input_rx) = mpsc::channel(16);
    let (output_tx, mut output_rx) = mpsc::channel(16);
    let worker_registry = registry.clone();
    let worker = tokio::task::spawn_blocking(move || {
        run_contained_job(worker_registry, launch, input_rx, output_tx)
    });
    let ClientStream {
        mut frames,
        mut queued,
        mut last_ping,
        mut last_sequence,
    } = stream;
    let transport_closed = connection.closed();
    tokio::pin!(transport_closed);
    loop {
        let heartbeat_deadline = last_ping + EXEC_HEARTBEAT_POLICY.peer_timeout;
        let end_deadline = grant_end
            .as_ref()
            .map_or(heartbeat_deadline, |end| deadline_at(end.end()));
        if tokio::time::Instant::now() >= heartbeat_deadline {
            registry.cancel(&exec_id, ExecCancelReason::Disconnected);
            drop(input_tx);
            drop(output_rx);
            let _ = worker.await;
            return Err(timed_out(
                "Exec-Peer beantwortet keine authentifizierten Heartbeats",
            ));
        }
        let reserve_sender = input_tx.clone();
        tokio::select! {
            incoming = frames.next(), if queued.is_empty() => {
                let frame = match incoming {
                    Some(Ok(frame)) => frame,
                    Some(Err(error)) => {
                        registry.cancel(&exec_id, ExecCancelReason::Disconnected);
                        drop(input_tx);
                        drop(output_rx);
                        let _ = worker.await;
                        return Err(error);
                    }
                    None => {
                        registry.cancel(&exec_id, ExecCancelReason::Disconnected);
                        drop(input_tx);
                        drop(output_rx);
                        let _ = worker.await;
                        return Err(eio("Exec-Frame-Reader endete unerwartet"));
                    }
                };
                if frame_exec_id(&frame) != &exec_id || protocol.accept(&frame).is_err() {
                    registry.cancel(&exec_id, ExecCancelReason::Disconnected);
                    drop(input_tx);
                    drop(output_rx);
                    let _ = worker.await;
                    return Err(eio("ungueltiger Exec-Frame-Ablauf"));
                }
                match frame {
                    ClientFrame::Stdin { data, .. } => {
                        queued.push_back(JobInput::Stdin(data));
                    }
                    ClientFrame::StdinEof { .. } => {
                        queued.push_back(JobInput::StdinEof);
                    }
                    ClientFrame::Resize { size, .. } => {
                        queued.push_back(JobInput::Resize(size));
                    }
                    ClientFrame::Cancel { .. } => {
                        registry.cancel(&exec_id, ExecCancelReason::User);
                    }
                    ClientFrame::Ping { sequence, .. } => {
                        if sequence <= last_sequence {
                            registry.cancel(&exec_id, ExecCancelReason::Disconnected);
                            drop(input_tx);
                            drop(output_rx);
                            let _ = worker.await;
                            return Err(eio("ungueltige Exec-Heartbeat-Sequenz"));
                        }
                        last_sequence = sequence;
                        last_ping = tokio::time::Instant::now();
                        if let Err(error) = send_frame_bounded(
                            &mut send,
                            &ServerFrame::Pong {
                                exec_id: exec_id.clone(),
                                sequence,
                            },
                            last_ping + EXEC_HEARTBEAT_POLICY.peer_timeout,
                        ).await {
                            registry.cancel(&exec_id, ExecCancelReason::Disconnected);
                            drop(input_tx);
                            drop(output_rx);
                            let _ = worker.await;
                            return Err(error);
                        }
                    }
                    ClientFrame::Start { .. } => {
                        registry.cancel(&exec_id, ExecCancelReason::Disconnected);
                        return Err(eio("doppelter Exec-Start"));
                    }
                    ClientFrame::ResultAck { .. } => {
                        registry.cancel(&exec_id, ExecCancelReason::Disconnected);
                        return Err(eio("vorzeitige Exec-Ergebnisbestaetigung"));
                    }
                }
            }
            permit = reserve_sender.reserve_owned(), if !queued.is_empty() => {
                let permit = match permit {
                    Ok(permit) => permit,
                    Err(error) => {
                        registry.cancel(&exec_id, ExecCancelReason::Disconnected);
                        drop(input_tx);
                        drop(output_rx);
                        let _ = worker.await;
                        return Err(eio(error));
                    }
                };
                let Some(input) = queued.pop_front() else {
                    registry.cancel(&exec_id, ExecCancelReason::Disconnected);
                    drop(input_tx);
                    drop(output_rx);
                    let _ = worker.await;
                    return Err(eio("reservierter Exec-Input fehlt"));
                };
                permit.send(input);
            }
            outgoing = output_rx.recv() => {
                let Some(frame) = outgoing else {
                    let result = worker.await.map_err(eio)?;
                    return result.and_then(|()| Err(eio("Exec-Worker endete ohne Terminalstatus")));
                };
                let terminal = matches!(frame, ServerFrame::Terminal(_) | ServerFrame::Error(_));
                if let Err(error) = send_frame_bounded(
                    &mut send,
                    &frame,
                    heartbeat_deadline,
                ).await {
                    registry.cancel(&exec_id, ExecCancelReason::Disconnected);
                    drop(input_tx);
                    drop(output_rx);
                    let _ = worker.await;
                    return Err(error);
                }
                if terminal {
                    protocol.terminal();
                    drop(input_tx);
                    let result = worker.await.map_err(eio)?;
                    wait_result_ack_from_reader(&mut frames, &mut protocol, &exec_id).await?;
                    acknowledge_result(&mut send, &exec_id, &connection).await?;
                    return result;
                }
            }
            _ = &mut transport_closed => {
                registry.cancel(&exec_id, ExecCancelReason::Disconnected);
                drop(input_tx);
                drop(output_rx);
                let _ = worker.await;
                return Err(eio("Exec-Transport wurde vor dem Terminalstatus geschlossen"));
            }
            _ = tokio::time::sleep_until(heartbeat_deadline) => {
                registry.cancel(&exec_id, ExecCancelReason::Disconnected);
                drop(input_tx);
                drop(output_rx);
                let _ = worker.await;
                return Err(timed_out("Exec-Peer beantwortet keine authentifizierten Heartbeats"));
            }
            _ = tokio::time::sleep_until(end_deadline), if grant_end.is_some() => {
                grant_end = grant_end.take().and_then(|end| end.expire(&registry, &exec_id));
            }
        }
    }
}

/// The monotonic instant of `unix_secs` (host clock), at the earliest now and
/// at the latest in a day; the end is checked again from there.
fn deadline_at(unix_secs: i64) -> tokio::time::Instant {
    const LONGEST_WAIT_SECS: u64 = 24 * 60 * 60;
    let remaining = u64::try_from(unix_secs.saturating_sub(now_secs())).unwrap_or(0);
    tokio::time::Instant::now() + Duration::from_secs(remaining.min(LONGEST_WAIT_SECS))
}

fn frame_exec_id(frame: &ClientFrame) -> &ExecId {
    match frame {
        ClientFrame::Start { start, .. } => &start.exec_id,
        ClientFrame::Stdin { exec_id, .. }
        | ClientFrame::StdinEof { exec_id }
        | ClientFrame::Cancel { exec_id }
        | ClientFrame::Ping { exec_id, .. }
        | ClientFrame::Resize { exec_id, .. }
        | ClientFrame::ResultAck { exec_id } => exec_id,
    }
}

async fn wait_result_ack<R: tokio::io::AsyncRead + Unpin>(
    recv: &mut R,
    protocol: &mut ServerProtocolState,
    exec_id: &ExecId,
) -> io::Result<()> {
    wait_result_ack_with_policy(recv, protocol, exec_id, EXEC_HEARTBEAT_POLICY).await
}

async fn wait_result_ack_with_policy<R: tokio::io::AsyncRead + Unpin>(
    recv: &mut R,
    protocol: &mut ServerProtocolState,
    exec_id: &ExecId,
    policy: ExecHeartbeatPolicy,
) -> io::Result<()> {
    let deadline = tokio::time::Instant::now() + policy.server_result_ack_timeout();
    loop {
        let frame = tokio::time::timeout_at(deadline, recv_client_frame(recv))
            .await
            .map_err(|_| timed_out("Exec-Ergebnisbestaetigung Timeout"))??;
        if validate_result_ack_frame(protocol, exec_id, frame)? {
            return Ok(());
        }
    }
}

async fn wait_result_ack_from_reader(
    frames: &mut exec_frame_reader::FrameReader<ClientFrame>,
    protocol: &mut ServerProtocolState,
    exec_id: &ExecId,
) -> io::Result<()> {
    let deadline = tokio::time::Instant::now() + EXEC_HEARTBEAT_POLICY.server_result_ack_timeout();
    loop {
        let frame = tokio::time::timeout_at(deadline, frames.next())
            .await
            .map_err(|_| timed_out("Exec-Ergebnisbestaetigung Timeout"))?
            .ok_or_else(|| eio("Exec-Frame-Reader endete vor der Ergebnisbestaetigung"))??;
        if validate_result_ack_frame(protocol, exec_id, frame)? {
            return Ok(());
        }
    }
}

fn validate_result_ack_frame(
    protocol: &mut ServerProtocolState,
    exec_id: &ExecId,
    frame: ClientFrame,
) -> io::Result<bool> {
    if frame_exec_id(&frame) != exec_id {
        return Err(eio("Exec-Ergebnisbestaetigung hat die falsche ID"));
    }
    protocol.accept(&frame)?;
    match frame {
        ClientFrame::ResultAck { .. } => Ok(true),
        ClientFrame::Stdin { .. }
        | ClientFrame::StdinEof { .. }
        | ClientFrame::Cancel { .. }
        | ClientFrame::Resize { .. }
        | ClientFrame::Ping { .. } => Ok(false),
        ClientFrame::Start { .. } => Err(eio("unerwarteter Start nach Exec-Terminalstatus")),
    }
}

async fn send_frame_bounded<W: AsyncWrite + Unpin>(
    send: &mut W,
    frame: &ServerFrame,
    peer_deadline: tokio::time::Instant,
) -> io::Result<()> {
    let write_deadline =
        (tokio::time::Instant::now() + EXEC_HEARTBEAT_POLICY.write_timeout).min(peer_deadline);
    tokio::time::timeout_at(write_deadline, send_server_frame(send, frame))
        .await
        .map_err(|_| timed_out("Exec-Peer nimmt keine Ausgabeframes mehr an"))?
}

async fn send_terminal_frame(
    send: &mut iroh::endpoint::SendStream,
    frame: &ServerFrame,
) -> io::Result<()> {
    send_frame_bounded(
        send,
        frame,
        tokio::time::Instant::now() + EXEC_HEARTBEAT_POLICY.write_timeout,
    )
    .await
}

async fn acknowledge_result(
    send: &mut iroh::endpoint::SendStream,
    exec_id: &ExecId,
    connection: &Connection,
) -> io::Result<()> {
    send_frame_bounded(
        send,
        &ServerFrame::ResultAcknowledged {
            exec_id: exec_id.clone(),
        },
        tokio::time::Instant::now() + EXEC_HEARTBEAT_POLICY.write_timeout,
    )
    .await?;
    finish_send_and_wait_until(
        send,
        connection,
        tokio::time::Instant::now() + EXEC_HEARTBEAT_POLICY.server_result_ack_timeout(),
    )
    .await
}

async fn finish_send_and_wait_until(
    send: &mut iroh::endpoint::SendStream,
    connection: &Connection,
    deadline: tokio::time::Instant,
) -> io::Result<()> {
    send.finish().map_err(eio)?;
    tokio::time::timeout_at(deadline, connection.closed())
        .await
        .map_err(|_| timed_out("Exec-Clientabschluss Timeout"))?;
    Ok(())
}

fn timed_out(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, message)
}

#[cfg(test)]
#[path = "exec_server_tests.rs"]
mod tests;
