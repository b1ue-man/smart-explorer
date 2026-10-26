//! The same owning login socket is used by desktop, Android, CLI and MCP.
use std::{io, net::{Shutdown, TcpStream}, sync::Arc, time::Duration};
use crate::login::{GatewayTarget, InstallationStatus};
use super::ipc_protocol::{read_response, set_stream_timeout, write_request, IpcRequest, IpcResponse,
    login::{self as wire, LoginCommand, LoginEnd, LoginFrame, LoginIpcReply, LoginIpcRequest, LoginRelayCommand,
        LoginRelayReply, LoginSessionView}};
const WAIT: Duration = Duration::from_secs(35);
fn open(request: LoginIpcRequest) -> Result<TcpStream, String> {
    crate::daemon::ensure_worker_ready()?;
    let address = super::ipc_storage::read_ipc_addr().ok_or_else(|| "background worker IPC is not ready".to_string())?;
    let token = super::ipc_storage::read_token().map_err(|error| error.to_string())?;
    let mut socket = TcpStream::connect_timeout(&address, Duration::from_secs(2)).map_err(|error| error.to_string())?;
    set_stream_timeout(&socket, Some(WAIT));
    write_request(&mut socket, &IpcRequest::Login { token, req: request }).map_err(|error| error.to_string())?;
    Ok(socket)
}
fn response(socket: &mut TcpStream) -> Result<LoginIpcReply, String> {
    match read_response(socket).map_err(|error| error.to_string())? {
        IpcResponse::Login(reply) => Ok(reply),
        IpcResponse::Err { msg, code } => Err(code.map_or(msg.clone(), |code| format!("{code}: {msg}"))),
        _ => Err("unexpected daemon login response".into()),
    }
}
fn exchange(request: LoginIpcRequest) -> Result<LoginIpcReply, String> { response(&mut open(request)?) }
pub(crate) fn login_status() -> Result<InstallationStatus, String> {
    match exchange(LoginIpcRequest::Status)? { LoginIpcReply::Status { installation } => Ok(installation), _ => Err("unexpected login installation response".into()) }
}
pub(crate) fn login_sessions() -> Result<Vec<LoginSessionView>, String> {
    match exchange(LoginIpcRequest::Sessions)? { LoginIpcReply::Sessions { sessions } => Ok(sessions), _ => Err("unexpected login sessions response".into()) }
}
pub(crate) fn login_session(session: String) -> Result<Option<LoginSessionView>, String> {
    match exchange(LoginIpcRequest::Session { session })? { LoginIpcReply::Session { session } => Ok(session), _ => Err("unexpected login session response".into()) }
}
pub(crate) fn login_stop(session: Option<String>) -> Result<usize, String> {
    match exchange(LoginIpcRequest::Stop { session })? { LoginIpcReply::Stopped { count } => Ok(count), _ => Err("unexpected login stop response".into()) }
}
pub(crate) struct LoginViewer {
    socket: TcpStream,
    close: LoginClose,
    view: LoginSessionView,
    sequence: u64,
    closed: bool,
}
#[derive(Clone)]
pub(crate) struct LoginClose { socket: Arc<TcpStream> }
impl LoginClose {
    /// Socket shutdown interrupts a pending read/write without locking it.
    pub(crate) fn close(&self) { let _ = self.socket.shutdown(Shutdown::Both); }
}
impl LoginViewer {
    /// A public target import is explicit user pinning, never normal peer discovery.
    pub(crate) fn open(target: GatewayTarget, target_user: String, acknowledge_machine_pin: bool) -> Result<Self, String> {
        Self::open_owned(target, target_user, acknowledge_machine_pin, |_| {})
    }
    /// The owner receives an interrupt handle before network admission waits.
    pub(crate) fn open_owned(target: GatewayTarget, target_user: String, acknowledge_machine_pin: bool,
        owner: impl FnOnce(LoginClose)) -> Result<Self, String> {
        target.validate().map_err(|error| error.to_string())?;
        if !acknowledge_machine_pin { return Err("explicit acknowledgement of the administrative machine pin is required".into()); }
        let mut socket = open(LoginIpcRequest::Stream { target: target.clone(), target_user: target_user.clone(), acknowledge_machine_pin })?;
        let close = LoginClose { socket: Arc::new(socket.try_clone().map_err(|error| error.to_string())?) };
        owner(close.clone());
        let session = match response(&mut socket)? { LoginIpcReply::StreamOpened { session } => session, _ => return Err("login stream was not opened".into()) };
        if !wire::valid_session(&session) { return Err("invalid login stream session ID".into()); }
        let (tag, bytes) = wire::read_frame(&mut socket, wire::MAX_CONTROL).map_err(|error| error.to_string())?;
        if tag == wire::END { return Err(end_reason(&bytes)); }
        if tag != wire::HELLO { return Err("login stream has no initial challenge".into()); }
        let view: LoginSessionView = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        if view.session != session || view.target != target || view.target_user != target_user { return Err("login stream differs from its pinned request".into()); }
        target.check_challenge(view.challenge.as_ref().ok_or_else(|| "login stream has no broker challenge".to_string())?, &target_user)
            .map_err(|error| error.to_string())?;
        Ok(Self { socket, close, view, sequence: 1, closed: false })
    }
    pub(crate) fn view(&self) -> &LoginSessionView { &self.view }
    pub(crate) fn control(&self) -> LoginClose { self.close.clone() }
    /// An idle owner can observe terminal EOF without sending an operation or
    /// renewing the broker lease. Call only while no command is in progress.
    pub(crate) fn poll_end(&mut self) -> Result<Option<LoginEnd>, String> {
        if self.closed { return Ok(Some(LoginEnd { code: "closed".into(), detail: None })); }
        self.socket.set_read_timeout(Some(Duration::from_millis(1))).map_err(|error| error.to_string())?;
        let available = self.socket.peek(&mut [0u8; 1]);
        self.socket.set_read_timeout(Some(WAIT)).map_err(|error| error.to_string())?;
        match available {
            Ok(0) => { self.close(); Ok(Some(LoginEnd { code: "eof".into(), detail: None })) }
            Ok(_) => {
                let result = wire::read_frame(&mut self.socket, wire::MAX_CONTROL).and_then(|(tag, bytes)| {
                    if tag != wire::END { return Err(wire::invalid("unexpected unsolicited login frame")); }
                    serde_json::from_slice(&bytes).map_err(wire::invalid)
                });
                self.close(); result.map(Some).map_err(|error| error.to_string())
            }
            Err(error) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => Ok(None),
            Err(error) => { self.close(); Err(error.to_string()) }
        }
    }
    /// Exactly one request/reply at a time. Any ambiguous failure closes the
    /// owner socket; the operation is never automatically signed or sent again.
    pub(crate) fn command(&mut self, command: LoginCommand) -> Result<LoginFrame, String> {
        if self.closed { return Err("login viewer is closed".into()); }
        command.validate().map_err(|error| error.to_string())?;
        let result = (|| -> io::Result<LoginFrame> {
            wire::write_json(&mut self.socket, wire::COMMAND, &LoginRelayCommand { sequence: self.sequence, command })?;
            let (tag, bytes) = wire::read_frame(&mut self.socket, wire::MAX_CONTROL)?;
            if tag == wire::END { return Err(io::Error::other(end_reason(&bytes))); }
            if tag != wire::REPLY { return Err(wire::invalid("unexpected login relay reply")); }
            let reply: LoginRelayReply = serde_json::from_slice(&bytes).map_err(wire::invalid)?;
            if reply.sequence != self.sequence || reply.view.session != self.view.session || reply.view.target != self.view.target
                || reply.view.target_user != self.view.target_user { return Err(wire::invalid("login relay reply changed sequence or pinned session")); }
            if let Some(challenge) = &reply.view.challenge { self.view.target.check_challenge(challenge, &self.view.target_user)?; }
            let pixels = wire::read_pixels(&mut self.socket, &reply)?;
            Ok(LoginFrame { reply, pixels })
        })();
        match result {
            Ok(frame) => {
                self.sequence = match self.sequence.checked_add(1) { Some(sequence) => sequence, None => { self.close(); return Err("login local sequence exhausted".into()); } };
                self.view = frame.reply.view.clone();
                if self.view.challenge.is_none() { self.close(); }
                Ok(frame)
            }
            Err(error) => { self.close(); Err(format!("{error}; do not repeat an ambiguous input operation")) }
        }
    }
    pub(crate) fn close(&mut self) { self.closed = true; self.view.challenge = None; self.close.close(); }
}
impl Drop for LoginViewer { fn drop(&mut self) { self.close(); } }
fn end_reason(bytes: &[u8]) -> String {
    serde_json::from_slice::<LoginEnd>(bytes).map(|end| end.detail.map_or(end.code.clone(), |detail| format!("{}: {detail}", end.code)))
        .unwrap_or_else(|_| "invalid login end response".into())
}
