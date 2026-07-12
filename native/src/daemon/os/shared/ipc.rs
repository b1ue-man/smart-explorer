use std::io;
use std::net::TcpStream;
use std::time::Instant;

use super::backend_server::serve_backend;
pub use super::ipc_client::{
    drain_share_worker_events, ensure_worker_ready, exec_share, open_share_backend,
    refresh_share_worker_checked, request_daemon_replacement, send_share_command,
};
pub(crate) use super::ipc_host::ShareHost;
pub(crate) use super::ipc_listener::start_listener;
use super::ipc_listener::{clear_pre_auth_deadline, read_pre_auth_line, PreAuthPermit};
pub use super::ipc_protocol::ShareWorkerSnapshot;
use super::ipc_protocol::{write_response, IpcRequest, IpcResponse};

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
    require_token(token, req.token())?;
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
        IpcRequest::DrainShareEvents { .. } => {
            let snapshot = host.drain_for_ui();
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
        IpcRequest::ExecShare { target, req, .. } => match host.exec_share(target, req) {
            Ok(result) => write_response(&mut stream, &IpcResponse::ExecResult { result }),
            Err(error) => write_response(&mut stream, &IpcResponse::Err { msg: error }),
        },
    }
}

fn require_token(want: &str, got: &str) -> io::Result<()> {
    if want == got {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "daemon IPC token rejected",
        ))
    }
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
