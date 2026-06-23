use crossbeam_channel::{unbounded, Sender};
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::core::{eio, sanitize_name};
use super::fs::{handle_fs_request, ShareExportConfig};
use super::protocol::{Channel, TAG_CTRL, TAG_DATA};
use super::session::{Answers, Session};
use super::system::{hostname, quarantine_dir, unique_in};
use super::types::{RemoteDevice, ShareEvent};
use super::wire::{Ctrl, FileMeta};

const CHUNK: usize = 60_000;

pub(crate) fn accept_loop(
    listener: TcpListener,
    session: Arc<Mutex<Session>>,
    answers: Answers,
    exports: Arc<Mutex<ShareExportConfig>>,
    ev: Sender<ShareEvent>,
) {
    let counter = Arc::new(Mutex::new(0u64));
    for conn in listener.incoming() {
        let stream = match conn {
            Ok(s) => s,
            Err(_) => continue,
        };
        let psk = session.lock().unwrap().psk;
        let psk = match psk {
            Some(p) => p,
            None => continue,
        };
        let ev = ev.clone();
        let answers = answers.clone();
        let exports = exports.clone();
        let counter = counter.clone();
        std::thread::Builder::new()
            .name("share-recv".into())
            .spawn(move || {
                let id = {
                    let mut c = counter.lock().unwrap();
                    *c += 1;
                    *c
                };
                if let Err(e) = recv_from_peer(stream, &psk, id, &answers, exports, &ev) {
                    let _ = ev.send(ShareEvent::Error(format!("Empfang: {}", e)));
                }
            })
            .ok();
    }
}

fn recv_from_peer(
    stream: TcpStream,
    psk: &[u8; 32],
    id: u64,
    answers: &Answers,
    exports: Arc<Mutex<ShareExportConfig>>,
    ev: &Sender<ShareEvent>,
) -> io::Result<()> {
    let mut ch = Channel::responder(stream, psk)?;
    let (tag, payload) = ch.recv()?;
    if tag != TAG_CTRL {
        return Err(eio("erwartete Steuernachricht"));
    }
    let ctrl: Ctrl = serde_json::from_slice(&payload).map_err(eio)?;
    let (from, files) = match ctrl {
        Ctrl::Offer { from, files } => (from, files),
        Ctrl::Fs { req } => return handle_fs_request(ch, req, exports),
        _ => return Err(eio("erwartetes Angebot")),
    };
    let (atx, arx) = unbounded::<bool>();
    answers.lock().unwrap().insert(id, atx);
    let _ = ev.send(ShareEvent::Incoming {
        id,
        from: from.clone(),
        files: files.iter().map(|f| (f.name.clone(), f.size)).collect(),
    });
    let accept = arx.recv_timeout(Duration::from_secs(120)).unwrap_or(false);
    answers.lock().unwrap().remove(&id);
    if !accept {
        ch.send(TAG_CTRL, &serde_json::to_vec(&Ctrl::Reject).unwrap())?;
        return Ok(());
    }
    ch.send(TAG_CTRL, &serde_json::to_vec(&Ctrl::Accept).unwrap())?;

    let dir = quarantine_dir()?;
    let total: u64 = files.iter().map(|f| f.size).sum();
    let mut done: u64 = 0;
    let mut count = 0usize;
    let mut cur: Option<std::fs::File> = None;
    loop {
        let (tag, payload) = ch.recv()?;
        if tag == TAG_DATA {
            if let Some(f) = cur.as_mut() {
                f.write_all(&payload)?;
                done += payload.len() as u64;
                let _ = ev.send(ShareEvent::Progress { done, total });
            }
            continue;
        }
        let ctrl: Ctrl = serde_json::from_slice(&payload).map_err(eio)?;
        match ctrl {
            Ctrl::FileStart { name, .. } => {
                let safe = sanitize_name(&name);
                let path = unique_in(&dir, &safe);
                cur = Some(std::fs::File::create(&path)?);
            }
            Ctrl::FileEnd => {
                if let Some(f) = cur.take() {
                    drop(f);
                    count += 1;
                }
            }
            Ctrl::Done => break,
            _ => {}
        }
    }
    let _ = ev.send(ShareEvent::Received {
        count,
        dir: dir.to_string_lossy().to_string(),
    });
    Ok(())
}

pub(crate) fn send_to_peer(
    peer: &RemoteDevice,
    psk: &[u8; 32],
    paths: &[String],
    ev: &Sender<ShareEvent>,
) -> io::Result<()> {
    let stream = dial_candidates(&peer.candidates)?;
    let mut ch = Channel::initiator(stream, psk)?;

    let mut metas = Vec::new();
    let mut real: Vec<(String, std::path::PathBuf, u64)> = Vec::new();
    for p in paths {
        let pb = std::path::PathBuf::from(p);
        if let Ok(md) = std::fs::metadata(&pb) {
            if md.is_file() {
                let name = pb
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                metas.push(FileMeta {
                    name: name.clone(),
                    size: md.len(),
                });
                real.push((name, pb, md.len()));
            }
        }
    }
    if real.is_empty() {
        return Err(eio(
            "keine Dateien zum Senden (Ordner werden noch nicht unterstützt)",
        ));
    }
    let device = hostname();
    ch.send(
        TAG_CTRL,
        &serde_json::to_vec(&Ctrl::Offer {
            from: device,
            files: metas,
        })
        .unwrap(),
    )?;
    let (tag, payload) = ch.recv()?;
    if tag != TAG_CTRL {
        return Err(eio("unerwartete Antwort"));
    }
    match serde_json::from_slice::<Ctrl>(&payload).map_err(eio)? {
        Ctrl::Accept => {}
        Ctrl::Reject => {
            let _ = ev.send(ShareEvent::Status(format!("{} hat abgelehnt", peer.device)));
            return Ok(());
        }
        _ => return Err(eio("unerwartete Antwort")),
    }
    let total: u64 = real.iter().map(|(_, _, s)| *s).sum();
    let mut done = 0u64;
    for (name, pb, _) in &real {
        ch.send(
            TAG_CTRL,
            &serde_json::to_vec(&Ctrl::FileStart {
                name: name.clone(),
                size: 0,
            })
            .unwrap(),
        )?;
        let mut f = std::fs::File::open(pb)?;
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            ch.send(TAG_DATA, &buf[..n])?;
            done += n as u64;
            let _ = ev.send(ShareEvent::Progress { done, total });
        }
        ch.send(TAG_CTRL, &serde_json::to_vec(&Ctrl::FileEnd).unwrap())?;
    }
    ch.send(TAG_CTRL, &serde_json::to_vec(&Ctrl::Done).unwrap())?;
    let _ = ev.send(ShareEvent::Sent { count: real.len() });
    Ok(())
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
