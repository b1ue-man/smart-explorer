use std::io::{self, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::time::Duration;

use tungstenite::{
    client::IntoClientRequest, client_tls, stream::MaybeTlsStream, Error as WsError, Message,
    WebSocket,
};

use super::core::eio;
use super::line::{read_line_limited, MAX_SIGNAL_LINE};

const SIGNAL_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const SIGNAL_DNS_TIMEOUT: Duration = Duration::from_secs(10);
const SIGNAL_READ_POLL: Duration = Duration::from_millis(500);
const SIGNAL_WRITE_TIMEOUT: Duration = Duration::from_secs(10);
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
        let authority: tungstenite::http::uri::Authority = addr
            .parse()
            .map_err(|error| eio(format!("ungueltige Share-Server-Adresse: {error}")))?;
        let stream = connect_resolved(resolve_host(
            authority.host(),
            authority.port_u16().unwrap_or(51820),
        )?)?;
        let _ = stream.set_nodelay(true);
        set_tcp_timeouts(&stream, SIGNAL_READ_POLL, SIGNAL_WRITE_TIMEOUT);
        let reader = io::BufReader::new(stream.try_clone()?);
        Ok(Self::Tcp {
            label: format!("tcp://{addr}"),
            stream,
            reader,
        })
    }

    fn connect_ws(url: &str) -> io::Result<Self> {
        let request = url.into_client_request().map_err(ws_to_io)?;
        let uri = request.uri();
        let host = uri
            .host()
            .ok_or_else(|| eio("Share-WebSocket-Host fehlt"))?;
        let host = host
            .strip_prefix('[')
            .and_then(|value| value.strip_suffix(']'))
            .unwrap_or(host);
        let port = uri.port_u16().unwrap_or(match uri.scheme_str() {
            Some("ws") => 80,
            Some("wss") => 443,
            _ => return Err(eio("unbekanntes Share-WebSocket-Schema")),
        });
        let stream = connect_resolved(resolve_host(host, port)?)?;
        let _ = stream.set_nodelay(true);
        // These socket deadlines also bound the TLS and WebSocket handshakes.
        set_tcp_timeouts(&stream, SIGNAL_CONNECT_TIMEOUT, SIGNAL_CONNECT_TIMEOUT);
        let (mut socket, _) = client_tls(request, stream).map_err(eio)?;
        set_ws_timeouts(socket.get_mut(), SIGNAL_READ_POLL, SIGNAL_WRITE_TIMEOUT);
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

    #[cfg(test)]
    pub(super) fn from_test_tcp(stream: TcpStream) -> io::Result<Self> {
        set_tcp_timeouts(&stream, SIGNAL_READ_POLL, SIGNAL_WRITE_TIMEOUT);
        let reader = io::BufReader::new(stream.try_clone()?);
        Ok(Self::Tcp {
            label: "tcp://test".into(),
            stream,
            reader,
        })
    }

    #[cfg(test)]
    pub(super) fn shutdown_test_transport(&mut self) -> io::Result<()> {
        match self {
            Self::Tcp { stream, .. } => stream.shutdown(std::net::Shutdown::Both),
            Self::WebSocket { .. } => Err(eio("test shutdown requires raw TCP")),
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

fn connect_resolved(addresses: impl IntoIterator<Item = SocketAddr>) -> io::Result<TcpStream> {
    let mut last_error = None;
    for address in addresses {
        match TcpStream::connect_timeout(&address, SIGNAL_CONNECT_TIMEOUT) {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| eio("Share-Server hat keine erreichbare Adresse")))
}

fn resolve_host(host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
    let host = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![SocketAddr::new(ip, port)]);
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(eio)?;
    let resolver = hickory_resolver::Resolver::builder_tokio()
        .map_err(eio)?
        .build()
        .map_err(eio)?;
    let lookup = runtime.block_on(async {
        tokio::time::timeout(SIGNAL_DNS_TIMEOUT, resolver.lookup_ip(host))
            .await
            .map_err(|_| {
                io::Error::new(io::ErrorKind::TimedOut, "Share-Server DNS lookup timed out")
            })?
            .map_err(eio)
    })?;
    let addresses: Vec<_> = lookup.iter().map(|ip| SocketAddr::new(ip, port)).collect();
    if addresses.is_empty() {
        Err(eio("Share-Server DNS lieferte keine Adresse"))
    } else {
        Ok(addresses)
    }
}

fn set_tcp_timeouts(stream: &TcpStream, read: Duration, write: Duration) {
    let _ = stream.set_read_timeout(Some(read));
    let _ = stream.set_write_timeout(Some(write));
}

fn set_ws_timeouts(stream: &mut MaybeTlsStream<TcpStream>, read: Duration, write: Duration) {
    match stream {
        MaybeTlsStream::Plain(tcp) => {
            set_tcp_timeouts(tcp, read, write);
        }
        MaybeTlsStream::Rustls(tls) => {
            set_tcp_timeouts(&tls.sock, read, write);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_signal_hosts_bypass_dns_and_preserve_port() {
        assert_eq!(
            resolve_host("127.0.0.1", 51820).unwrap(),
            vec!["127.0.0.1:51820".parse().unwrap()]
        );
        assert_eq!(
            resolve_host("[::1]", 51821).unwrap(),
            vec!["[::1]:51821".parse().unwrap()]
        );
        assert!(!SIGNAL_DNS_TIMEOUT.is_zero());
        assert!(!SIGNAL_CONNECT_TIMEOUT.is_zero());
        assert!(SIGNAL_WRITE_TIMEOUT > SIGNAL_READ_POLL);
    }
}
