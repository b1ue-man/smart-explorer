use std::io;
use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::Connection;
use tokio::sync::mpsc;

use super::core::{eio, now_secs, random_token};
use super::exec_auth::authorize_client_hello;
use super::exec_frame_reader;
use super::exec_heartbeat::{ExecHeartbeatPolicy, EXEC_HEARTBEAT_POLICY};
use super::exec_job::{run_contained_job, JobInput};
use super::exec_protocol::{
    recv_client_frame, recv_client_hello, send_hello_error, send_hello_ok, send_server_frame,
    send_server_hello, ClientFrame, ExecHelloOk, ExecServerHello, ExecWireError, ServerFrame,
    ServerProtocolState,
};
use super::exec_registry::{ExecAdmission, ExecCancelReason, ExecRegistry, ExecReservation};
use super::exec_types::{ExecId, ExecStart};
use super::handshake_limits::ApplicationHandshakePermit;
use super::io_deadline;
use super::node::ShareIrohNode;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);

#[path = "exec_admission.rs"]
mod admission;
pub(super) use admission::handle_connection;

async fn serve_job(
    registry: Arc<ExecRegistry>,
    start: ExecStart,
    reservation: ExecReservation,
    mut send: iroh::endpoint::SendStream,
    recv: iroh::endpoint::RecvStream,
    mut protocol: ServerProtocolState,
    connection: Connection,
) -> io::Result<()> {
    let exec_id = start.exec_id.clone();
    let (input_tx, input_rx) = mpsc::channel(16);
    let (output_tx, mut output_rx) = mpsc::channel(16);
    let worker_registry = registry.clone();
    let worker = tokio::task::spawn_blocking(move || {
        run_contained_job(worker_registry, start, reservation, input_rx, output_tx)
    });
    let mut frames = exec_frame_reader::client_frames(recv);
    let mut last_ping = tokio::time::Instant::now();
    let mut last_sequence = 0u64;
    let mut pending_input: Option<JobInput> = None;
    let transport_closed = connection.closed();
    tokio::pin!(transport_closed);
    loop {
        let heartbeat_deadline = last_ping + EXEC_HEARTBEAT_POLICY.peer_timeout;
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
            incoming = frames.next(), if pending_input.is_none() => {
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
                        pending_input = Some(JobInput::Stdin(data));
                    }
                    ClientFrame::StdinEof { .. } => {
                        pending_input = Some(JobInput::StdinEof);
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
            permit = reserve_sender.reserve_owned(), if pending_input.is_some() => {
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
                let Some(input) = pending_input.take() else {
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
        }
    }
}

fn frame_exec_id(frame: &ClientFrame) -> &ExecId {
    match frame {
        ClientFrame::Start { start, .. } => &start.exec_id,
        ClientFrame::Stdin { exec_id, .. }
        | ClientFrame::StdinEof { exec_id }
        | ClientFrame::Cancel { exec_id }
        | ClientFrame::Ping { exec_id, .. }
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
        | ClientFrame::Ping { .. } => Ok(false),
        ClientFrame::Start { .. } => Err(eio("unerwarteter Start nach Exec-Terminalstatus")),
    }
}

async fn send_frame_bounded(
    send: &mut iroh::endpoint::SendStream,
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
