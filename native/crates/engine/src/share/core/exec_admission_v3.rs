//! Host side of `share-exec/3` multiplexed connections (plan 2026-11-14 E10,
//! D4 B3). Stream 0 authenticates the connection once with `v3|` transcript
//! separation; each subsequent client-opened bidirectional stream carries one
//! job and runs fresh authorization, grant/capability/approval checks, audit,
//! interactive presence and heartbeats from scratch.
use std::io;
use std::sync::Arc;

use iroh::endpoint::{Connection, RecvStream, SendStream, VarInt};
use tokio::task::JoinSet;

use super::super::{
    eio, finish_send_and_wait_until, finish_stream_ack, host_connections::HostStreamGuard,
    recv_client_frame, send_hello_error, send_hello_ok, timed_out, ApplicationHandshakePermit,
    ClientFrame, ExecHelloOk, ExecWireError, ServerProtocolState, ShareIrohNode, StreamEnd,
    EXEC_HEARTBEAT_POLICY, HANDSHAKE_TIMEOUT,
};
use super::admit_and_serve_start;
use super::approval::ApprovedStart;
use crate::share::access_audit::{self, AccessEntry, AccessEvent, AccessKind};
use crate::share::exec_access_view::{host_access_features, ExecAccessView, HelloOkWithAccess};
use crate::share::exec_audit::{self, AuditEvent};
use crate::share::exec_auth::{authorize_stream, AuthorizedExecPeer};
use crate::share::exec_client_messages::{
    account_helper_unavailable, account_not_allowed, permission_denied, ACCOUNT_HELPER_UNAVAILABLE,
    ACCOUNT_NOT_ALLOWED, PERMISSION_DENIED,
};
use crate::share::exec_helper_protocol::ACCOUNT_UNAVAILABLE;
use crate::share::exec_terminal::host_features;
use crate::share::exec_types::{ExecProviderStatus, ExecStart};
use crate::share::relation_proof::ProvenPrincipal;

pub(super) struct AdmittedConnection<'a> {
    pub(super) node: &'a Arc<ShareIrohNode>,
    pub(super) connection: &'a Connection,
    pub(super) host: &'a str,
    pub(super) client_features: &'a [String],
    pub(super) provider: &'a ExecProviderStatus,
    pub(super) terminal_supported: bool,
}

pub(super) async fn probe_host_provider(
    #[allow(unused_variables)] node: &ShareIrohNode,
    deadline: tokio::time::Instant,
) -> io::Result<(ExecProviderStatus, bool)> {
    #[cfg(test)]
    if let Some(runner) = node.exec_host.test_runner() {
        return Ok(runner.provider());
    }
    crate::share::route_protocol::timeout_until(
        deadline,
        tokio::task::spawn_blocking(|| {
            (
                crate::share::exec_platform::provider_status(),
                crate::share::exec_platform::terminal_supported(),
            )
        }),
    )
    .await
    .map_err(|_| timed_out("Exec-Providerpruefung Timeout"))?
    .map_err(eio)
}

pub(super) fn build_hello_ok(
    authorized: &AuthorizedExecPeer,
    provider: &ExecProviderStatus,
    terminal_supported: bool,
) -> (ExecHelloOk, ExecAccessView) {
    let mut features = host_features(terminal_supported);
    features.push("route_policy_v1".into());
    features.push(crate::share::exec_shell::EXEC_SHELL_FEATURE.into());
    features.extend(host_access_features().map(String::from));
    let hello_ok = ExecHelloOk {
        authorization: authorized.authorization.clone(),
        provider: provider.clone(),
        features,
    };
    let access = ExecAccessView::of(&authorized.grant, &provider.user_label);
    (hello_ok, access)
}

pub(super) async fn serve_v3_connection(
    node: Arc<ShareIrohNode>,
    connection: Connection,
    handshake_permit: ApplicationHandshakePermit,
    mut send: SendStream,
    authorized: AuthorizedExecPeer,
    provider: ExecProviderStatus,
    client_features: Vec<String>,
    host: String,
) -> io::Result<()> {
    drop(handshake_permit);
    if !provider.available {
        finish_send_and_wait_until(
            &mut send,
            &connection,
            crate::share::route_protocol::active_now()
                + EXEC_HEARTBEAT_POLICY.server_result_ack_timeout(),
        )
        .await?;
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("{}: {}", provider.provider, provider.detail),
        ));
    }
    send.finish().map_err(eio)?;
    let proven = ProvenPrincipal::from(&authorized.principal);
    let host_guard = match node.exec_host.register(&node.auth, &connection, proven) {
        Ok(guard) => guard,
        Err(rejection) => {
            connection.close(VarInt::from_u32(0x5345), b"authorization changed");
            return Err(rejection.into());
        }
    };
    let client_features: Arc<[String]> = Arc::from(client_features);
    let host: Arc<str> = Arc::from(host);
    let mut streams = JoinSet::new();
    loop {
        enum NextAccept {
            Accepted(Result<(SendStream, RecvStream), iroh::endpoint::ConnectionError>),
            Reaped,
            Closed,
        }
        let next = tokio::select! {
            accepted = connection.accept_bi() => NextAccept::Accepted(accepted),
            Some(_) = streams.join_next(), if !streams.is_empty() => NextAccept::Reaped,
            _ = connection.closed() => NextAccept::Closed,
        };
        let (stream_send, stream_recv) = match next {
            NextAccept::Accepted(Ok(pair)) => pair,
            NextAccept::Reaped => continue,
            NextAccept::Accepted(Err(_)) | NextAccept::Closed => break,
        };
        if node.require_sharing_active().is_err() {
            break;
        }
        let Some(stream_guard) = host_guard.open_stream() else {
            drop((stream_send, stream_recv));
            continue;
        };
        let stream_node = Arc::clone(&node);
        let stream_conn = connection.clone();
        let stream_proven = host_guard.principal().clone();
        let stream_features = Arc::clone(&client_features);
        let stream_host = Arc::clone(&host);
        streams.spawn(async move {
            let _ = crate::share::route_protocol::scope(
                stream_conn.clone(),
                handle_v3_stream(
                    stream_node,
                    stream_conn,
                    stream_proven,
                    stream_features,
                    stream_host,
                    stream_send,
                    stream_recv,
                    stream_guard,
                ),
            )
            .await;
        });
    }
    while streams.join_next().await.is_some() {}
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_v3_stream(
    node: Arc<ShareIrohNode>,
    connection: Connection,
    proven: ProvenPrincipal,
    client_features: Arc<[String]>,
    host: Arc<str>,
    mut send: SendStream,
    mut recv: RecvStream,
    stream_guard: HostStreamGuard,
) -> io::Result<()> {
    node.require_sharing_active()?;
    let first =
        crate::share::route_protocol::timeout_for(HANDSHAKE_TIMEOUT, recv_client_frame(&mut recv))
            .await
            .map_err(|_| timed_out("Exec-Start Timeout"))??;
    let mut protocol = ServerProtocolState::default();
    protocol.accept(&first)?;
    let ClientFrame::Start { start, .. } = first else {
        return Err(eio("Exec-Start fehlt"));
    };
    let deadline = crate::share::route_protocol::active_now() + HANDSHAKE_TIMEOUT;
    let remote_node = connection.remote_id().to_string();
    let authorized = match authorize_stream(&proven, &node.auth) {
        Ok(authorized) => authorized,
        Err(rejection) => {
            exec_audit::record(AuditEvent::AuthRejected {
                remote_node: &remote_node,
                claimed_device: &proven.device_id,
                reason: rejection.audit_reason(),
            });
            let denied = ExecWireError {
                code: PERMISSION_DENIED.into(),
                message: permission_denied(&host),
            };
            let _ = crate::share::route_protocol::run_until_active(
                deadline,
                "Exec-Ablehnung Timeout",
                send_hello_error(&mut send, &denied),
            )
            .await;
            let ack_deadline = crate::share::route_protocol::active_now()
                + EXEC_HEARTBEAT_POLICY.server_result_ack_timeout();
            let _ = finish_stream_ack(&mut send, stream_guard, ack_deadline).await;
            return Err(rejection.into());
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
    let (provider, terminal_supported) = probe_host_provider(&node, deadline).await?;
    let (hello_ok, access) = build_hello_ok(&authorized, &provider, terminal_supported);
    crate::share::route_protocol::run_until_active(
        deadline,
        "Exec-HelloOk Timeout",
        send_hello_ok(
            &mut send,
            &HelloOkWithAccess {
                hello: &hello_ok,
                access: Some(&access),
            },
        ),
    )
    .await?;
    if !provider.available {
        let ack_deadline = crate::share::route_protocol::active_now()
            + EXEC_HEARTBEAT_POLICY.server_result_ack_timeout();
        finish_stream_ack(&mut send, stream_guard, ack_deadline).await?;
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("{}: {}", provider.provider, provider.detail),
        ));
    }
    admit_and_serve_start(
        AdmittedConnection {
            node: &node,
            connection: &connection,
            host: &host,
            client_features: &client_features,
            provider: &provider,
            terminal_supported,
        },
        authorized,
        start,
        protocol,
        send,
        recv,
        StreamEnd::FinishStream(stream_guard),
    )
    .await
}

pub(super) fn account_refusal(error: &io::Error, name: &str, host: &str) -> ExecWireError {
    let code =
        crate::share::exec_platform::account_error_code(error).unwrap_or(ACCOUNT_UNAVAILABLE);
    let message = match code {
        ACCOUNT_NOT_ALLOWED => account_not_allowed(host, name),
        ACCOUNT_HELPER_UNAVAILABLE => account_helper_unavailable(host, name, &error.to_string()),
        _ => format!("run-as account {name:?} of this device: {error}"),
    };
    ExecWireError {
        code: code.into(),
        message,
    }
}

pub(super) fn record_helper_refusal(
    authorized: &AuthorizedExecPeer,
    approved: &ApprovedStart,
    start: &ExecStart,
    reason: &str,
) {
    let access = approved.decided.grant.access_grant();
    let principal = ProvenPrincipal::from(&authorized.principal);
    access_audit::record_access(AccessEntry {
        kind: AccessKind::Exec { start: Some(start) },
        principal: &principal,
        outcome: Some(AccessEvent::HelperRefused),
        capability: approved.decided.capability,
        approval: Some(approved.approval),
        approval_id: None,
        account_profile: Some(access.account.key()),
        account: access.account.account_name(),
        reason: Some(reason),
    });
}
