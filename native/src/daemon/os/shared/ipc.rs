use std::io::{self, Read};
use std::net::{Shutdown, TcpStream};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;
use std::time::Instant;

use super::backend_server::serve_backend;
#[allow(unused_imports)]
pub(crate) use super::ipc_client::mutate_exec_grant;
pub use super::ipc_client::{
    drain_share_worker_events, ensure_worker_ready, exec_share, open_share_backend,
    refresh_share_worker_checked, request_daemon_replacement, send_share_command,
};
pub(crate) use super::ipc_host::ShareHost;
pub(crate) use super::ipc_listener::start_listener;
use super::ipc_listener::{clear_pre_auth_deadline, read_pre_auth_line, PreAuthPermit};
pub use super::ipc_protocol::ShareWorkerSnapshot;
use super::ipc_protocol::{bound_snapshot_for_ipc, write_response, IpcRequest, IpcResponse};

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
            Err(msg) => write_response(&mut stream, &IpcResponse::Err { msg }),
        },
        IpcRequest::ShareCommand { cmd, .. } => match host.send_command(cmd) {
            Ok(()) => write_response(&mut stream, &IpcResponse::Ok),
            Err(msg) => write_response(&mut stream, &IpcResponse::Err { msg }),
        },
        IpcRequest::MutateExecGrant {
            target, enabled, ..
        } => match host.mutate_exec_grant(target, enabled) {
            Ok(result) => write_response(&mut stream, &IpcResponse::ExecGrantMutation { result }),
            Err(msg) => write_response(&mut stream, &IpcResponse::Err { msg }),
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
        IpcRequest::OpenShare { target, .. } => match host.open_share(target) {
            Ok((label, backend, status)) => {
                write_response(&mut stream, &IpcResponse::OpenOk { label, status })?;
                let read = stream.try_clone()?;
                serve_backend(read, stream, backend)
            }
            Err(error) => write_response(&mut stream, &IpcResponse::Err { msg: error }),
        },
        IpcRequest::ProbeShareMount { target, root, .. } => {
            match host.probe_share_mount_capabilities(target, &root) {
                Ok(capabilities) => write_response(
                    &mut stream,
                    &IpcResponse::MountPathCapabilities {
                        capabilities: capabilities.into(),
                    },
                ),
                Err(error) => write_response(&mut stream, &IpcResponse::Err { msg: error }),
            }
        }
        IpcRequest::ExecShare { target, req, .. } => match host.exec_share(target, req) {
            Ok(result) => write_response(&mut stream, &IpcResponse::ExecResult { result }),
            Err(error) => write_response(&mut stream, &IpcResponse::Err { msg: error }),
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
                Err(error) => write_response(&mut stream, &IpcResponse::Err { msg: error }),
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
            Err(msg) => write_response(&mut stream, &IpcResponse::Err { msg }),
        },
        IpcRequest::StopMount { id, .. } => match host.mounts.stop(&id) {
            Ok(mount) => write_response(&mut stream, &IpcResponse::Mount { mount }),
            Err(msg) => write_response(&mut stream, &IpcResponse::Err { msg }),
        },
        IpcRequest::ListMounts { .. } => match host.mounts.list() {
            Ok(mounts) => write_response(&mut stream, &IpcResponse::Mounts { mounts }),
            Err(msg) => write_response(&mut stream, &IpcResponse::Err { msg }),
        },
        IpcRequest::RetryMount { id, .. } => match host.mounts.retry(&id, &host) {
            Ok(mount) => write_response(&mut stream, &IpcResponse::Mount { mount }),
            Err(msg) => write_response(&mut stream, &IpcResponse::Err { msg }),
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
            Err(msg) => write_response(&mut stream, &IpcResponse::Err { msg }),
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
            Err(msg) => write_response(&mut stream, &IpcResponse::Err { msg }),
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
            Err(msg) => write_response(&mut stream, &IpcResponse::Err { msg }),
        },
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
        _ => Err("daemon IPC request has no authentication capability".into()),
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
    use super::require_token;

    #[test]
    fn token_rejects_mismatch() {
        assert!(require_token("abc", "abc").is_ok());
        assert!(require_token("abc", "def").is_err());
    }
}
