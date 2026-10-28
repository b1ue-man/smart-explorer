//! CLI, desktop, Android and MCP use this same authenticated daemon path.
use super::ipc_protocol::tcp::{TcpIpcReply, TcpIpcRequest};
use super::ipc_protocol::{
    read_response, set_stream_timeout, write_request, IpcRequest, IpcResponse,
};
use crate::share::{
    tcp_forward::{ListenerAdmission, TcpStart, TunnelId, TunnelView},
    PeerOpenTarget,
};
use std::{
    io::Write,
    net::{Ipv4Addr, TcpStream},
    time::{Duration, Instant},
};

/// Local daemon IPC that answers at once (design 7: protection against a
/// hanging local service, no peer wait).
const LOCAL_REPLY: Duration = Duration::from_secs(20);

fn connect(request: TcpIpcRequest, wait: Option<Duration>) -> Result<TcpStream, String> {
    let address = super::ipc_storage::read_ipc_addr()
        .ok_or_else(|| "Background worker IPC is not ready".to_string())?;
    let token = super::ipc_storage::read_token().map_err(|error| error.to_string())?;
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))
        .map_err(|error| error.to_string())?;
    set_stream_timeout(&stream, wait);
    write_request(
        &mut stream,
        &IpcRequest::Tcp {
            token,
            req: request,
        },
    )
    .map_err(|error| error.to_string())?;
    Ok(stream)
}

/// Reads replies until the final one; `None` waits without a limit (a host
/// confirmation has none, design 7).
fn final_reply(
    stream: &mut TcpStream,
    wait: Option<Duration>,
    mut pending: impl FnMut(),
) -> Result<TcpIpcReply, String> {
    let deadline = wait.and_then(|wait| Instant::now().checked_add(wait));
    if wait.is_some() && deadline.is_none() {
        return Err("TCP setup wait is too large".into());
    }
    loop {
        if let Some(deadline) = deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("TCP setup wait expired; pending setup cancelled".into());
            }
            set_stream_timeout(stream, Some(remaining));
        }
        match read_response(stream).map_err(|error| format!("TCP daemon request: {error}"))? {
            IpcResponse::Tcp(TcpIpcReply::Pending) => pending(),
            IpcResponse::Tcp(reply) => return Ok(reply),
            IpcResponse::Err { msg, code } => {
                return Err(code.map_or(msg.clone(), |code| format!("{code}: {msg}")))
            }
            _ => return Err("Unexpected daemon reply to TCP request".into()),
        }
    }
}

fn exchange(
    request: TcpIpcRequest,
    wait: Option<Duration>,
    pending: impl FnMut(),
) -> Result<TcpIpcReply, String> {
    let mut stream = connect(request, wait)?;
    final_reply(&mut stream, wait, pending)
}

fn start_request(
    target: PeerOpenTarget,
    mut request: TcpStart,
    admission: ListenerAdmission,
    bind: Option<Ipv4Addr>,
) -> Result<TcpIpcRequest, String> {
    // Version 2 meaning (design 4.4): the listener checks the origin of
    // every connection; no confirmation by the caller.
    request.acknowledge_local_access = true;
    request.validate()?;
    Ok(TcpIpcRequest::Start {
        target,
        request,
        admission,
        bind,
    })
}

/// A listener for processes of this account on 127.0.0.1.
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
    tcp_start_with(
        target,
        request,
        ListenerAdmission::SameAccount,
        None,
        wait,
        pending,
    )
}
/// A listener with an explicit origin rule (Android: `AnyLocal`) and,
/// optionally, the device address of design 4.3.
pub(crate) fn tcp_start_with(
    target: PeerOpenTarget,
    request: TcpStart,
    admission: ListenerAdmission,
    bind: Option<Ipv4Addr>,
    wait: Option<Duration>,
    pending: impl FnMut(),
) -> Result<TunnelView, String> {
    if admission == ListenerAdmission::AwaitProcess {
        return Err("AwaitProcess braucht tcp_start_for_process".into());
    }
    let start = start_request(target, request, admission, bind)?;
    crate::daemon::ensure_worker_ready()?;
    match exchange(start, wait, pending)? {
        TcpIpcReply::Opened { tunnel } => Ok(tunnel),
        _ => Err("Unexpected TCP start reply".into()),
    }
}

/// An opened `AwaitProcess` tunnel whose client process is not bound yet.
/// Dropping it without `bind` ends the IPC stream, and the daemon closes the
/// tunnel (design 2.6).
pub(crate) struct UnboundTunnel {
    stream: TcpStream,
    tunnel: TunnelView,
}
impl UnboundTunnel {
    pub(crate) fn tunnel(&self) -> &TunnelView {
        &self.tunnel
    }
    /// Binds the one process whose connections the listener forwards: PID
    /// and start time (`share::os_sockets::process_identity`).
    pub(crate) fn bind(mut self, pid: u32, started: u64) -> Result<TunnelView, String> {
        let request = TcpIpcRequest::BindProcess {
            id: self.tunnel.id.clone(),
            pid,
            started,
        };
        let mut line = serde_json::to_string(&request).map_err(|error| error.to_string())?;
        line.push('\n');
        set_stream_timeout(&self.stream, Some(LOCAL_REPLY));
        self.stream
            .write_all(line.as_bytes())
            .and_then(|()| self.stream.flush())
            .map_err(|error| format!("Prozessbindung nicht gesendet: {error}"))?;
        match final_reply(&mut self.stream, Some(LOCAL_REPLY), || {})? {
            TcpIpcReply::Bound { tunnel } if tunnel.id == self.tunnel.id => Ok(tunnel),
            _ => Err("Unexpected TCP bind reply".into()),
        }
    }
}

/// Design 9.3: a listener on `bind` that forwards only the connections of
/// one client process, bound after the client started. `wait` `None`: no
/// time limit for the host's answer (`pending` hears a host confirmation).
pub(crate) fn tcp_start_for_process(
    target: PeerOpenTarget,
    request: TcpStart,
    bind: Option<Ipv4Addr>,
    wait: Option<Duration>,
    pending: impl FnMut(),
) -> Result<UnboundTunnel, String> {
    let start = start_request(target, request, ListenerAdmission::AwaitProcess, bind)?;
    crate::daemon::ensure_worker_ready()?;
    let mut stream = connect(start, wait)?;
    match final_reply(&mut stream, wait, pending)? {
        TcpIpcReply::Opened { tunnel } => Ok(UnboundTunnel { stream, tunnel }),
        _ => Err("Unexpected TCP start reply".into()),
    }
}

pub(crate) fn tcp_list() -> Result<Vec<TunnelView>, String> {
    if super::ipc_storage::read_ipc_addr().is_none() {
        return Ok(Vec::new());
    }
    match exchange(TcpIpcRequest::List, Some(LOCAL_REPLY), || {})? {
        TcpIpcReply::Listed { tunnels } => Ok(tunnels),
        _ => Err("Unexpected TCP list reply".into()),
    }
}
pub(crate) fn tcp_status(id: TunnelId) -> Result<Option<TunnelView>, String> {
    if super::ipc_storage::read_ipc_addr().is_none() {
        return Ok(None);
    }
    match exchange(TcpIpcRequest::Status { id }, Some(LOCAL_REPLY), || {})? {
        TcpIpcReply::Status { tunnel } => Ok(tunnel),
        _ => Err("Unexpected TCP status reply".into()),
    }
}
pub(crate) fn tcp_close(id: TunnelId) -> Result<bool, String> {
    if super::ipc_storage::read_ipc_addr().is_none() {
        return Ok(false);
    }
    match exchange(TcpIpcRequest::Close { id }, Some(LOCAL_REPLY), || {})? {
        TcpIpcReply::Closed { closed } => Ok(closed),
        _ => Err("Unexpected TCP close reply".into()),
    }
}
