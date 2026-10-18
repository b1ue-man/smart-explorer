//! Connection authentication and job admission before Exec streaming. The
//! handshake admits a device whose commands or terminal are not off; each
//! start is then checked against the grant of that moment for the
//! capability it needs (C1, D.2.2). A capability on `ask` waits for the host
//! user first (PRM-M3, `access_approval_exec.rs`), and open confirmations
//! count against the Exec limits with the running jobs (C5). A run-as
//! account the host refuses is answered with the code the platform gave the
//! refusal (D.2.2) and logged as `helper_refused`.
use super::super::access_audit::{self, AccessEntry, AccessEvent, AccessKind};
use super::super::auth_penalty::PenaltyKind;
use super::super::consent_launch;
use super::super::exec_access_view::{host_access_features, ExecAccessView, HelloOkWithAccess};
use super::super::exec_audit::{self, AuditEvent, StartAccess};
use super::super::exec_auth::AuthorizedExecPeer;
use super::super::exec_client_messages::{
    account_helper_unavailable, account_not_allowed, permission_denied, ACCOUNT_HELPER_UNAVAILABLE,
    ACCOUNT_NOT_ALLOWED, PERMISSION_DENIED,
};
use super::super::exec_helper_protocol::ACCOUNT_UNAVAILABLE;
use super::super::exec_policy::ExecGrant;
use super::super::exec_supervisor_protocol::RunAsAccount;
use super::super::exec_terminal::{host_denial, host_features};
use super::super::exec_types::{ExecProviderStatus, ExecStart};
use super::super::relation_proof::ProvenPrincipal;
use super::*;

// The decision about one start, with the host user's answer on `ask`.
#[path = "access_approval_exec.rs"]
mod approval;
#[cfg(test)]
pub(in crate::share) use approval::start_grant;
use approval::{decide_start, limit_error, ApprovedStart, Refusal, StartApproval};

/// How often the grant is read again while the host user decides.
const GRANT_POLL: Duration = Duration::from_secs(1);

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
    // How refusals name this host to the connecting device.
    let host = if identity.device_name.trim().is_empty() {
        identity.device_id.clone()
    } else {
        identity.device_name.clone()
    };
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
    let client_hello = match io_deadline::run_until(
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
    let authorized =
        match authorize_client_hello(&server_hello, &client_hello, &remote_node, &node.auth) {
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
    let (provider, terminal_supported) = tokio::time::timeout_at(
        handshake_deadline,
        tokio::task::spawn_blocking(|| {
            (
                super::super::exec_platform::provider_status(),
                super::super::exec_platform::terminal_supported(),
            )
        }),
    )
    .await
    .map_err(|_| timed_out("Exec-Providerpruefung Timeout"))?
    .map_err(eio)?;
    let mut features = host_features(terminal_supported);
    features.extend(host_access_features().map(String::from));
    let hello_ok = ExecHelloOk {
        authorization: authorized.authorization.clone(),
        provider: provider.clone(),
        features,
    };
    let access = ExecAccessView::of(&authorized.grant, &provider.user_label);
    io_deadline::run_until(
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
    // A terminal start is answered like a denied admission: no job is
    // prepared and no process exists.
    if let Some(denial) = host_denial(&start, terminal_supported) {
        exec_audit::record(AuditEvent::StartRefused {
            principal: &authorized.principal,
            start: &start,
            reason: &denial.message,
        });
        protocol.terminal();
        send_terminal_frame(&mut send, &ServerFrame::Error(denial)).await?;
        wait_result_ack(&mut recv, &mut protocol, &start.exec_id).await?;
        return acknowledge_result(&mut send, &start.exec_id, &connection).await;
    }
    // From here the client stream has its reader: a start on `ask` waits for
    // the host user while heartbeats, early input and a cancel arrive.
    let mut stream = ClientStream::new(exec_frame_reader::client_frames(recv));
    let approvals = consent_launch::approvals();
    let registry = node.exec_registry().clone();
    let record = |entry: AccessEntry<'_>| access_audit::record_access(entry);
    let approval = StartApproval {
        service: approvals,
        gauge: registry.as_ref(),
        auth: &node.auth,
        authorized: &authorized,
        start: &start,
        host: &host,
        client_features: &client_hello.client_features,
        own_label: &provider.user_label,
        audit: &record,
        grant_poll: GRANT_POLL,
    };
    // The capability the start needs (on `ask` with the host user's answer),
    // the grant's program list and its run-as account are host decisions
    // made before any process exists; a refusal is answered like a denied
    // admission.
    let decided = decide_start(
        &approval,
        &mut send,
        &mut stream,
        &mut protocol,
        EXEC_HEARTBEAT_POLICY,
    )
    .await?;
    let checked = match decided {
        Ok(approved) => match grant_launch(&approved.decided.grant, &start, &provider, &host).await
        {
            Ok(launch) => Ok((approved, launch)),
            Err(refusal) => {
                if refusal.account {
                    record_helper_refusal(&authorized, &approved, &start, &refusal.error.message);
                }
                Err(Refusal::start(refusal.error))
            }
        },
        Err(refusal) => Err(refusal),
    };
    let (approved, launch) = match checked {
        Ok(checked) => checked,
        Err(refusal) => {
            // The host user's answers have entries of their own.
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
                &connection,
                frame,
            )
            .await;
        }
    };
    // Open confirmations hold reservations as well (C5).
    if let Err(limit) = approvals.admits(registry.as_ref(), &authorized.principal.device_id) {
        let frame = ServerFrame::Error(limit_error(limit));
        return answer_without_job(
            &mut send,
            &mut stream,
            &mut protocol,
            &start,
            &connection,
            frame,
        )
        .await;
    }
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
        node.auth.clone(),
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
                },
                send,
                stream,
                protocol,
                connection.clone(),
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
        &connection,
        frame,
    )
    .await
}

/// Answers a start that gets no job (a refusal or a cached result) and
/// completes the result handshake on the client stream's reader.
async fn answer_without_job(
    send: &mut iroh::endpoint::SendStream,
    stream: &mut ClientStream,
    protocol: &mut ServerProtocolState,
    start: &ExecStart,
    connection: &Connection,
    frame: ServerFrame,
) -> io::Result<()> {
    protocol.terminal();
    send_terminal_frame(send, &frame).await?;
    wait_result_ack_from_reader(&mut stream.frames, protocol, &start.exec_id).await?;
    acknowledge_result(send, &start.exec_id, connection).await
}

/// What the grant decided about one start.
struct GrantLaunch {
    account: Option<RunAsAccount>,
    /// The account as the audit log names it.
    account_label: String,
    /// The program file of a grant with a program list.
    program: Option<std::path::PathBuf>,
}

/// Why the grant refused a start before any job process exists.
struct LaunchRefusal {
    error: ExecWireError,
    /// This host refused the grant's run-as account (`helper_refused`).
    account: bool,
}

/// Applies the grant to `start`: its program list and environment rule may
/// refuse it (file system lookups, so on the blocking pool), and its run-as
/// account is resolved (user database lookups and the account service block
/// as well). Both are host decisions made before any job process exists.
/// `host` names this device in the refusal texts (C6).
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

/// The refusal of a run-as account: the code the platform gave it
/// (`account_not_allowed`, `account_helper_unavailable`, otherwise
/// `account_unavailable`) with the text and next step of C6.
fn account_refusal(error: &io::Error, name: &str, host: &str) -> ExecWireError {
    let code =
        super::super::exec_platform::account_error_code(error).unwrap_or(ACCOUNT_UNAVAILABLE);
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

/// `helper_refused` (C5): this host refused the run-as account of a start.
fn record_helper_refusal(
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

fn refused(message: impl Into<String>) -> LaunchRefusal {
    LaunchRefusal {
        error: ExecWireError {
            code: "program_not_permitted".into(),
            message: message.into(),
        },
        account: false,
    }
}
