use crossbeam_channel::{unbounded, Receiver};
use std::io::{self, BufRead, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
        match TcpStream::connect(&server) {
            Ok(mut stream) => {
                let _ = stream.set_nodelay(true);
                let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                if let Err(e) = send_hello(&mut stream, &identity, listen_port) {
                    let _ = ev.send(ShareEvent::ServerDisconnected(e.to_string()));
                    std::thread::sleep(backoff);
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                    continue;
                }
                let _ = ev.send(ShareEvent::ServerConnected);
                let _ = ev.send(ShareEvent::Status("Share-Server verbunden".into()));
                backoff = Duration::from_secs(1);
                let mut reader = match stream.try_clone() {
                    Ok(s) => io::BufReader::new(s),
                    Err(e) => {
                        let _ = ev.send(ShareEvent::ServerDisconnected(e.to_string()));
                        continue;
                    }
                };
                let _ = publish_all(&mut stream, &auth, listen_port);
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
                                let _ = publish_all(&mut stream, &auth, listen_port);
                            }
                            ShareCmd::Refresh => {
                                let _ = publish_all(&mut stream, &auth, listen_port);
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
                                    let _ = publish_all(&mut stream, &auth, listen_port);
                                } else if !lookup_id.is_empty() {
                                    let _ = send_line(
                                        &mut stream,
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
                                let _ = send_line(&mut stream, &ClientMsg::LeaveRoom { room_id });
                            }
                            ShareCmd::Send(_) | ShareCmd::Answer { .. } => {}
                        }
                    }
                    if stopped {
                        break;
                    }
                    if last_heartbeat.elapsed() >= Duration::from_secs(20) {
                        if send_line(&mut stream, &ClientMsg::Heartbeat).is_err() {
                            break;
                        }
                        last_heartbeat = Instant::now();
                    }
                    let mut line = String::new();
                    match reader.read_line(&mut line) {
                        Ok(0) => break,
                        Ok(_) => handle_server_msg(line.trim(), &auth, &ev),
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
    stream: &mut TcpStream,
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
    stream: &mut TcpStream,
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

fn send_line(stream: &mut TcpStream, msg: &ClientMsg) -> io::Result<()> {
    let mut line = serde_json::to_string(msg).map_err(eio)?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    stream.flush()
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
    use super::{remember_nonce, ShareService};
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
