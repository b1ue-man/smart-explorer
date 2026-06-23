//! Smart Explorer share **rendezvous/signaling server** (standalone, headless).
//!
//! It ONLY routes discovery: it introduces two devices that share a pairing
//! code, and tracks room membership, so peers can then connect **directly** and
//! transfer files **peer-to-peer, end-to-end encrypted**. File bytes never pass
//! through this server — it sees only the small JSON control messages below.
//!
//! Wire protocol: newline-delimited JSON over TCP.
//!   client → server:  Hello { mode:"pair"|"room", code, device, listen_port, lan[], pubkey }
//!   server → client:  Peer | Roster | Joined | Left | Error
//!
//! Usage: `se-share-server [BIND_ADDR]`   (default 0.0.0.0:51820; env SE_SHARE_BIND)
//!
//! Runs on Linux and Windows (thread-per-connection, std only + serde). For a
//! personal/self-hosted deployment this scales fine.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

const MAX_CODE: usize = 64;
const MAX_LAN: usize = 8;
const MAX_ROOM: usize = 32;

#[derive(Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
enum In {
    Hello {
        mode: String,
        code: String,
        device: String,
        listen_port: u16,
        #[serde(default)]
        lan: Vec<String>,
        #[serde(default)]
        pubkey: String,
    },
}

#[derive(Serialize, Clone)]
struct Member {
    device: String,
    candidates: Vec<String>,
    pubkey: String,
}

#[derive(Serialize, Clone)]
#[serde(tag = "t", rename_all = "lowercase")]
enum Out {
    Peer {
        device: String,
        candidates: Vec<String>,
        pubkey: String,
    },
    Roster {
        members: Vec<Member>,
    },
    Joined {
        member: Member,
    },
    Left {
        device: String,
        pubkey: String,
    },
    Error {
        msg: String,
    },
}

/// A live connection we can push control messages to (its socket write half).
type Writer = Arc<Mutex<TcpStream>>;

#[derive(Clone)]
struct Peer {
    member: Member,
    writer: Writer,
}

#[derive(Default)]
struct State {
    /// code → the single peer waiting to be paired.
    pairing: HashMap<String, Peer>,
    /// code → current room members.
    rooms: HashMap<String, Vec<Peer>>,
}

/// Build a peer's reachable candidates: each LAN IP and the public IP the server
/// observed, all at the peer's advertised listen port.
fn build_candidates(lan: &[String], public_ip: &str, port: u16) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for ip in lan.iter().take(MAX_LAN) {
        let ip = ip.trim();
        if !ip.is_empty() {
            out.push(format!("{}:{}", ip, port));
        }
    }
    let pub_c = format!("{}:{}", public_ip, port);
    if !out.contains(&pub_c) {
        out.push(pub_c);
    }
    out
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
    eprintln!("se-share-server listening on {bind} (routes discovery only; no file data)");
    let state = Arc::new(Mutex::new(State::default()));
    for conn in listener.incoming() {
        let stream = match conn {
            Ok(s) => s,
            Err(_) => continue,
        };
        let state = state.clone();
        std::thread::spawn(move || {
            if let Err(e) = handle(stream, state) {
                let _ = e;
            }
        });
    }
}

fn handle(stream: TcpStream, state: Arc<Mutex<State>>) -> std::io::Result<()> {
    let public_ip = stream
        .peer_addr()
        .map(|a| a.ip().to_string())
        .unwrap_or_default();
    let writer: Writer = Arc::new(Mutex::new(stream.try_clone()?));
    let mut reader = BufReader::new(stream);

    // First line must be Hello.
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
                    msg: "bad hello".into(),
                },
            );
            return Ok(());
        }
    };
    let In::Hello {
        mode,
        code,
        device,
        listen_port,
        lan,
        pubkey,
    } = hello;
    if code.is_empty() || code.len() > MAX_CODE {
        send(
            &writer,
            &Out::Error {
                msg: "bad code".into(),
            },
        );
        return Ok(());
    }
    let candidates = build_candidates(&lan, &public_ip, listen_port);
    let me = Member {
        device: device.clone(),
        candidates,
        pubkey: pubkey.clone(),
    };
    let peer = Peer {
        member: me.clone(),
        writer: writer.clone(),
    };

    match mode.as_str() {
        "pair" => handle_pair(&code, peer, &state, &mut reader),
        "room" => handle_room(&code, peer, &state, &mut reader),
        _ => {
            send(
                &writer,
                &Out::Error {
                    msg: "bad mode".into(),
                },
            );
            Ok(())
        }
    }
}

fn handle_pair(
    code: &str,
    peer: Peer,
    state: &Arc<Mutex<State>>,
    reader: &mut BufReader<TcpStream>,
) -> std::io::Result<()> {
    // If someone is already waiting on this code, introduce them and finish.
    let waiting = {
        let mut st = state.lock().unwrap();
        st.pairing.remove(code)
    };
    if let Some(other) = waiting {
        // Tell each peer about the other; both then connect directly.
        send(
            &other.writer,
            &Out::Peer {
                device: peer.member.device.clone(),
                candidates: peer.member.candidates.clone(),
                pubkey: peer.member.pubkey.clone(),
            },
        );
        send(
            &peer.writer,
            &Out::Peer {
                device: other.member.device.clone(),
                candidates: other.member.candidates.clone(),
                pubkey: other.member.pubkey.clone(),
            },
        );
        return Ok(());
    }
    // Otherwise wait to be matched; hold the socket open until the client leaves.
    {
        let mut st = state.lock().unwrap();
        st.pairing.insert(code.to_string(), peer);
    }
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break, // disconnected
            Ok(_) => {}              // ignore keepalives
        }
    }
    // Clean up if we left before being matched.
    let mut st = state.lock().unwrap();
    st.pairing.remove(code);
    Ok(())
}

fn handle_room(
    code: &str,
    peer: Peer,
    state: &Arc<Mutex<State>>,
    reader: &mut BufReader<TcpStream>,
) -> std::io::Result<()> {
    // Join: send the newcomer the current roster, tell members someone joined.
    {
        let mut st = state.lock().unwrap();
        let members = st.rooms.entry(code.to_string()).or_default();
        if members.len() >= MAX_ROOM {
            drop(st);
            send(
                &peer.writer,
                &Out::Error {
                    msg: "room full".into(),
                },
            );
            return Ok(());
        }
        let roster: Vec<Member> = members.iter().map(|p| p.member.clone()).collect();
        send(&peer.writer, &Out::Roster { members: roster });
        for m in members.iter() {
            send(
                &m.writer,
                &Out::Joined {
                    member: peer.member.clone(),
                },
            );
        }
        members.push(peer.clone());
    }
    // Stay connected for roster updates until the client leaves.
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
    // Leave: drop from the room and notify the rest.
    let mut st = state.lock().unwrap();
    if let Some(members) = st.rooms.get_mut(code) {
        members.retain(|p| !Arc::ptr_eq(&p.writer, &peer.writer));
        let remaining: Vec<Writer> = members.iter().map(|p| p.writer.clone()).collect();
        if members.is_empty() {
            st.rooms.remove(code);
        }
        drop(st);
        for w in &remaining {
            send(
                w,
                &Out::Left {
                    device: peer.member.device.clone(),
                    pubkey: peer.member.pubkey.clone(),
                },
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_include_lan_and_public_no_dupes() {
        let c = build_candidates(
            &["192.168.1.5".into(), "10.0.0.2".into()],
            "203.0.113.9",
            51737,
        );
        assert_eq!(
            c,
            vec![
                "192.168.1.5:51737".to_string(),
                "10.0.0.2:51737".to_string(),
                "203.0.113.9:51737".to_string(),
            ]
        );
        // If the public IP equals a LAN entry (same network), no duplicate.
        let c2 = build_candidates(&["203.0.113.9".into()], "203.0.113.9", 22);
        assert_eq!(c2, vec!["203.0.113.9:22".to_string()]);
    }

    #[test]
    fn hello_parses() {
        let h: In = serde_json::from_str(
            r#"{"t":"hello","mode":"pair","code":"K7P2QX9F","device":"Laptop","listen_port":51737,"lan":["192.168.1.5"],"pubkey":"AAAA"}"#,
        )
        .unwrap();
        let In::Hello {
            mode,
            code,
            listen_port,
            ..
        } = h;
        assert_eq!(mode, "pair");
        assert_eq!(code, "K7P2QX9F");
        assert_eq!(listen_port, 51737);
    }

    #[test]
    fn out_messages_serialize_tagged() {
        let m = Out::Left {
            device: "X".into(),
            pubkey: "k".into(),
        };
        assert_eq!(
            serde_json::to_string(&m).unwrap(),
            r#"{"t":"left","device":"X","pubkey":"k"}"#
        );
        let r = Out::Roster { members: vec![] };
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            r#"{"t":"roster","members":[]}"#
        );
    }
}
