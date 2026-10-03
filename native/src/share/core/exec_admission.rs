//! Connection authentication and job admission before Exec streaming.
use super::*;

pub(in crate::share) async fn handle_connection(
    node: Arc<ShareIrohNode>,
    connection: Connection,
    handshake_permit: ApplicationHandshakePermit,
) -> io::Result<()> {
    let _incoming = node.track_incoming(&connection)?;
    node.require_sharing_active()?;
    let remote_node = connection.remote_id().to_string();
    let handshake_deadline = tokio::time::Instant::now() + HANDSHAKE_TIMEOUT;
    // The server must write the fresh challenge first. Opening the stream here
    // makes that first write announce it to the client and avoids an empty-QUIC
    // stream deadlock during the initial Exec handshake.
    let (mut send, mut recv) = tokio::time::timeout_at(handshake_deadline, connection.open_bi())
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
    io_deadline::run_until(
        handshake_deadline,
        "Exec-ServerHello Timeout",
        send_server_hello(&mut send, &server_hello),
    )
    .await?;
    let client_hello = io_deadline::run_until(
        handshake_deadline,
        "Exec-Authentifizierung Timeout",
        recv_client_hello(&mut recv),
    )
    .await?;
    let authorized =
        match authorize_client_hello(&server_hello, &client_hello, &remote_node, &node.auth) {
            Ok(authorized) => authorized,
            Err(error) => {
                let denied = ExecWireError {
                    code: "permission_denied".into(),
                    message: "exec authentication failed".into(),
                };
                if io_deadline::run_until(
                    handshake_deadline,
                    "Exec-Ablehnung Timeout",
                    send_hello_error(&mut send, &denied),
                )
                .await
                .is_ok()
                {
                    let _ = finish_send_and_wait_until(&mut send, &connection, handshake_deadline)
                        .await;
                }
                return Err(error);
            }
        };
    node.bind_incoming_principal(
        &connection,
        super::super::session::PeerPrincipal::from_exec(&authorized.principal),
    )
    .await?;
    node.exec_registry()
        .apply_authorization(
            &authorized.principal,
            authorized.authorization.policy_revision,
            authorized.authorization.authorization_epoch,
            true,
        )
        .map_err(eio)?;
    let provider = tokio::time::timeout_at(
        handshake_deadline,
        tokio::task::spawn_blocking(super::super::exec_platform::provider_status),
    )
    .await
    .map_err(|_| timed_out("Exec-Providerpruefung Timeout"))?
    .map_err(eio)?;
    io_deadline::run_until(
        handshake_deadline,
        "Exec-HelloOk Timeout",
        send_hello_ok(
            &mut send,
            &ExecHelloOk {
                authorization: authorized.authorization.clone(),
                provider: provider.clone(),
            },
        ),
    )
    .await?;
    drop(handshake_permit);
    if !provider.available {
        finish_send_and_wait_until(
            &mut send,
            &connection,
            tokio::time::Instant::now() + EXEC_HEARTBEAT_POLICY.server_result_ack_timeout(),
        )
        .await?;
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
                connection.clone(),
            )
            .await
        }
        Ok(ExecAdmission::AlreadyRunning(view)) => {
            protocol.terminal();
            send_terminal_frame(
                &mut send,
                &ServerFrame::Error(ExecWireError {
                    code: "already_running".into(),
                    message: format!("execution {} is already running", view.exec_id),
                }),
            )
            .await?;
            wait_result_ack(&mut recv, &mut protocol, &start.exec_id).await?;
            acknowledge_result(&mut send, &start.exec_id, &connection).await
        }
        Ok(ExecAdmission::CachedTerminal(view)) => {
            let terminal = view
                .terminal
                .ok_or_else(|| eio("cached execution has no terminal result"))?;
            protocol.terminal();
            send_terminal_frame(&mut send, &ServerFrame::Terminal(terminal)).await?;
            wait_result_ack(&mut recv, &mut protocol, &start.exec_id).await?;
            acknowledge_result(&mut send, &start.exec_id, &connection).await
        }
        Err(error) => {
            protocol.terminal();
            send_terminal_frame(
                &mut send,
                &ServerFrame::Error(ExecWireError {
                    code: "admission_denied".into(),
                    message: error.to_string(),
                }),
            )
            .await?;
            wait_result_ack(&mut recv, &mut protocol, &start.exec_id).await?;
            acknowledge_result(&mut send, &start.exec_id, &connection).await
        }
    }
}
