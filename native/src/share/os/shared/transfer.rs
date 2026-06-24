use crossbeam_channel::Sender;
use std::collections::HashSet;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::core::{b64_decode, eio, relation_psk, room_psk, sanitize_name};
use super::fs::{handle_fs_request, ShareExportConfig};
use super::identity::ShareIdentity;
use super::protocol::{read_raw_frame, Channel, TAG_CTRL, TAG_DATA};
use super::types::{DirectContact, DirectGrant, DirectGrantState, RoomProfile, ShareEvent};
use super::wire::{Ctrl, FileMeta, PeerPrelude};

#[derive(Clone)]
pub(crate) struct ShareAuthState {
    pub(crate) identity: ShareIdentity,
    pub(crate) direct_secret: Vec<u8>,
    pub(crate) default_direct_exports: ShareExportConfig,
    pub(crate) direct_contacts: Vec<DirectContact>,
    pub(crate) direct_grants: Vec<DirectGrant>,
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
        let _ = stream.set_nonblocking(false);
        let auth = auth.clone();
        let ev = ev.clone();
        let counter = counter.clone();
        std::thread::Builder::new()
            .name("share-peer".into())
            .spawn(move || {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(20)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(20)));
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

pub(crate) fn recv_from_peer<S>(
    mut stream: S,
    _id: u64,
    auth: Arc<Mutex<ShareAuthState>>,
    ev: &Sender<ShareEvent>,
) -> io::Result<()>
where
    S: Read + Write + Send + 'static,
{
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
            let grant = state
                .direct_grants
                .iter()
                .find(|g| {
                    g.device_id == prelude.from_device_id && g.state == DirectGrantState::Accepted
                })
                .ok_or_else(|| eio("Direktfreigabe nicht akzeptiert"))?;
            let expected = b64_decode(&grant.public_key).ok();
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
    let mut attempts = Vec::new();
    for c in candidates {
        match c.parse::<std::net::SocketAddr>() {
            Ok(addr) => match TcpStream::connect_timeout(&addr, Duration::from_secs(3)) {
                Ok(s) => {
                    let _ = s.set_nodelay(true);
                    return Ok(s);
                }
                Err(e) => attempts.push(format!("{c}: {e}")),
            },
            Err(e) => attempts.push(format!("{c}: ungueltig ({e})")),
        }
    }
    if attempts.is_empty() {
        Err(eio("keine Kandidaten"))
    } else {
        Err(eio(format!(
            "kein direkter Peer-Kandidat erreichbar: {}",
            attempts.join("; ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::dial_candidates;

    #[test]
    fn dial_error_lists_candidates() {
        let err = dial_candidates(&["not-an-addr".to_string()]).unwrap_err();
        assert!(err.to_string().contains("not-an-addr"));
    }
}
