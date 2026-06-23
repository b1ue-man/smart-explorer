//! Smart Explorer share rendezvous/signaling server.
//!
//! The server is intentionally untrusted: it stores and routes signed presence
//! blobs for persistent direct contacts and rooms. It never sees relation
//! secrets, private keys, file names, file bytes, or export configuration.
//! Clients validate HMAC proofs and Noise static keys before opening a peer.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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

type Writer = Arc<Mutex<TcpStream>>;

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
    if let Ok(mut s) = w.lock() {
        if let Ok(mut line) = serde_json::to_string(msg) {
            line.push('\n');
            let _ = s.write_all(line.as_bytes());
            let _ = s.flush();
        }
    }
}

fn main() {
    let bind = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("SE_SHARE_BIND").ok())
        .unwrap_or_else(|| "0.0.0.0:51820".to_string());
    let listener = match TcpListener::bind(&bind) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("se-share-server: cannot bind {bind}: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("se-share-server listening on {bind} (rendezvous only)");
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

fn handle(stream: TcpStream, state: Arc<Mutex<State>>) -> std::io::Result<()> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(60)));
    let writer: Writer = Arc::new(Mutex::new(stream.try_clone()?));
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(());
    }
    let hello: In = match serde_json::from_str(line.trim()) {
        Ok(h) => h,
        Err(_) => {
            send(
                &writer,
                &Out::Error {
                    scope: "server".into(),
                    msg: "bad hello".into(),
                },
            );
            return Ok(());
        }
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
        return Ok(());
    };
    if protocol_version != 2 || device_id.trim().is_empty() {
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

fn dispatch(id: u64, writer: &Writer, msg: In, state: &Arc<Mutex<State>>) {
    match msg {
        In::PublishDirect { presence } => publish_direct(id, presence, state),
        In::UnpublishDirect { lookup_id } => unpublish_direct(id, &lookup_id, state),
        In::WatchDirect { lookup_id } => watch_direct(id, writer, &lookup_id, state),
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
            r#"{"t":"hello","protocol_version":2,"device_id":"a","device_name":"Laptop","listen_port":51737,"lan":["192.168.1.5"],"public_key":"pk","fingerprint":"fp"}"#,
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
        assert_eq!(protocol_version, 2);
        assert_eq!(device_id, "a");
        assert_eq!(listen_port, 51737);
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
}
