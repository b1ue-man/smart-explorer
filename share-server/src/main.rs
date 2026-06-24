//! Smart Explorer share rendezvous/signaling server.
//!
//! The server is intentionally untrusted: it stores and routes signed presence
//! blobs for persistent direct contacts and rooms. It never sees relation
//! secrets, private keys, file names, file bytes, or export configuration.
//! Clients validate HMAC proofs, pinned SmartExplorer identities, and Iroh
//! NodeIds before opening a peer session.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tungstenite::{accept, Error as WsError, Message, WebSocket};

const MAX_ROOM: usize = 64;
const MAX_WATCHES: usize = 256;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PeerPresence {
    kind: String,
    relation_id: String,
    device_id: String,
    device_name: String,
    public_key: String,
    fingerprint: String,
    #[serde(default)]
    node_id: String,
    #[serde(default)]
    relay_url: String,
    candidates: Vec<String>,
    expires_at: i64,
    nonce: String,
    proof: String,
}

#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
enum In {
    Hello {
        protocol_version: u32,
        device_id: String,
        device_name: String,
        listen_port: u16,
        #[serde(default)]
        lan: Vec<String>,
        public_key: String,
        fingerprint: String,
    },
    PublishDirect {
        presence: PeerPresence,
    },
    UnpublishDirect {
        lookup_id: String,
    },
    WatchDirect {
        lookup_id: String,
    },
    RequestDirect {
        lookup_id: String,
        presence: PeerPresence,
    },
    DirectAccessAccepted {
        lookup_id: String,
        requester_device_id: String,
        accepted: bool,
        presence: Option<PeerPresence>,
        msg: Option<String>,
    },
    UnwatchDirect {
        lookup_id: String,
    },
    JoinRoom {
        room_id: String,
        presence: PeerPresence,
    },
    LeaveRoom {
        room_id: String,
    },
    Heartbeat,
}

#[derive(Serialize, Clone)]
#[serde(tag = "t", rename_all = "snake_case")]
enum Out {
    HelloOk,
    DirectAvailable {
        lookup_id: String,
        presence: PeerPresence,
    },
    DirectOffline {
        lookup_id: String,
    },
    DirectAccessRequest {
        lookup_id: String,
        presence: PeerPresence,
    },
    DirectAccessAccepted {
        lookup_id: String,
        requester_device_id: String,
        accepted: bool,
        presence: Option<PeerPresence>,
        msg: Option<String>,
    },
    RoomRoster {
        room_id: String,
        members: Vec<PeerPresence>,
    },
    RoomJoined {
        room_id: String,
        presence: PeerPresence,
    },
    RoomLeft {
        room_id: String,
        device_id: String,
    },
    Error {
        scope: String,
        msg: String,
    },
    Pong,
}

type Writer = Sender<Out>;

#[derive(Clone)]
struct Client {
    writer: Writer,
    device_id: String,
    direct_lookup_ids: HashSet<String>,
    watched_lookup_ids: HashSet<String>,
    rooms: HashSet<String>,
}

#[derive(Default)]
struct State {
    next_id: u64,
    clients: HashMap<u64, Client>,
    direct: HashMap<String, (u64, PeerPresence)>,
    watchers: HashMap<String, HashSet<u64>>,
    rooms: HashMap<String, HashMap<String, (u64, PeerPresence)>>,
}

fn send(w: &Writer, msg: &Out) {
    let _ = w.send(msg.clone());
}

fn main() {
    let bind = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("SE_SHARE_BIND").ok())
        .unwrap_or_else(|| "0.0.0.0:51820".to_string());
    let _relay_guard = start_iroh_relay(&bind);
    let listener = match TcpListener::bind(&bind) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("se-share-server: cannot bind {bind}: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("se-share-server listening on {bind} (raw TCP + ws upgrade, rendezvous only)");
    let state = Arc::new(Mutex::new(State::default()));
    for conn in listener.incoming() {
        let stream = match conn {
            Ok(s) => s,
            Err(_) => continue,
        };
        let state = state.clone();
        std::thread::spawn(move || {
            let _ = handle(stream, state);
        });
    }
}

struct RelayGuard {
    _rt: tokio::runtime::Runtime,
    _server: iroh_relay::server::Server,
}

fn start_iroh_relay(signal_bind: &str) -> Option<RelayGuard> {
    if std::env::var("SE_IROH_RELAY_DISABLE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        eprintln!("iroh relay disabled via SE_IROH_RELAY_DISABLE");
        return None;
    }
    let bind = std::env::var("SE_IROH_RELAY_BIND")
        .ok()
        .unwrap_or_else(|| default_relay_bind(signal_bind));
    let addr = match bind.parse::<std::net::SocketAddr>() {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!("iroh relay disabled: invalid SE_IROH_RELAY_BIND/default {bind}: {e}");
            return None;
        }
    };
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("se-iroh-relay")
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("iroh relay disabled: cannot create runtime: {e}");
            return None;
        }
    };
    let server = match rt.block_on(async {
        let mut cfg = iroh_relay::server::ServerConfig::default();
        cfg.relay = Some(iroh_relay::server::RelayConfig::new(addr));
        iroh_relay::server::Server::spawn(cfg).await
    }) {
        Ok(server) => server,
        Err(e) => {
            eprintln!("iroh relay disabled: cannot bind {addr}: {e}");
            return None;
        }
    };
    let relay_url = server
        .http_addr()
        .map(|addr| format!("http://{addr}"))
        .unwrap_or_else(|| format!("http://{addr}"));
    eprintln!("se-share-server iroh relay listening on {relay_url}");
    Some(RelayGuard {
        _rt: rt,
        _server: server,
    })
}

fn default_relay_bind(signal_bind: &str) -> String {
    let host_port = signal_bind
        .trim()
        .trim_start_matches("tcp://")
        .trim_end_matches('/');
    if let Ok(addr) = host_port.parse::<std::net::SocketAddr>() {
        let mut next = addr;
        next.set_port(addr.port().saturating_add(1));
        return next.to_string();
    }
    "0.0.0.0:51821".to_string()
}

fn handle(stream: TcpStream, state: Arc<Mutex<State>>) -> std::io::Result<()> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut probe = [0u8; 3];
    let n = stream.peek(&mut probe)?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(60)));
    if n >= 1 && probe[0] == b'G' {
        return handle_ws(stream, state);
    }
    handle_tcp(stream, state)
}

fn handle_tcp(mut stream: TcpStream, state: Arc<Mutex<State>>) -> std::io::Result<()> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(60)));
    let mut line = String::new();
    if read_tcp_line_raw(&mut stream, &mut line)? == 0 {
        return Ok(());
    }
    let hello: In = match serde_json::from_str(line.trim()) {
        Ok(h) => h,
        Err(_) => return Ok(()),
    };
    let writer = spawn_tcp_writer(stream.try_clone()?);
    let mut reader = BufReader::new(stream);
    let In::Hello {
        protocol_version,
        device_id,
        device_name: _,
        listen_port: _,
        lan: _,
        public_key: _,
        fingerprint: _,
    } = hello
    else {
        send(
            &writer,
            &Out::Error {
                scope: "server".into(),
                msg: "first message must be hello".into(),
            },
        );
        return Ok(());
    };
    if protocol_version != 3 || device_id.trim().is_empty() {
        send(
            &writer,
            &Out::Error {
                scope: "server".into(),
                msg: "unsupported hello".into(),
            },
        );
        return Ok(());
    }

    let id = {
        let mut st = state.lock().unwrap();
        st.next_id += 1;
        let id = st.next_id;
        st.clients.insert(
            id,
            Client {
                writer: writer.clone(),
                device_id: device_id.clone(),
                direct_lookup_ids: HashSet::new(),
                watched_lookup_ids: HashSet::new(),
                rooms: HashSet::new(),
            },
        );
        id
    };
    send(&writer, &Out::HelloOk);

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {
                break;
            }
            Err(_) => break,
            Ok(_) => {}
        }
        let msg: In = match serde_json::from_str(line.trim()) {
            Ok(m) => m,
            Err(_) => continue,
        };
        dispatch(id, &writer, msg, &state);
    }
    cleanup(id, &state);
    Ok(())
}

fn read_tcp_line_raw(stream: &mut TcpStream, line: &mut String) -> io::Result<usize> {
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

fn handle_ws(stream: TcpStream, state: Arc<Mutex<State>>) -> std::io::Result<()> {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let mut ws = accept(stream).map_err(io_other)?;
    let (writer, out_rx) = mpsc::channel::<Out>();
    let hello = match read_ws_json(&mut ws, &out_rx, Duration::from_secs(60)) {
        Ok(Some(msg)) => msg,
        Ok(None) => return Ok(()),
        Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {
            return Ok(())
        }
        Err(e) => return Err(e),
    };
    let In::Hello {
        protocol_version,
        device_id,
        device_name: _,
        listen_port: _,
        lan: _,
        public_key: _,
        fingerprint: _,
    } = hello
    else {
        send(
            &writer,
            &Out::Error {
                scope: "server".into(),
                msg: "first message must be hello".into(),
            },
        );
        flush_ws_out(&mut ws, &out_rx)?;
        return Ok(());
    };
    if protocol_version != 3 || device_id.trim().is_empty() {
        send(
            &writer,
            &Out::Error {
                scope: "server".into(),
                msg: "unsupported hello".into(),
            },
        );
        flush_ws_out(&mut ws, &out_rx)?;
        return Ok(());
    }

    let id = register_client(&state, writer.clone(), device_id);
    send(&writer, &Out::HelloOk);

    loop {
        flush_ws_out(&mut ws, &out_rx)?;
        match read_ws_json(&mut ws, &out_rx, Duration::from_millis(500)) {
            Ok(Some(msg)) => dispatch(id, &writer, msg, &state),
            Ok(None) => break,
            Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {}
            Err(e) => return Err(e),
        }
    }
    cleanup(id, &state);
    Ok(())
}

fn register_client(state: &Arc<Mutex<State>>, writer: Writer, device_id: String) -> u64 {
    let mut st = state.lock().unwrap();
    st.next_id += 1;
    let id = st.next_id;
    st.clients.insert(
        id,
        Client {
            writer,
            device_id,
            direct_lookup_ids: HashSet::new(),
            watched_lookup_ids: HashSet::new(),
            rooms: HashSet::new(),
        },
    );
    id
}

fn spawn_tcp_writer(mut stream: TcpStream) -> Writer {
    let (tx, rx) = mpsc::channel::<Out>();
    std::thread::Builder::new()
        .name("share-server-tcp-writer".into())
        .spawn(move || {
            while let Ok(msg) = rx.recv() {
                if write_tcp_msg(&mut stream, &msg).is_err() {
                    break;
                }
            }
        })
        .ok();
    tx
}

fn write_tcp_msg(stream: &mut TcpStream, msg: &Out) -> io::Result<()> {
    let mut line = serde_json::to_string(msg).map_err(io_other)?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    stream.flush()
}

fn flush_ws_out(ws: &mut WebSocket<TcpStream>, rx: &Receiver<Out>) -> io::Result<()> {
    while let Ok(msg) = rx.try_recv() {
        let text = serde_json::to_string(&msg).map_err(io_other)?;
        ws.send(Message::Text(text)).map_err(ws_to_io)?;
        ws.flush().map_err(ws_to_io)?;
    }
    Ok(())
}

fn read_ws_json(
    ws: &mut WebSocket<TcpStream>,
    out_rx: &Receiver<Out>,
    timeout: Duration,
) -> io::Result<Option<In>> {
    let deadline = Instant::now() + timeout;
    loop {
        flush_ws_out(ws, out_rx)?;
        match ws.read() {
            Ok(Message::Text(text)) => {
                let text = text.trim();
                if text.is_empty() {
                    continue;
                }
                match serde_json::from_str(text) {
                    Ok(msg) => return Ok(Some(msg)),
                    Err(_) => continue,
                }
            }
            Ok(Message::Binary(bytes)) => {
                let text = String::from_utf8(bytes).map_err(io_other)?;
                let text = text.trim();
                if text.is_empty() {
                    continue;
                }
                match serde_json::from_str(text) {
                    Ok(msg) => return Ok(Some(msg)),
                    Err(_) => continue,
                }
            }
            Ok(Message::Ping(payload)) => {
                ws.send(Message::Pong(payload)).map_err(ws_to_io)?;
                ws.flush().map_err(ws_to_io)?;
            }
            Ok(Message::Pong(_)) => {}
            Ok(Message::Close(_)) => return Ok(None),
            Ok(_) => {}
            Err(WsError::Io(e))
                if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut =>
            {
                if Instant::now() >= deadline {
                    return Err(io::Error::new(ErrorKind::TimedOut, "websocket idle"));
                }
            }
            Err(WsError::ConnectionClosed | WsError::AlreadyClosed) => return Ok(None),
            Err(e) => return Err(ws_to_io(e)),
        }
    }
}

fn ws_to_io(err: WsError) -> io::Error {
    match err {
        WsError::Io(e) => e,
        other => io_other(other),
    }
}

fn io_other<E: std::fmt::Display>(err: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, err.to_string())
}

fn dispatch(id: u64, writer: &Writer, msg: In, state: &Arc<Mutex<State>>) {
    match msg {
        In::PublishDirect { presence } => publish_direct(id, presence, state),
        In::UnpublishDirect { lookup_id } => unpublish_direct(id, &lookup_id, state),
        In::WatchDirect { lookup_id } => watch_direct(id, writer, &lookup_id, state),
        In::RequestDirect {
            lookup_id,
            presence,
        } => request_direct(writer, &lookup_id, presence, state),
        In::DirectAccessAccepted {
            lookup_id,
            requester_device_id,
            accepted,
            presence,
            msg,
        } => direct_access_accepted(
            &lookup_id,
            &requester_device_id,
            accepted,
            presence,
            msg,
            state,
        ),
        In::UnwatchDirect { lookup_id } => {
            let mut st = state.lock().unwrap();
            if let Some(c) = st.clients.get_mut(&id) {
                c.watched_lookup_ids.remove(&lookup_id);
            }
            if let Some(w) = st.watchers.get_mut(&lookup_id) {
                w.remove(&id);
            }
        }
        In::JoinRoom { room_id, presence } => join_room(id, writer, &room_id, presence, state),
        In::LeaveRoom { room_id } => leave_room(id, &room_id, state),
        In::Heartbeat => send(writer, &Out::Pong),
        In::Hello { .. } => {}
    }
}

fn request_direct(
    writer: &Writer,
    lookup_id: &str,
    presence: PeerPresence,
    state: &Arc<Mutex<State>>,
) {
    let target = {
        let st = state.lock().unwrap();
        st.direct
            .get(lookup_id)
            .and_then(|(owner_id, _)| st.clients.get(owner_id).map(|c| c.writer.clone()))
    };
    if let Some(target) = target {
        send(
            &target,
            &Out::DirectAccessRequest {
                lookup_id: lookup_id.to_string(),
                presence,
            },
        );
    } else {
        send(
            writer,
            &Out::Error {
                scope: "direct".into(),
                msg: "Direktgeraet nicht online".into(),
            },
        );
    }
}

fn direct_access_accepted(
    lookup_id: &str,
    requester_device_id: &str,
    accepted: bool,
    presence: Option<PeerPresence>,
    msg: Option<String>,
    state: &Arc<Mutex<State>>,
) {
    let targets = writers_by_device(requester_device_id, state);
    for target in targets {
        send(
            &target,
            &Out::DirectAccessAccepted {
                lookup_id: lookup_id.to_string(),
                requester_device_id: requester_device_id.to_string(),
                accepted,
                presence: presence.clone(),
                msg: msg.clone(),
            },
        );
    }
}

fn publish_direct(id: u64, presence: PeerPresence, state: &Arc<Mutex<State>>) {
    let lookup_id = presence.relation_id.clone();
    let watchers = {
        let mut st = state.lock().unwrap();
        st.direct.insert(lookup_id.clone(), (id, presence.clone()));
        if let Some(c) = st.clients.get_mut(&id) {
            c.direct_lookup_ids.insert(lookup_id.clone());
        }
        st.watchers.get(&lookup_id).cloned().unwrap_or_default()
    };
    notify_direct_available(&lookup_id, &presence, watchers, state);
}

fn unpublish_direct(id: u64, lookup_id: &str, state: &Arc<Mutex<State>>) {
    let watchers = {
        let mut st = state.lock().unwrap();
        if st.direct.get(lookup_id).map(|(owner, _)| *owner) == Some(id) {
            st.direct.remove(lookup_id);
        }
        if let Some(c) = st.clients.get_mut(&id) {
            c.direct_lookup_ids.remove(lookup_id);
        }
        st.watchers.get(lookup_id).cloned().unwrap_or_default()
    };
    notify_direct_offline(lookup_id, watchers, state);
}

fn watch_direct(id: u64, writer: &Writer, lookup_id: &str, state: &Arc<Mutex<State>>) {
    let current = {
        let mut st = state.lock().unwrap();
        if st
            .clients
            .get(&id)
            .map(|c| c.watched_lookup_ids.len() >= MAX_WATCHES)
            .unwrap_or(false)
        {
            send(
                writer,
                &Out::Error {
                    scope: "direct".into(),
                    msg: "too many watches".into(),
                },
            );
            return;
        }
        st.watchers
            .entry(lookup_id.to_string())
            .or_default()
            .insert(id);
        if let Some(c) = st.clients.get_mut(&id) {
            c.watched_lookup_ids.insert(lookup_id.to_string());
        }
        st.direct.get(lookup_id).map(|(_, p)| p.clone())
    };
    if let Some(presence) = current {
        send(
            writer,
            &Out::DirectAvailable {
                lookup_id: lookup_id.to_string(),
                presence,
            },
        );
    }
}

fn join_room(
    id: u64,
    writer: &Writer,
    room_id: &str,
    presence: PeerPresence,
    state: &Arc<Mutex<State>>,
) {
    let (roster, joined_targets) = {
        let mut st = state.lock().unwrap();
        let members = st.rooms.entry(room_id.to_string()).or_default();
        if members.len() >= MAX_ROOM && !members.contains_key(&presence.device_id) {
            send(
                writer,
                &Out::Error {
                    scope: "room".into(),
                    msg: "room full".into(),
                },
            );
            return;
        }
        let roster: Vec<PeerPresence> = members.values().map(|(_, p)| p.clone()).collect();
        let target_ids: Vec<u64> = members.values().map(|(client_id, _)| *client_id).collect();
        members.insert(presence.device_id.clone(), (id, presence.clone()));
        if let Some(c) = st.clients.get_mut(&id) {
            c.rooms.insert(room_id.to_string());
        }
        let targets: Vec<Writer> = target_ids
            .into_iter()
            .filter_map(|client_id| st.clients.get(&client_id).map(|c| c.writer.clone()))
            .collect();
        (roster, targets)
    };
    send(
        writer,
        &Out::RoomRoster {
            room_id: room_id.to_string(),
            members: roster,
        },
    );
    for target in joined_targets {
        send(
            &target,
            &Out::RoomJoined {
                room_id: room_id.to_string(),
                presence: presence.clone(),
            },
        );
    }
}

fn leave_room(id: u64, room_id: &str, state: &Arc<Mutex<State>>) {
    let (device_id, targets) = {
        let mut st = state.lock().unwrap();
        let device_id = st
            .clients
            .get(&id)
            .map(|c| c.device_id.clone())
            .unwrap_or_default();
        let mut targets = Vec::new();
        if let Some(members) = st.rooms.get_mut(room_id) {
            members.retain(|_, (client_id, _)| *client_id != id);
            let remaining_ids: Vec<u64> =
                members.values().map(|(client_id, _)| *client_id).collect();
            if members.is_empty() {
                st.rooms.remove(room_id);
            }
            for client_id in remaining_ids {
                if let Some(c) = st.clients.get(&client_id) {
                    targets.push(c.writer.clone());
                }
            }
        }
        if let Some(c) = st.clients.get_mut(&id) {
            c.rooms.remove(room_id);
        }
        (device_id, targets)
    };
    for target in targets {
        send(
            &target,
            &Out::RoomLeft {
                room_id: room_id.to_string(),
                device_id: device_id.clone(),
            },
        );
    }
}

fn cleanup(id: u64, state: &Arc<Mutex<State>>) {
    let (directs, watched, rooms) = {
        let mut st = state.lock().unwrap();
        let client = match st.clients.remove(&id) {
            Some(c) => c,
            None => return,
        };
        for lookup in &client.watched_lookup_ids {
            if let Some(w) = st.watchers.get_mut(lookup) {
                w.remove(&id);
            }
        }
        (
            client.direct_lookup_ids.into_iter().collect::<Vec<_>>(),
            client.watched_lookup_ids.into_iter().collect::<Vec<_>>(),
            client.rooms.into_iter().collect::<Vec<_>>(),
        )
    };
    for lookup in directs {
        unpublish_direct(id, &lookup, state);
    }
    for lookup in watched {
        let mut st = state.lock().unwrap();
        if let Some(w) = st.watchers.get_mut(&lookup) {
            w.remove(&id);
        }
    }
    for room in rooms {
        leave_room(id, &room, state);
    }
}

fn notify_direct_available(
    lookup_id: &str,
    presence: &PeerPresence,
    watchers: HashSet<u64>,
    state: &Arc<Mutex<State>>,
) {
    let writers = writers_for(watchers, state);
    for w in writers {
        send(
            &w,
            &Out::DirectAvailable {
                lookup_id: lookup_id.to_string(),
                presence: presence.clone(),
            },
        );
    }
}

fn notify_direct_offline(lookup_id: &str, watchers: HashSet<u64>, state: &Arc<Mutex<State>>) {
    let writers = writers_for(watchers, state);
    for w in writers {
        send(
            &w,
            &Out::DirectOffline {
                lookup_id: lookup_id.to_string(),
            },
        );
    }
}

fn writers_for(ids: HashSet<u64>, state: &Arc<Mutex<State>>) -> Vec<Writer> {
    let st = state.lock().unwrap();
    ids.into_iter()
        .filter_map(|id| st.clients.get(&id).map(|c| c.writer.clone()))
        .collect()
}

fn writers_by_device(device_id: &str, state: &Arc<Mutex<State>>) -> Vec<Writer> {
    let st = state.lock().unwrap();
    st.clients
        .values()
        .filter(|c| c.device_id == device_id)
        .map(|c| c.writer.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn presence(kind: &str, relation: &str, device: &str) -> PeerPresence {
        PeerPresence {
            kind: kind.into(),
            relation_id: relation.into(),
            device_id: device.into(),
            device_name: device.into(),
            public_key: "pk".into(),
            fingerprint: "fp".into(),
            node_id: "node".into(),
            relay_url: "http://127.0.0.1:51821".into(),
            candidates: vec!["127.0.0.1:1".into()],
            expires_at: 99,
            nonce: "n".into(),
            proof: "proof".into(),
        }
    }

    #[test]
    fn out_messages_serialize_tagged() {
        let m = Out::DirectOffline {
            lookup_id: "x".into(),
        };
        assert_eq!(
            serde_json::to_string(&m).unwrap(),
            r#"{"t":"direct_offline","lookup_id":"x"}"#
        );
        let r = Out::RoomRoster {
            room_id: "r".into(),
            members: vec![],
        };
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            r#"{"t":"room_roster","room_id":"r","members":[]}"#
        );
    }

    #[test]
    fn hello_parses() {
        let h: In = serde_json::from_str(
            r#"{"t":"hello","protocol_version":3,"device_id":"a","device_name":"Laptop","listen_port":0,"lan":["192.168.1.5"],"public_key":"pk","fingerprint":"fp"}"#,
        )
        .unwrap();
        let In::Hello {
            protocol_version,
            device_id,
            listen_port,
            ..
        } = h
        else {
            panic!("not hello");
        };
        assert_eq!(protocol_version, 3);
        assert_eq!(device_id, "a");
        assert_eq!(listen_port, 0);
    }

    #[test]
    fn presence_roundtrips() {
        let p = presence("room", "r", "d");
        let s = serde_json::to_string(&p).unwrap();
        let back: PeerPresence = serde_json::from_str(&s).unwrap();
        assert_eq!(back.kind, "room");
        assert_eq!(back.relation_id, "r");
        assert_eq!(back.device_id, "d");
    }

    #[test]
    fn direct_request_routes_to_lookup_owner() {
        let mut state = State::default();
        let (owner_tx, owner_rx) = mpsc::channel();
        let (requester_tx, requester_rx) = mpsc::channel();
        state.clients.insert(
            1,
            Client {
                writer: owner_tx,
                device_id: "owner".into(),
                direct_lookup_ids: HashSet::from(["lookup".into()]),
                watched_lookup_ids: HashSet::new(),
                rooms: HashSet::new(),
            },
        );
        state.clients.insert(
            2,
            Client {
                writer: requester_tx.clone(),
                device_id: "requester".into(),
                direct_lookup_ids: HashSet::new(),
                watched_lookup_ids: HashSet::new(),
                rooms: HashSet::new(),
            },
        );
        state
            .direct
            .insert("lookup".into(), (1, presence("direct", "lookup", "owner")));
        let state = Arc::new(Mutex::new(state));
        request_direct(
            &requester_tx,
            "lookup",
            presence("direct", "lookup", "requester"),
            &state,
        );
        match owner_rx.recv().unwrap() {
            Out::DirectAccessRequest {
                lookup_id,
                presence,
            } => {
                assert_eq!(lookup_id, "lookup");
                assert_eq!(presence.device_id, "requester");
            }
            _ => panic!("wrong message"),
        }
        assert!(requester_rx.try_recv().is_err());
    }

    #[test]
    fn direct_accept_routes_to_requester_device() {
        let mut state = State::default();
        let (requester_tx, requester_rx) = mpsc::channel();
        state.clients.insert(
            1,
            Client {
                writer: requester_tx,
                device_id: "requester".into(),
                direct_lookup_ids: HashSet::new(),
                watched_lookup_ids: HashSet::new(),
                rooms: HashSet::new(),
            },
        );
        let state = Arc::new(Mutex::new(state));
        direct_access_accepted(
            "lookup",
            "requester",
            true,
            Some(presence("direct", "lookup", "owner")),
            None,
            &state,
        );
        match requester_rx.recv().unwrap() {
            Out::DirectAccessAccepted {
                lookup_id,
                requester_device_id,
                accepted,
                presence,
                ..
            } => {
                assert_eq!(lookup_id, "lookup");
                assert_eq!(requester_device_id, "requester");
                assert!(accepted);
                assert_eq!(presence.unwrap().device_id, "owner");
            }
            _ => panic!("wrong message"),
        }
    }

    #[test]
    fn relay_bind_defaults_to_next_port() {
        assert_eq!(default_relay_bind("127.0.0.1:51820"), "127.0.0.1:51821");
        assert_eq!(default_relay_bind("0.0.0.0:443"), "0.0.0.0:444");
    }

    #[test]
    fn websocket_client_can_hello_and_heartbeat() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(Mutex::new(State::default()));
        let server_state = state.clone();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle(stream, server_state).unwrap();
        });

        let (mut ws, _) = tungstenite::connect(format!("ws://{addr}/se-share")).unwrap();
        ws.send(Message::Text(
            r#"{"t":"hello","protocol_version":3,"device_id":"a","device_name":"Laptop","listen_port":0,"lan":["127.0.0.1"],"public_key":"pk","fingerprint":"fp"}"#.to_string(),
        ))
        .unwrap();
        let first = ws.read().unwrap().into_text().unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&first).unwrap()["t"],
            "hello_ok"
        );

        std::thread::sleep(Duration::from_millis(750));
        ws.send(Message::Text(r#"{"t":"heartbeat"}"#.to_string()))
            .unwrap();
        let second = ws.read().unwrap().into_text().unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&second).unwrap()["t"],
            "pong"
        );

        ws.close(None).unwrap();
        handle.join().unwrap();
    }
}
