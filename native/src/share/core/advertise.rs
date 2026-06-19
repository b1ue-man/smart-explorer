use crossbeam_channel::Sender;
use std::io::{self, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};

use super::core::psk_from_code;
use super::session::Session;
use super::system::lan_ips;
use super::types::{RemoteDevice, ShareEvent};
use super::wire::{Hello, SrvMsg};

#[allow(clippy::too_many_arguments)]
pub(crate) fn start_session(
    code: String,
    mode: &str,
    server: &str,
    device: &str,
    fingerprint: &str,
    listen_port: u16,
    ev: &Sender<ShareEvent>,
    session: &Arc<Mutex<Session>>,
) {
    let code = code.trim().to_uppercase();
    if code.is_empty() {
        let _ = ev.send(ShareEvent::Error("Code fehlt".into()));
        return;
    }
    let psk = psk_from_code(&code);
    {
        let mut s = session.lock().unwrap();
        if let Some(sig) = s.signaling.take() {
            let _ = sig.shutdown(std::net::Shutdown::Both);
        }
        s.code = Some(code.clone());
        s.psk = Some(psk);
        s.peers.clear();
    }
    let hello = Hello {
        t: "hello",
        mode: mode.to_string(),
        code: code.clone(),
        device: device.to_string(),
        listen_port,
        lan: lan_ips(),
        pubkey: fingerprint.to_string(),
    };
    let stream = match TcpStream::connect(server) {
        Ok(s) => s,
        Err(e) => {
            let _ = ev.send(ShareEvent::Error(format!("Server nicht erreichbar: {}", e)));
            return;
        }
    };
    let _ = stream.set_nodelay(true);
    {
        let mut s = session.lock().unwrap();
        s.signaling = Some(match stream.try_clone() {
            Ok(c) => c,
            Err(_) => return,
        });
    }
    let ev2 = ev.clone();
    let session2 = session.clone();
    let _ = ev.send(ShareEvent::Status(format!(
        "Verbunden mit Server ({mode})…"
    )));
    std::thread::Builder::new()
        .name("share-signaling".into())
        .spawn(move || signaling_loop(stream, hello, session2, ev2))
        .ok();
}

fn signaling_loop(
    stream: TcpStream,
    hello: Hello,
    session: Arc<Mutex<Session>>,
    ev: Sender<ShareEvent>,
) {
    let mut w = match stream.try_clone() {
        Ok(w) => w,
        Err(_) => return,
    };
    if let Ok(mut line) = serde_json::to_string(&hello) {
        line.push('\n');
        if w.write_all(line.as_bytes()).is_err() {
            return;
        }
    }
    let mut reader = io::BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        match io::BufRead::read_line(&mut reader, &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let msg: SrvMsg = match serde_json::from_str(line.trim()) {
            Ok(m) => m,
            Err(_) => continue,
        };
        match msg {
            SrvMsg::Peer {
                device,
                candidates,
                pubkey,
            } => {
                let dev = RemoteDevice {
                    device,
                    fingerprint: pubkey,
                    candidates,
                };
                let mut s = session.lock().unwrap();
                s.peers = vec![dev];
                let _ = ev.send(ShareEvent::Status("Mit Gerät gekoppelt".into()));
                let _ = ev.send(ShareEvent::Roster(s.peers.clone()));
            }
            SrvMsg::Roster { members } => {
                let mut s = session.lock().unwrap();
                s.peers = members.into_iter().map(Into::into).collect();
                let _ = ev.send(ShareEvent::Roster(s.peers.clone()));
            }
            SrvMsg::Joined { member } => {
                let mut s = session.lock().unwrap();
                s.peers.push(member.into());
                let _ = ev.send(ShareEvent::Roster(s.peers.clone()));
            }
            SrvMsg::Left { pubkey, .. } => {
                let mut s = session.lock().unwrap();
                s.peers.retain(|p| p.fingerprint != pubkey);
                let _ = ev.send(ShareEvent::Roster(s.peers.clone()));
            }
            SrvMsg::Error { msg } => {
                let _ = ev.send(ShareEvent::Error(format!("Server: {}", msg)));
            }
        }
    }
}
