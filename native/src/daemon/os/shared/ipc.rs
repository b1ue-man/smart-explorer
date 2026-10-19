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

pub(super) fn handle_client(
    mut stream: TcpStream,
    host: ShareHost,
    token: &str,
    pre_auth: PreAuthPermit,
    auth_deadline: Instant,
) -> io::Result<()> {
    let mut line = String::new();
    read_pre_auth_line(&mut stream, &mut line, auth_deadline)?;
    let req: IpcRequest = serde_json::from_str(line.trim()).map_err(eio)?;
    require_request_auth(token, &req, &host)?;
    drop(pre_auth);
    clear_pre_auth_deadline(&stream)?;
    super::drive_modules::run_start_reason::admit_request(&host, &req);
    match req {
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
        IpcRequest::ExecStream { target, start, .. } => {
            match super::exec_ipc::start_remote(&host, target, start) {
                Ok(remote) => {
                    write_response(
                        &mut stream,
                        &IpcResponse::ExecReady {
                            exec_id: remote.session.exec_id().clone(),
                        },
                    )?;
                    super::exec_ipc::serve(stream, remote, host.exec_state.clone())
                }
                Err(msg) => write_response(&mut stream, &IpcResponse::Err { msg, code: None }),
            }
        }
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
        IpcRequest::StartMount { config, .. } => match host.mounts.start(config, &host) {
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
        IpcRequest::RetryMount { id, .. } => match host.mounts.retry(&id, &host) {
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
