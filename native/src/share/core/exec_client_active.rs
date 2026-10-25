use std::future::Future;
use std::pin::Pin;

use tokio::io::AsyncWrite;
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;

use super::exec_access_view::ExecClientNotice;
use super::exec_client::{
    emit_or_cancel, emit_with_timeout, write_frame_with_timeout, ExecClientEvent,
    ExecClientFailure, ExecClientFailureKind, ExecClientInput,
};
use super::exec_client_messages::{classify_io, failure, protocol};
use super::exec_frame_reader;
use super::exec_heartbeat::ExecHeartbeatPolicy;
use super::exec_protocol::{ClientFrame, ExecWireError, ServerFrame};
use super::exec_types::{ExecId, ExecTerminal};

pub(super) type TransportClosed = Pin<Box<dyn Future<Output = ()> + Send>>;

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(super) async fn run<W, R>(
    send: W,
    recv: R,
    input: mpsc::Receiver<ExecClientInput>,
    events: mpsc::Sender<ExecClientEvent>,
    exec_id: ExecId,
    terminal_start: bool,
    heartbeat_policy: ExecHeartbeatPolicy,
    transport_closed: TransportClosed,
) -> Result<ExecTerminal, ExecClientFailure>
where
    W: AsyncWrite + Unpin,
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let (notice, _) = watch::channel(ExecClientNotice::default());
    run_with_notice(
        send,
        recv,
        input,
        events,
        exec_id,
        terminal_start,
        heartbeat_policy,
        transport_closed,
        &notice,
    )
    .await
}

/// The active phase after `Start`. `APPROVAL_PENDING` of the host, before
/// `Started`, goes to `notice` and keeps the start waiting (D.2.2).
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_with_notice<W, R>(
    mut send: W,
    recv: R,
    mut input: mpsc::Receiver<ExecClientInput>,
    events: mpsc::Sender<ExecClientEvent>,
    exec_id: ExecId,
    terminal_start: bool,
    heartbeat_policy: ExecHeartbeatPolicy,
    mut transport_closed: TransportClosed,
    notice: &watch::Sender<ExecClientNotice>,
) -> Result<ExecTerminal, ExecClientFailure>
where
    W: AsyncWrite + Unpin,
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let mut frames = exec_frame_reader::server_frames(recv);
    let mut server_state = ClientServerState::default();
    let mut stdin_closed = false;
    let mut cancelling = false;
    let mut next_sequence = 0u64;
    let mut outstanding_ping: Option<u64> = None;
    let mut peer_deadline = super::route_protocol::active_now() + heartbeat_policy.peer_timeout;
    let mut heartbeat_at = super::route_protocol::active_now() + heartbeat_policy.interval;

    loop {
        if super::route_protocol::active_now() >= peer_deadline {
            return Err(peer_unresponsive());
        }
        tokio::select! {
            _ = events.closed(), if !cancelling => {
                cancelling=true;
                write_frame_with_timeout(
                    &mut send,&ClientFrame::Cancel {exec_id:exec_id.clone()},true,
                    active_write_timeout(heartbeat_policy,peer_deadline)?,
                ).await?;
            }
            peer = next_server_or_closed(&mut frames, &mut transport_closed) => {
                let frame = match peer {
                    ServerRead::Frame(frame) => frame,
                    ServerRead::Closed => {
                        return Err(disconnected("Exec transport closed before a terminal result"));
                    }
                }
                    .ok_or_else(|| disconnected("Exec peer frame reader stopped"))?
                    .map_err(|error| classify_io(error, true, "server_frame"))?;
                if let ServerFrame::Pong { exec_id: pong_id, sequence } = frame {
                    if pong_id != exec_id {
                        return Err(protocol("heartbeat Pong has the wrong Exec ID", true, "heartbeat_id"));
                    }
                    match outstanding_ping {
                        Some(expected) if sequence == expected => {
                            outstanding_ping = None;
                            peer_deadline = super::route_protocol::active_now() + heartbeat_policy.peer_timeout;
                            continue;
                        }
                        _ => {
                            return Err(protocol("heartbeat Pong is stale or unexpected", true, "heartbeat_sequence"));
                        }
                    }
                }
                if let ServerFrame::ApprovalPending(pending) = frame {
                    if pending.exec_id != exec_id || server_state.started {
                        let message = "approval notice for another or a started Exec";
                        return Err(protocol(message, true, "approval_order"));
                    }
                    notice.send_modify(|notice| notice.approval = Some(pending));
                    continue;
                }
                let event = match server_state.accept(frame, &exec_id, true) {
                    Ok(event) => event,
                    Err(error) if error.kind == ExecClientFailureKind::Remote => {
                        complete_result_handshake(
                            &mut send,
                            &mut frames,
                            &mut transport_closed,
                            &exec_id,
                            heartbeat_policy,
                        ).await?;
                        return Err(error);
                    }
                    Err(error) => return Err(error),
                };
                if let ExecClientEvent::Terminal(terminal) = &event {
                    complete_result_handshake(
                        &mut send,
                        &mut frames,
                        &mut transport_closed,
                        &exec_id,
                        heartbeat_policy,
                    ).await?;
                    // The authenticated terminal exchange has completed, so a
                    // deliberately delayed confirmation must not inherit the
                    // pre-terminal heartbeat deadline for local event delivery.
                    peer_deadline = super::route_protocol::active_now() + heartbeat_policy.peer_timeout;
                    emit_with_timeout(
                        &events,
                        event.clone(),
                        true,
                        active_write_timeout(heartbeat_policy, peer_deadline)?,
                    ).await?;
                    return Ok(terminal.clone());
                }
                if event == ExecClientEvent::Started {
                    notice.send_modify(|notice| notice.approval = None);
                }
                emit_or_cancel(
                    &events,
                    event,
                    &mut send,
                    &exec_id,
                    true,
                    peer_deadline,
                ).await?;
            }
            _ = super::route_protocol::sleep_until_active(peer_deadline) => {
                return Err(peer_unresponsive());
            }
            command = input.recv() => {
                match command {
                    Some(ExecClientInput::Stdin(data)) if !stdin_closed && !cancelling => {
                        if data.len() > super::exec_types::MAX_EXEC_DATA_BYTES {
                            return Err(protocol("stdin chunk exceeds 64 KiB", true, "stdin_too_large"));
                        }
                        write_frame_with_timeout(
                            &mut send,
                            &ClientFrame::Stdin { exec_id: exec_id.clone(), data },
                            true,
                            active_write_timeout(heartbeat_policy, peer_deadline)?,
                        ).await?;
                    }
                    Some(ExecClientInput::StdinEof) if !stdin_closed && !cancelling => {
                        stdin_closed = true;
                        write_frame_with_timeout(
                            &mut send,
                            &ClientFrame::StdinEof { exec_id: exec_id.clone() },
                            true,
                            active_write_timeout(heartbeat_policy, peer_deadline)?,
                        ).await?;
                    }
                    Some(ExecClientInput::Cancel) if !cancelling => {
                        cancelling = true;
                        write_frame_with_timeout(
                            &mut send,
                            &ClientFrame::Cancel { exec_id: exec_id.clone() },
                            true,
                            active_write_timeout(heartbeat_policy, peer_deadline)?,
                        ).await?;
                    }
                    Some(ExecClientInput::Cancel) => {}
                    // A window size only means something to a terminal start
                    // that is not being cancelled, and only a usable one is
                    // sent; the previous size stays otherwise.
                    Some(ExecClientInput::Resize(size)) => {
                        if terminal_start && !cancelling && size.validate().is_ok() {
                            write_frame_with_timeout(
                                &mut send,
                                &ClientFrame::Resize { exec_id: exec_id.clone(), size },
                                true,
                                active_write_timeout(heartbeat_policy, peer_deadline)?,
                            ).await?;
                        }
                    }
                    Some(_) => return Err(protocol("stdin arrived after EOF or Cancel", true, "input_order")),
                    None => {
                        let _ = write_frame_with_timeout(
                            &mut send,
                            &ClientFrame::Cancel { exec_id: exec_id.clone() },
                            true,
                            active_write_timeout(heartbeat_policy, peer_deadline)?,
                        ).await;
                        return Err(failure(
                            ExecClientFailureKind::Local,
                            "input_closed",
                            "local Exec input channel closed",
                            true,
                        ));
                    }
                }
            }
            _ = super::route_protocol::sleep_until_active(heartbeat_at), if outstanding_ping.is_none() => {
                heartbeat_at = super::route_protocol::active_now() + heartbeat_policy.interval;
                next_sequence = next_sequence.checked_add(1).ok_or_else(|| {
                    protocol("heartbeat sequence exhausted", true, "heartbeat_sequence")
                })?;
                write_frame_with_timeout(
                    &mut send,
                    &ClientFrame::Ping {
                        exec_id: exec_id.clone(),
                        sequence: next_sequence,
                    },
                    true,
                    active_write_timeout(heartbeat_policy, peer_deadline)?,
                ).await?;
                outstanding_ping = Some(next_sequence);
            }
        }
    }
}

async fn complete_result_handshake<W: AsyncWrite + Unpin>(
    send: &mut W,
    frames: &mut exec_frame_reader::FrameReader<ServerFrame>,
    transport_closed: &mut TransportClosed,
    exec_id: &ExecId,
    policy: ExecHeartbeatPolicy,
) -> Result<(), ExecClientFailure> {
    let deadline = super::route_protocol::active_now() + policy.client_result_acknowledged_timeout();
    write_frame_with_timeout(
        send,
        &ClientFrame::ResultAck {
            exec_id: exec_id.clone(),
        },
        true,
        policy.write_timeout.min(policy.peer_timeout),
    )
    .await?;
    tokio::select! {
        peer = next_server_or_closed(frames, transport_closed) => {
            let frame = match peer {
                ServerRead::Frame(frame) => frame,
                ServerRead::Closed => {
                    return Err(disconnected("Exec transport closed before acknowledging the terminal result"));
                }
            }
                .ok_or_else(|| disconnected("Exec peer frame reader stopped before acknowledging the terminal result"))?
                .map_err(|error| classify_io(error, true, "result_acknowledged"))?;
            match frame {
                ServerFrame::ResultAcknowledged { exec_id: acknowledged } if acknowledged == *exec_id => Ok(()),
                ServerFrame::ResultAcknowledged { .. } => Err(protocol(
                    "terminal result acknowledgement has the wrong Exec ID",
                    true,
                    "result_acknowledged_id",
                )),
                _ => Err(protocol(
                    "unexpected server frame after terminal result",
                    true,
                    "result_acknowledged_order",
                )),
            }
        }
        _ = super::route_protocol::sleep_until_active(deadline) => {
            Err(failure(
                ExecClientFailureKind::Disconnected,
                "result_acknowledged_timeout",
                "Exec peer did not acknowledge the terminal result",
                true,
            ))
        }
    }
}

enum ServerRead {
    Frame(Option<std::io::Result<ServerFrame>>),
    Closed,
}

async fn next_server_or_closed(
    frames: &mut exec_frame_reader::FrameReader<ServerFrame>,
    transport_closed: &mut TransportClosed,
) -> ServerRead {
    tokio::select! {
        biased;
        frame = frames.next() => ServerRead::Frame(frame),
        _ = transport_closed.as_mut() => ServerRead::Closed,
    }
}

fn active_write_timeout(
    policy: ExecHeartbeatPolicy,
    peer_deadline: Instant,
) -> Result<std::time::Duration, ExecClientFailure> {
    let remaining = peer_deadline.saturating_duration_since(super::route_protocol::active_now());
    if remaining.is_zero() {
        Err(peer_unresponsive())
    } else {
        Ok(policy.write_timeout.min(remaining))
    }
}

fn peer_unresponsive() -> ExecClientFailure {
    failure(
        ExecClientFailureKind::Disconnected,
        "peer_unresponsive",
        "Exec peer stopped answering authenticated heartbeats",
        true,
    )
}

fn disconnected(message: &str) -> ExecClientFailure {
    failure(
        ExecClientFailureKind::Disconnected,
        "disconnected",
        message,
        true,
    )
}

#[derive(Default)]
pub(super) struct ClientServerState {
    started: bool,
    terminal: bool,
}

impl ClientServerState {
    pub(super) fn accept(
        &mut self,
        frame: ServerFrame,
        want: &ExecId,
        start_sent: bool,
    ) -> Result<ExecClientEvent, ExecClientFailure> {
        let event = match frame {
            ServerFrame::Started { exec_id } if exec_id == *want && !self.started => {
                self.started = true;
                ExecClientEvent::Started
            }
            ServerFrame::Stdout { exec_id, data } if exec_id == *want && self.started => {
                ExecClientEvent::Stdout(data)
            }
            ServerFrame::Stderr { exec_id, data } if exec_id == *want && self.started => {
                ExecClientEvent::Stderr(data)
            }
            ServerFrame::Terminal(terminal) if terminal.exec_id == *want && !self.terminal => {
                self.terminal = true;
                ExecClientEvent::Terminal(terminal)
            }
            ServerFrame::Error(ExecWireError { code, message }) => {
                return Err(failure(
                    ExecClientFailureKind::Remote,
                    &code,
                    &message,
                    start_sent,
                ));
            }
            ServerFrame::Pong { .. } => {
                return Err(protocol(
                    "heartbeat Pong bypassed validation",
                    start_sent,
                    "heartbeat_order",
                ));
            }
            ServerFrame::ResultAcknowledged { .. } => {
                return Err(protocol(
                    "terminal result acknowledgement arrived before a terminal result",
                    start_sent,
                    "result_acknowledged_order",
                ));
            }
            _ => {
                return Err(protocol(
                    "invalid Exec server frame order or id",
                    start_sent,
                    "server_order",
                ));
            }
        };
        Ok(event)
    }
}

#[cfg(test)]
#[path = "exec_client_active_tests.rs"]
mod tests;
