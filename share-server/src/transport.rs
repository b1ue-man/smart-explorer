use std::io::{self, BufReader, ErrorKind};
use std::net::TcpStream;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tungstenite::handshake::HandshakeError;
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{accept_with_config, Error as WsError, Message, WebSocket};

use super::hello_session::HelloSession;
use super::idle::{self, SignalTiming};
use super::signal_stream::SignalStream;
use super::limits::SourceKey;
use super::line::{read_line_limited_until, MAX_JSON_LINE};
use super::rate_limits::InboundRateLimiter;
use super::registration_guard::RegistrationGuard;
use super::state::{register_client_with_identity, State};
use super::tracked_direct;
use super::websocket_read_limit::WebSocketReadLimit;
use super::writer::QueuedMessage;
use super::{dispatch, send, In, Out, Writer};

#[path = "transport_serve.rs"]
mod serve;
use serve::{serve_tcp, serve_websocket};

const PRE_REGISTRATION_TIMEOUT: Duration = Duration::from_secs(10);
const HANDSHAKE_POLL_INTERVAL: Duration = Duration::from_millis(5);
const WS_WRITE_BUFFER_SIZE: usize = 128 * 1024;
const WS_FRAME_OVERHEAD: usize = 64;
const MAX_WS_WRITE_BUFFER: usize = WS_WRITE_BUFFER_SIZE + MAX_JSON_LINE + WS_FRAME_OVERHEAD;

type SignalingWebSocket = WebSocket<WebSocketReadLimit<SignalStream>>;

#[cfg(test)]
pub(super) fn handle(stream: TcpStream, state: Arc<Mutex<State>>) -> io::Result<()> {
    handle_with_timing(stream, state, &SignalTiming::default())
}

#[cfg(test)]
pub(super) fn handle_with_timing(
    stream: TcpStream,
    state: Arc<Mutex<State>>,
    timing: &SignalTiming,
) -> io::Result<()> {
    let source = SourceKey::from_socket(stream.peer_addr()?);
    handle_with_source(stream, state, source, timing)
}

pub(super) fn handle_with_source(
    stream: TcpStream,
    state: Arc<Mutex<State>>,
    source: SourceKey,
    timing: &SignalTiming,
) -> io::Result<()> {
    let registration_deadline = Instant::now() + PRE_REGISTRATION_TIMEOUT;
    handle_until(stream, state, source, registration_deadline, timing, None, true)
}

#[cfg(test)]
pub(super) fn handle_with_timeout(
    stream: TcpStream,
    state: Arc<Mutex<State>>,
    timeout: Duration,
) -> io::Result<()> {
    let source = SourceKey::from_socket(stream.peer_addr()?);
    let timing = SignalTiming::default();
    handle_until(stream, state, source, Instant::now() + timeout, &timing, None, true)
}

pub(super) fn handle_with_security(
    stream: TcpStream, state: Arc<Mutex<State>>, source: SourceKey, timing: &SignalTiming,
    tls: Option<Arc<rustls::ServerConfig>>, allow_plaintext: bool,
) -> io::Result<()> {
    handle_until(stream, state, source, Instant::now() + PRE_REGISTRATION_TIMEOUT,
        timing, tls, allow_plaintext)
}

fn handle_until(
    stream: TcpStream,
    state: Arc<Mutex<State>>,
    source: SourceKey,
    registration_deadline: Instant,
    timing: &SignalTiming,
    tls: Option<Arc<rustls::ServerConfig>>,
    allow_plaintext: bool,
) -> io::Result<()> {
    let mut inbound_rate = InboundRateLimiter::new();
    set_remaining_read_timeout(&stream, registration_deadline)?;
    let mut probe = [0u8; 3];
    let length = stream.peek(&mut probe)?;
    if length == 0 { return Ok(()); }
    if probe[0] == 0x16 {
        let config = tls.ok_or_else(|| io::Error::new(ErrorKind::PermissionDenied, "TLS is not configured"))?;
        return handle_websocket(SignalStream::tls(stream, config)?, state, source,
            registration_deadline, timing, &mut inbound_rate);
    }
    if !allow_plaintext {
        return Err(io::Error::new(ErrorKind::PermissionDenied, "plaintext signaling is disabled"));
    }
    if probe[0] == b'G' {
        return handle_websocket(
            SignalStream::plain(stream)?,
            state,
            source,
            registration_deadline,
            timing,
            &mut inbound_rate,
        );
    }
    handle_tcp(
        stream,
        state,
        source,
        registration_deadline,
        timing,
        &mut inbound_rate,
    )
}

fn handle_tcp(
    stream: TcpStream,
    state: Arc<Mutex<State>>,
    source: SourceKey,
    registration_deadline: Instant,
    timing: &SignalTiming,
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
    let mut hello = match HelloSession::begin(hello) {
        Ok(hello) => hello,
        Err(error) => { send_server_error(&writer, &error.to_string()); return Ok(()); }
    };
    if let Some(challenge) = hello.challenge() {
        send(&writer, &challenge);
        if read_line_limited_until(&mut reader, &mut line, MAX_JSON_LINE, registration_deadline)? == 0 {
            return Ok(());
        }
        require_inbound_budget(inbound_rate, line.len())?;
        let auth = serde_json::from_str(line.trim()).map_err(io_other)?;
        if hello.answer(auth).is_err() {
            send_server_error(&writer, crate::login::LOGIN_FAILED);
            return Ok(());
        }
    }
    let Some(id) = finish_registration(hello, &writer, source, &state, timing)? else { return Ok(()); };
    let _registration = RegistrationGuard::new(id, &state);

    serve_tcp(&mut reader, id, &writer, &state, timing, inbound_rate)
}

fn handle_websocket(
    mut stream: SignalStream,
    state: Arc<Mutex<State>>,
    source: SourceKey,
    registration_deadline: Instant,
    timing: &SignalTiming,
    inbound_rate: &mut InboundRateLimiter,
) -> io::Result<()> {
    stream.set_write_deadline(Some(registration_deadline));
    let (writer, outbound) = Writer::websocket(stream.socket())?;
    writer.set_wake(stream.notify());
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
        &mut false,
    ) {
        Ok(Some(message)) => message,
        Ok(None) => return Ok(()),
        Err(error) => return Err(error),
    };
    let mut hello = match HelloSession::begin(hello) {
        Ok(hello) => hello,
        Err(error) => {
            send_server_error(&writer, &error.to_string());
            flush_websocket_out(&mut websocket, &outbound)?;
            return Ok(());
        }
    };
    if let Some(challenge) = hello.challenge() {
        send(&writer, &challenge);
        let Some(auth) = read_websocket_json_until(&mut websocket, &outbound,
            registration_deadline, inbound_rate, &mut false)? else { return Ok(()); };
        if hello.answer(auth).is_err() {
            send_server_error(&writer, crate::login::LOGIN_FAILED);
            flush_websocket_out(&mut websocket, &outbound)?;
            return Ok(());
        }
    }
    let Some(id) = finish_registration(hello, &writer, source, &state, timing)? else {
        flush_websocket_out(&mut websocket, &outbound)?;
        return Ok(());
    };
    let _registration = RegistrationGuard::new(id, &state);

    websocket.get_mut().set_write_deadline(None);
    let result = serve_websocket(
        &mut websocket,
        &outbound,
        &writer,
        id,
        &state,
        timing,
        inbound_rate,
    );
    writer.close();
    result
}

fn finish_registration(
    hello: HelloSession, writer: &Writer, source: SourceKey,
    state: &Arc<Mutex<State>>, timing: &SignalTiming,
) -> io::Result<Option<u64>> {
    if hello.capabilities.contains(idle::CAPABILITY) { writer.enable_idle(timing); }
    let id = match register_client_with_identity(state, writer.clone(), source,
        hello.device_id, hello.capabilities.clone(), hello.identity) {
        Ok(id) => id,
        Err(error) => { send_server_error(writer, error.message()); return Ok(None); }
    };
    send(writer, &Out::HelloOk { capabilities: tracked_direct::capability_list(&hello.capabilities) });
    Ok(Some(id))
}

fn accept_websocket_until(
    stream: WebSocketReadLimit<SignalStream>,
    config: WebSocketConfig,
    deadline: Instant,
) -> io::Result<SignalingWebSocket> {
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

/// Reads until one JSON message, close, or `deadline`. `received` reports any
/// frame, including control and unparsable frames.
fn read_websocket_json_until(
    websocket: &mut SignalingWebSocket,
    outbound: &Receiver<QueuedMessage>,
    deadline: Instant,
    inbound_rate: &mut InboundRateLimiter,
    received: &mut bool,
) -> io::Result<Option<In>> {
    loop {
        if Instant::now() >= deadline {
            return Err(pre_registration_timeout());
        }
        flush_websocket_out(websocket, outbound)?;
        let frame = websocket.read();
        *received |= frame.is_ok();
        match frame {
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
                websocket.get_ref().wait_readable(deadline)?;
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
