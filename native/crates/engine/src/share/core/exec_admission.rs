//! Connection authentication and job admission before Exec streaming. The
//! handshake admits a device whose commands or terminal are not off; each
//! start is then checked against the grant of that moment for the
//! capability it needs (C1, D.2.2). A capability on `ask` waits for the host
//! user first (PRM-M3, `access_approval_exec.rs`), and open confirmations
//! count against the Exec limits with the running jobs (C5). A run-as
//! account the host refuses is answered with the code the platform gave the
//! refusal (D.2.2) and logged as `helper_refused`.
use super::super::access_audit::{self, AccessEntry};
use super::super::auth_penalty::PenaltyKind;
use super::super::consent_launch;
use super::super::exec_access_view::HelloOkWithAccess;
use super::super::exec_audit::{self, AuditEvent, StartAccess};
use super::super::exec_auth::AuthorizedExecPeer;
use super::super::exec_client_messages::{permission_denied, PERMISSION_DENIED};
use super::super::exec_helper_protocol::ACCOUNT_UNAVAILABLE;
use super::super::exec_policy::ExecGrant;
use super::super::exec_supervisor_protocol::RunAsAccount;
use super::super::exec_terminal::host_denial;
use super::super::exec_types::{ExecProviderStatus, ExecStart};
use super::super::relation_proof::ProvenPrincipal;
use super::*;

// The decision about one start, with the host user's answer on `ask`.
#[path = "access_approval_exec.rs"]
pub(super) mod approval;
#[path = "exec_admission_v3.rs"]
mod v3;
#[cfg(any(test, feature = "test-support"))]
pub(in crate::share) use approval::start_grant;
use approval::{decide_start, Refusal, StartApproval};
use v3::{
    account_refusal, build_hello_ok, probe_host_provider, record_helper_refusal, AdmittedConnection,
};

/// How often the grant is read again while the host user decides.
const GRANT_POLL: Duration = Duration::from_secs(1);

pub(in crate::share) async fn handle_connection(
    node: Arc<ShareIrohNode>,
    connection: Connection,
    handshake_permit: ApplicationHandshakePermit,
) -> io::Result<()> {
    let version = exec_version_for_alpn(connection.alpn())
        .ok_or_else(|| eio("Unbekanntes Exec-Protokoll"))?;
    let _incoming = node.track_incoming(&connection)?;
    node.require_sharing_active()?;
    node.await_route(&connection, None, None).await?;
    let remote_node = connection.remote_id().to_string();
    let handshake_deadline = crate::share::route_protocol::active_now() + HANDSHAKE_TIMEOUT;
    // The server must write the fresh challenge first. Opening the stream here
    // makes that first write announce it to the client and avoids an empty-QUIC
    // stream deadlock during the initial Exec handshake.
    let (mut send, mut recv) =
        crate::share::route_protocol::timeout_until(handshake_deadline, connection.open_bi())
            .await
            .map_err(|_| timed_out("Exec-Handshake Timeout"))?
            .map_err(eio)?;
    let identity = node
        .auth
        .lock()
        .map_err(|_| eio("Share Exec authorization state is locked"))?
        .identity
        .clone();
    // How refusals name this host to the connecting device.
    let host = if identity.device_name.trim().is_empty() {
        identity.device_id.clone()
    } else {
        identity.device_name.clone()
    };
    let base_hello = ExecServerHello::new(
        random_token(32).map_err(eio)?,
        identity.device_id,
        identity.public_key,
        identity.fingerprint,
        identity.node_id,
    );
    let server_hello = if version == EXEC_PROTOCOL_VERSION_V3 {
        base_hello.into_v3()
    } else {
        base_hello
    };
    crate::share::route_protocol::run_until_active(
        handshake_deadline,
        "Exec-ServerHello Timeout",
        send_server_hello(&mut send, &server_hello),
    )
    .await?;
    let client_hello = match crate::share::route_protocol::run_until_active(
        handshake_deadline,
        "Exec-Authentifizierung Timeout",
        recv_client_hello(&mut recv),
    )
    .await
    {
        Ok(hello) => hello,
        Err(error) => {
            node.auth_penalties().record(
                &remote_node,
                PenaltyKind::NoAuthentication,
                std::time::Instant::now(),
            );
            return Err(error);
        }
    };
    let authorized = match authorize_client_hello_for(
        version,
        &server_hello,
        &client_hello,
        &remote_node,
        &node.auth,
    ) {
        Ok(authorized) => authorized,
        Err(rejection) => {
            if rejection.counts_as_failed_authentication() {
                node.auth_penalties().record(
                    &remote_node,
                    PenaltyKind::AuthFailure,
                    std::time::Instant::now(),
                );
            }
            exec_audit::record(AuditEvent::AuthRejected {
                remote_node: &remote_node,
                claimed_device: &client_hello.device_id,
                reason: rejection.audit_reason(),
            });
            let error = io::Error::from(rejection);
            let denied = ExecWireError {
                code: PERMISSION_DENIED.into(),
                message: permission_denied(&host),
            };
            if crate::share::route_protocol::run_until_active(
                handshake_deadline,
                "Exec-Ablehnung Timeout",
                send_hello_error(&mut send, &denied),
            )
            .await
            .is_ok()
            {
                let _ =
                    finish_send_and_wait_until(&mut send, &connection, handshake_deadline).await;
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
    let (provider, terminal_supported) = probe_host_provider(&node, handshake_deadline).await?;
    crate::share::route_monitor::authenticated_remote_capability(
        &connection,
        client_hello
            .client_features
            .iter()
            .any(|feature| feature == "route_policy_v1"),
    );
    let (hello_ok, access) = build_hello_ok(&authorized, &provider, terminal_supported);
    crate::share::route_protocol::run_until_active(
        handshake_deadline,
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
    if version == EXEC_PROTOCOL_VERSION_V3 {
        return v3::serve_v3_connection(
            node,
            connection,
            handshake_permit,
            send,
            authorized,
            provider,
            client_hello.client_features,
            host,
        )
        .await;
    }
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

    let first =
        crate::share::route_protocol::timeout_for(HANDSHAKE_TIMEOUT, recv_client_frame(&mut recv))
            .await
            .map_err(|_| timed_out("Exec-Start Timeout"))??;
    let mut protocol = ServerProtocolState::default();
    protocol.accept(&first)?;
    let ClientFrame::Start { start, .. } = first else {
        return Err(eio("Exec-Start fehlt"));
    };
    let end_mode = StreamEnd::CloseConnection(connection.clone());
    admit_and_serve_start(
        AdmittedConnection {
            node: &node,
            connection: &connection,
            host: &host,
            client_features: &client_hello.client_features,
            provider: &provider,
            terminal_supported,
        },
        authorized,
        start,
        protocol,
        send,
        recv,
        end_mode,
    )
    .await
}

pub(super) async fn admit_and_serve_start(
    ctx: AdmittedConnection<'_>,
    authorized: AuthorizedExecPeer,
    start: ExecStart,
    mut protocol: ServerProtocolState,
    mut send: iroh::endpoint::SendStream,
    mut recv: iroh::endpoint::RecvStream,
    end_mode: StreamEnd,
) -> io::Result<()> {
    // A Share folder becomes this device's disk folder before anyone sees
    // or approves the start (`exec_host_options.rs`).
    let (start, denial) = match super::exec_types::host_options::resolve_on_host(
        start.clone(),
        &ctx.node.auth,
        &authorized.principal,
    ) {
        Ok(resolved) => (resolved, None),
        Err(error) => (start, Some(error)),
    };
    if let Some(denial) = denial.or_else(|| host_denial(&start, ctx.terminal_supported)) {
        exec_audit::record(AuditEvent::StartRefused {
            principal: &authorized.principal,
            start: &start,
            reason: &denial.message,
        });
        protocol.terminal();
        send_terminal_frame(&mut send, &ServerFrame::Error(denial)).await?;
        wait_result_ack(&mut recv, &mut protocol, &start.exec_id).await?;
        return acknowledge_result(&mut send, &start.exec_id, end_mode).await;
    }
    let mut stream = ClientStream::new(exec_frame_reader::client_frames(recv));
    let approvals = consent_launch::approvals();
    let registry = ctx.node.exec_registry().clone();
    let record = |entry: AccessEntry<'_>| access_audit::record_access(entry);
    let approval = StartApproval {
        service: approvals,
        auth: &ctx.node.auth,
        authorized: &authorized,
        start: &start,
        host: ctx.host,
        client_features: ctx.client_features,
        own_label: &ctx.provider.user_label,
        audit: &record,
        grant_poll: GRANT_POLL,
    };
    let decided = decide_start(
        &approval,
        &mut send,
        &mut stream,
        &mut protocol,
        EXEC_HEARTBEAT_POLICY,
    )
    .await?;
    let checked = match decided {
        Ok(approved) => {
            match grant_launch(&approved.decided.grant, &start, ctx.provider, ctx.host).await {
                Ok(launch) => Ok((approved, launch)),
                Err(refusal) => {
                    if refusal.account {
                        record_helper_refusal(
                            &authorized,
                            &approved,
                            &start,
                            &refusal.error.message,
                        );
                    }
                    Err(Refusal::start(refusal.error))
                }
            }
        }
        Err(refusal) => Err(refusal),
    };
    let (approved, launch) = match checked {
        Ok(checked) => checked,
        Err(refusal) => {
            if refusal.start_refused {
                exec_audit::record(AuditEvent::StartRefused {
                    principal: &authorized.principal,
                    start: &start,
                    reason: &refusal.error.message,
                });
            }
            let frame = ServerFrame::Error(refusal.error);
            return answer_without_job(
                &mut send,
                &mut stream,
                &mut protocol,
                &start,
                end_mode,
                frame,
            )
            .await;
        }
    };
    let decided = approved.decided;
    exec_audit::note_start(
        &start.exec_id,
        StartAccess {
            capability: decided.capability,
            account_profile: decided.grant.access_grant().account.key(),
            approval: approved.approval.key(),
        },
    );
    let grant_end = GrantEndWatch::new(
        ctx.node.auth.clone(),
        ProvenPrincipal::from(&authorized.principal),
        decided.capability,
        decided.end,
    );
    let admission = registry.prepare(
        authorized.principal,
        decided.authorization,
        &start,
        now_secs(),
    );
    if !matches!(admission, Ok(ExecAdmission::Prepared(_))) {
        exec_audit::forget_start(&start.exec_id);
    }
    let frame = match admission {
        Ok(ExecAdmission::Prepared(reservation)) => {
            return serve_job(
                registry,
                JobLaunch {
                    start,
                    reservation,
                    account: launch.account,
                    account_label: launch.account_label,
                    program: launch.program,
                    #[cfg(test)]
                    test_runner: ctx.node.exec_host.test_runner(),
                },
                send,
                stream,
                protocol,
                ctx.connection.clone(),
                end_mode,
                grant_end,
            )
            .await;
        }
        Ok(ExecAdmission::AlreadyRunning(view)) => ServerFrame::Error(ExecWireError {
            code: "already_running".into(),
            message: format!("execution {} is already running", view.exec_id),
        }),
        Ok(ExecAdmission::CachedTerminal(view)) => ServerFrame::Terminal(
            view.terminal
                .ok_or_else(|| eio("cached execution has no terminal result"))?,
        ),
        Err(error) => ServerFrame::Error(ExecWireError {
            code: "admission_denied".into(),
            message: error.to_string(),
        }),
    };
    answer_without_job(
        &mut send,
        &mut stream,
        &mut protocol,
        &start,
        end_mode,
        frame,
    )
    .await
}

async fn answer_without_job(
    send: &mut iroh::endpoint::SendStream,
    stream: &mut ClientStream,
    protocol: &mut ServerProtocolState,
    start: &ExecStart,
    end_mode: StreamEnd,
    frame: ServerFrame,
) -> io::Result<()> {
    protocol.terminal();
    send_terminal_frame(send, &frame).await?;
    wait_result_ack_from_reader(&mut stream.frames, protocol, &start.exec_id).await?;
    acknowledge_result(send, &start.exec_id, end_mode).await
}

struct GrantLaunch {
    account: Option<RunAsAccount>,
    account_label: String,
    program: Option<std::path::PathBuf>,
}

struct LaunchRefusal {
    error: ExecWireError,
    account: bool,
}

async fn grant_launch(
    grant: &ExecGrant,
    start: &ExecStart,
    provider: &ExecProviderStatus,
    host: &str,
) -> Result<GrantLaunch, LaunchRefusal> {
    let checked_grant = grant.clone();
    let checked = start.clone();
    let program = tokio::task::spawn_blocking(move || {
        checked_grant.authorize_start(&checked, &super::super::exec_platform::program_search())
    })
    .await
    .map_err(|_| refused("the program check of this host did not complete"))?
    .map_err(refused)?;
    let run_as = grant.run_as.clone();
    let name = run_as.account_name().unwrap_or("standard").to_string();
    let resolved =
        tokio::task::spawn_blocking(move || super::super::exec_platform::resolve_run_as(&run_as))
            .await
            .map_err(|error| LaunchRefusal {
                error: ExecWireError {
                    code: ACCOUNT_UNAVAILABLE.into(),
                    message: error.to_string(),
                },
                account: false,
            })?;
    match resolved {
        Ok(Some(account)) => Ok(GrantLaunch {
            account_label: format!("{} (uid {})", account.name, account.uid),
            account: Some(account),
            program,
        }),
        Ok(None) => Ok(GrantLaunch {
            account: None,
            account_label: format!("standard ({})", provider.user_label),
            program,
        }),
        Err(error) => Err(LaunchRefusal {
            error: account_refusal(&error, &name, host),
            account: true,
        }),
    }
}

fn refused(message: impl Into<String>) -> LaunchRefusal {
    LaunchRefusal {
        error: ExecWireError {
            code: "program_not_permitted".into(),
            message: message.into(),
        },
        account: false,
    }
}
