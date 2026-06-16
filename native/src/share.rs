//! Peer file sharing (#21) — client side. The maintainer's server only *routes
//! discovery* (see `share-server/`); here we connect to it, learn a peer's
//! reachable candidates, then open a **direct** TCP connection and transfer
//! files **end-to-end encrypted** with Noise `NNpsk0` keyed by a PSK derived
//! from the shared pairing/room **code**. The server never sees file bytes.
//!
//! Two modes: **pair** (two devices, one code) and **room** (many devices, one
//! code, share to all). The GUI drives this through `ShareCmd`/`ShareEvent`.
//!
//! NOTE: the live networked path (NAT traversal, Noise handshake, transfer)
//! cannot be exercised in the headless build env; it compiles for host +
//! windows-gnu and the pure logic is unit-tested. Needs a real two-machine test.

use crossbeam_channel::{unbounded, Receiver, Sender};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const NOISE_PARAMS: &str = "Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s";
const CHUNK: usize = 60_000;
const QUARANTINE: &str = "SmartExplorer-Empfangen";

fn eio<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e.to_string())
}

// ── public API (used by app.rs) ──────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct RemoteDevice {
    pub device: String,
    pub fingerprint: String,
    pub candidates: Vec<String>,
}

/// What the UI tells the share worker to do.
pub enum ShareCmd {
    /// Announce ourselves for a 1:1 pairing under `code`.
    Pair(String),
    /// Join (or create) a room under `code`.
    JoinRoom(String),
    /// Leave the current pairing/room.
    Leave,
    /// Send these local paths to every current peer (pair = the one peer).
    Send(Vec<String>),
    /// Answer an incoming offer (accept or reject).
    Answer { id: u64, accept: bool },
}

/// What the worker reports back to the UI (drained each frame).
pub enum ShareEvent {
    Status(String),
    Error(String),
    /// Current peers in the session (pair has 0 or 1; room has N).
    Roster(Vec<RemoteDevice>),
    /// An inbound transfer is awaiting accept/reject.
    Incoming { id: u64, from: String, files: Vec<(String, u64)> },
    Progress { done: u64, total: u64 },
    Received { count: usize, dir: String },
    Sent { count: usize },
}

pub struct ShareService {
    pub events: Receiver<ShareEvent>,
    cmds: Sender<ShareCmd>,
    pub fingerprint: String,
    pub listen_port: u16,
}

impl ShareService {
    pub fn cmd(&self, c: ShareCmd) {
        let _ = self.cmds.send(c);
    }

    /// Start the background worker: bind a listener, spawn the accept loop, and
    /// process commands. `server` is the rendezvous host:port; `device` is our
    /// display name.
    pub fn start(server: String, device: String) -> io::Result<ShareService> {
        let fingerprint = random_fingerprint();
        let listener = TcpListener::bind("0.0.0.0:0")?;
        let listen_port = listener.local_addr()?.port();
        let (cmd_tx, cmd_rx) = unbounded::<ShareCmd>();
        let (ev_tx, ev_rx) = unbounded::<ShareEvent>();

        let session: Arc<Mutex<Session>> = Arc::new(Mutex::new(Session::default()));
        let answers: Answers = Arc::new(Mutex::new(HashMap::new()));

        // Accept loop: inbound direct connections (someone sending to us).
        {
            let session = session.clone();
            let answers = answers.clone();
            let ev = ev_tx.clone();
            std::thread::Builder::new()
                .name("share-accept".into())
                .spawn(move || accept_loop(listener, session, answers, ev))
                .ok();
        }

        // Command worker.
        {
            let ev = ev_tx.clone();
            let device = device.clone();
            let fp = fingerprint.clone();
            std::thread::Builder::new()
                .name("share-worker".into())
                .spawn(move || {
                    worker(server, device, fp, listen_port, cmd_rx, ev, session, answers)
                })
                .ok();
        }

        Ok(ShareService { events: ev_rx, cmds: cmd_tx, fingerprint, listen_port })
    }
}

// ── shared session state ─────────────────────────────────────────────────────

#[derive(Default)]
struct Session {
    code: Option<String>,
    psk: Option<[u8; 32]>,
    peers: Vec<RemoteDevice>,
    /// Handle to the live signaling socket so `Leave` can close it.
    signaling: Option<TcpStream>,
}

type Answers = Arc<Mutex<HashMap<u64, Sender<bool>>>>;

// ── command worker ───────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn worker(
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

#[allow(clippy::too_many_arguments)]
fn start_session(
    code: String,
    mode: &str,
    server: &str,
    device: &str,
    fingerprint: &str,
    listen_port: u16,
    ev: &Sender<ShareEvent>,
    session: &Arc<Mutex<Session>>,
) {
    // Normalize so the code is case-insensitive for both the PSK and the
    // server's string match (both peers must agree).
    let code = code.trim().to_uppercase();
    if code.is_empty() {
        let _ = ev.send(ShareEvent::Error("Code fehlt".into()));
        return;
    }
    let psk = psk_from_code(&code);
    // Reset any previous session.
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
    let _ = ev.send(ShareEvent::Status(format!("Verbunden mit Server ({mode})…")));
    std::thread::Builder::new()
        .name("share-signaling".into())
        .spawn(move || signaling_loop(stream, hello, session2, ev2))
        .ok();
}

fn signaling_loop(stream: TcpStream, hello: Hello, session: Arc<Mutex<Session>>, ev: Sender<ShareEvent>) {
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
            SrvMsg::Peer { device, candidates, pubkey } => {
                let dev = RemoteDevice { device, fingerprint: pubkey, candidates };
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

// ── inbound (someone sends to us) ────────────────────────────────────────────

fn accept_loop(listener: TcpListener, session: Arc<Mutex<Session>>, answers: Answers, ev: Sender<ShareEvent>) {
    let counter = Arc::new(Mutex::new(0u64));
    for conn in listener.incoming() {
        let stream = match conn {
            Ok(s) => s,
            Err(_) => continue,
        };
        let psk = session.lock().unwrap().psk;
        let psk = match psk {
            Some(p) => p,
            None => continue, // no active session → ignore
        };
        let ev = ev.clone();
        let answers = answers.clone();
        let counter = counter.clone();
        std::thread::Builder::new()
            .name("share-recv".into())
            .spawn(move || {
                let id = {
                    let mut c = counter.lock().unwrap();
                    *c += 1;
                    *c
                };
                if let Err(e) = recv_from_peer(stream, &psk, id, &answers, &ev) {
                    let _ = ev.send(ShareEvent::Error(format!("Empfang: {}", e)));
                }
            })
            .ok();
    }
}

fn recv_from_peer(stream: TcpStream, psk: &[u8; 32], id: u64, answers: &Answers, ev: &Sender<ShareEvent>) -> io::Result<()> {
    let mut ch = Channel::responder(stream, psk)?;
    // First control frame must be an Offer.
    let (tag, payload) = ch.recv()?;
    if tag != TAG_CTRL {
        return Err(eio("erwartete Steuernachricht"));
    }
    let ctrl: Ctrl = serde_json::from_slice(&payload).map_err(eio)?;
    let (from, files) = match ctrl {
        Ctrl::Offer { from, files } => (from, files),
        _ => return Err(eio("erwartetes Angebot")),
    };
    // Ask the UI.
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
    let _ = ev.send(ShareEvent::Received { count, dir: dir.to_string_lossy().to_string() });
    Ok(())
}

// ── outbound (we send to a peer) ─────────────────────────────────────────────

fn send_to_peer(peer: &RemoteDevice, psk: &[u8; 32], paths: &[String], ev: &Sender<ShareEvent>) -> io::Result<()> {
    let stream = dial_candidates(&peer.candidates)?;
    let mut ch = Channel::initiator(stream, psk)?;

    // Gather file metadata (skip dirs / unreadable for v1).
    let mut metas = Vec::new();
    let mut real: Vec<(String, std::path::PathBuf, u64)> = Vec::new();
    for p in paths {
        let pb = std::path::PathBuf::from(p);
        if let Ok(md) = std::fs::metadata(&pb) {
            if md.is_file() {
                let name = pb.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                metas.push(FileMeta { name: name.clone(), size: md.len() });
                real.push((name, pb, md.len()));
            }
        }
    }
    if real.is_empty() {
        return Err(eio("keine Dateien zum Senden (Ordner werden noch nicht unterstützt)"));
    }
    let device = hostname();
    ch.send(TAG_CTRL, &serde_json::to_vec(&Ctrl::Offer { from: device, files: metas }).unwrap())?;
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
        ch.send(TAG_CTRL, &serde_json::to_vec(&Ctrl::FileStart { name: name.clone(), size: 0 }).unwrap())?;
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

/// Try every candidate in parallel-ish (sequential with short timeouts); first
/// that connects wins. LAN candidates come first.
fn dial_candidates(candidates: &[String]) -> io::Result<TcpStream> {
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

// ── Noise-encrypted framed channel ───────────────────────────────────────────

const TAG_CTRL: u8 = 0;
const TAG_DATA: u8 = 1;

struct Channel {
    t: snow::TransportState,
    s: TcpStream,
}

impl Channel {
    fn initiator(mut s: TcpStream, psk: &[u8; 32]) -> io::Result<Channel> {
        let params = NOISE_PARAMS.parse().map_err(eio)?;
        let mut hs = snow::Builder::new(params).psk(0, psk).build_initiator().map_err(eio)?;
        let mut buf = vec![0u8; 1024];
        let n = hs.write_message(&[], &mut buf).map_err(eio)?;
        write_frame(&mut s, &buf[..n])?;
        let msg = read_frame(&mut s)?;
        let mut out = vec![0u8; 1024];
        hs.read_message(&msg, &mut out).map_err(eio)?;
        let t = hs.into_transport_mode().map_err(eio)?;
        Ok(Channel { t, s })
    }

    fn responder(mut s: TcpStream, psk: &[u8; 32]) -> io::Result<Channel> {
        let params = NOISE_PARAMS.parse().map_err(eio)?;
        let mut hs = snow::Builder::new(params).psk(0, psk).build_responder().map_err(eio)?;
        let msg = read_frame(&mut s)?;
        let mut out = vec![0u8; 1024];
        hs.read_message(&msg, &mut out).map_err(eio)?;
        let mut buf = vec![0u8; 1024];
        let n = hs.write_message(&[], &mut buf).map_err(eio)?;
        write_frame(&mut s, &buf[..n])?;
        let t = hs.into_transport_mode().map_err(eio)?;
        Ok(Channel { t, s })
    }

    fn send(&mut self, tag: u8, payload: &[u8]) -> io::Result<()> {
        let mut plain = Vec::with_capacity(payload.len() + 1);
        plain.push(tag);
        plain.extend_from_slice(payload);
        let mut buf = vec![0u8; plain.len() + 32];
        let n = self.t.write_message(&plain, &mut buf).map_err(eio)?;
        write_frame(&mut self.s, &buf[..n])
    }

    fn recv(&mut self) -> io::Result<(u8, Vec<u8>)> {
        let cipher = read_frame(&mut self.s)?;
        let mut out = vec![0u8; cipher.len()];
        let n = self.t.read_message(&cipher, &mut out).map_err(eio)?;
        out.truncate(n);
        if out.is_empty() {
            return Err(eio("leerer Frame"));
        }
        let tag = out[0];
        Ok((tag, out[1..].to_vec()))
    }
}

fn write_frame(s: &mut TcpStream, data: &[u8]) -> io::Result<()> {
    s.write_all(&(data.len() as u32).to_be_bytes())?;
    s.write_all(data)?;
    s.flush()
}

fn read_frame(s: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut len4 = [0u8; 4];
    s.read_exact(&mut len4)?;
    let n = u32::from_be_bytes(len4) as usize;
    if n > 70_000 {
        return Err(eio("Frame zu groß"));
    }
    let mut buf = vec![0u8; n];
    s.read_exact(&mut buf)?;
    Ok(buf)
}

// ── wire types ───────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct Hello {
    t: &'static str,
    mode: String,
    code: String,
    device: String,
    listen_port: u16,
    lan: Vec<String>,
    pubkey: String,
}

#[derive(Deserialize)]
struct SrvMember {
    device: String,
    candidates: Vec<String>,
    pubkey: String,
}
impl From<SrvMember> for RemoteDevice {
    fn from(m: SrvMember) -> Self {
        RemoteDevice { device: m.device, fingerprint: m.pubkey, candidates: m.candidates }
    }
}

#[derive(Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
enum SrvMsg {
    Peer { device: String, candidates: Vec<String>, pubkey: String },
    Roster { members: Vec<SrvMember> },
    Joined { member: SrvMember },
    Left { #[allow(dead_code)] device: String, pubkey: String },
    Error { msg: String },
}

#[derive(Serialize, Deserialize)]
struct FileMeta {
    name: String,
    size: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "c")]
enum Ctrl {
    Offer { from: String, files: Vec<FileMeta> },
    Accept,
    Reject,
    FileStart { name: String, size: u64 },
    FileEnd,
    Done,
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Derive the 32-byte Noise PSK from the human code via HKDF-SHA256.
fn psk_from_code(code: &str) -> [u8; 32] {
    let norm = code.trim().to_uppercase();
    let hk = hkdf::Hkdf::<sha2::Sha256>::new(Some(b"smart-explorer-share-v1"), norm.as_bytes());
    let mut okm = [0u8; 32];
    hk.expand(b"psk", &mut okm).expect("32 bytes is a valid HKDF length");
    okm
}

/// A user-presentable random pairing/room code: 8 Crockford-base32 chars (~40
/// bits), unambiguous (no I/L/O/U).
pub fn gen_code() -> String {
    const A: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let mut raw = [0u8; 8];
    let _ = getrandom::getrandom(&mut raw);
    raw.iter().map(|b| A[(*b as usize) % A.len()] as char).collect()
}

fn random_fingerprint() -> String {
    let mut raw = [0u8; 6];
    let _ = getrandom::getrandom(&mut raw);
    raw.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(":")
}

fn lan_ips() -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(s) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if s.connect("8.8.8.8:80").is_ok() {
            if let Ok(a) = s.local_addr() {
                v.push(a.ip().to_string());
            }
        }
    }
    v
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "Gerät".to_string())
}

fn quarantine_dir() -> io::Result<std::path::PathBuf> {
    let base = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let d = base.join(QUARANTINE);
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

fn sanitize_name(name: &str) -> String {
    let n: String = name
        .chars()
        .map(|c| if "/\\:*?\"<>|".contains(c) || c.is_control() { '_' } else { c })
        .collect();
    let n = n.trim().trim_matches('.').to_string();
    if n.is_empty() {
        "datei".to_string()
    } else {
        n
    }
}

fn unique_in(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    if !p.exists() {
        return p;
    }
    let stem = std::path::Path::new(name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let ext = std::path::Path::new(name).extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    for i in 2..10_000 {
        let cand = dir.join(format!("{} ({}){}", stem, i, ext));
        if !cand.exists() {
            return cand;
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn psk_is_deterministic_per_code_and_differs() {
        assert_eq!(psk_from_code("ABC123"), psk_from_code("abc123 ")); // trimmed; case kept? no — trim only
        assert_ne!(psk_from_code("ABC123"), psk_from_code("XYZ789"));
        assert_eq!(psk_from_code("K7P2QX9F").len(), 32);
    }

    #[test]
    fn code_is_8_unambiguous_chars() {
        let c = gen_code();
        assert_eq!(c.len(), 8);
        assert!(c.chars().all(|ch| "0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(ch)));
    }

    #[test]
    fn sanitize_strips_separators() {
        // separators → "_", and leading dots stripped (blocks ".."/path tricks).
        assert_eq!(sanitize_name("../e/t\\c:passwd"), "_e_t_c_passwd");
        assert_eq!(sanitize_name(""), "datei");
    }

    #[test]
    fn ctrl_roundtrips() {
        let o = Ctrl::Offer { from: "A".into(), files: vec![FileMeta { name: "x".into(), size: 3 }] };
        let j = serde_json::to_vec(&o).unwrap();
        assert!(matches!(serde_json::from_slice::<Ctrl>(&j).unwrap(), Ctrl::Offer { .. }));
    }
}
