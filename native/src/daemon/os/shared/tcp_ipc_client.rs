//! CLI, desktop, Android and MCP use this same authenticated daemon path.
use super::ipc_protocol::tcp::{TcpIpcReply, TcpIpcRequest};
use super::ipc_protocol::{
    read_response, set_stream_timeout, write_request, IpcRequest, IpcResponse,
};
use crate::share::{
    tcp_forward::{TcpStart, TunnelId, TunnelView},
    PeerOpenTarget,
};
use std::{
    net::TcpStream,
    time::{Duration, Instant},
};

fn exchange(
    request: TcpIpcRequest,
    wait: Option<Duration>,
    mut pending: impl FnMut(),
) -> Result<TcpIpcReply, String> {
    let address = super::ipc_storage::read_ipc_addr()
        .ok_or_else(|| "Background worker IPC is not ready".to_string())?;
    let token = super::ipc_storage::read_token().map_err(|error| error.to_string())?;
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))
        .map_err(|error| error.to_string())?;
    let deadline = wait.and_then(|wait| Instant::now().checked_add(wait));
    if wait.is_some() && deadline.is_none() {
        return Err("TCP setup wait is too large".into());
    }
    set_stream_timeout(&stream, wait);
    write_request(
        &mut stream,
        &IpcRequest::Tcp {
            token,
            req: request,
        },
    )
    .map_err(|error| error.to_string())?;
    loop {
        if let Some(deadline) = deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("TCP setup wait expired; pending setup cancelled".into());
            }
            set_stream_timeout(&stream, Some(remaining));
        }
        match read_response(&mut stream).map_err(|error| format!("TCP daemon request: {error}"))? {
            IpcResponse::Tcp(TcpIpcReply::Pending) => pending(),
            IpcResponse::Tcp(reply) => return Ok(reply),
            IpcResponse::Err { msg, code } => {
                return Err(code.map_or(msg.clone(), |code| format!("{code}: {msg}")))
            }
            _ => return Err("Unexpected daemon reply to TCP request".into()),
        }
    }
}
pub(crate) fn tcp_start(
    target: PeerOpenTarget,
    request: TcpStart,
    pending: impl FnMut(),
) -> Result<TunnelView, String> {
    tcp_start_within(target, request, None, pending)
}
pub(crate) fn tcp_start_within(
    target: PeerOpenTarget,
    request: TcpStart,
    wait: Option<Duration>,
    pending: impl FnMut(),
) -> Result<TunnelView, String> {
    request.validate()?;
    crate::daemon::ensure_worker_ready()?;
    match exchange(TcpIpcRequest::Start { target, request }, wait, pending)? {
        TcpIpcReply::Opened { tunnel } => Ok(tunnel),
        _ => Err("Unexpected TCP start reply".into()),
    }
}
pub(crate) fn tcp_list() -> Result<Vec<TunnelView>, String> {
    if super::ipc_storage::read_ipc_addr().is_none() {
        return Ok(Vec::new());
    }
    match exchange(TcpIpcRequest::List, Some(Duration::from_secs(20)), || {})? {
        TcpIpcReply::Listed { tunnels } => Ok(tunnels),
        _ => Err("Unexpected TCP list reply".into()),
    }
}
pub(crate) fn tcp_status(id: TunnelId) -> Result<Option<TunnelView>, String> {
    if super::ipc_storage::read_ipc_addr().is_none() {
        return Ok(None);
    }
    match exchange(
        TcpIpcRequest::Status { id },
        Some(Duration::from_secs(20)),
        || {},
    )? {
        TcpIpcReply::Status { tunnel } => Ok(tunnel),
        _ => Err("Unexpected TCP status reply".into()),
    }
}
pub(crate) fn tcp_close(id: TunnelId) -> Result<bool, String> {
    if super::ipc_storage::read_ipc_addr().is_none() {
        return Ok(false);
    }
    match exchange(
        TcpIpcRequest::Close { id },
        Some(Duration::from_secs(20)),
        || {},
    )? {
        TcpIpcReply::Closed { closed } => Ok(closed),
        _ => Err("Unexpected TCP close reply".into()),
    }
}
