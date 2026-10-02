use std::io::{self, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::time::Duration;

use tungstenite::{stream::MaybeTlsStream, Error as WsError, Message, WebSocket};

use super::core::eio;
use super::line::{SignalLineReader, MAX_SIGNAL_LINE};
use super::server_address::{SignalEndpoint, SignalScheme, SignalServerConfig};

#[path = "signal_connection_tls.rs"]
pub(super) mod tls;

const SIGNAL_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const SIGNAL_DNS_TIMEOUT: Duration = Duration::from_secs(10);
const SIGNAL_READ_POLL: Duration = Duration::from_millis(500);
const SIGNAL_WRITE_TIMEOUT: Duration = Duration::from_secs(10);
/// Messages one drain hands over at most; the rest follows at once.
const MAX_DRAINED_MESSAGES: usize = 256;
/// Bytes one drain buffers at most before it hands them over (S56).
const MAX_DRAINED_BYTES: usize = 4 * 1024 * 1024;

pub(super) struct SignalConnection {
    label: String,
    transport: Transport,
    /// TLS protects the connection (`wss`); otherwise it is plaintext.
    encrypted: bool,
    /// The server confirmed this device's key login (`key_login_v1`);
    /// relation access proofs are sent from then on.
    key_login: bool,
    /// Read timeout the socket keeps between this side's own reads: the
    /// short poll until a readiness watcher (`signal_readiness`) waits on
    /// the socket, then the watcher's long wait.
    resting_read_timeout: Duration,
}

enum Transport {
    Tcp {
        stream: TcpStream,
        reader: io::BufReader<TcpStream>,
        decoder: SignalLineReader,
    },
    WebSocket {
        socket: Box<WebSocket<MaybeTlsStream<TcpStream>>>,
    },
}

/// Messages a drain read without waiting, and how the drain ended.
#[derive(Debug, Default)]
pub(super) struct Drained {
    pub(super) messages: Vec<String>,
    /// The server closed the connection.
    pub(super) closed: bool,
    /// More data is already buffered; drain again before waiting.
    pub(super) more: bool,
    /// The transport failed after `messages`.
    pub(super) error: Option<io::Error>,
}

impl SignalConnection {
    /// Connects to the first reachable endpoint of a stored server address.
    /// With a TLS endpoint configured, plaintext endpoints are never tried.
    pub(super) fn connect(config: &str) -> io::Result<Self> {
        let config = SignalServerConfig::parse_stored(config).map_err(eio)?;
        let endpoints = config.active_endpoints();
        if endpoints.is_empty() {
            return Err(eio("Share-Server-Adresse fehlt"));
        }
        let mut errors = Vec::new();
        for endpoint in endpoints {
            match Self::connect_one(endpoint) {
                Ok(connection) => return Ok(connection),
                Err(error) => errors.push(format!("{}: {error}", endpoint.label())),
            }
        }
        Err(eio(format!(
            "keine Signaling-Verbindung moeglich ({})",
            errors.join("; ")
        )))
    }

    fn connect_one(endpoint: &SignalEndpoint) -> io::Result<Self> {
        let stream = connect_resolved(resolve_host(endpoint.host(), endpoint.port())?)?;
        let _ = stream.set_nodelay(true);
        if endpoint.scheme() == SignalScheme::Tcp {
            return Self::from_tcp(endpoint.label(), stream);
        }
        // These socket deadlines also bound the TLS and WebSocket handshakes.
        set_tcp_timeouts(&stream, SIGNAL_CONNECT_TIMEOUT, SIGNAL_CONNECT_TIMEOUT);
        let mut socket = tls::handshake(endpoint, stream)?;
        set_ws_timeouts(socket.get_mut(), SIGNAL_READ_POLL, SIGNAL_WRITE_TIMEOUT);
        Ok(Self::from_websocket(
            endpoint.label(),
            socket,
            endpoint.is_encrypted(),
        ))
    }

    fn from_tcp(label: String, stream: TcpStream) -> io::Result<Self> {
        set_tcp_timeouts(&stream, SIGNAL_READ_POLL, SIGNAL_WRITE_TIMEOUT);
        let reader = io::BufReader::new(stream.try_clone()?);
        Ok(Self {
            label,
            transport: Transport::Tcp {
                stream,
                reader,
                decoder: SignalLineReader::default(),
            },
            encrypted: false,
            key_login: false,
            resting_read_timeout: SIGNAL_READ_POLL,
        })
    }

    fn from_websocket(
        label: String,
        socket: WebSocket<MaybeTlsStream<TcpStream>>,
        encrypted: bool,
    ) -> Self {
        Self {
            label,
            transport: Transport::WebSocket {
                socket: Box::new(socket),
            },
            encrypted,
            key_login: false,
            resting_read_timeout: SIGNAL_READ_POLL,
        }
    }

    pub(super) fn label(&self) -> &str {
        &self.label
    }

    pub(super) fn encrypted(&self) -> bool {
        self.encrypted
    }

    pub(super) fn key_login(&self) -> bool {
        self.key_login
    }

    /// The server confirmed the key login of this connection.
    pub(super) fn confirm_key_login(&mut self) {
        self.key_login = true;
    }

    #[cfg(test)]
    pub(super) fn from_test_tcp(stream: TcpStream) -> io::Result<Self> {
        Self::from_tcp("tcp://test".into(), stream)
    }

    /// A client WebSocket over an already upgraded test socket.
    #[cfg(test)]
    pub(super) fn from_test_websocket(mut socket: WebSocket<MaybeTlsStream<TcpStream>>) -> Self {
        set_ws_timeouts(socket.get_mut(), SIGNAL_READ_POLL, SIGNAL_WRITE_TIMEOUT);
        Self::from_websocket("ws://test".into(), socket, false)
    }

    #[cfg(test)]
    pub(super) fn shutdown_test_transport(&mut self) -> io::Result<()> {
        match &self.transport {
            Transport::Tcp { stream, .. } => stream.shutdown(std::net::Shutdown::Both),
            Transport::WebSocket { .. } => Err(eio("test shutdown requires raw TCP")),
        }
    }

    /// The TCP socket below the transport, if this transport exposes one.
    fn raw_socket(&self) -> Option<&TcpStream> {
        match &self.transport {
            Transport::Tcp { stream, .. } => Some(stream),
            Transport::WebSocket { socket } => match socket.get_ref() {
                MaybeTlsStream::Plain(tcp) => Some(tcp),
                MaybeTlsStream::Rustls(tls) => Some(&tls.sock),
                #[allow(unreachable_patterns)]
                _ => None,
            },
        }
    }

    /// A second handle of the socket for a readiness watcher.
    pub(super) fn watch_handle(&self) -> io::Result<TcpStream> {
        self.raw_socket()
            .ok_or_else(|| eio("Signal-Transport bietet keinen Socket zum Warten"))?
            .try_clone()
    }

    pub(super) fn resting_read_timeout(&self) -> Duration {
        self.resting_read_timeout
    }

    /// Sets the read timeout the socket keeps between this side's own reads.
    pub(super) fn rest_reads_for(&mut self, timeout: Duration) {
        self.resting_read_timeout = timeout;
        self.apply_read_timeout(timeout);
    }

    fn apply_read_timeout(&mut self, timeout: Duration) {
        match &mut self.transport {
            Transport::Tcp { stream, .. } => {
                let _ = stream.set_read_timeout(Some(timeout));
            }
            Transport::WebSocket { socket } => {
                set_ws_timeouts(socket.get_mut(), timeout, SIGNAL_WRITE_TIMEOUT);
            }
        }
    }

    fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        match &self.transport {
            Transport::Tcp { stream, reader, .. } => {
                stream.set_nonblocking(nonblocking)?;
                reader.get_ref().set_nonblocking(nonblocking)
            }
            Transport::WebSocket { .. } => self
                .raw_socket()
                .ok_or_else(|| eio("Signal-Transport bietet keinen Socket"))?
                .set_nonblocking(nonblocking),
        }
    }

    fn send<T: serde::Serialize>(&mut self, msg: &T) -> io::Result<()> {
        // A TLS write may read the socket; while a watcher waits with its
        // long timeout, bound such reads like the handshake's poll.
        let guard_reads = self.resting_read_timeout > SIGNAL_READ_POLL;
        if guard_reads {
            self.apply_read_timeout(SIGNAL_READ_POLL);
        }
        let result = self.write_message(msg);
        if guard_reads {
            let resting = self.resting_read_timeout;
            self.apply_read_timeout(resting);
        }
        result
    }

    fn write_message<T: serde::Serialize>(&mut self, msg: &T) -> io::Result<()> {
        match &mut self.transport {
            Transport::Tcp { stream, .. } => {
                let mut line = serde_json::to_string(msg).map_err(eio)?;
                line.push('\n');
                stream.write_all(line.as_bytes())?;
                stream.flush()
            }
            Transport::WebSocket { socket } => {
                let text = serde_json::to_string(msg).map_err(eio)?;
                socket.send(Message::Text(text)).map_err(ws_to_io)?;
                socket.flush().map_err(ws_to_io)
            }
        }
    }

    pub(super) fn read_message(&mut self) -> io::Result<Option<String>> {
        match &mut self.transport {
            Transport::Tcp {
                reader, decoder, ..
            } => decoder.read(reader, MAX_SIGNAL_LINE),
            Transport::WebSocket { socket } => loop {
                match socket.read() {
                    Ok(Message::Text(text)) => return Ok(Some(text)),
                    Ok(Message::Binary(bytes)) => {
                        return String::from_utf8(bytes).map(Some).map_err(eio);
                    }
                    Ok(Message::Ping(payload)) => {
                        match socket.send(Message::Pong(payload)) {
                            Ok(()) => {}
                            // Queued; the next read or write flushes it.
                            Err(WsError::Io(error))
                                if error.kind() == io::ErrorKind::WouldBlock => {}
                            Err(error) => return Err(ws_to_io(error)),
                        }
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

    /// Reads every message that is available without waiting. A partial
    /// line or frame stays buffered for the next drain.
    pub(super) fn drain_messages(&mut self) -> io::Result<Drained> {
        self.set_nonblocking(true)?;
        let mut drained = Drained::default();
        let mut drained_bytes = 0usize;
        loop {
            match self.read_message() {
                Ok(Some(message)) => {
                    drained_bytes = drained_bytes.saturating_add(message.len());
                    drained.messages.push(message);
                    if drained.messages.len() >= MAX_DRAINED_MESSAGES
                        || drained_bytes >= MAX_DRAINED_BYTES
                    {
                        drained.more = true;
                        break;
                    }
                }
                Ok(None) => {
                    drained.closed = true;
                    break;
                }
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock
                        || error.kind() == io::ErrorKind::TimedOut =>
                {
                    break
                }
                Err(error) => {
                    drained.error = Some(error);
                    break;
                }
            }
        }
        self.set_nonblocking(false)?;
        Ok(drained)
    }
}

pub(super) fn send_line<T: serde::Serialize>(
    stream: &mut SignalConnection,
    msg: &T,
) -> io::Result<()> {
    stream.send(msg)
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
