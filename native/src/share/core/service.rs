use crossbeam_channel::{unbounded, Receiver};
use std::io::{self, BufRead, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tungstenite::{
    connect as ws_connect, stream::MaybeTlsStream, Error as WsError, Message, WebSocket,
};

use super::backend::PeerBackend;
use super::core::{
    b64_decode, eio, hmac_proof, now_secs, presence_payload, random_token, verify_hmac,
};
use super::identity::ShareIdentity;
use super::profiles::{fingerprint_matches, ShareProfiles};
use super::system::lan_ips;
use super::transfer::{accept_loop, ShareAuthState};
use super::types::{PeerEndpoint, PeerOpenTarget, ShareCmd, ShareEvent, ShareScope};
use super::wire::{ClientMsg, SrvMsg};
use std::collections::HashSet;

pub struct ShareService {
    pub events: Receiver<ShareEvent>,
    cmds: super::types::CmdTx,
    pub identity: ShareIdentity,
    pub listen_port: u16,
    auth: Arc<Mutex<ShareAuthState>>,
    stopped: Arc<AtomicBool>,
    owner: bool,
}

impl ShareService {
    pub fn cmd(&self, c: ShareCmd) {
        if matches!(c, ShareCmd::Stop) {
            self.stopped.store(true, Ordering::Relaxed);
        }
        if let ShareCmd::Configure {
            direct,
            rooms,
            default_direct_exports,
        } = &c
        {
            if let Ok(mut s) = self.auth.lock() {
                s.direct_contacts = direct.clone();
                s.rooms = rooms.clone();
                s.default_direct_exports = default_direct_exports.clone();
            }
        }
        let _ = self.cmds.send(c);
    }

    pub fn probe_backend_for_target(
        &self,
        target: &PeerOpenTarget,
    ) -> Result<(String, crate::vfs::BackendHandle), String> {
        let endpoint = self.endpoint_for_target(target)?;
        let label = endpoint.label.clone();
        let be = PeerBackend::new(endpoint, self.identity.clone());
        be.probe_root().map_err(|e| e.to_string())?;
        Ok((label, Arc::new(be)))
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
                let expected_public_key = contact
                    .remote_public_key
                    .as_ref()
                    .and_then(|s| b64_decode(s).ok())
                    .or_else(|| b64_decode(&presence.public_key).ok());
                Ok(PeerEndpoint {
                    label: format!("Share Direkt: {}", contact.display_name),
                    scope: ShareScope::Direct {
                        contact_id: contact.id.clone(),
                    },
                    presence,
                    relation_secret: secret,
                    expected_public_key,
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
                    expected_public_key: b64_decode(&member.public_key).ok(),
                })
            }
        }
    }

    pub fn start(
        server: String,
        identity: ShareIdentity,
        profiles: ShareProfiles,
    ) -> io::Result<ShareService> {
        let listener = TcpListener::bind("0.0.0.0:0")?;
        let listen_port = listener.local_addr()?.port();
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
            rooms: profiles.rooms.clone(),
            seen_nonces: HashSet::new(),
            direct_online: true,
        }));

        {
            let auth = auth.clone();
            let ev = ev_tx.clone();
            let stopped = stopped.clone();
            std::thread::Builder::new()
                .name("share-accept".into())
                .spawn(move || accept_loop(listener, auth, ev, stopped))
                .ok();
        }

        {
            let auth = auth.clone();
            let ev = ev_tx.clone();
            let identity_worker = identity.clone();
            let stopped = stopped.clone();
            std::thread::Builder::new()
                .name("share-signal".into())
                .spawn(move || {
                    worker(
                        server,
                        identity_worker,
                        listen_port,
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
            stopped,
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
            stopped: self.stopped.clone(),
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
    listen_port: u16,
    auth: Arc<Mutex<ShareAuthState>>,
    cmds: Receiver<ShareCmd>,
    ev: crossbeam_channel::Sender<ShareEvent>,
    stopped_flag: Arc<AtomicBool>,
) {
    let mut stopped = false;
    let mut backoff = Duration::from_secs(1);
    while !stopped && !stopped_flag.load(Ordering::Relaxed) {
        match SignalConnection::connect(&server) {
            Ok(mut signal) => {
                let transport = signal.label().to_string();
                if let Err(e) = send_hello(&mut signal, &identity, listen_port) {
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
                let _ = publish_all(&mut signal, &auth, listen_port);
                let mut last_heartbeat = Instant::now();
                loop {
                    if stopped_flag.load(Ordering::Relaxed) {
                        stopped = true;
                        break;
                    }
                    while let Ok(cmd) = cmds.try_recv() {
                        match cmd {
                            ShareCmd::Configure {
                                direct,
                                rooms,
                                default_direct_exports,
                            } => {
                                if let Ok(mut s) = auth.lock() {
                                    s.direct_contacts = direct;
                                    s.rooms = rooms;
                                    s.default_direct_exports = default_direct_exports;
                                }
                                let _ = publish_all(&mut signal, &auth, listen_port);
                            }
                            ShareCmd::Refresh => {
                                let _ = publish_all(&mut signal, &auth, listen_port);
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
                                    let _ = publish_all(&mut signal, &auth, listen_port);
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
                            ShareCmd::Send(_) | ShareCmd::Answer { .. } => {}
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

fn send_hello(
    stream: &mut SignalConnection,
    identity: &ShareIdentity,
    listen_port: u16,
) -> io::Result<()> {
    send_line(
        stream,
        &ClientMsg::Hello {
            protocol_version: 2,
            device_id: identity.device_id.clone(),
            device_name: identity.device_name.clone(),
            listen_port,
            lan: lan_ips(),
            public_key: identity.public_key.clone(),
            fingerprint: identity.fingerprint.clone(),
        },
    )
}

fn publish_all(
    stream: &mut SignalConnection,
    auth: &Arc<Mutex<ShareAuthState>>,
    listen_port: u16,
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
            listen_port,
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
        if let Some(secret) = ShareProfiles::direct_secret(contact) {
            let request = build_presence(
                "direct",
                &contact.lookup_id,
                &state.identity,
                &secret,
                listen_port,
            );
            send_line(
                stream,
                &ClientMsg::RequestDirect {
                    lookup_id: contact.lookup_id.clone(),
                    presence: request,
                },
            )?;
        }
    }
    for room in state.rooms.iter().filter(|r| r.auto_join) {
        if let Some(secret) = ShareProfiles::room_secret(room) {
            let presence =
                build_presence("room", &room.room_id, &state.identity, &secret, listen_port);
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

fn build_presence(
    kind: &str,
    relation_id: &str,
    identity: &ShareIdentity,
    secret: &[u8],
    listen_port: u16,
) -> super::types::PeerPresence {
    let candidates: Vec<String> = lan_ips()
        .into_iter()
        .map(|ip| format!("{ip}:{listen_port}"))
        .collect();
    let expires_at = now_secs() + 90;
    let nonce = random_token(12);
    let payload = presence_payload(
        kind,
        relation_id,
        &identity.device_id,
        &identity.public_key,
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
        socket: WebSocket<MaybeTlsStream<TcpStream>>,
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
            socket,
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
                match reader.read_line(&mut line) {
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
        signal_endpoints, verify_local_direct_request, ShareService,
    };
    use crate::share::fs::ShareExportConfig;
    use crate::share::identity::ShareIdentity;
    use crate::share::transfer::ShareAuthState;
    use crate::share::types::{DirectContact, RoomProfile, ShareCmd, ShareStatus};
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
            remote_device_id: None,
            remote_public_key: None,
            auto_connect: true,
            auto_open: false,
            last_seen: None,
            status: ShareStatus::Waiting,
            last_error: None,
            presence: None,
            exports: ShareExportConfig::default(),
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
            12345,
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
            12345,
        );
        assert!(!verify_local_direct_request(
            &identity.direct_lookup_id,
            &wrong,
            &svc.auth
        ));
    }

    fn test_service() -> ShareService {
        let (cmd_tx, _cmd_rx) = unbounded();
        let (_ev_tx, ev_rx) = unbounded();
        let identity = ShareIdentity {
            device_id: "device-a".into(),
            device_name: "Device A".into(),
            direct_lookup_id: "lookup-local".into(),
            public_key: String::new(),
            fingerprint: String::new(),
            private_key: Vec::new(),
        };
        let auth = Arc::new(Mutex::new(ShareAuthState {
            identity: identity.clone(),
            direct_secret: vec![0u8; 32],
            default_direct_exports: ShareExportConfig::default(),
            direct_contacts: Vec::new(),
            rooms: Vec::new(),
            seen_nonces: HashSet::new(),
            direct_online: true,
        }));
        ShareService {
            events: ev_rx,
            cmds: cmd_tx,
            identity,
            listen_port: 0,
            auth,
            stopped: Arc::new(AtomicBool::new(false)),
            owner: true,
        }
    }
}
