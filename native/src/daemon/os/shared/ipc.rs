use std::io::{self, Read};
use std::net::{Shutdown, TcpStream};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;
use std::time::Instant;

use super::backend_server::serve_backend;
#[allow(unused_imports)]
pub(crate) use super::ipc_client::mutate_exec_grant;
pub use super::ipc_client::{
    drain_share_worker_events, ensure_worker_ready, exec_share, hand_off_running_worker,
    open_share_backend, refresh_share_worker_checked, request_daemon_replacement,
    send_share_command, share_command, share_worker_snapshot, WorkerHandoff,
};
pub(crate) use super::ipc_host::ShareHost;
pub(crate) use super::ipc_listener::start_listener;
use super::ipc_listener::{clear_pre_auth_deadline, read_pre_auth_line, PreAuthPermit};
use super::ipc_protocol::{bound_snapshot_for_ipc, write_response, IpcRequest, IpcResponse};
pub use super::ipc_protocol::{ShareCommandReply, ShareWorkerSnapshot};

/// Code of an `IpcResponse::Err` for a request whose capability was refused
/// (here and in the prelude, `ipc_admission.rs`).
pub(super) const AUTH_REJECTED_CODE: &str = "ipc_auth_rejected";

pub(super) fn handle_client(
    mut stream: TcpStream,
    host: ShareHost,
    token: &str,
    pre_auth: PreAuthPermit,
    auth_deadline: Instant,
) -> io::Result<()> {
    let mut line = String::new();
    read_pre_auth_line(&mut stream, &mut line, auth_deadline)?;
    // The line is read: a refusal is answered before the stream closes
    // (FIN, not RST), so the client sees the reason, not a reset.
    let req: IpcRequest = match serde_json::from_str(line.trim()) {
        Ok(req) => req,
        Err(error) => return refuse(&mut stream, eio(error), None, Some(MALFORMED)),
    };
    let bound = matches!(&req, IpcRequest::RuntimeCall { .. });
    if let Err(error) = require_request_auth(token, &req, &host) {
        return refuse(&mut stream, error, Some(AUTH_REJECTED_CODE), None);
    }
    let runtime_context = match &req {
        IpcRequest::RuntimeCall { hello, .. } => Some(hello.clone()),
        _ => None,
    };
    let req = match super::runtime_hello::unwrap_request(req, host.generation(), host.initialized())
    {
        Ok(req) => req,
        Err(error) => return refuse(&mut stream, error, Some("reconnect_required"), None),
    };
    if bound {
        if let Err(error) = require_request_auth(token, &req, &host) {
            return refuse(&mut stream, error, Some(AUTH_REJECTED_CODE), None);
        }
    }
    let runtime_lease = match super::runtime_hello::request_lease(runtime_context.as_ref(), &req) {
        Ok(lease) => lease,
        Err(error) => {
            let msg = error.to_string();
            let code = if msg.contains("update_recovery_required") {
                "update_recovery_required"
            } else if msg.contains("update_in_progress") {
                "update_in_progress"
            } else {
                "reconnect_required"
            };
            write_response(
                &mut stream,
                &IpcResponse::Err {
                    msg,
                    code: Some(code.into()),
                },
            )?;
            return Ok(());
        }
    };
    drop(pre_auth);
    clear_pre_auth_deadline(&stream)?;
    if !matches!(&req, IpcRequest::RuntimeHello { .. }) {
        super::drive_modules::run_start_reason::admit_request(&host, &req);
    }
    let controlled_exec = matches!(&req, IpcRequest::ExecStreamControlled { .. });
    match req {
        IpcRequest::DiscoveryPairing { request, .. } => super::discovery_pairing_ipc::serve(
            stream,
            &host,
            request,
            runtime_lease.as_ref().map(|lease| {
                let retained: std::sync::Arc<dyn Send + Sync> = lease.clone();
                retained
            }),
        ),
        IpcRequest::RuntimeCall { .. } => Err(eio("nested runtime request rejected")),
        IpcRequest::RuntimeHello { hello, .. } => write_response(
            &mut stream,
            &IpcResponse::RuntimeHello {
                reply: super::runtime_hello::ready_reply(
                    &hello,
                    host.generation(),
                    host.initialized(),
                ),
            },
        ),
        IpcRequest::Ping { .. } => write_response(
            &mut stream,
            &IpcResponse::Pong {
                version: env!("CARGO_PKG_VERSION").to_string(),
                generation: host.generation().to_string(),
                initialized: host.initialized(),
            },
        ),
        IpcRequest::RefreshShare { .. } => match host.refresh_now() {
            Ok(running) => write_response(&mut stream, &IpcResponse::RefreshOk { running }),
            Err(msg) => write_response(&mut stream, &IpcResponse::Err { msg, code: None }),
        },
        IpcRequest::ShareCommand { cmd, .. } => match host.send_command(cmd) {
            Ok(ShareCommandReply::Applied) => write_response(&mut stream, &IpcResponse::Ok),
            Ok(reply) => write_response(&mut stream, &IpcResponse::ShareCommand { reply }),
            Err(msg) => write_response(&mut stream, &IpcResponse::Err { msg, code: None }),
        },
        IpcRequest::MutateExecGrant {
            target,
            enabled,
            settings,
            ..
        } => match host.mutate_exec_grant(target, enabled, settings) {
            Ok(result) => write_response(&mut stream, &IpcResponse::ExecGrantMutation { result }),
            Err(msg) => write_response(&mut stream, &IpcResponse::Err { msg, code: None }),
        },
        IpcRequest::DrainShareEvents { .. } => {
            let snapshot = bound_snapshot_for_ipc(host.drain_for_ui());
            write_response(
                &mut stream,
                &IpcResponse::ShareEvents {
                    snapshot: Box::new(snapshot),
                },
            )
        }
        IpcRequest::ShareSnapshot { .. } | IpcRequest::ShareSnapshotPeek { .. } => {
            let snapshot = bound_snapshot_for_ipc(host.snapshot_for_client());
            write_response(
                &mut stream,
                &IpcResponse::ShareEvents {
                    snapshot: Box::new(snapshot),
                },
            )
        }
        IpcRequest::AnalyzeShare {
            target,
            root,
            node_budget,
            ..
        } => super::ipc_analysis::serve(stream, root, node_budget, || {
            host.open_share(target)
                .map(|(_, backend, _)| backend)
                .map_err(io::Error::other)
        }),
        IpcRequest::OpenShare { target, .. } => match host.open_share(target) {
            Ok((label, backend, status)) => {
                write_response(&mut stream, &IpcResponse::OpenOk { label, status })?;
                let read = stream.try_clone()?;
                serve_backend(read, stream, backend)
            }
            Err(msg) => write_response(&mut stream, &IpcResponse::Err { msg, code: None }),
        },
        IpcRequest::ProbeShareMount { target, root, .. } => {
            match host.probe_share_mount_capabilities(target, &root) {
                Ok(capabilities) => write_response(
                    &mut stream,
                    &IpcResponse::MountPathCapabilities {
                        capabilities: capabilities.into(),
                    },
                ),
                Err(msg) => write_response(&mut stream, &IpcResponse::Err { msg, code: None }),
            }
        }
        IpcRequest::ExecShare { target, req, .. } => match host.exec_share(target, req) {
            Ok(result) => write_response(&mut stream, &IpcResponse::ExecResult { result }),
            Err(msg) => write_response(&mut stream, &IpcResponse::Err { msg, code: None }),
        },
        IpcRequest::ExecStream { target, start, .. }
        | IpcRequest::ExecStreamControlled { target, start, .. } => {
            let setup = super::exec_ipc::SetupWatch::start(&stream)?;
            let deadline = Instant::now() + Duration::from_secs(45);
            match super::exec_ipc::start_remote_cancellable(
                &host,
                target,
                start,
                runtime_lease.as_ref().map(|lease| {
                    let retained: std::sync::Arc<dyn Send + Sync> = lease.clone();
                    retained
                }),
                setup.cancellation(),
                deadline,
            ) {
                Ok(remote) => {
                    if controlled_exec {
                        return super::exec_ipc::serve_controlled(
                            stream,
                            remote,
                            host.exec_state.clone(),
                            setup,
                        );
                    }
                    setup.check()?;
                    write_response(
                        &mut stream,
                        &IpcResponse::ExecReady {
                            exec_id: remote.session.exec_id().clone(),
                        },
                    )?;
                    setup.handoff()?;
                    super::exec_ipc::serve(stream, remote, host.exec_state.clone())
                }
                Err(msg) => write_response(&mut stream, &IpcResponse::Err { msg, code: None }),
            }
        }
        IpcRequest::ExecControl {
            exec_id,
            control_token,
            ..
        } => super::exec_ipc::serve_control(stream, &host.exec_state, &exec_id, &control_token),
        IpcRequest::ExecJobs { .. } => write_response(
            &mut stream,
            &IpcResponse::ExecJobs {
                snapshot: super::exec_state::snapshot(&host),
            },
        ),
        IpcRequest::CancelExec { target, .. } => write_response(
            &mut stream,
            &IpcResponse::ExecCancelled {
                found: super::exec_state::cancel(&host, &target),
            },
        ),
        IpcRequest::StartMount { config, .. } => match host.mounts.start_retained(
            config,
            &host,
            runtime_lease.as_ref().map(|lease| {
                let retained: std::sync::Arc<dyn Send + Sync> = lease.clone();
                retained
            }),
        ) {
            Ok(mount) => write_response(&mut stream, &IpcResponse::Mount { mount }),
            Err(msg) => write_response(&mut stream, &IpcResponse::Err { msg, code: None }),
        },
        IpcRequest::StopMount { id, .. } => match host.mounts.stop(&id) {
            Ok(mount) => write_response(&mut stream, &IpcResponse::Mount { mount }),
            Err(msg) => write_response(&mut stream, &IpcResponse::Err { msg, code: None }),
        },
        IpcRequest::ListMounts { .. } => match host.mounts.list() {
            Ok(mounts) => write_response(&mut stream, &IpcResponse::Mounts { mounts }),
            Err(msg) => write_response(&mut stream, &IpcResponse::Err { msg, code: None }),
        },
        IpcRequest::RetryMount { id, .. } => match host.mounts.retry_retained(
            &id,
            &host,
            runtime_lease.as_ref().map(|lease| {
                let retained: std::sync::Arc<dyn Send + Sync> = lease.clone();
                retained
            }),
        ) {
            Ok(mount) => write_response(&mut stream, &IpcResponse::Mount { mount }),
            Err(msg) => write_response(&mut stream, &IpcResponse::Err { msg, code: None }),
        },
        IpcRequest::MountHostAttach {
            id, launch_token, ..
        } => match host.mounts.grant_host(&id, &launch_token) {
            Ok(grant) => {
                write_response(
                    &mut stream,
                    &IpcResponse::MountHostReady {
                        config: grant.config,
                        scheme: grant.scheme,
                        capabilities: grant.capabilities,
                        session_token: grant.session_token.clone(),
                        backend_token: grant.backend_token,
                    },
                )?;
                let stop = host
                    .mounts
                    .register_control(&id, &grant.session_token)
                    .map_err(eio)?;
                serve_mount_control(stream, host, id, stop)
            }
            Err(msg) => write_response(&mut stream, &IpcResponse::Err { msg, code: None }),
        },
        IpcRequest::MountHostBackend {
            id, backend_token, ..
        } => match host.mounts.take_backend(&id, &backend_token) {
            Ok((backend, _generation_lease)) => {
                super::mount_proxy::prepare_stream(&stream)?;
                write_response(&mut stream, &IpcResponse::Ok)?;
                let read = stream.try_clone()?;
                serve_backend(read, stream, backend)
            }
            Err(msg) => write_response(&mut stream, &IpcResponse::Err { msg, code: None }),
        },
        IpcRequest::MountHostStatus {
            id,
            session_token,
            status,
            recovery,
            recovery_required,
            ..
        } => match host.mounts.update_status(
            &id,
            &session_token,
            status,
            recovery.or_else(|| recovery_required.map(crate::mount::MountRecovery::from_required)),
        ) {
            Ok(_) => write_response(&mut stream, &IpcResponse::Ok),
            Err(msg) => write_response(&mut stream, &IpcResponse::Err { msg, code: None }),
        },
        request @ IpcRequest::Access { .. } => host.serve_extension(stream, request),
        request @ IpcRequest::Drive { .. } => host.serve_extension(stream, request),
        request @ IpcRequest::Vault { .. } => host.serve_extension(stream, request),
        request @ IpcRequest::Screen { .. } => host.serve_extension(stream, request),
        IpcRequest::Tcp { req, .. } => host.serve_tcp(stream, req),
        IpcRequest::Apps { req, .. } => host.serve_apps(stream, req),
        IpcRequest::Transfer { req, .. } => host.serve_transfer(stream, req, runtime_lease),
        request @ IpcRequest::Login { .. } => host.serve_extension(stream, request),
        request @ IpcRequest::MountHostExt { .. } => host.serve_extension(stream, request),
    }
}

fn require_request_auth(token: &str, req: &IpcRequest, host: &ShareHost) -> io::Result<()> {
    if let Some(request_token) = req.daemon_token() {
        return require_token(token, request_token);
    }
    let result = match req {
        IpcRequest::MountHostAttach {
            id, launch_token, ..
        } => host.mounts.check_launch_token(id, launch_token),
        IpcRequest::MountHostBackend {
            id, backend_token, ..
        } => host.mounts.check_backend_token(id, backend_token),
        IpcRequest::MountHostStatus {
            id, session_token, ..
        } => host.mounts.check_session_token(id, session_token),
        _ => host.authorize_extension(req),
    };
    result.map_err(|message| io::Error::new(io::ErrorKind::PermissionDenied, message))
}

fn serve_mount_control(
    mut stream: TcpStream,
    host: ShareHost,
    id: crate::mount::MountId,
    stop: Receiver<()>,
) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_millis(100)))?;
    let mut byte = [0u8; 1];
    loop {
        match stop.recv_timeout(Duration::from_millis(50)) {
            Ok(()) => return write_response(&mut stream, &IpcResponse::MountHostStop),
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
            Err(RecvTimeoutError::Timeout) => {}
        }
        match stream.read(&mut byte) {
            Ok(0) => {
                let _ = stream.shutdown(Shutdown::Both);
                let _ = host.mounts.stop(&id);
                return Ok(());
            }
            Ok(_) => {
                let _ = stream.shutdown(Shutdown::Both);
                let _ = host.mounts.stop(&id);
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "mount host sent unexpected control data",
                ));
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(error) => {
                let _ = stream.shutdown(Shutdown::Both);
                let _ = host.mounts.stop(&id);
                return Err(error);
            }
        }
    }
}

/// Answers a refused request (`public`, else the token-free text of
/// `error`; parser errors can quote values) and keeps `error` for the log.
fn refuse(
    stream: &mut TcpStream,
    error: io::Error,
    code: Option<&str>,
    public: Option<&str>,
) -> io::Result<()> {
    let msg = public.map_or_else(|| error.to_string(), str::to_string);
    let code = code.map(str::to_string);
    let _ = write_response(stream, &IpcResponse::Err { msg, code });
    Err(error)
}

const MALFORMED: &str = "Daemon-IPC-Anfrage ist ungueltig";

fn require_token(want: &str, got: &str) -> io::Result<()> {
    if constant_time_eq(want, got) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "daemon IPC token rejected",
        ))
    }
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.as_bytes()
        .iter()
        .zip(right.as_bytes())
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

fn eio<E: std::fmt::Display>(error: E) -> io::Error {
    io::Error::other(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::super::ipc_protocol::VaultIpcRequest;
    use super::{require_request_auth, require_token, IpcRequest, ShareHost};
    use crate::daemon::ipc_protocol_mount_host::MountHostRequest;
    use crate::mount::MountId;

    const DAEMON: &str = "daemon-token";

    #[test]
    fn token_rejects_mismatch() {
        assert!(require_token("abc", "abc").is_ok());
        assert!(require_token("abc", "def").is_err());
    }

    fn spool_key(token: &str, mount_id: &str) -> IpcRequest {
        IpcRequest::Vault {
            token: token.into(),
            req: VaultIpcRequest::SpoolKey {
                mount_id: mount_id.into(),
            },
        }
    }

    /// R5: the daemon token of the GUI, `se` or `se mcp` never reaches
    /// `SpoolKey`; only the session token of the named drive's host does,
    /// and a worker without that drive refuses every token.
    #[test]
    fn vault_task_spool_key_refuses_daemon_token_and_unknown_mount() {
        let host = ShareHost::new("0".repeat(32));
        let unknown = "f".repeat(32);
        assert_eq!(spool_key(DAEMON, &unknown).daemon_token(), None);
        for request in [
            spool_key(DAEMON, &unknown),
            spool_key("guessed-session-token", &unknown),
            spool_key(DAEMON, "../not-a-mount-id"),
        ] {
            let refused = require_request_auth(DAEMON, &request, &host).unwrap_err();
            assert_eq!(refused.kind(), std::io::ErrorKind::PermissionDenied);
        }
        // Every other vault request stays on the daemon token.
        let status = IpcRequest::Vault {
            token: DAEMON.into(),
            req: VaultIpcRequest::Status,
        };
        assert_eq!(status.daemon_token(), Some(DAEMON));
        assert!(require_request_auth(DAEMON, &status, &host).is_ok());
        assert!(require_request_auth("other", &status, &host).is_err());
    }

    /// R5: `MountHostExt` carries no daemon token; a session token that
    /// belongs to no running drive host is refused.
    #[test]
    fn drive_task_mount_host_ext_requires_session_token_of_its_mount() {
        let host = ShareHost::new("0".repeat(32));
        let request = IpcRequest::MountHostExt {
            id: MountId::parse(&"e".repeat(32)).unwrap(),
            session_token: DAEMON.into(),
            req: MountHostRequest::PrepareLock { deadline_ms: 1000 },
        };
        assert_eq!(request.daemon_token(), None);
        let refused = require_request_auth(DAEMON, &request, &host).unwrap_err();
        assert_eq!(refused.kind(), std::io::ErrorKind::PermissionDenied);
    }
}

#[cfg(test)]
#[path = "windows_analysis_task_tests.rs"]
mod windows_analysis_task_tests;
