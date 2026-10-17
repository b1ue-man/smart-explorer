//! Connection authentication and job admission before Exec streaming. The
//! handshake admits a device whose commands or terminal are not off; each
//! start is then checked against the grant of that moment for the
//! capability it needs (C1, D.2.2).
use super::super::access_authorize::{capability_for, principal_grant, start_capability};
use super::super::access_grant::{Capability, CapabilityDecision};
use super::super::auth_penalty::PenaltyKind;
use super::super::exec_access_view::{host_access_features, ExecAccessView, HelloOkWithAccess};
use super::super::exec_audit::{self, AuditEvent};
use super::super::exec_auth::AuthorizedExecPeer;
use super::super::exec_client_messages::{
    approval_unavailable, permission_denied, right_not_granted, APPROVAL_UNAVAILABLE,
    PERMISSION_DENIED, RIGHT_NOT_GRANTED,
};
use super::super::exec_policy::ExecGrant;
use super::super::exec_supervisor_protocol::RunAsAccount;
use super::super::exec_terminal::{host_denial, host_features};
use super::super::exec_types::{ExecAuthorization, ExecProviderStatus, ExecStart};
use super::super::relation_proof::ProvenPrincipal;
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
    // The capability the start needs, the grant's program list and its
    // run-as account are host decisions made before any process exists; a
    // refusal is answered like a denied admission.
    let checked = match start_grant(&node.auth, &authorized, &start, &host, now_secs()) {
        Ok(decided) => grant_launch(&decided.grant, &start, &provider)
            .await
            .map(|launch| (decided, launch)),
        Err(denial) => Err(denial),
    };
    let (decided, launch) = match checked {
        Ok(checked) => checked,
        Err(denial) => {
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
    };
    let grant_end = GrantEndWatch::new(
        node.auth.clone(),
        ProvenPrincipal::from(&authorized.principal),
        decided.capability,
        decided.end,
    );
    match node.exec_registry().prepare(
        authorized.principal,
        decided.authorization,
        &start,
        now_secs(),
    ) {
        Ok(ExecAdmission::Prepared(reservation)) => {
            serve_job(
                node.exec_registry().clone(),
                JobLaunch {
                    start,
                    reservation,
                    account: launch.account,
                    account_label: launch.account_label,
                    program: launch.program,
                },
                send,
                ClientStream::new(exec_frame_reader::client_frames(recv)),
                protocol,
                connection.clone(),
                grant_end,
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

/// The grant at the start and what it decides for the start's capability.
pub(in crate::share) struct StartGrant {
    pub(in crate::share) grant: ExecGrant,
    /// The handshake's authorization at the grant's current revision: a
    /// grant that only widened since keeps the connection's authorization;
    /// a narrowing raised the registry's barrier above its epoch.
    pub(in crate::share) authorization: ExecAuthorization,
    pub(in crate::share) capability: Capability,
    /// Unix seconds at which the grant ends.
    pub(in crate::share) end: Option<i64>,
}

/// Checks the capability `start` needs against the grant at `now`: `allow`
/// starts, `ask` needs the host user (PRM-M3; without its approval service
/// the start is refused as unavailable), and `off` is `right_not_granted`
/// while the grant still allows the other Exec capability, else (ended or
/// withdrawn since the handshake) `permission_denied`.
pub(in crate::share) fn start_grant(
    auth: &std::sync::Mutex<super::super::types::ShareAuthState>,
    authorized: &AuthorizedExecPeer,
    start: &ExecStart,
    host: &str,
    now: i64,
) -> Result<StartGrant, ExecWireError> {
    let capability = start_capability(&start.command);
    let proven = ProvenPrincipal::from(&authorized.principal);
    let denied = || ExecWireError {
        code: PERMISSION_DENIED.into(),
        message: permission_denied(host),
    };
    let (decision, grant) = {
        let state = auth.lock().map_err(|_| denied())?;
        (
            capability_for(&state, &proven, capability, now),
            principal_grant(&state, &proven).cloned(),
        )
    };
    let Some(grant) = grant else {
        return Err(denied());
    };
    match decision.decision {
        CapabilityDecision::Allow => Ok(StartGrant {
            end: grant.valid_until(),
            grant,
            authorization: ExecAuthorization {
                policy_revision: decision.policy_revision,
                ..authorized.authorization.clone()
            },
            capability,
        }),
        CapabilityDecision::Ask => Err(ExecWireError {
            code: APPROVAL_UNAVAILABLE.into(),
            message: approval_unavailable(host),
        }),
        CapabilityDecision::Off => {
            let other = Capability::EXEC
                .into_iter()
                .any(|other| other != capability && grant.decision_at(other, now).is_on());
            if other {
                Err(ExecWireError {
                    code: RIGHT_NOT_GRANTED.into(),
                    message: right_not_granted(host, capability),
                })
            } else {
                Err(denied())
            }
        }
    }
}

/// What the grant decided about one start.
struct GrantLaunch {
    account: Option<RunAsAccount>,
    /// The account as the audit log names it.
    account_label: String,
    /// The program file of a grant with a program list.
    program: Option<std::path::PathBuf>,
}

/// Applies the grant to `start`: its program list and environment rule may
/// refuse it (file system lookups, so on the blocking pool), and its run-as
/// account is resolved (user database lookups block as well). Both are host
/// decisions made before any job process exists.
async fn grant_launch(
    grant: &ExecGrant,
    start: &ExecStart,
    provider: &ExecProviderStatus,
) -> Result<GrantLaunch, ExecWireError> {
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
            .map_err(|error| ExecWireError {
                code: "account_unavailable".into(),
                message: error.to_string(),
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
        Err(error) => Err(ExecWireError {
            code: "account_unavailable".into(),
            message: format!("run-as account {name:?} of this device: {error}"),
        }),
    }
}

fn refused(message: impl Into<String>) -> ExecWireError {
    ExecWireError {
        code: "program_not_permitted".into(),
        message: message.into(),
    }
}
