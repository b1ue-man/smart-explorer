use crossbeam_channel::Sender;
use std::collections::HashSet;
use std::io;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::core::{b64_decode, eio, relation_psk, room_psk, sanitize_name};
use super::fs::{handle_fs_request, ShareExportConfig};
use super::identity::ShareIdentity;
use super::protocol::{read_raw_frame, Channel, TAG_CTRL, TAG_DATA};
use super::types::{DirectContact, RoomProfile, ShareEvent};
use super::wire::{Ctrl, FileMeta, PeerPrelude};

#[derive(Clone)]
pub(crate) struct ShareAuthState {
    pub(crate) identity: ShareIdentity,
    pub(crate) direct_secret: Vec<u8>,
    pub(crate) default_direct_exports: ShareExportConfig,
    pub(crate) direct_contacts: Vec<DirectContact>,
    pub(crate) rooms: Vec<RoomProfile>,
    pub(crate) seen_nonces: HashSet<String>,
    pub(crate) direct_online: bool,
}

pub(crate) fn accept_loop(
    listener: TcpListener,
    auth: Arc<Mutex<ShareAuthState>>,
    ev: Sender<ShareEvent>,
    stopped: Arc<AtomicBool>,
) {
    let _ = listener.set_nonblocking(true);
    let counter = Arc::new(Mutex::new(0u64));
    while !stopped.load(Ordering::Relaxed) {
        let stream = match listener.accept() {
            Ok((s, _)) => s,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
            Err(e) => {
                let _ = ev.send(ShareEvent::Error(format!("Listener: {e}")));
                break;
            }
        };
        let auth = auth.clone();
        let ev = ev.clone();
        let counter = counter.clone();
        std::thread::Builder::new()
            .name("share-peer".into())
            .spawn(move || {
                let id = {
                    let mut c = counter.lock().unwrap();
                    *c += 1;
                    *c
                };
                if let Err(e) = recv_from_peer(stream, id, auth, &ev) {
                    let _ = ev.send(ShareEvent::Error(format!("Peer: {e}")));
                }
            })
            .ok();
    }
}

fn recv_from_peer(
    mut stream: TcpStream,
    _id: u64,
    auth: Arc<Mutex<ShareAuthState>>,
    ev: &Sender<ShareEvent>,
) -> io::Result<()> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(20)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(20)));
    let prelude: PeerPrelude =
        serde_json::from_slice(&read_raw_frame(&mut stream)?).map_err(eio)?;
    let (psk, expected_public, exports, identity) = resolve_incoming(&prelude, &auth)?;
    let mut ch = Channel::responder(
        stream,
        &psk,
        &identity.private_key,
        expected_public.as_deref(),
    )?;
    let (tag, payload) = ch.recv()?;
    if tag != TAG_CTRL {
        return Err(eio("PeerHello fehlt"));
    }
    let ctrl: Ctrl = serde_json::from_slice(&payload).map_err(eio)?;
    let Ctrl::PeerHello { hello } = ctrl else {
        return Err(eio("PeerHello erwartet"));
    };
    if hello.protocol_version != 2 {
        return Err(eio("Inkompatible Peer-Protokollversion"));
    }
    if hello.relation_kind != prelude.relation_kind
        || hello.relation_id != prelude.relation_id
        || hello.device_id != prelude.from_device_id
    {
        return Err(eio("PeerHello passt nicht zum Prelude"));
    }
    let public = b64_decode(&hello.public_key).map_err(eio)?;
    if public != ch.remote_static() {
        return Err(eio("PeerHello Public Key passt nicht zum Handshake"));
    }
    let _ = ev.send(ShareEvent::Status(format!(
        "Peer authentifiziert: {} ({})",
        hello.device_id,
        ch.remote_fingerprint()
    )));
    ch.send(
        TAG_CTRL,
        &serde_json::to_vec(&Ctrl::PeerHelloOk).map_err(eio)?,
    )?;

    let (tag, payload) = ch.recv()?;
    if tag != TAG_CTRL {
        return Err(eio("Steuerframe erwartet"));
    }
    match serde_json::from_slice::<Ctrl>(&payload).map_err(eio)? {
        Ctrl::Fs { req } => handle_fs_request(ch, req, Arc::new(Mutex::new(exports))),
        Ctrl::Offer { from, files } => recv_offer(ch, from, files, ev),
        _ => Err(eio("Dateioperation erwartet")),
    }
}

fn resolve_incoming(
    prelude: &PeerPrelude,
    auth: &Arc<Mutex<ShareAuthState>>,
) -> io::Result<([u8; 32], Option<Vec<u8>>, ShareExportConfig, ShareIdentity)> {
    let state = auth.lock().map_err(|_| eio("Share-Auth gesperrt"))?.clone();
    match prelude.relation_kind.as_str() {
        "direct" if prelude.relation_id == state.identity.direct_lookup_id => {
            if !state.direct_online {
                return Err(eio("Direktverbindung ist offline"));
            }
            let expected = state
                .direct_contacts
                .iter()
                .find(|c| c.remote_device_id.as_deref() == Some(&prelude.from_device_id))
                .and_then(|c| c.remote_public_key.as_ref())
                .and_then(|pk| b64_decode(pk).ok());
            Ok((
                relation_psk(
                    "direct",
                    &state.direct_secret,
                    &state.identity.device_id,
                    &prelude.from_device_id,
                ),
                expected,
                state.default_direct_exports,
                state.identity,
            ))
        }
        "room" => {
            let room = state
                .rooms
                .iter()
                .find(|r| r.room_id == prelude.relation_id)
                .cloned()
                .ok_or_else(|| eio("Unbekannter Raum"))?;
            let secret = super::profiles::ShareProfiles::room_secret(&room)
                .ok_or_else(|| eio("Raum-Secret fehlt"))?;
            let expected = room
                .members
                .iter()
                .find(|m| m.device_id == prelude.from_device_id)
                .and_then(|m| b64_decode(&m.public_key).ok());
            Ok((
                room_psk(&secret, &room.room_id),
                expected,
                room.exports,
                state.identity,
            ))
        }
        _ => Err(eio("Unbekannte oder nicht autorisierte Relation")),
    }
}

fn recv_offer(
    mut ch: Channel,
    from: String,
    files: Vec<FileMeta>,
    ev: &Sender<ShareEvent>,
) -> io::Result<()> {
    let count = files.len();
    let dir = super::system::quarantine_dir()?;
    ch.send(TAG_CTRL, &serde_json::to_vec(&Ctrl::Accept).map_err(eio)?)?;
    loop {
        let (tag, payload) = ch.recv()?;
        if tag != TAG_CTRL {
            return Err(eio("Steuerframe erwartet"));
        }
        match serde_json::from_slice::<Ctrl>(&payload).map_err(eio)? {
            Ctrl::FileStart { name, size: _ } => {
                let safe = sanitize_name(&name);
                let path = super::system::unique_in(&dir, &safe);
                let mut f = std::fs::File::create(&path)?;
                loop {
                    let (tag, payload) = ch.recv()?;
                    if tag == TAG_DATA {
                        std::io::Write::write_all(&mut f, &payload)?;
                        continue;
                    }
                    if tag != TAG_CTRL {
                        return Err(eio("Steuerframe erwartet"));
                    }
                    match serde_json::from_slice::<Ctrl>(&payload).map_err(eio)? {
                        Ctrl::FileEnd => break,
                        _ => return Err(eio("Dateiende erwartet")),
                    }
                }
            }
            Ctrl::Done => {
                let _ = ev.send(ShareEvent::Received {
                    count,
                    dir: dir.display().to_string(),
                });
                let _ = ev.send(ShareEvent::Status(format!("{count} Datei(en) von {from}")));
                return Ok(());
            }
            _ => return Err(eio("Dateistart erwartet")),
        }
    }
}

#[allow(dead_code)]
pub(crate) fn send_to_peer(
    _peer: &super::types::PeerEndpoint,
    _identity: &ShareIdentity,
    _paths: &[String],
    _ev: &Sender<ShareEvent>,
) -> io::Result<()> {
    Err(eio(
        "Push-Transfer ist fuer Share-Server-Verbindungen deaktiviert",
    ))
}

pub(crate) fn dial_candidates(candidates: &[String]) -> io::Result<TcpStream> {
    let mut last = eio("keine Kandidaten");
    for c in candidates {
        if let Ok(addr) = c.parse::<std::net::SocketAddr>() {
            match TcpStream::connect_timeout(&addr, Duration::from_secs(3)) {
                Ok(s) => {
                    let _ = s.set_nodelay(true);
                    return Ok(s);
                }
                Err(e) => last = e,
            }
        }
    }
    Err(last)
}
