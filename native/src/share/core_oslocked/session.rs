use crossbeam_channel::{Receiver, Sender};
use std::collections::HashMap;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};

use super::advertise::start_session;
use super::transfer::send_to_peer;
use super::types::{RemoteDevice, ShareCmd, ShareEvent};

#[derive(Default)]
pub(crate) struct Session {
    pub(crate) code: Option<String>,
    pub(crate) psk: Option<[u8; 32]>,
    pub(crate) peers: Vec<RemoteDevice>,
    /// Handle to the live signaling socket so `Leave` can close it.
    pub(crate) signaling: Option<TcpStream>,
}

pub(crate) type Answers = Arc<Mutex<HashMap<u64, Sender<bool>>>>;

#[allow(clippy::too_many_arguments)]
pub(crate) fn worker(
    server: String,
    device: String,
    fingerprint: String,
    listen_port: u16,
    cmds: Receiver<ShareCmd>,
    ev: Sender<ShareEvent>,
    session: Arc<Mutex<Session>>,
    answers: Answers,
) {
    while let Ok(cmd) = cmds.recv() {
        dispatch(cmd, &server, &device, &fingerprint, listen_port, &ev, &session, &answers);
    }
}

#[allow(clippy::too_many_arguments)]
fn dispatch(
    cmd: ShareCmd,
    server: &str,
    device: &str,
    fingerprint: &str,
    listen_port: u16,
    ev: &Sender<ShareEvent>,
    session: &Arc<Mutex<Session>>,
    answers: &Answers,
) {
    match cmd {
        ShareCmd::Pair(code) => start_session(code, "pair", server, device, fingerprint, listen_port, ev, session),
        ShareCmd::JoinRoom(code) => start_session(code, "room", server, device, fingerprint, listen_port, ev, session),
        ShareCmd::Leave => {
            let mut s = session.lock().unwrap();
            if let Some(sig) = s.signaling.take() {
                let _ = sig.shutdown(std::net::Shutdown::Both);
            }
            s.code = None;
            s.psk = None;
            s.peers.clear();
            let _ = ev.send(ShareEvent::Roster(Vec::new()));
            let _ = ev.send(ShareEvent::Status("Sitzung verlassen".into()));
        }
        ShareCmd::Send(paths) => {
            let (psk, peers) = {
                let s = session.lock().unwrap();
                (s.psk, s.peers.clone())
            };
            let psk = match psk {
                Some(p) => p,
                None => {
                    let _ = ev.send(ShareEvent::Error("Keine aktive Sitzung".into()));
                    return;
                }
            };
            if peers.is_empty() {
                let _ = ev.send(ShareEvent::Error("Keine verbundenen Geräte".into()));
                return;
            }
            for peer in peers {
                let ev = ev.clone();
                let paths = paths.clone();
                std::thread::Builder::new()
                    .name("share-send".into())
                    .spawn(move || {
                        if let Err(e) = send_to_peer(&peer, &psk, &paths, &ev) {
                            let _ = ev.send(ShareEvent::Error(format!("Senden an {}: {}", peer.device, e)));
                        }
                    })
                    .ok();
            }
        }
        ShareCmd::Answer { id, accept } => {
            if let Some(tx) = answers.lock().unwrap().remove(&id) {
                let _ = tx.send(accept);
            }
        }
    }
}
