use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::backend_server::serve_backend;
use super::state::{clear_heartbeat, clear_stop, log, stop_requested};

const IPC_ADDR_FILE: &str = "daemon.ipc";
const IPC_TOKEN_FILE: &str = "daemon.token";

#[derive(Clone)]
pub(crate) struct ShareHost {
    state: Arc<Mutex<ShareHostState>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ShareWorkerSnapshot {
    pub events: Vec<crate::share::ShareEvent>,
    pub pending_direct_requests: Vec<crate::share::PeerPresence>,
    pub running: bool,
    pub relay_url: String,
    pub candidates: Vec<String>,
}

struct ShareHostState {
    service: Option<crate::share::ShareService>,
    identity: crate::share::ShareIdentity,
    profiles: crate::share::ShareProfiles,
    server: String,
    running_server: String,
    last_reload: Instant,
    ui_events: Vec<crate::share::ShareEvent>,
    pending_direct_requests: Vec<crate::share::PeerPresence>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
enum IpcRequest {
    Ping {
        token: String,
    },
    RefreshShare {
        token: String,
    },
    ShareCommand {
        token: String,
        cmd: crate::share::ShareCmd,
    },
    DrainShareEvents {
        token: String,
    },
    OpenShare {
        token: String,
        target: crate::share::PeerOpenTarget,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
enum IpcResponse {
    Pong,
    Ok,
    OpenOk {
        label: String,
        status: crate::share::ShareStatus,
    },
    ShareEvents {
        snapshot: ShareWorkerSnapshot,
    },
    Err {
        msg: String,
    },
}

pub(crate) fn start_listener(host: ShareHost) -> io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let addr = listener.local_addr()?;
    write_ipc_addr(addr)?;
    let token = load_or_create_token()?;
    std::thread::Builder::new()
        .name("daemon-ipc".into())
        .spawn(move || {
            log(&format!("background worker IPC listening on {addr}"));
            loop {
                if stop_requested() {
                    let _ = std::fs::remove_file(ipc_addr_path());
                    return;
                }
                match listener.accept() {
                    Ok((stream, peer)) => {
                        if !peer.ip().is_loopback() {
                            continue;
                        }
                        let host = host.clone();
                        let token = token.clone();
                        std::thread::spawn(move || {
                            if let Err(e) = handle_client(stream, host, &token) {
                                log(&format!("daemon IPC client error: {e}"));
                            }
                        });
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    Err(e) => {
                        log(&format!("daemon IPC accept failed: {e}"));
                        std::thread::sleep(Duration::from_secs(1));
                    }
                }
            }
        })?;
    Ok(())
}

fn handle_client(mut stream: TcpStream, host: ShareHost, token: &str) -> io::Result<()> {
    let mut line = String::new();
    {
        let mut reader = io::BufReader::new(stream.try_clone()?);
        reader.read_line(&mut line)?;
    }
    let req: IpcRequest = serde_json::from_str(line.trim()).map_err(eio)?;
    match req {
        IpcRequest::Ping { token: t } => {
            require_token(token, &t)?;
            write_response(&mut stream, &IpcResponse::Pong)
        }
        IpcRequest::RefreshShare { token: t } => {
            require_token(token, &t)?;
            host.reload_now();
            write_response(&mut stream, &IpcResponse::Ok)
        }
        IpcRequest::ShareCommand { token: t, cmd } => {
            require_token(token, &t)?;
            host.send_command(cmd);
            write_response(&mut stream, &IpcResponse::Ok)
        }
        IpcRequest::DrainShareEvents { token: t } => {
            require_token(token, &t)?;
            let snapshot = host.drain_for_ui();
            write_response(&mut stream, &IpcResponse::ShareEvents { snapshot })
        }
        IpcRequest::OpenShare { token: t, target } => {
            require_token(token, &t)?;
            match host.open_share(target) {
                Ok((label, backend, status)) => {
                    write_response(&mut stream, &IpcResponse::OpenOk { label, status })?;
                    let read = stream.try_clone()?;
                    serve_backend(read, stream, backend)
                }
                Err(e) => write_response(&mut stream, &IpcResponse::Err { msg: e }),
            }
        }
    }
}

fn write_response(stream: &mut TcpStream, response: &IpcResponse) -> io::Result<()> {
    let mut line = serde_json::to_string(response).map_err(eio)?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    stream.flush()
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

impl ShareHost {
    pub(crate) fn new() -> Self {
        let identity = crate::share::ShareIdentity::load_or_create(default_device_name());
        let profiles = crate::share::ShareProfiles::load(Some(default_home()));
        let server = load_share_server();
        let state = ShareHostState {
            service: None,
            identity,
            profiles,
            server,
            running_server: String::new(),
            last_reload: Instant::now() - Duration::from_secs(60),
            ui_events: Vec::new(),
            pending_direct_requests: Vec::new(),
        };
        let host = ShareHost {
            state: Arc::new(Mutex::new(state)),
        };
        host.reload_now();
        host
    }

    pub(crate) fn tick(&self) {
        self.drain_events();
        let should_reload = self
            .state
            .lock()
            .map(|s| s.last_reload.elapsed() >= Duration::from_secs(5))
            .unwrap_or(false);
        if should_reload {
            self.reload_now();
        }
    }

    pub(crate) fn reload_now(&self) {
        let mut state = match self.state.lock() {
            Ok(s) => s,
            Err(_) => return,
        };
        state.last_reload = Instant::now();
        state.server = load_share_server();
        state.identity = crate::share::ShareIdentity::load_or_create(default_device_name());
        state.profiles = crate::share::ShareProfiles::load(Some(default_home()));
        configure_or_restart_locked(&mut state);
    }

    fn send_command(&self, cmd: crate::share::ShareCmd) {
        let mut answer: Option<(String, crate::share::PeerPresence)> = None;
        {
            let mut state = match self.state.lock() {
                Ok(s) => s,
                Err(_) => return,
            };
            match &cmd {
                crate::share::ShareCmd::Stop => {
                    if let Some(svc) = state.service.take() {
                        svc.cmd(crate::share::ShareCmd::Stop);
                    }
                    state.running_server.clear();
                    state.ui_events.push(crate::share::ShareEvent::Status(
                        "Share-Worker getrennt".to_string(),
                    ));
                    return;
                }
                crate::share::ShareCmd::AnswerDirectRequest {
                    lookup_id,
                    presence,
                    accepted: _,
                } => {
                    state
                        .pending_direct_requests
                        .retain(|p| p.device_id != presence.device_id);
                    answer = Some((lookup_id.clone(), presence.clone()));
                }
                _ => {}
            }
        }
        self.reload_now();
        let service = {
            let state = match self.state.lock() {
                Ok(s) => s,
                Err(_) => return,
            };
            state.service.clone()
        };
        if let Some(service) = service {
            service.cmd(cmd);
        } else if let Some((lookup_id, presence)) = answer {
            if let Ok(mut state) = self.state.lock() {
                state
                    .pending_direct_requests
                    .retain(|p| p.device_id != presence.device_id);
                state
                    .ui_events
                    .push(crate::share::ShareEvent::Error(format!(
                        "Share-Antwort konnte nicht gesendet werden: {lookup_id}"
                    )));
            }
        }
    }

    fn drain_for_ui(&self) -> ShareWorkerSnapshot {
        let should_reload = self
            .state
            .lock()
            .map(|s| s.last_reload.elapsed() >= Duration::from_secs(5))
            .unwrap_or(false);
        if should_reload {
            self.reload_now();
        }
        self.drain_events();
        let mut state = match self.state.lock() {
            Ok(s) => s,
            Err(_) => return ShareWorkerSnapshot::default(),
        };
        let running = state.service.is_some();
        let (relay_url, candidates) = state
            .service
            .as_ref()
            .map(|svc| (svc.relay_url(), svc.peer_candidates()))
            .unwrap_or_default();
        ShareWorkerSnapshot {
            events: std::mem::take(&mut state.ui_events),
            pending_direct_requests: state.pending_direct_requests.clone(),
            running,
            relay_url,
            candidates,
        }
    }

    fn drain_events(&self) {
        let mut state = match self.state.lock() {
            Ok(s) => s,
            Err(_) => return,
        };
        let Some(service) = state.service.clone() else {
            return;
        };
        let events: Vec<_> = service.events.try_iter().collect();
        if events.is_empty() {
            return;
        }
        let mut changed = false;
        let mut answers = Vec::new();
        for event in events {
            use crate::share::ShareEvent as E;
            let mut ui_event = Some(event.clone());
            match event {
                E::Status(s) => log(&format!("share: {s}")),
                E::Error(e) => log(&format!("share error: {e}")),
                E::ServerConnected => log("share signaling connected"),
                E::ServerDisconnected(e) => log(&format!("share signaling disconnected: {e}")),
                E::DirectAvailable {
                    lookup_id,
                    presence,
                } => {
                    if let Some(c) = state
                        .profiles
                        .direct_contacts
                        .iter_mut()
                        .find(|c| c.lookup_id == lookup_id)
                    {
                        if !c.expected_node_id.trim().is_empty()
                            && c.expected_node_id != presence.node_id
                        {
                            c.status = crate::share::ShareStatus::IdentityConflict;
                            c.last_error = Some("Iroh NodeId passt nicht zum Code".into());
                            changed = true;
                            continue;
                        }
                        if c.expected_node_id.trim().is_empty() {
                            c.expected_node_id = presence.node_id.clone();
                        }
                        c.remote_device_id = Some(presence.device_id.clone());
                        c.remote_public_key = Some(presence.public_key.clone());
                        c.last_seen = Some(crate::share::core_now_secs());
                        c.status = if c.access_state == crate::share::DirectAccessState::Accepted {
                            crate::share::ShareStatus::Available
                        } else {
                            crate::share::ShareStatus::WaitingForAccess
                        };
                        c.last_error = None;
                        c.presence = Some(presence);
                        changed = true;
                    }
                }
                E::DirectOffline { lookup_id } => {
                    if let Some(c) = state
                        .profiles
                        .direct_contacts
                        .iter_mut()
                        .find(|c| c.lookup_id == lookup_id)
                    {
                        c.status = crate::share::ShareStatus::Offline;
                        changed = true;
                    }
                }
                E::DirectAccessRequest {
                    lookup_id,
                    presence,
                } => match state.profiles.grant_for(&presence.device_id) {
                    Some(g)
                        if g.public_key == presence.public_key
                            && g.node_id == presence.node_id
                            && g.state == crate::share::DirectGrantState::Accepted =>
                    {
                        answers.push((lookup_id, presence, true));
                        ui_event = None;
                    }
                    Some(g)
                        if g.public_key == presence.public_key
                            && g.node_id == presence.node_id
                            && g.state == crate::share::DirectGrantState::Ignored =>
                    {
                        ui_event = None;
                    }
                    Some(_) => log("share direct request identity conflict"),
                    None => {
                        if !state
                            .pending_direct_requests
                            .iter()
                            .any(|p| p.device_id == presence.device_id)
                        {
                            state.pending_direct_requests.push(presence.clone());
                        } else if let Some(existing) = state
                            .pending_direct_requests
                            .iter_mut()
                            .find(|p| p.device_id == presence.device_id)
                        {
                            *existing = presence.clone();
                        }
                        log("share direct request pending in GUI");
                    }
                },
                E::DirectAccessAccepted {
                    lookup_id,
                    requester_device_id,
                    accepted,
                    presence,
                    msg,
                } => {
                    if requester_device_id != state.identity.device_id {
                        continue;
                    }
                    if let Some(c) = state
                        .profiles
                        .direct_contacts
                        .iter_mut()
                        .find(|c| c.lookup_id == lookup_id)
                    {
                        if accepted {
                            c.access_state = crate::share::DirectAccessState::Accepted;
                            c.accepted_at = Some(crate::share::core_now_secs());
                            if let Some(p) = presence {
                                c.remote_device_id = Some(p.device_id.clone());
                                c.remote_public_key = Some(p.public_key.clone());
                                c.accepted_public_key = Some(p.public_key.clone());
                                if c.expected_node_id.trim().is_empty() {
                                    c.expected_node_id = p.node_id.clone();
                                }
                                c.presence = Some(p);
                            }
                            c.status = crate::share::ShareStatus::Available;
                            c.last_error = None;
                        } else {
                            c.access_state = crate::share::DirectAccessState::Ignored;
                            c.status = crate::share::ShareStatus::Failed(
                                msg.unwrap_or_else(|| "Freigabe abgelehnt".into()),
                            );
                        }
                        changed = true;
                    }
                }
                E::RoomRoster { room_id, members } => {
                    let local_device = state.identity.device_id.clone();
                    if let Some(r) = state
                        .profiles
                        .rooms
                        .iter_mut()
                        .find(|r| r.room_id == room_id)
                    {
                        r.status = crate::share::ShareStatus::Available;
                        r.last_seen = Some(crate::share::core_now_secs());
                        for p in members {
                            if p.device_id != local_device {
                                upsert_room_member(r, p);
                            }
                        }
                        changed = true;
                    }
                }
                E::RoomJoined { room_id, presence } => {
                    let local_device = state.identity.device_id.clone();
                    if let Some(r) = state
                        .profiles
                        .rooms
                        .iter_mut()
                        .find(|r| r.room_id == room_id)
                    {
                        if presence.device_id != local_device {
                            upsert_room_member(r, presence);
                            changed = true;
                        }
                    }
                }
                E::RoomLeft { room_id, device_id } => {
                    if let Some(r) = state
                        .profiles
                        .rooms
                        .iter_mut()
                        .find(|r| r.room_id == room_id)
                    {
                        if let Some(m) = r.members.iter_mut().find(|m| m.device_id == device_id) {
                            m.status = crate::share::ShareStatus::Offline;
                            changed = true;
                        }
                    }
                }
            }
            if let Some(event) = ui_event {
                state.ui_events.push(event);
                let overflow = state.ui_events.len().saturating_sub(512);
                if overflow > 0 {
                    state.ui_events.drain(0..overflow);
                }
            }
        }
        if changed {
            let _ = state.profiles.save();
            if let Some(svc) = &state.service {
                configure_service(svc, &state.profiles);
            }
        }
        drop(state);
        for (lookup_id, presence, accepted) in answers {
            service.cmd(crate::share::ShareCmd::AnswerDirectRequest {
                lookup_id,
                presence,
                accepted,
            });
        }
    }

    pub(crate) fn open_share(
        &self,
        target: crate::share::PeerOpenTarget,
    ) -> Result<(String, crate::vfs::BackendHandle, crate::share::ShareStatus), String> {
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            self.reload_now();
            self.drain_events();
            let service = {
                let state = self
                    .state
                    .lock()
                    .map_err(|_| "Share-Worker gesperrt".to_string())?;
                state.service.clone()
            };
            let Some(service) = service else {
                return Err("Share-Server ist nicht konfiguriert oder Auto-Connect ist aus".into());
            };
            service.cmd(crate::share::ShareCmd::Refresh);
            match service.probe_backend_for_target(&target) {
                Ok(opened) => return Ok(opened),
                Err(e) => {
                    if Instant::now() >= deadline {
                        return Err(e);
                    }
                    std::thread::sleep(Duration::from_millis(750));
                }
            }
        }
    }
}

fn configure_or_restart_locked(state: &mut ShareHostState) {
    if state.server.trim().is_empty() || !state.profiles.auto_connect {
        if let Some(svc) = state.service.take() {
            svc.cmd(crate::share::ShareCmd::Stop);
        }
        state.running_server.clear();
        return;
    }
    let needs_restart = state
        .service
        .as_ref()
        .map(|svc| {
            svc.identity.node_id != state.identity.node_id || state.running_server != state.server
        })
        .unwrap_or(true);
    if needs_restart {
        if let Some(svc) = state.service.take() {
            svc.cmd(crate::share::ShareCmd::Stop);
        }
        state.running_server.clear();
        match crate::share::ShareService::start(
            state.server.clone(),
            state.identity.clone(),
            state.profiles.clone(),
        ) {
            Ok(svc) => {
                log("share worker started");
                configure_service(&svc, &state.profiles);
                state.running_server = state.server.clone();
                state.service = Some(svc);
            }
            Err(e) => log(&format!("share worker start failed: {e}")),
        }
    } else if let Some(svc) = &state.service {
        configure_service(svc, &state.profiles);
    }
}

fn configure_service(svc: &crate::share::ShareService, profiles: &crate::share::ShareProfiles) {
    svc.cmd(crate::share::ShareCmd::Configure {
        direct: profiles.direct_contacts.clone(),
        direct_grants: profiles.direct_grants.clone(),
        rooms: profiles.rooms.clone(),
        default_direct_exports: profiles.default_direct_exports.clone(),
    });
}

pub fn open_share_backend(
    target: crate::share::PeerOpenTarget,
) -> Result<(String, crate::vfs::BackendHandle, crate::share::ShareStatus), String> {
    ensure_worker_ready();
    let token = read_token().map_err(|e| format!("Background-Worker Token: {e}"))?;
    let mut last = "Background-Worker nicht erreichbar".to_string();
    for _ in 0..30 {
        match read_ipc_addr()
            .and_then(|addr| TcpStream::connect_timeout(&addr, Duration::from_secs(2)).ok())
        {
            Some(mut stream) => {
                let req = IpcRequest::OpenShare {
                    token: token.clone(),
                    target: target.clone(),
                };
                if let Err(e) = write_request(&mut stream, &req) {
                    last = e.to_string();
                    std::thread::sleep(Duration::from_millis(250));
                    continue;
                }
                let response = match read_response(&mut stream) {
                    Ok(r) => r,
                    Err(e) => {
                        last = e.to_string();
                        std::thread::sleep(Duration::from_millis(250));
                        continue;
                    }
                };
                match response {
                    IpcResponse::OpenOk { label, status } => {
                        let read = stream.try_clone().map_err(|e| e.to_string())?;
                        let inner: crate::vfs::BackendHandle = Arc::new(UnavailableBackend {
                            label: label.clone(),
                        });
                        let agent = match crate::agent::AgentBackend::from_streams(
                            Box::new(read),
                            Box::new(stream),
                            inner,
                        ) {
                            Ok(agent) => agent,
                            Err(e) => {
                                last = format!("Worker-Backend: {e}");
                                clear_stop();
                                clear_heartbeat();
                                clear_ipc_addr();
                                crate::autostart::spawn_daemon_now();
                                std::thread::sleep(Duration::from_millis(300));
                                continue;
                            }
                        };
                        return Ok((label, Arc::new(agent), status));
                    }
                    IpcResponse::Err { msg } => return Err(msg),
                    _ => return Err("Unerwartete Worker-Antwort".into()),
                }
            }
            None => {
                clear_stop();
                clear_heartbeat();
                clear_ipc_addr();
                crate::autostart::spawn_daemon_now();
                std::thread::sleep(Duration::from_millis(300));
            }
        }
    }
    Err(last)
}

pub fn refresh_share_worker() {
    ensure_worker_ready();
    if let (Ok(token), Some(addr)) = (read_token(), read_ipc_addr()) {
        if let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_secs(1)) {
            let _ = write_request(&mut stream, &IpcRequest::RefreshShare { token });
            let _ = read_response(&mut stream);
        }
    }
}

pub fn send_share_command(cmd: crate::share::ShareCmd) -> Result<(), String> {
    ensure_worker_ready();
    let token = read_token().map_err(|e| format!("Background-Worker Token: {e}"))?;
    let addr = read_ipc_addr().ok_or_else(|| "Background-Worker IPC nicht bereit".to_string())?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .map_err(|e| format!("Background-Worker IPC: {e}"))?;
    write_request(&mut stream, &IpcRequest::ShareCommand { token, cmd })
        .map_err(|e| e.to_string())?;
    match read_response(&mut stream).map_err(|e| e.to_string())? {
        IpcResponse::Ok => Ok(()),
        IpcResponse::Err { msg } => Err(msg),
        _ => Err("Unerwartete Worker-Antwort".into()),
    }
}

pub fn drain_share_worker_events() -> Result<ShareWorkerSnapshot, String> {
    ensure_worker_ready();
    let token = read_token().map_err(|e| format!("Background-Worker Token: {e}"))?;
    let addr = read_ipc_addr().ok_or_else(|| "Background-Worker IPC nicht bereit".to_string())?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(1))
        .map_err(|e| format!("Background-Worker IPC: {e}"))?;
    write_request(&mut stream, &IpcRequest::DrainShareEvents { token })
        .map_err(|e| e.to_string())?;
    match read_response(&mut stream).map_err(|e| e.to_string())? {
        IpcResponse::ShareEvents { snapshot } => Ok(snapshot),
        IpcResponse::Err { msg } => Err(msg),
        _ => Err("Unerwartete Worker-Antwort".into()),
    }
}

pub fn ensure_worker_ready() {
    let _ = crate::autostart::enable();
    if ping_worker(Duration::from_millis(700)) {
        return;
    }

    clear_stop();
    clear_heartbeat();
    clear_ipc_addr();
    crate::autostart::spawn_daemon_now();

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if ping_worker(Duration::from_millis(700)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn ping_worker(timeout: Duration) -> bool {
    let Ok(token) = read_token() else {
        return false;
    };
    let Some(addr) = read_ipc_addr() else {
        return false;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, timeout) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    if write_request(&mut stream, &IpcRequest::Ping { token }).is_err() {
        return false;
    }
    matches!(read_response(&mut stream), Ok(IpcResponse::Pong))
}

fn write_request(stream: &mut TcpStream, req: &IpcRequest) -> io::Result<()> {
    let mut line = serde_json::to_string(req).map_err(eio)?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    stream.flush()
}

fn read_response(stream: &mut TcpStream) -> io::Result<IpcResponse> {
    let mut line = String::new();
    read_line_no_buffer(stream, &mut line)?;
    serde_json::from_str(line.trim()).map_err(eio)
}

fn read_line_no_buffer(stream: &mut TcpStream, line: &mut String) -> io::Result<usize> {
    let mut total = 0usize;
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) => return Ok(total),
            Ok(1) => {
                total += 1;
                if byte[0] == b'\n' {
                    return Ok(total);
                }
                line.push(byte[0] as char);
            }
            Ok(_) => unreachable!(),
            Err(e) => return Err(e),
        }
    }
}

fn load_share_server() -> String {
    std::fs::read_to_string(crate::support_dirs::app_data_file("share_server.txt"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn default_device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "Mein Geraet".to_string())
}

fn default_home() -> String {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .to_string_lossy()
        .replace('\\', "/")
}

fn ipc_addr_path() -> std::path::PathBuf {
    crate::support_dirs::sync_data_dir().join(IPC_ADDR_FILE)
}

fn clear_ipc_addr() {
    let _ = std::fs::remove_file(ipc_addr_path());
}

fn ipc_token_path() -> std::path::PathBuf {
    crate::support_dirs::sync_data_dir().join(IPC_TOKEN_FILE)
}

fn write_ipc_addr(addr: SocketAddr) -> io::Result<()> {
    std::fs::write(ipc_addr_path(), addr.to_string())
}

fn read_ipc_addr() -> Option<SocketAddr> {
    std::fs::read_to_string(ipc_addr_path())
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

fn load_or_create_token() -> io::Result<String> {
    if let Ok(token) = read_token() {
        return Ok(token);
    }
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(eio)?;
    let token = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    std::fs::write(ipc_token_path(), &token)?;
    Ok(token)
}

fn read_token() -> io::Result<String> {
    let token = std::fs::read_to_string(ipc_token_path())?
        .trim()
        .to_string();
    if token.len() < 32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "daemon IPC token too short",
        ));
    }
    Ok(token)
}

fn upsert_room_member(room: &mut crate::share::RoomProfile, p: crate::share::PeerPresence) {
    if let Some(m) = room.members.iter_mut().find(|m| m.device_id == p.device_id) {
        if m.public_key != p.public_key || (!m.node_id.is_empty() && m.node_id != p.node_id) {
            m.status = crate::share::ShareStatus::IdentityConflict;
            return;
        }
        m.device_name = p.device_name.clone();
        m.fingerprint = p.fingerprint.clone();
        m.candidates = p.candidates.clone();
        m.node_id = p.node_id.clone();
        m.relay_url = p.relay_url.clone();
        m.last_seen = Some(crate::share::core_now_secs());
        m.status = crate::share::ShareStatus::Available;
        m.presence = Some(p);
    } else {
        room.members.push(crate::share::RoomMember {
            device_id: p.device_id.clone(),
            device_name: p.device_name.clone(),
            fingerprint: p.fingerprint.clone(),
            public_key: p.public_key.clone(),
            node_id: p.node_id.clone(),
            relay_url: p.relay_url.clone(),
            candidates: p.candidates.clone(),
            last_seen: Some(crate::share::core_now_secs()),
            status: crate::share::ShareStatus::Available,
            blocked: false,
            presence: Some(p),
        });
    }
}

struct UnavailableBackend {
    label: String,
}

impl crate::vfs::Backend for UnavailableBackend {
    fn scheme(&self) -> crate::vfs::Scheme {
        crate::vfs::Scheme::Peer
    }

    fn root_display(&self) -> String {
        self.label.clone()
    }

    fn list_dir(&self, _path: &str) -> crate::vfs::VfsResult<Vec<crate::vfs::VfsMeta>> {
        Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "Background-Worker-Verbindung geschlossen",
        ))
    }

    fn stat(&self, _path: &str) -> crate::vfs::VfsResult<crate::vfs::VfsMeta> {
        Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "Background-Worker-Verbindung geschlossen",
        ))
    }

    fn open_read(&self, _path: &str) -> crate::vfs::VfsResult<Box<dyn Read + Send>> {
        Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "Background-Worker-Verbindung geschlossen",
        ))
    }

    fn open_write(&self, _path: &str) -> crate::vfs::VfsResult<Box<dyn Write + Send>> {
        Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "Background-Worker-Verbindung geschlossen",
        ))
    }

    fn rename(&self, _src: &str, _dst: &str) -> crate::vfs::VfsResult<()> {
        Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "Background-Worker-Verbindung geschlossen",
        ))
    }

    fn remove_file(&self, _path: &str) -> crate::vfs::VfsResult<()> {
        Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "Background-Worker-Verbindung geschlossen",
        ))
    }

    fn remove_dir(&self, _path: &str) -> crate::vfs::VfsResult<()> {
        Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "Background-Worker-Verbindung geschlossen",
        ))
    }

    fn mkdir_all(&self, _path: &str) -> crate::vfs::VfsResult<()> {
        Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "Background-Worker-Verbindung geschlossen",
        ))
    }
}

fn eio<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::other(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_rejects_mismatch() {
        assert!(require_token("abc", "abc").is_ok());
        assert!(require_token("abc", "def").is_err());
    }

    #[test]
    fn response_read_preserves_following_stream_bytes() {
        use std::net::{TcpListener, TcpStream};
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            sock.write_all(br#"{"t":"ok"}"#).unwrap();
            sock.write_all(b"\nAGENT").unwrap();
            sock.flush().unwrap();
        });
        let mut client = TcpStream::connect(addr).unwrap();
        match read_response(&mut client).unwrap() {
            IpcResponse::Ok => {}
            other => panic!("unexpected response: {other:?}"),
        }
        let mut rest = [0u8; 5];
        client.read_exact(&mut rest).unwrap();
        assert_eq!(&rest, b"AGENT");
        server.join().unwrap();
    }
}
