use std::io::{self, BufReader, ErrorKind};
use std::net::TcpStream;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tungstenite::handshake::HandshakeError;
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{accept_with_config, Error as WsError, Message, WebSocket};

use super::limits::SourceKey;
use super::line::{read_line_limited, read_line_limited_until, MAX_JSON_LINE};
use super::rate_limits::InboundRateLimiter;
use super::registration_guard::RegistrationGuard;
use super::state::{register_client, State};
use super::tracked_direct;
use super::websocket_read_limit::WebSocketReadLimit;
use super::writer::QueuedMessage;
use super::{dispatch, send, In, Out, Writer};

const PRE_REGISTRATION_TIMEOUT: Duration = Duration::from_secs(10);
const HANDSHAKE_POLL_INTERVAL: Duration = Duration::from_millis(5);
const WS_WRITE_BUFFER_SIZE: usize = 128 * 1024;
const WS_FRAME_OVERHEAD: usize = 64;
const MAX_WS_WRITE_BUFFER: usize = WS_WRITE_BUFFER_SIZE + MAX_JSON_LINE + WS_FRAME_OVERHEAD;

type SignalingWebSocket = WebSocket<WebSocketReadLimit<TcpStream>>;

#[cfg(test)]
pub(super) fn handle(stream: TcpStream, state: Arc<Mutex<State>>) -> io::Result<()> {
    let source = SourceKey::from_socket(stream.peer_addr()?);
    handle_with_source(stream, state, source)
}

pub(super) fn handle_with_source(
    stream: TcpStream,
    state: Arc<Mutex<State>>,
    source: SourceKey,
) -> io::Result<()> {
    handle_until(
        stream,
        state,
        source,
        Instant::now() + PRE_REGISTRATION_TIMEOUT,
    )
}

#[cfg(test)]
pub(super) fn handle_with_timeout(
    stream: TcpStream,
    state: Arc<Mutex<State>>,
    timeout: Duration,
) -> io::Result<()> {
    let source = SourceKey::from_socket(stream.peer_addr()?);
    handle_until(stream, state, source, Instant::now() + timeout)
}

fn handle_until(
    stream: TcpStream,
    state: Arc<Mutex<State>>,
    source: SourceKey,
    registration_deadline: Instant,
) -> io::Result<()> {
    let mut inbound_rate = InboundRateLimiter::new();
    set_remaining_read_timeout(&stream, registration_deadline)?;
    let mut probe = [0u8; 3];
    let length = stream.peek(&mut probe)?;
    if length >= 1 && probe[0] == b'G' {
        return handle_websocket(
            stream,
            state,
            source,
            registration_deadline,
            &mut inbound_rate,
        );
    }
    handle_tcp(
        stream,
        state,
        source,
        registration_deadline,
        &mut inbound_rate,
    )
}

fn handle_tcp(
    stream: TcpStream,
    state: Arc<Mutex<State>>,
    source: SourceKey,
    registration_deadline: Instant,
    inbound_rate: &mut InboundRateLimiter,
) -> io::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if read_line_limited_until(&mut reader, &mut line, MAX_JSON_LINE, registration_deadline)? == 0 {
        return Ok(());
    }
    require_inbound_budget(inbound_rate, line.len())?;
    let hello: In = match serde_json::from_str(line.trim()) {
        Ok(hello) => hello,
        Err(_) => return Ok(()),
    };
    let writer = Writer::tcp(reader.get_ref().try_clone()?)?;
    reader
        .get_ref()
        .set_read_timeout(Some(Duration::from_secs(60)))?;
    let In::Hello {
        protocol_version,
        device_id,
        device_name: _,
        listen_port: _,
        lan: _,
        public_key: _,
        fingerprint: _,
        capabilities,
    } = hello
    else {
        send_server_error(&writer, "first message must be hello");
        return Ok(());
    };
    if protocol_version != 3 || device_id.trim().is_empty() {
        send_server_error(&writer, "unsupported hello");
        return Ok(());
    }

    let capabilities = tracked_direct::negotiate_capabilities(capabilities);
    let id = match register_client(
        &state,
        writer.clone(),
        source,
        device_id,
        capabilities.clone(),
    ) {
        Ok(id) => id,
        Err(error) => {
            send_server_error(&writer, error.message());
            return Ok(());
        }
    };
    let _registration = RegistrationGuard::new(id, &state);
    send(
        &writer,
        &Out::HelloOk {
            capabilities: tracked_direct::capability_list(&capabilities),
        },
    );

    loop {
        line.clear();
        match read_line_limited(&mut reader, &mut line, MAX_JSON_LINE) {
            Ok(0) => break,
            Err(error)
                if error.kind() == ErrorKind::WouldBlock || error.kind() == ErrorKind::TimedOut =>
            {
                break;
            }
            Err(_) => break,
            Ok(_) => {}
        }
        require_inbound_budget(inbound_rate, line.len())?;
        let message: In = match serde_json::from_str(line.trim()) {
            Ok(message) => message,
            Err(_) => continue,
        };
        dispatch(id, &writer, message, &state);
    }
    Ok(())
}

fn handle_websocket(
    stream: TcpStream,
    state: Arc<Mutex<State>>,
    source: SourceKey,
    registration_deadline: Instant,
    inbound_rate: &mut InboundRateLimiter,
) -> io::Result<()> {
    let (writer, outbound) = Writer::websocket(&stream)?;
    let config = WebSocketConfig {
        write_buffer_size: WS_WRITE_BUFFER_SIZE,
        max_write_buffer_size: MAX_WS_WRITE_BUFFER,
        max_message_size: Some(MAX_JSON_LINE),
        max_frame_size: Some(MAX_JSON_LINE),
        ..WebSocketConfig::default()
    };
    let mut websocket = accept_websocket_until(
        WebSocketReadLimit::new(stream),
        config,
        registration_deadline,
    )?;
    let hello = match read_websocket_json_until(
        &mut websocket,
        &outbound,
        registration_deadline,
        inbound_rate,
    ) {
        Ok(Some(message)) => message,
        Ok(None) => return Ok(()),
        Err(error) => return Err(error),
    };
    let In::Hello {
        protocol_version,
        device_id,
        device_name: _,
        listen_port: _,
        lan: _,
        public_key: _,
        fingerprint: _,
        capabilities,
    } = hello
    else {
        send_server_error(&writer, "first message must be hello");
        flush_websocket_out(&mut websocket, &outbound)?;
        return Ok(());
    };
    if protocol_version != 3 || device_id.trim().is_empty() {
        send_server_error(&writer, "unsupported hello");
        flush_websocket_out(&mut websocket, &outbound)?;
        return Ok(());
    }

    let capabilities = tracked_direct::negotiate_capabilities(capabilities);
    let id = match register_client(
        &state,
        writer.clone(),
        source,
        device_id,
        capabilities.clone(),
    ) {
        Ok(id) => id,
        Err(error) => {
            send_server_error(&writer, error.message());
            flush_websocket_out(&mut websocket, &outbound)?;
            return Ok(());
        }
    };
    let _registration = RegistrationGuard::new(id, &state);
    send(
        &writer,
        &Out::HelloOk {
            capabilities: tracked_direct::capability_list(&capabilities),
        },
    );

    websocket.get_mut().set_nonblocking(false)?;
    websocket
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(500)))?;
    let result = loop {
        if let Err(error) = flush_websocket_out(&mut websocket, &outbound) {
            break Err(error);
        }
        match read_websocket_json_until(
            &mut websocket,
            &outbound,
            Instant::now() + Duration::from_millis(500),
            inbound_rate,
        ) {
            Ok(Some(message)) => dispatch(id, &writer, message, &state),
            Ok(None) => break Ok(()),
            Err(error)
                if error.kind() == ErrorKind::WouldBlock || error.kind() == ErrorKind::TimedOut => {
            }
            Err(error) => break Err(error),
        }
    };
    writer.close();
    result
}

fn accept_websocket_until(
    stream: WebSocketReadLimit<TcpStream>,
    config: WebSocketConfig,
    deadline: Instant,
) -> io::Result<SignalingWebSocket> {
    stream.set_nonblocking(true)?;
    let mut handshake = match accept_with_config(stream, Some(config)) {
        Ok(websocket) => return Ok(websocket),
        Err(HandshakeError::Interrupted(handshake)) => handshake,
        Err(HandshakeError::Failure(error)) => return Err(websocket_to_io(error)),
    };
    loop {
        wait_for_handshake_poll(deadline)?;
        match handshake.handshake() {
            Ok(websocket) => return Ok(websocket),
            Err(HandshakeError::Interrupted(next)) => handshake = next,
            Err(HandshakeError::Failure(error)) => return Err(websocket_to_io(error)),
        }
    }
}

fn wait_for_handshake_poll(deadline: Instant) -> io::Result<()> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(pre_registration_timeout());
    }
    std::thread::sleep(remaining.min(HANDSHAKE_POLL_INTERVAL));
    if Instant::now() >= deadline {
        Err(pre_registration_timeout())
    } else {
        Ok(())
    }
}

fn send_server_error(writer: &Writer, message: &str) {
    send(
        writer,
        &Out::Error {
            scope: "server".into(),
            msg: message.to_string(),
        },
    );
}

fn flush_websocket_out(
    websocket: &mut SignalingWebSocket,
    receiver: &Receiver<QueuedMessage>,
) -> io::Result<()> {
    while let Ok(message) = receiver.try_recv() {
        let text = message.text()?.to_string();
        websocket
            .send(Message::Text(text))
            .map_err(websocket_to_io)?;
        websocket.flush().map_err(websocket_to_io)?;
    }
    Ok(())
}

fn read_websocket_json_until(
    websocket: &mut SignalingWebSocket,
    outbound: &Receiver<QueuedMessage>,
    deadline: Instant,
    inbound_rate: &mut InboundRateLimiter,
) -> io::Result<Option<In>> {
    loop {
        if Instant::now() >= deadline {
            return Err(pre_registration_timeout());
        }
        flush_websocket_out(websocket, outbound)?;
        match websocket.read() {
            Ok(Message::Text(text)) => {
                require_inbound_message_budget(inbound_rate)?;
                if let Some(message) = parse_websocket_json(text.as_bytes())? {
                    return Ok(Some(message));
                }
            }
            Ok(Message::Binary(bytes)) => {
                require_inbound_message_budget(inbound_rate)?;
                if let Some(message) = parse_websocket_json(&bytes)? {
                    return Ok(Some(message));
                }
            }
            Ok(Message::Ping(payload)) => {
                require_inbound_message_budget(inbound_rate)?;
                websocket
                    .send(Message::Pong(payload))
                    .map_err(websocket_to_io)?;
                websocket.flush().map_err(websocket_to_io)?;
            }
            Ok(Message::Pong(_)) => {
                require_inbound_message_budget(inbound_rate)?;
            }
            Ok(Message::Close(_)) => return Ok(None),
            Ok(_) => require_inbound_message_budget(inbound_rate)?,
            Err(WsError::Io(error))
                if error.kind() == ErrorKind::WouldBlock || error.kind() == ErrorKind::TimedOut =>
            {
                if Instant::now() >= deadline {
                    return Err(pre_registration_timeout());
                }
                std::thread::sleep(HANDSHAKE_POLL_INTERVAL);
            }
            Err(WsError::ConnectionClosed | WsError::AlreadyClosed) => return Ok(None),
            Err(error) => return Err(websocket_to_io(error)),
        }
    }
}

fn require_inbound_message_budget(limiter: &mut InboundRateLimiter) -> io::Result<()> {
    if limiter.try_consume_message() {
        Ok(())
    } else {
        Err(io::Error::new(
            ErrorKind::PermissionDenied,
            "signaling input message rate limit exceeded",
        ))
    }
}

fn require_inbound_budget(limiter: &mut InboundRateLimiter, bytes: usize) -> io::Result<()> {
    if limiter.try_consume(bytes) {
        Ok(())
    } else {
        Err(io::Error::new(
            ErrorKind::PermissionDenied,
            "signaling input rate limit exceeded",
        ))
    }
}

fn set_remaining_read_timeout(stream: &TcpStream, deadline: Instant) -> io::Result<()> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(pre_registration_timeout());
    }
    stream.set_read_timeout(Some(remaining))
}

fn pre_registration_timeout() -> io::Error {
    io::Error::new(ErrorKind::TimedOut, "pre-registration deadline expired")
}

fn parse_websocket_json(bytes: &[u8]) -> io::Result<Option<In>> {
    if bytes.len() > MAX_JSON_LINE {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "json line too large",
        ));
    }
    let text = std::str::from_utf8(bytes).map_err(io_other)?.trim();
    if text.is_empty() {
        return Ok(None);
    }
    Ok(serde_json::from_str(text).ok())
}

fn websocket_to_io(error: WsError) -> io::Error {
    match error {
        WsError::Io(error) => error,
        WsError::Capacity(error) => io::Error::new(ErrorKind::InvalidData, error.to_string()),
        other => io_other(other),
    }
}

fn io_other(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}
