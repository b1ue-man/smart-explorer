use std::io;
use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::Connection;
use tokio::sync::{mpsc, OwnedSemaphorePermit};

use super::core::{eio, now_secs, random_token};
use super::exec_auth::authorize_client_hello;
use super::exec_job::{run_contained_job, JobInput};
use super::exec_protocol::{
    recv_client_frame, recv_client_hello, send_hello_error, send_hello_ok, send_server_frame,
    send_server_hello, ClientFrame, ExecHelloOk, ExecServerHello, ExecWireError, ServerFrame,
    ServerProtocolState,
};
use super::exec_registry::{ExecAdmission, ExecCancelReason, ExecRegistry, ExecReservation};
use super::exec_types::ExecStart;
use super::node::ShareIrohNode;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);

pub(super) async fn handle_connection(
    node: Arc<ShareIrohNode>,
    connection: Connection,
    handshake_permit: OwnedSemaphorePermit,
) -> io::Result<()> {
    let _incoming = node.track_incoming(&connection)?;
    let remote_node = connection.remote_id().to_string();
    // The server must write the fresh challenge first. Opening the stream here
    // makes that first write announce it to the client and avoids an empty-QUIC
    // stream deadlock during the initial Exec handshake.
    let (mut send, mut recv) = tokio::time::timeout(HANDSHAKE_TIMEOUT, connection.open_bi())
        .await
        .map_err(|_| timed_out("Exec-Handshake Timeout"))?
        .map_err(eio)?;
    let identity = node
        .auth
        .lock()
        .map_err(|_| eio("Share Exec authorization state is locked"))?
        .identity
        .clone();
    let server_hello = ExecServerHello::new(
        random_token(32).map_err(eio)?,
        identity.device_id,
        identity.public_key,
        identity.fingerprint,
        identity.node_id,
    );
    send_server_hello(&mut send, &server_hello).await?;
    let client_hello = tokio::time::timeout(HANDSHAKE_TIMEOUT, recv_client_hello(&mut recv))
        .await
        .map_err(|_| timed_out("Exec-Authentifizierung Timeout"))??;
    let authorized =
        match authorize_client_hello(&server_hello, &client_hello, &remote_node, &node.auth) {
            Ok(authorized) => authorized,
            Err(error) => {
                let denied = ExecWireError {
                    code: "permission_denied".into(),
                    message: "exec authentication failed".into(),
                };
                let _ = send_hello_error(&mut send, &denied).await;
                return Err(error);
            }
        };
    node.exec_registry()
        .apply_authorization(
            &authorized.principal,
            authorized.authorization.policy_revision,
            authorized.authorization.authorization_epoch,
            true,
        )
        .map_err(eio)?;
    let provider = tokio::task::spawn_blocking(super::exec_platform::provider_status)
        .await
        .map_err(eio)?;
    send_hello_ok(
        &mut send,
        &ExecHelloOk {
            authorization: authorized.authorization.clone(),
            provider: provider.clone(),
        },
    )
    .await?;
    drop(handshake_permit);
    if !provider.available {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("{}: {}", provider.provider, provider.detail),
        ));
    }

    let first = tokio::time::timeout(HANDSHAKE_TIMEOUT, recv_client_frame(&mut recv))
        .await
        .map_err(|_| timed_out("Exec-Start Timeout"))??;
    let mut protocol = ServerProtocolState::default();
    protocol.accept(&first)?;
    let ClientFrame::Start { start, .. } = first else {
        return Err(eio("Exec-Start fehlt"));
    };
    match node.exec_registry().prepare(
        authorized.principal,
        authorized.authorization,
        &start,
        now_secs(),
    ) {
        Ok(ExecAdmission::Prepared(reservation)) => {
            serve_job(
                node.exec_registry().clone(),
                start,
                reservation,
                send,
                recv,
                protocol,
            )
            .await
        }
        Ok(ExecAdmission::AlreadyRunning(view)) => {
            send_server_frame(
                &mut send,
                &ServerFrame::Error(ExecWireError {
                    code: "already_running".into(),
                    message: format!("execution {} is already running", view.exec_id),
                }),
            )
            .await
        }
        Ok(ExecAdmission::CachedTerminal(view)) => {
            let terminal = view
                .terminal
                .ok_or_else(|| eio("cached execution has no terminal result"))?;
            send_server_frame(&mut send, &ServerFrame::Terminal(terminal)).await
        }
        Err(error) => {
            send_server_frame(
                &mut send,
                &ServerFrame::Error(ExecWireError {
                    code: "admission_denied".into(),
                    message: error.to_string(),
                }),
            )
            .await
        }
    }
}

async fn serve_job(
    registry: Arc<ExecRegistry>,
    start: ExecStart,
    reservation: ExecReservation,
    mut send: iroh::endpoint::SendStream,
    mut recv: iroh::endpoint::RecvStream,
    mut protocol: ServerProtocolState,
) -> io::Result<()> {
    let exec_id = start.exec_id.clone();
    let (input_tx, input_rx) = mpsc::channel(16);
    let (output_tx, mut output_rx) = mpsc::channel(16);
    let worker_registry = registry.clone();
    let worker = tokio::task::spawn_blocking(move || {
        run_contained_job(worker_registry, start, reservation, input_rx, output_tx)
    });
    loop {
        tokio::select! {
            incoming = recv_client_frame(&mut recv) => {
                let frame = match incoming {
                    Ok(frame) => frame,
                    Err(error) => {
                        registry.cancel(&exec_id, ExecCancelReason::Disconnected);
                        drop(input_tx);
                        drop(output_rx);
                        let _ = worker.await;
                        return Err(error);
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
                        input_tx.send(JobInput::Stdin(data)).await.map_err(eio)?;
                    }
                    ClientFrame::StdinEof { .. } => {
                        input_tx.send(JobInput::StdinEof).await.map_err(eio)?;
                    }
                    ClientFrame::Cancel { .. } => {
                        registry.cancel(&exec_id, ExecCancelReason::User);
                    }
                    ClientFrame::Start { .. } => {
                        registry.cancel(&exec_id, ExecCancelReason::Disconnected);
                        return Err(eio("doppelter Exec-Start"));
                    }
                }
            }
            outgoing = output_rx.recv() => {
                let Some(frame) = outgoing else {
                    let result = worker.await.map_err(eio)?;
                    return result.and_then(|()| Err(eio("Exec-Worker endete ohne Terminalstatus")));
                };
                let terminal = matches!(frame, ServerFrame::Terminal(_) | ServerFrame::Error(_));
                if let Err(error) = send_server_frame(&mut send, &frame).await {
                    registry.cancel(&exec_id, ExecCancelReason::Disconnected);
                    drop(input_tx);
                    drop(output_rx);
                    let _ = worker.await;
                    return Err(error);
                }
                if terminal {
                    protocol.terminal();
                    drop(input_tx);
                    return worker.await.map_err(eio)?;
                }
            }
        }
    }
}

fn frame_exec_id(frame: &ClientFrame) -> &super::exec_types::ExecId {
    match frame {
        ClientFrame::Start { start, .. } => &start.exec_id,
        ClientFrame::Stdin { exec_id, .. }
        | ClientFrame::StdinEof { exec_id }
        | ClientFrame::Cancel { exec_id } => exec_id,
    }
}

fn timed_out(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, message)
}
