use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use tungstenite::{
    connect as ws_connect, stream::MaybeTlsStream, Error as WsError, Message, WebSocket,
};

use super::core::{eio, hmac_proof, now_secs, presence_payload, random_token};
use super::identity::ShareIdentity;
use super::types::PeerPresence;
use super::wire::ClientMsg;

pub(crate) struct RelayStream {
    inner: RelayInner,
    read_buf: Vec<u8>,
    read_pos: usize,
}

enum RelayInner {
    Tcp(TcpStream),
    Ws(WebSocket<MaybeTlsStream<TcpStream>>),
}

impl RelayStream {
    pub(crate) fn connect(server: &str, relay_id: &str, device_id: &str) -> io::Result<Self> {
        let msg = ClientMsg::RelayJoin {
            relay_id: relay_id.to_string(),
            device_id: device_id.to_string(),
        };
        let mut errors = Vec::new();
        for endpoint in signal_endpoints(server) {
            match connect_one(&endpoint, &msg) {
                Ok(inner) => {
                    return Ok(Self {
                        inner,
                        read_buf: Vec::new(),
                        read_pos: 0,
                    })
                }
                Err(e) => errors.push(format!("{endpoint}: {e}")),
            }
        }
        Err(eio(format!(
            "Relay-Verbindung nicht moeglich ({})",
            errors.join("; ")
        )))
    }
}

impl Read for RelayStream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        while self.read_pos >= self.read_buf.len() {
            self.read_buf.clear();
            self.read_pos = 0;
            match &mut self.inner {
                RelayInner::Tcp(s) => {
                    let mut buf = vec![0u8; 16 * 1024];
                    let n = s.read(&mut buf)?;
                    if n == 0 {
                        return Ok(0);
                    }
                    buf.truncate(n);
                    self.read_buf = buf;
                }
                RelayInner::Ws(ws) => loop {
                    match ws.read() {
                        Ok(Message::Binary(bytes)) => {
                            self.read_buf = bytes;
                            break;
                        }
                        Ok(Message::Ping(payload)) => {
                            ws.send(Message::Pong(payload)).map_err(ws_to_io)?;
                        }
                        Ok(Message::Text(_)) | Ok(Message::Pong(_)) => {}
                        Ok(Message::Close(_)) => return Ok(0),
                        Ok(_) => {}
                        Err(WsError::Io(e)) => return Err(e),
                        Err(WsError::ConnectionClosed | WsError::AlreadyClosed) => return Ok(0),
                        Err(e) => return Err(ws_to_io(e)),
                    }
                },
            }
        }
        let n = out.len().min(self.read_buf.len() - self.read_pos);
        out[..n].copy_from_slice(&self.read_buf[self.read_pos..self.read_pos + n]);
        self.read_pos += n;
        Ok(n)
    }
}

impl Write for RelayStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match &mut self.inner {
            RelayInner::Tcp(s) => {
                s.write_all(buf)?;
                Ok(buf.len())
            }
            RelayInner::Ws(ws) => {
                ws.send(Message::Binary(buf.to_vec())).map_err(ws_to_io)?;
                Ok(buf.len())
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.inner {
            RelayInner::Tcp(s) => s.flush(),
            RelayInner::Ws(ws) => ws.flush().map_err(ws_to_io),
        }
    }
}

pub(crate) fn send_relay_request(
    server: &str,
    identity: &ShareIdentity,
    relay_id: &str,
    relation_kind: &str,
    relation_id: &str,
    target_device_id: &str,
    relation_secret: &[u8],
) -> io::Result<()> {
    let presence = relation_presence(relation_kind, relation_id, identity, relation_secret);
    let request = ClientMsg::RelayRequest {
        relay_id: relay_id.to_string(),
        relation_kind: relation_kind.to_string(),
        relation_id: relation_id.to_string(),
        target_device_id: target_device_id.to_string(),
        requester_presence: presence,
    };
    let hello = ClientMsg::Hello {
        protocol_version: 2,
        device_id: identity.device_id.clone(),
        device_name: identity.device_name.clone(),
        listen_port: 0,
        lan: Vec::new(),
        public_key: identity.public_key.clone(),
        fingerprint: identity.fingerprint.clone(),
    };
    let mut errors = Vec::new();
    for endpoint in signal_endpoints(server) {
        match send_signal_pair(&endpoint, &hello, &request) {
            Ok(()) => return Ok(()),
            Err(e) => errors.push(format!("{endpoint}: {e}")),
        }
    }
    Err(eio(format!(
        "Relay-Anfrage nicht moeglich ({})",
        errors.join("; ")
    )))
}

pub(crate) fn relation_presence(
    kind: &str,
    relation_id: &str,
    identity: &ShareIdentity,
    secret: &[u8],
) -> PeerPresence {
    let candidates = Vec::new();
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
    PeerPresence {
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

fn connect_one(endpoint: &str, msg: &ClientMsg) -> io::Result<RelayInner> {
    let normalized = normalize_signal_endpoint(endpoint);
    if normalized.starts_with("ws://") || normalized.starts_with("wss://") {
        let (mut socket, _) = ws_connect(&normalized).map_err(ws_to_io)?;
        set_ws_timeout(socket.get_mut(), Duration::from_secs(20));
        socket
            .send(Message::Text(serde_json::to_string(msg).map_err(eio)?))
            .map_err(ws_to_io)?;
        socket.flush().map_err(ws_to_io)?;
        Ok(RelayInner::Ws(socket))
    } else {
        let addr = normalized
            .strip_prefix("tcp://")
            .map(normalize_tcp_addr)
            .unwrap_or_else(|| normalize_tcp_addr(&normalized));
        let mut stream = TcpStream::connect(addr)?;
        let _ = stream.set_nodelay(true);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(20)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(20)));
        let mut line = serde_json::to_string(msg).map_err(eio)?;
        line.push('\n');
        stream.write_all(line.as_bytes())?;
        stream.flush()?;
        Ok(RelayInner::Tcp(stream))
    }
}

fn send_signal_pair(endpoint: &str, hello: &ClientMsg, msg: &ClientMsg) -> io::Result<()> {
    let normalized = normalize_signal_endpoint(endpoint);
    if normalized.starts_with("ws://") || normalized.starts_with("wss://") {
        let (mut socket, _) = ws_connect(&normalized).map_err(ws_to_io)?;
        set_ws_timeout(socket.get_mut(), Duration::from_secs(5));
        socket
            .send(Message::Text(serde_json::to_string(hello).map_err(eio)?))
            .map_err(ws_to_io)?;
        socket
            .send(Message::Text(serde_json::to_string(msg).map_err(eio)?))
            .map_err(ws_to_io)?;
        socket.flush().map_err(ws_to_io)
    } else {
        let addr = normalized
            .strip_prefix("tcp://")
            .map(normalize_tcp_addr)
            .unwrap_or_else(|| normalize_tcp_addr(&normalized));
        let mut stream = TcpStream::connect(addr)?;
        let _ = stream.set_nodelay(true);
        write_line(&mut stream, hello)?;
        write_line(&mut stream, msg)
    }
}

fn write_line(stream: &mut TcpStream, msg: &ClientMsg) -> io::Result<()> {
    let mut line = serde_json::to_string(msg).map_err(eio)?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    stream.flush()
}

fn signal_endpoints(config: &str) -> Vec<String> {
    config
        .split([',', ';'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn normalize_signal_endpoint(endpoint: &str) -> String {
    let trimmed = endpoint.trim();
    if let Some(rest) = trimmed.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = trimmed.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        trimmed.to_string()
    }
}

fn normalize_tcp_addr(addr: &str) -> String {
    let addr = addr.trim().trim_end_matches('/');
    if addr.is_empty() || addr.starts_with('[') || addr.rsplit_once(':').is_some() {
        addr.to_string()
    } else {
        format!("{addr}:51820")
    }
}

fn set_ws_timeout(stream: &mut MaybeTlsStream<TcpStream>, timeout: Duration) {
    match stream {
        MaybeTlsStream::Plain(tcp) => {
            let _ = tcp.set_read_timeout(Some(timeout));
            let _ = tcp.set_write_timeout(Some(timeout));
        }
        MaybeTlsStream::Rustls(tls) => {
            let _ = tls.sock.set_read_timeout(Some(timeout));
            let _ = tls.sock.set_write_timeout(Some(timeout));
        }
        #[allow(unreachable_patterns)]
        _ => {}
    }
}

fn ws_to_io(err: WsError) -> io::Error {
    match err {
        WsError::Io(e) => e,
        other => eio(other),
    }
}
