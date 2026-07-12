use std::io::{self, Write};
use std::net::TcpStream;
use std::time::Duration;

use tungstenite::{
    connect as ws_connect, stream::MaybeTlsStream, Error as WsError, Message, WebSocket,
};

use super::core::eio;
use super::line::{read_line_limited, MAX_SIGNAL_LINE};
pub(super) enum SignalConnection {
    Tcp {
        label: String,
        stream: TcpStream,
        reader: io::BufReader<TcpStream>,
    },
    WebSocket {
        label: String,
        socket: Box<WebSocket<MaybeTlsStream<TcpStream>>>,
    },
}

impl SignalConnection {
    pub(super) fn connect(config: &str) -> io::Result<Self> {
        let endpoints = signal_endpoints(config);
        if endpoints.is_empty() {
            return Err(eio("Share-Server-Adresse fehlt"));
        }
        let mut errors = Vec::new();
        for endpoint in endpoints {
            match Self::connect_one(&endpoint) {
                Ok(connection) => return Ok(connection),
                Err(error) => errors.push(format!("{endpoint}: {error}")),
            }
        }
        Err(eio(format!(
            "keine Signaling-Verbindung moeglich ({})",
            errors.join("; ")
        )))
    }

    fn connect_one(endpoint: &str) -> io::Result<Self> {
        let normalized = normalize_signal_endpoint(endpoint);
        if normalized.starts_with("ws://") || normalized.starts_with("wss://") {
            return Self::connect_ws(&normalized);
        }
        if let Some(raw) = normalized.strip_prefix("tcp://") {
            return Self::connect_tcp(&normalize_tcp_addr(raw));
        }
        if normalized.contains("://") {
            return Err(eio("unbekanntes Share-Server-Schema"));
        }
        Self::connect_tcp(&normalize_tcp_addr(&normalized))
    }

    fn connect_tcp(addr: &str) -> io::Result<Self> {
        let stream = TcpStream::connect(addr)?;
        let _ = stream.set_nodelay(true);
        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
        let reader = io::BufReader::new(stream.try_clone()?);
        Ok(Self::Tcp {
            label: format!("tcp://{addr}"),
            stream,
            reader,
        })
    }

    fn connect_ws(url: &str) -> io::Result<Self> {
        let (mut socket, _) = ws_connect(url).map_err(ws_to_io)?;
        set_ws_timeout(socket.get_mut(), Duration::from_millis(500));
        Ok(Self::WebSocket {
            label: url.to_string(),
            socket: Box::new(socket),
        })
    }

    pub(super) fn label(&self) -> &str {
        match self {
            Self::Tcp { label, .. } | Self::WebSocket { label, .. } => label,
        }
    }

    fn send<T: serde::Serialize>(&mut self, msg: &T) -> io::Result<()> {
        match self {
            Self::Tcp { stream, .. } => {
                let mut line = serde_json::to_string(msg).map_err(eio)?;
                line.push('\n');
                stream.write_all(line.as_bytes())?;
                stream.flush()
            }
            Self::WebSocket { socket, .. } => {
                let text = serde_json::to_string(msg).map_err(eio)?;
                socket.send(Message::Text(text)).map_err(ws_to_io)?;
                socket.flush().map_err(ws_to_io)
            }
        }
    }

    pub(super) fn read_message(&mut self) -> io::Result<Option<String>> {
        match self {
            Self::Tcp { reader, .. } => {
                let mut line = String::new();
                match read_line_limited(reader, &mut line, MAX_SIGNAL_LINE) {
                    Ok(0) => Ok(None),
                    Ok(_) => Ok(Some(line)),
                    Err(error) => Err(error),
                }
            }
            Self::WebSocket { socket, .. } => loop {
                match socket.read() {
                    Ok(Message::Text(text)) => return Ok(Some(text)),
                    Ok(Message::Binary(bytes)) => {
                        return String::from_utf8(bytes).map(Some).map_err(eio);
                    }
                    Ok(Message::Ping(payload)) => {
                        socket.send(Message::Pong(payload)).map_err(ws_to_io)?;
                        socket.flush().map_err(ws_to_io)?;
                    }
                    Ok(Message::Pong(_)) => {}
                    Ok(Message::Close(_)) => return Ok(None),
                    Ok(_) => {}
                    Err(WsError::Io(error))
                        if error.kind() == io::ErrorKind::WouldBlock
                            || error.kind() == io::ErrorKind::TimedOut =>
                    {
                        return Err(error)
                    }
                    Err(WsError::ConnectionClosed | WsError::AlreadyClosed) => return Ok(None),
                    Err(error) => return Err(ws_to_io(error)),
                }
            },
        }
    }
}

pub(super) fn send_line<T: serde::Serialize>(
    stream: &mut SignalConnection,
    msg: &T,
) -> io::Result<()> {
    stream.send(msg)
}

pub(super) fn signal_endpoints(config: &str) -> Vec<String> {
    config
        .split([',', ';'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}

pub(super) fn normalize_signal_endpoint(endpoint: &str) -> String {
    let trimmed = endpoint.trim();
    if let Some(rest) = trimmed.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = trimmed.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        trimmed.to_string()
    }
}

pub(super) fn normalize_tcp_addr(addr: &str) -> String {
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

fn ws_to_io(error: WsError) -> io::Error {
    match error {
        WsError::Io(error) => error,
        other => eio(other),
    }
}
