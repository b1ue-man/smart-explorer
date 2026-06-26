use crossbeam_channel::{unbounded, Receiver};
use std::io::{self, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tungstenite::{
    connect as ws_connect, stream::MaybeTlsStream, Error as WsError, Message, WebSocket,
};

use super::backend::PeerBackend;
use super::backend::ShareIrohNode;
use super::core::{eio, hmac_proof, now_secs, presence_payload, random_token, verify_hmac};
use super::identity::ShareIdentity;
use super::line::{read_line_limited, MAX_SIGNAL_LINE};
use super::profiles::{fingerprint_matches, ShareProfiles};
use super::system::lan_ips;
use super::types::{
    DirectAccessState, PeerEndpoint, PeerOpenTarget, ShareAuthState, ShareCmd, ShareEvent,
    ShareScope, ShareStatus,
};
use super::wire::{ClientMsg, SrvMsg};
use std::collections::HashSet;

pub struct ShareService {
    pub events: Receiver<ShareEvent>,
    cmds: super::types::CmdTx,
    pub identity: ShareIdentity,
    pub listen_port: u16,
    auth: Arc<Mutex<ShareAuthState>>,
    iroh: Arc<ShareIrohNode>,
    stopped: Arc<AtomicBool>,
    server: String,
    owner: bool,
}

impl ShareService {
    pub fn cmd(&self, c: ShareCmd) {
        if matches!(c, ShareCmd::Stop) {
            self.stopped.store(true, Ordering::Relaxed);
        }
        if let ShareCmd::Configure {
            direct,
            direct_grants,
            rooms,
            default_direct_exports,
        } = &c
        {
            if let Ok(mut s) = self.auth.lock() {
                s.direct_contacts = direct.clone();
                s.direct_grants = direct_grants.clone();
                s.rooms = rooms.clone();
                s.default_direct_exports = default_direct_exports.clone();
            }
        }
        let _ = self.cmds.send(c);
    }

    pub fn probe_backend_for_target(
        &self,
        target: &PeerOpenTarget,
    ) -> Result<(String, crate::vfs::BackendHandle, ShareStatus), String> {
        let endpoint = self.endpoint_for_target(target)?;
        let label = endpoint.label.clone();
        let be = PeerBackend::new(endpoint, self.identity.clone(), self.iroh.clone());
        be.probe_root().map_err(|e| e.to_string())?;
        let status = be.transport_status();
        Ok((label, Arc::new(be), status))
    }

    pub fn relay_url(&self) -> String {
        self.iroh.relay_url().to_string()
    }

    pub fn peer_candidates(&self) -> Vec<String> {
        self.iroh.candidates()
    }

    fn endpoint_for_target(&self, target: &PeerOpenTarget) -> Result<PeerEndpoint, String> {
        let state = self
            .auth
            .lock()
            .map_err(|_| "Share-State gesperrt")?
            .clone();
        match target {
            PeerOpenTarget::Direct { contact_id } => {
                let contact = state
                    .direct_contacts
                    .iter()
                    .find(|c| &c.id == contact_id)
                    .ok_or_else(|| "Direktgeraet nicht gefunden".to_string())?;
                let presence = contact
                    .presence
                    .clone()
                    .ok_or_else(|| "Direktgeraet ist nicht online".to_string())?;
                let secret = ShareProfiles::direct_secret(contact)
                    .ok_or_else(|| "Direkt-Secret fehlt".to_string())?;
                let expected_node_id = if contact.expected_node_id.trim().is_empty() {
                    Some(presence.node_id.clone())
                } else {
                    Some(contact.expected_node_id.clone())
                };
                Ok(PeerEndpoint {
                    label: format!("Share Direkt: {}", contact.display_name),
                    scope: ShareScope::Direct {
                        contact_id: contact.id.clone(),
                    },
                    presence,
                    relation_secret: secret,
                    expected_node_id,
                })
            }
            PeerOpenTarget::RoomDevice { room_id, device_id } => {
                let room = state
                    .rooms
                    .iter()
                    .find(|r| &r.id == room_id || &r.room_id == room_id)
                    .ok_or_else(|| "Raum nicht gefunden".to_string())?;
                let member = room
                    .members
                    .iter()
                    .find(|m| &m.device_id == device_id)
                    .ok_or_else(|| "Geraet nicht im Raum".to_string())?;
                if member.blocked {
                    return Err("Geraet ist blockiert".into());
                }
                let presence = member
                    .presence
                    .clone()
                    .ok_or_else(|| "Raumgeraet ist nicht online".to_string())?;
                let secret = ShareProfiles::room_secret(room)
                    .ok_or_else(|| "Raum-Secret fehlt".to_string())?;
                Ok(PeerEndpoint {
                    label: format!("Share Raum {} / {}", room.name, member.device_name),
                    scope: ShareScope::Room {
                        room_id: room.room_id.clone(),
                    },
                    presence,
                    relation_secret: secret,
                    expected_node_id: Some(member.node_id.clone()),
                })
            }
        }
    }

    pub fn start(
        server: String,
        identity: ShareIdentity,
        profiles: ShareProfiles,
    ) -> io::Result<ShareService> {
        let listen_port = 0;
        let (cmd_tx, cmd_rx) = unbounded::<ShareCmd>();
        let (ev_tx, ev_rx) = unbounded::<ShareEvent>();
        let stopped = Arc::new(AtomicBool::new(false));
        match super::system::ensure_firewall_rule() {
            Ok(msg) => {
                let _ = ev_tx.send(ShareEvent::Status(msg));
            }
            Err(e) => {
                let _ = ev_tx.send(ShareEvent::Status(format!(
                    "Firewall-Regel fuer Peer-Listener nicht gesetzt: {e}"
                )));
            }
        }

        let auth = Arc::new(Mutex::new(ShareAuthState {
            direct_secret: identity.direct_secret(),
            identity: identity.clone(),
            default_direct_exports: profiles.default_direct_exports.clone(),
            direct_contacts: profiles.direct_contacts.clone(),
            direct_grants: profiles.direct_grants.clone(),
            rooms: profiles.rooms.clone(),
            seen_nonces: HashSet::new(),
            direct_online: true,
        }));

        let iroh = ShareIrohNode::start(&server, &identity, auth.clone(), ev_tx.clone())?;
        let _ = ev_tx.send(ShareEvent::Status(format!(
            "Iroh bereit: node={}, relay={}",
            identity.node_id,
            iroh.relay_url()
        )));

        {
            let auth = auth.clone();
            let ev = ev_tx.clone();
            let identity_worker = identity.clone();
            let iroh_worker = iroh.clone();
            let stopped = stopped.clone();
            let worker_server = server.clone();
            std::thread::Builder::new()
                .name("share-signal".into())
                .spawn(move || {
                    worker(
                        worker_server,
                        identity_worker,
                        iroh_worker,
                        auth,
                        cmd_rx,
                        ev,
                        stopped,
                    )
                })
                .ok();
        }

        Ok(ShareService {
            events: ev_rx,
            cmds: cmd_tx,
            identity,
            listen_port,
            auth,
            iroh,
            stopped,
            server,
            owner: true,
        })
    }
}

impl Clone for ShareService {
    fn clone(&self) -> Self {
        Self {
            events: self.events.clone(),
            cmds: self.cmds.clone(),
            identity: self.identity.clone(),
            listen_port: self.listen_port,
            auth: self.auth.clone(),
            iroh: self.iroh.clone(),
            stopped: self.stopped.clone(),
            server: self.server.clone(),
            owner: false,
        }
    }
}

impl Drop for ShareService {
    fn drop(&mut self) {
        if self.owner {
            self.stopped.store(true, Ordering::Relaxed);
        }
    }
}

fn worker(
    server: String,
    identity: ShareIdentity,
    iroh: Arc<ShareIrohNode>,
    auth: Arc<Mutex<ShareAuthState>>,
    cmds: Receiver<ShareCmd>,
    ev: crossbeam_channel::Sender<ShareEvent>,
    stopped_flag: Arc<AtomicBool>,
) {
    let mut stopped = false;
    let mut backoff = Duration::from_secs(1);
    let mut direct_requests_sent = HashSet::new();
    while !stopped && !stopped_flag.load(Ordering::Relaxed) {
        match SignalConnection::connect(&server) {
            Ok(mut signal) => {
                let transport = signal.label().to_string();
                if let Err(e) = send_hello(&mut signal, &identity) {
                    let _ = ev.send(ShareEvent::ServerDisconnected(e.to_string()));
                    std::thread::sleep(backoff);
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                    continue;
                }
                let _ = ev.send(ShareEvent::ServerConnected);
                let _ = ev.send(ShareEvent::Status(format!(
                    "Share-Server verbunden ({transport})"
                )));
                backoff = Duration::from_secs(1);
                let _ = publish_all(&mut signal, &auth, &iroh, &mut direct_requests_sent);
                let mut last_heartbeat = Instant::now();
                let mut last_publish = Instant::now();
                loop {
                    if stopped_flag.load(Ordering::Relaxed) {
                        stopped = true;
                        break;
                    }
                    while let Ok(cmd) = cmds.try_recv() {
                        match cmd {
                            ShareCmd::Configure {
                                direct,
                                direct_grants,
                                rooms,
                                default_direct_exports,
                            } => {
                                if let Ok(mut s) = auth.lock() {
                                    s.direct_contacts = direct;
                                    s.direct_grants = direct_grants;
                                    s.rooms = rooms;
                                    s.default_direct_exports = default_direct_exports;
                                }
                                let _ = publish_all(
                                    &mut signal,
                                    &auth,
                                    &iroh,
                                    &mut direct_requests_sent,
                                );
                            }
                            ShareCmd::Refresh => {
                                let _ = publish_all(
                                    &mut signal,
                                    &auth,
                                    &iroh,
                                    &mut direct_requests_sent,
                                );
                                last_publish = Instant::now();
                            }
                            ShareCmd::SetDirectOnline { online } => {
                                let lookup_id = {
                                    if let Ok(mut s) = auth.lock() {
                                        s.direct_online = online;
                                        s.identity.direct_lookup_id.clone()
                                    } else {
                                        String::new()
                                    }
                                };
                                if online {
                                    let _ = publish_all(
                                        &mut signal,
                                        &auth,
                                        &iroh,
                                        &mut direct_requests_sent,
                                    );
                                    last_publish = Instant::now();
                                } else if !lookup_id.is_empty() {
                                    let _ = send_line(
                                        &mut signal,
                                        &ClientMsg::UnpublishDirect { lookup_id },
                                    );
                                }
                            }
                            ShareCmd::Stop => {
                                stopped = true;
                                stopped_flag.store(true, Ordering::Relaxed);
                                break;
                            }
                            ShareCmd::LeaveRoom { room_id } => {
                                let _ = send_line(&mut signal, &ClientMsg::LeaveRoom { room_id });
                            }
                            ShareCmd::RequestDirect { contact_id } => {
                                let _ = send_direct_request(&mut signal, &auth, &iroh, &contact_id);
                                direct_requests_sent.insert(contact_id);
                            }
                            ShareCmd::AnswerDirectRequest {
                                lookup_id,
                                presence,
                                accepted,
                            } => {
                                let _ = send_direct_answer(
                                    &mut signal,
                                    &auth,
                                    &iroh,
                                    lookup_id,
                                    presence,
                                    accepted,
                                );
                            }
                        }
                    }
                    if stopped {
                        break;
                    }
                    if last_heartbeat.elapsed() >= Duration::from_secs(20) {
                        if send_line(&mut signal, &ClientMsg::Heartbeat).is_err() {
                            break;
                        }
                        last_heartbeat = Instant::now();
                    }
                    if last_publish.elapsed() >= Duration::from_secs(60) {
                        if publish_all(&mut signal, &auth, &iroh, &mut direct_requests_sent)
                            .is_err()
                        {
                            break;
                        }
                        last_publish = Instant::now();
                    }
                    match signal.read_message() {
                        Ok(Some(line)) => handle_server_msg(line.trim(), &auth, &ev),
                        Ok(None) => break,
                        Err(e)
                            if e.kind() == io::ErrorKind::WouldBlock
                                || e.kind() == io::ErrorKind::TimedOut => {}
                        Err(_) => break,
                    }
                }
                let _ = ev.send(ShareEvent::ServerDisconnected("Signaling getrennt".into()));
            }
            Err(e) => {
                let _ = ev.send(ShareEvent::ServerDisconnected(format!(
                    "Share-Server nicht erreichbar: {e}"
                )));
            }
        }
        if !stopped && !stopped_flag.load(Ordering::Relaxed) {
            std::thread::sleep(backoff);
            backoff = (backoff * 2).min(Duration::from_secs(30));
        }
    }
}

fn send_hello(stream: &mut SignalConnection, identity: &ShareIdentity) -> io::Result<()> {
    send_line(
        stream,
        &ClientMsg::Hello {
            protocol_version: 3,
            device_id: identity.device_id.clone(),
            device_name: identity.device_name.clone(),
            listen_port: 0,
            lan: lan_ips(),
            public_key: identity.public_key.clone(),
            fingerprint: identity.fingerprint.clone(),
        },
    )
}

fn publish_all(
    stream: &mut SignalConnection,
    auth: &Arc<Mutex<ShareAuthState>>,
    iroh: &ShareIrohNode,
    direct_requests_sent: &mut HashSet<String>,
) -> io::Result<()> {
    let state = auth
        .lock()
        .map_err(|_| eio("Share-State gesperrt"))?
        .clone();
    if state.direct_online {
        let direct = build_presence(
            "direct",
            &state.identity.direct_lookup_id,
            &state.identity,
            &state.direct_secret,
            iroh,
        );
        send_line(stream, &ClientMsg::PublishDirect { presence: direct })?;
    }
    for contact in state.direct_contacts.iter().filter(|c| c.auto_connect) {
        send_line(
            stream,
            &ClientMsg::WatchDirect {
                lookup_id: contact.lookup_id.clone(),
            },
        )?;
        if contact.access_state == DirectAccessState::Pending
            && !direct_requests_sent.contains(&contact.id)
        {
            send_direct_request_locked(stream, &state, contact, iroh)?;
            direct_requests_sent.insert(contact.id.clone());
        }
    }
    for room in state.rooms.iter().filter(|r| r.auto_join) {
        if let Some(secret) = ShareProfiles::room_secret(room) {
            let presence = build_presence("room", &room.room_id, &state.identity, &secret, iroh);
            send_line(
                stream,
                &ClientMsg::JoinRoom {
                    room_id: room.room_id.clone(),
                    presence,
                },
            )?;
        }
    }
    Ok(())
}

fn send_direct_request(
    stream: &mut SignalConnection,
    auth: &Arc<Mutex<ShareAuthState>>,
    iroh: &ShareIrohNode,
    contact_id: &str,
) -> io::Result<()> {
    let state = auth
        .lock()
        .map_err(|_| eio("Share-State gesperrt"))?
        .clone();
    let contact = state
        .direct_contacts
        .iter()
        .find(|c| c.id == contact_id)
        .ok_or_else(|| eio("Direktgeraet nicht gefunden"))?;
    send_direct_request_locked(stream, &state, contact, iroh)
}

fn send_direct_request_locked(
    stream: &mut SignalConnection,
    state: &ShareAuthState,
    contact: &super::types::DirectContact,
    iroh: &ShareIrohNode,
) -> io::Result<()> {
    let secret = ShareProfiles::direct_secret(contact).ok_or_else(|| eio("Direkt-Secret fehlt"))?;
    let request = build_presence("direct", &contact.lookup_id, &state.identity, &secret, iroh);
    send_line(
        stream,
        &ClientMsg::RequestDirect {
            lookup_id: contact.lookup_id.clone(),
            presence: request,
        },
    )
}

fn send_direct_answer(
    stream: &mut SignalConnection,
    auth: &Arc<Mutex<ShareAuthState>>,
    iroh: &ShareIrohNode,
    lookup_id: String,
    requester: super::types::PeerPresence,
    accepted: bool,
) -> io::Result<()> {
    let state = auth
        .lock()
        .map_err(|_| eio("Share-State gesperrt"))?
        .clone();
    let presence = Some(build_presence(
        "direct",
        &lookup_id,
        &state.identity,
        &state.direct_secret,
        iroh,
    ));
    send_line(
        stream,
        &ClientMsg::DirectAccessAccepted {
            lookup_id,
            requester_device_id: requester.device_id,
            accepted,
            presence,
            msg: None,
        },
    )
}

fn build_presence(
    kind: &str,
    relation_id: &str,
    identity: &ShareIdentity,
    secret: &[u8],
    iroh: &ShareIrohNode,
) -> super::types::PeerPresence {
    let candidates = iroh.candidates();
    let relay_url = iroh.relay_url().to_string();
    let expires_at = now_secs() + 300;
    let nonce = random_token(12);
    let payload = presence_payload(
        kind,
        relation_id,
        &identity.device_id,
        &identity.public_key,
        &identity.node_id,
        &relay_url,
        &candidates,
        expires_at,
        &nonce,
    );
    super::types::PeerPresence {
        kind: kind.to_string(),
        relation_id: relation_id.to_string(),
        device_id: identity.device_id.clone(),
        device_name: identity.device_name.clone(),
        public_key: identity.public_key.clone(),
        fingerprint: identity.fingerprint.clone(),
        node_id: identity.node_id.clone(),
        relay_url,
        candidates,
        expires_at,
        nonce,
        proof: hmac_proof(secret, &payload),
    }
}

enum SignalConnection {
    Tcp {
        label: String,
        stream: TcpStream,
        reader: io::BufReader<TcpStream>,
    },
    WebSocket {
        label: String,
        socket: Box<WebSocket<MaybeTlsStream<TcpStream>>>,
    },
}

impl SignalConnection {
    fn connect(config: &str) -> io::Result<Self> {
        let endpoints = signal_endpoints(config);
        if endpoints.is_empty() {
            return Err(eio("Share-Server-Adresse fehlt"));
        }
        let mut errors = Vec::new();
        for endpoint in endpoints {
            match Self::connect_one(&endpoint) {
                Ok(conn) => return Ok(conn),
                Err(e) => errors.push(format!("{endpoint}: {e}")),
            }
        }
        Err(eio(format!(
            "keine Signaling-Verbindung moeglich ({})",
            errors.join("; ")
        )))
    }

    fn connect_one(endpoint: &str) -> io::Result<Self> {
        let normalized = normalize_signal_endpoint(endpoint);
        if normalized.starts_with("ws://") || normalized.starts_with("wss://") {
            return Self::connect_ws(&normalized);
        }
        if let Some(raw) = normalized.strip_prefix("tcp://") {
            return Self::connect_tcp(&normalize_tcp_addr(raw));
        }
        if normalized.contains("://") {
            return Err(eio("unbekanntes Share-Server-Schema"));
        }
        Self::connect_tcp(&normalize_tcp_addr(&normalized))
    }

    fn connect_tcp(addr: &str) -> io::Result<Self> {
        let stream = TcpStream::connect(addr)?;
        let _ = stream.set_nodelay(true);
        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
        let reader = io::BufReader::new(stream.try_clone()?);
        Ok(Self::Tcp {
            label: format!("tcp://{addr}"),
            stream,
            reader,
        })
    }

    fn connect_ws(url: &str) -> io::Result<Self> {
        let (mut socket, _) = ws_connect(url).map_err(ws_to_io)?;
        set_ws_timeout(socket.get_mut(), Duration::from_millis(500));
        Ok(Self::WebSocket {
            label: url.to_string(),
            socket: Box::new(socket),
        })
    }

    fn label(&self) -> &str {
        match self {
            Self::Tcp { label, .. } | Self::WebSocket { label, .. } => label,
        }
    }

    fn send(&mut self, msg: &ClientMsg) -> io::Result<()> {
        match self {
            Self::Tcp { stream, .. } => {
                let mut line = serde_json::to_string(msg).map_err(eio)?;
                line.push('\n');
                stream.write_all(line.as_bytes())?;
                stream.flush()
            }
            Self::WebSocket { socket, .. } => {
                let text = serde_json::to_string(msg).map_err(eio)?;
                socket.send(Message::Text(text)).map_err(ws_to_io)?;
                socket.flush().map_err(ws_to_io)
            }
        }
    }

    fn read_message(&mut self) -> io::Result<Option<String>> {
        match self {
            Self::Tcp { reader, .. } => {
                let mut line = String::new();
                match read_line_limited(reader, &mut line, MAX_SIGNAL_LINE) {
                    Ok(0) => Ok(None),
                    Ok(_) => Ok(Some(line)),
                    Err(e) => Err(e),
                }
            }
            Self::WebSocket { socket, .. } => loop {
                match socket.read() {
                    Ok(Message::Text(text)) => return Ok(Some(text)),
                    Ok(Message::Binary(bytes)) => {
                        return String::from_utf8(bytes).map(Some).map_err(eio);
                    }
                    Ok(Message::Ping(payload)) => {
                        socket.send(Message::Pong(payload)).map_err(ws_to_io)?;
                        socket.flush().map_err(ws_to_io)?;
                    }
                    Ok(Message::Pong(_)) => {}
                    Ok(Message::Close(_)) => return Ok(None),
                    Ok(_) => {}
                    Err(WsError::Io(e))
                        if e.kind() == io::ErrorKind::WouldBlock
                            || e.kind() == io::ErrorKind::TimedOut =>
                    {
                        return Err(e)
                    }
                    Err(WsError::ConnectionClosed | WsError::AlreadyClosed) => return Ok(None),
                    Err(e) => return Err(ws_to_io(e)),
                }
            },
        }
    }
}

fn send_line(stream: &mut SignalConnection, msg: &ClientMsg) -> io::Result<()> {
    stream.send(msg)
}

fn signal_endpoints(config: &str) -> Vec<String> {
    config
        .split([',', ';'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn normalize_signal_endpoint(endpoint: &str) -> String {
    let trimmed = endpoint.trim();
    if let Some(rest) = trimmed.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = trimmed.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        trimmed.to_string()
    }
}

fn normalize_tcp_addr(addr: &str) -> String {
    let addr = addr.trim().trim_end_matches('/');
    if addr.is_empty() || addr.starts_with('[') || addr.rsplit_once(':').is_some() {
        addr.to_string()
    } else {
        format!("{addr}:51820")
    }
}

fn set_ws_timeout(stream: &mut MaybeTlsStream<TcpStream>, timeout: Duration) {
    match stream {
        MaybeTlsStream::Plain(tcp) => {
            let _ = tcp.set_read_timeout(Some(timeout));
            let _ = tcp.set_write_timeout(Some(timeout));
        }
        MaybeTlsStream::Rustls(tls) => {
            let _ = tls.sock.set_read_timeout(Some(timeout));
            let _ = tls.sock.set_write_timeout(Some(timeout));
        }
        #[allow(unreachable_patterns)]
        _ => {}
    }
}

fn ws_to_io(err: WsError) -> io::Error {
    match err {
        WsError::Io(e) => e,
        other => eio(other),
    }
}

fn handle_server_msg(
    line: &str,
    auth: &Arc<Mutex<ShareAuthState>>,
    ev: &crossbeam_channel::Sender<ShareEvent>,
) {
    if line.is_empty() {
        return;
    }
    let msg: SrvMsg = match serde_json::from_str(line) {
        Ok(m) => m,
        Err(e) => {
            let _ = ev.send(ShareEvent::Error(format!("Server-Nachricht: {e}")));
            return;
        }
    };
    match msg {
        SrvMsg::HelloOk | SrvMsg::Pong => {}
        SrvMsg::DirectAvailable {
            lookup_id,
            presence,
        } => {
            if verify_direct_presence(&lookup_id, &presence, auth) {
                let _ = ev.send(ShareEvent::DirectAvailable {
                    lookup_id,
                    presence,
                });
            }
        }
        SrvMsg::DirectOffline { lookup_id } => {
            let _ = ev.send(ShareEvent::DirectOffline { lookup_id });
        }
        SrvMsg::DirectAccessRequest {
            lookup_id,
            presence,
        } => {
            if verify_local_direct_request(&lookup_id, &presence, auth) {
                let _ = ev.send(ShareEvent::DirectAccessRequest {
                    lookup_id,
                    presence,
                });
            }
        }
        SrvMsg::DirectAccessAccepted {
            lookup_id,
            requester_device_id,
            accepted,
            presence,
            msg,
        } => {
            if verify_direct_access_accepted(
                &lookup_id,
                &requester_device_id,
                accepted,
                presence.as_ref(),
                auth,
            ) {
                let _ = ev.send(ShareEvent::DirectAccessAccepted {
                    lookup_id,
                    requester_device_id,
                    accepted,
                    presence,
                    msg,
                });
            }
        }
        SrvMsg::RoomRoster { room_id, members } => {
            let valid: Vec<_> = members
                .into_iter()
                .filter(|p| verify_room_presence(&room_id, p, auth))
                .collect();
            let _ = ev.send(ShareEvent::RoomRoster {
                room_id,
                members: valid,
            });
        }
        SrvMsg::RoomJoined { room_id, presence } => {
            if verify_room_presence(&room_id, &presence, auth) {
                let _ = ev.send(ShareEvent::RoomJoined { room_id, presence });
            }
        }
        SrvMsg::RoomLeft { room_id, device_id } => {
            let _ = ev.send(ShareEvent::RoomLeft { room_id, device_id });
        }
        SrvMsg::Error { scope, msg } => {
            let _ = ev.send(ShareEvent::Error(format!("{scope}: {msg}")));
        }
    }
}

fn verify_local_direct_request(
    lookup_id: &str,
    presence: &super::types::PeerPresence,
    auth: &Arc<Mutex<ShareAuthState>>,
) -> bool {
    if presence.expires_at < now_secs()
        || presence.kind != "direct"
        || presence.relation_id != lookup_id
    {
        return false;
    }
    let mut state = match auth.lock() {
        Ok(s) => s,
        Err(_) => return false,
    };
    if lookup_id != state.identity.direct_lookup_id {
        return false;
    }
    let replay_key = format!(
        "direct-request:{lookup_id}:{}:{}",
        presence.device_id, presence.nonce
    );
    if state.seen_nonces.contains(&replay_key) {
        return false;
    }
    let payload = presence_payload(
        "direct",
        lookup_id,
        &presence.device_id,
        &presence.public_key,
        &presence.node_id,
        &presence.relay_url,
        &presence.candidates,
        presence.expires_at,
        &presence.nonce,
    );
    if !verify_hmac(&state.direct_secret, &payload, &presence.proof) {
        return false;
    }
    remember_nonce(&mut state.seen_nonces, replay_key);
    true
}

fn verify_direct_access_accepted(
    lookup_id: &str,
    requester_device_id: &str,
    _accepted: bool,
    presence: Option<&super::types::PeerPresence>,
    auth: &Arc<Mutex<ShareAuthState>>,
) -> bool {
    verify_direct_access_accepted_using(
        lookup_id,
        requester_device_id,
        presence,
        auth,
        ShareProfiles::direct_secret,
    )
}

fn verify_direct_access_accepted_using<F>(
    lookup_id: &str,
    requester_device_id: &str,
    presence: Option<&super::types::PeerPresence>,
    auth: &Arc<Mutex<ShareAuthState>>,
    secret_for: F,
) -> bool
where
    F: FnOnce(&super::types::DirectContact) -> Option<Vec<u8>>,
{
    let mut state = match auth.lock() {
        Ok(s) => s,
        Err(_) => return false,
    };
    if requester_device_id != state.identity.device_id {
        return false;
    }
    let Some(presence) = presence else {
        return false;
    };
    if presence.expires_at < now_secs()
        || presence.kind != "direct"
        || presence.relation_id != lookup_id
    {
        return false;
    }
    let Some(contact) = state
        .direct_contacts
        .iter()
        .find(|c| c.lookup_id == lookup_id)
    else {
        return false;
    };
    if !fingerprint_matches(&presence.public_key, &contact.expected_fingerprint) {
        return false;
    }
    if !contact.expected_node_id.trim().is_empty() && contact.expected_node_id != presence.node_id {
        return false;
    }
    let Some(secret) = secret_for(contact) else {
        return false;
    };
    let replay_key = format!(
        "direct-accepted:{lookup_id}:{}:{}",
        presence.device_id, presence.nonce
    );
    if state.seen_nonces.contains(&replay_key) {
        return false;
    }
    let payload = presence_payload(
        "direct",
        lookup_id,
        &presence.device_id,
        &presence.public_key,
        &presence.node_id,
        &presence.relay_url,
        &presence.candidates,
        presence.expires_at,
        &presence.nonce,
    );
    if !verify_hmac(&secret, &payload, &presence.proof) {
        return false;
    }
    remember_nonce(&mut state.seen_nonces, replay_key);
    true
}

fn verify_direct_presence(
    lookup_id: &str,
    presence: &super::types::PeerPresence,
    auth: &Arc<Mutex<ShareAuthState>>,
) -> bool {
    if presence.expires_at < now_secs()
        || presence.kind != "direct"
        || presence.relation_id != lookup_id
    {
        return false;
    }
    let mut state = match auth.lock() {
        Ok(s) => s,
        Err(_) => return false,
    };
    let Some(contact) = state
        .direct_contacts
        .iter()
        .find(|c| c.lookup_id == lookup_id)
    else {
        return false;
    };
    if !fingerprint_matches(&presence.public_key, &contact.expected_fingerprint) {
        return false;
    }
    if !contact.expected_node_id.trim().is_empty() && contact.expected_node_id != presence.node_id {
        return false;
    }
    let Some(secret) = ShareProfiles::direct_secret(contact) else {
        return false;
    };
    let replay_key = format!("direct:{lookup_id}:{}", presence.nonce);
    if state.seen_nonces.contains(&replay_key) {
        return false;
    }
    let payload = presence_payload(
        "direct",
        lookup_id,
        &presence.device_id,
        &presence.public_key,
        &presence.node_id,
        &presence.relay_url,
        &presence.candidates,
        presence.expires_at,
        &presence.nonce,
    );
    if !verify_hmac(&secret, &payload, &presence.proof) {
        return false;
    }
    remember_nonce(&mut state.seen_nonces, replay_key);
    true
}

fn verify_room_presence(
    room_id: &str,
    presence: &super::types::PeerPresence,
    auth: &Arc<Mutex<ShareAuthState>>,
) -> bool {
    if presence.expires_at < now_secs()
        || presence.kind != "room"
        || presence.relation_id != room_id
    {
        return false;
    }
    let mut state = match auth.lock() {
        Ok(s) => s,
        Err(_) => return false,
    };
    let Some(room) = state.rooms.iter().find(|r| r.room_id == room_id) else {
        return false;
    };
    let Some(secret) = ShareProfiles::room_secret(room) else {
        return false;
    };
    let replay_key = format!("room:{room_id}:{}:{}", presence.device_id, presence.nonce);
    if state.seen_nonces.contains(&replay_key) {
        return false;
    }
    let payload = presence_payload(
        "room",
        room_id,
        &presence.device_id,
        &presence.public_key,
        &presence.node_id,
        &presence.relay_url,
        &presence.candidates,
        presence.expires_at,
        &presence.nonce,
    );
    if !verify_hmac(&secret, &payload, &presence.proof) {
        return false;
    }
    remember_nonce(&mut state.seen_nonces, replay_key);
    true
}

fn remember_nonce(seen: &mut HashSet<String>, key: String) {
    if seen.len() > 4096 {
        seen.clear();
    }
    seen.insert(key);
}

#[cfg(test)]
mod tests {
    use super::{
        build_presence, normalize_signal_endpoint, normalize_tcp_addr, remember_nonce,
        signal_endpoints, verify_direct_access_accepted_using, verify_local_direct_request,
        ShareIrohNode, ShareService,
    };
    use crate::share::core::public_fingerprint;
    use crate::share::fs::ShareExportConfig;
    use crate::share::identity::ShareIdentity;
    use crate::share::types::ShareAuthState;
    use crate::share::types::{
        DirectAccessState, DirectContact, DirectGrant, RoomProfile, ShareCmd, ShareStatus,
    };
    use crossbeam_channel::unbounded;
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    #[test]
    fn nonce_cache_detects_replay() {
        let mut seen = HashSet::new();
        let key = "direct:lookup:nonce".to_string();
        assert!(!seen.contains(&key));
        remember_nonce(&mut seen, key.clone());
        assert!(seen.contains(&key));
    }

    #[test]
    fn dropping_probe_clone_does_not_stop_owner_service() {
        let svc = test_service();
        let stopped = svc.stopped.clone();
        let probe_clone = svc.clone();
        drop(probe_clone);
        assert!(!stopped.load(Ordering::Relaxed));
        drop(svc);
        assert!(stopped.load(Ordering::Relaxed));
    }

    #[test]
    fn configure_updates_auth_state_synchronously() {
        let svc = test_service();
        let contact = DirectContact {
            id: "contact-a".into(),
            display_name: "A".into(),
            lookup_id: "lookup-a".into(),
            expected_fingerprint: "00".repeat(16),
            expected_node_id: "node-a".into(),
            remote_device_id: None,
            remote_public_key: None,
            auto_connect: true,
            auto_open: false,
            last_seen: None,
            status: ShareStatus::Waiting,
            last_error: None,
            presence: None,
            exports: ShareExportConfig::default(),
            access_state: DirectAccessState::Pending,
            request_sent_at: None,
            accepted_at: None,
            accepted_public_key: None,
        };
        let room = RoomProfile {
            id: "room-profile-a".into(),
            name: "Room A".into(),
            room_id: "room-a".into(),
            auto_join: true,
            last_seen: None,
            status: ShareStatus::Waiting,
            members: Vec::new(),
            exports: ShareExportConfig::default(),
        };
        svc.cmd(ShareCmd::Configure {
            direct: vec![contact],
            direct_grants: Vec::new(),
            rooms: vec![room],
            default_direct_exports: ShareExportConfig::default(),
        });
        assert_eq!(svc.auth.lock().unwrap().direct_contacts[0].id, "contact-a");
        assert_eq!(svc.auth.lock().unwrap().rooms[0].room_id, "room-a");
    }

    #[test]
    fn signal_endpoint_config_supports_https_and_fallbacks() {
        assert_eq!(
            signal_endpoints(" wss://share.example/ws ; 10.0.0.5:51820 "),
            vec![
                "wss://share.example/ws".to_string(),
                "10.0.0.5:51820".to_string()
            ]
        );
        assert_eq!(
            normalize_signal_endpoint("https://share.example/ws"),
            "wss://share.example/ws"
        );
        assert_eq!(
            normalize_signal_endpoint("http://share.example/ws"),
            "ws://share.example/ws"
        );
    }

    #[test]
    fn tcp_endpoint_defaults_to_share_port() {
        assert_eq!(normalize_tcp_addr("share.example"), "share.example:51820");
        assert_eq!(normalize_tcp_addr("share.example:443"), "share.example:443");
        assert_eq!(normalize_tcp_addr("[::1]:51820"), "[::1]:51820");
    }

    #[test]
    fn local_direct_request_requires_own_direct_secret() {
        let svc = test_service();
        let identity = svc.identity.clone();
        let secret = svc.auth.lock().unwrap().direct_secret.clone();
        let presence = build_presence(
            "direct",
            &identity.direct_lookup_id,
            &identity,
            &secret,
            &svc.iroh,
        );
        assert!(verify_local_direct_request(
            &identity.direct_lookup_id,
            &presence,
            &svc.auth
        ));

        let wrong = build_presence(
            "direct",
            &identity.direct_lookup_id,
            &identity,
            &[9u8; 32],
            &svc.iroh,
        );
        assert!(!verify_local_direct_request(
            &identity.direct_lookup_id,
            &wrong,
            &svc.auth
        ));
    }

    #[test]
    fn direct_accept_or_reject_requires_signed_owner_presence() {
        let svc = test_service();
        let secret = vec![7u8; 32];
        let owner = test_identity("owner", "Owner", "lookup-owner");
        let contact = DirectContact {
            id: "contact-owner".into(),
            display_name: "Owner".into(),
            lookup_id: "lookup-owner".into(),
            expected_fingerprint: owner.fingerprint.clone(),
            expected_node_id: owner.node_id.clone(),
            remote_device_id: None,
            remote_public_key: None,
            auto_connect: true,
            auto_open: false,
            last_seen: None,
            status: ShareStatus::WaitingForAccess,
            last_error: None,
            presence: None,
            exports: ShareExportConfig::default(),
            access_state: DirectAccessState::Pending,
            request_sent_at: None,
            accepted_at: None,
            accepted_public_key: None,
        };
        svc.auth.lock().unwrap().direct_contacts = vec![contact];
        let signed = build_presence("direct", "lookup-owner", &owner, &secret, &svc.iroh);
        assert!(verify_direct_access_accepted_using(
            "lookup-owner",
            &svc.identity.device_id,
            Some(&signed),
            &svc.auth,
            |_| Some(secret.clone())
        ));
        assert!(!verify_direct_access_accepted_using(
            "lookup-owner",
            &svc.identity.device_id,
            None,
            &svc.auth,
            |_| Some(secret.clone())
        ));
        let wrong = build_presence("direct", "lookup-owner", &owner, &[9u8; 32], &svc.iroh);
        assert!(!verify_direct_access_accepted_using(
            "lookup-owner",
            &svc.identity.device_id,
            Some(&wrong),
            &svc.auth,
            |_| Some(secret.clone())
        ));
    }

    #[test]
    fn presence_binds_node_id_and_relay_url() {
        let svc = test_service();
        let relation_id = "lookup-owner";
        let secret = svc.auth.lock().unwrap().direct_secret.clone();
        let owner = test_identity("owner", "Owner", relation_id);
        let contact = DirectContact {
            id: "contact-owner".into(),
            display_name: "Owner".into(),
            lookup_id: relation_id.into(),
            expected_fingerprint: owner.fingerprint.clone(),
            expected_node_id: owner.node_id.clone(),
            remote_device_id: None,
            remote_public_key: None,
            auto_connect: true,
            auto_open: false,
            last_seen: None,
            status: ShareStatus::WaitingForAccess,
            last_error: None,
            presence: None,
            exports: ShareExportConfig::default(),
            access_state: DirectAccessState::Pending,
            request_sent_at: None,
            accepted_at: None,
            accepted_public_key: None,
        };
        svc.auth.lock().unwrap().direct_contacts = vec![contact];
        let presence = build_presence("direct", relation_id, &owner, &secret, &svc.iroh);
        let mut tampered = presence.clone();
        tampered.node_id.push('x');
        assert!(!verify_direct_access_accepted_using(
            relation_id,
            &svc.identity.device_id,
            Some(&tampered),
            &svc.auth,
            |_| Some(secret.clone())
        ));
        assert!(verify_direct_access_accepted_using(
            relation_id,
            &svc.identity.device_id,
            Some(&presence),
            &svc.auth,
            |_| Some(secret.clone())
        ));
    }

    fn test_service() -> ShareService {
        let (cmd_tx, _cmd_rx) = unbounded();
        let (ev_tx, ev_rx) = unbounded();
        let identity = test_identity("device-a", "Device A", "lookup-local");
        let auth = Arc::new(Mutex::new(ShareAuthState {
            identity: identity.clone(),
            direct_secret: vec![0u8; 32],
            default_direct_exports: ShareExportConfig::default(),
            direct_contacts: Vec::new(),
            direct_grants: Vec::<DirectGrant>::new(),
            rooms: Vec::new(),
            seen_nonces: HashSet::new(),
            direct_online: true,
        }));
        let iroh = ShareIrohNode::start("127.0.0.1:0", &identity, auth.clone(), ev_tx).unwrap();
        ShareService {
            events: ev_rx,
            cmds: cmd_tx,
            identity,
            listen_port: 0,
            auth,
            iroh,
            stopped: Arc::new(AtomicBool::new(false)),
            server: "127.0.0.1:0".into(),
            owner: true,
        }
    }

    fn test_identity(device_id: &str, device_name: &str, lookup: &str) -> ShareIdentity {
        let secret = iroh::SecretKey::generate();
        let node_id = secret.public().to_string();
        let fingerprint = public_fingerprint(node_id.as_bytes());
        ShareIdentity {
            device_id: device_id.into(),
            device_name: device_name.into(),
            direct_lookup_id: lookup.into(),
            public_key: node_id.clone(),
            fingerprint,
            node_id,
            iroh_secret: secret,
        }
    }
}
