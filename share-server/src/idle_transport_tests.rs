//! Liveness timing on both transports with an injected clock (contract V1,
//! critic findings K5/K15/K16): idle keepalive and reply window on TCP and
//! WebSocket; clients that are not idle close after 60 s of silence; partial
//! TCP lines survive read timeouts.

use std::io::{self, BufRead, BufReader, ErrorKind, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::Value;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Error as WsError, Message, WebSocket};

use super::idle::{
    parse_keepalive, Keepalive, SignalClock, SignalTiming, TestClock, ACTIVE_READ_WINDOW,
    CAPABILITY,
};
use super::signal_session::{SignalSession, Step};
use super::state::{lock_state, State};
use super::transport::handle_with_timing;
use super::Writer;

const START_UNIX: i64 = 1_800_000_000;
const SECOND: Duration = Duration::from_secs(1);
const REPLY: Duration = Duration::from_secs(2);
const QUIET: Duration = Duration::from_millis(100);

type Server = JoinHandle<io::Result<()>>;

fn spawn_server(timing: SignalTiming) -> (SocketAddr, Arc<Mutex<State>>, Server) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let state = Arc::new(Mutex::new(State::default()));
    let server_state = Arc::clone(&state);
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        handle_with_timing(stream, server_state, &timing)
    });
    (address, state, server)
}

fn hello(capabilities: &[&str]) -> String {
    serde_json::json!({
        "t": "hello", "protocol_version": 3, "device_id": "phone",
        "device_name": "Phone", "listen_port": 0, "lan": [],
        "public_key": "pk", "fingerprint": "fp", "capabilities": capabilities,
    })
    .to_string()
}

struct TcpClient {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
}

impl TcpClient {
    fn connect(address: SocketAddr, capabilities: &[&str]) -> (Self, Value) {
        let stream = TcpStream::connect(address).unwrap();
        let reader = BufReader::new(stream.try_clone().unwrap());
        let mut client = Self { stream, reader };
        client.send(&hello(capabilities));
        let hello_ok = client.recv();
        assert_eq!(hello_ok["t"], "hello_ok");
        (client, hello_ok)
    }

    fn send(&mut self, line: &str) {
        self.send_raw(&format!("{line}\n"));
    }

    fn send_raw(&mut self, bytes: &str) {
        self.stream.write_all(bytes.as_bytes()).unwrap();
        self.stream.flush().unwrap();
    }

    fn recv(&mut self) -> Value {
        self.stream.set_read_timeout(Some(REPLY)).unwrap();
        let mut line = String::new();
        assert!(self.reader.read_line(&mut line).unwrap() > 0, "closed");
        serde_json::from_str(line.trim()).unwrap()
    }

    fn assert_quiet(&mut self) {
        self.stream.set_read_timeout(Some(QUIET)).unwrap();
        let error = self.reader.fill_buf().map(<[u8]>::to_vec).unwrap_err();
        assert!(
            matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut),
            "{error}"
        );
    }

    fn assert_closed(&mut self) {
        self.stream.set_read_timeout(Some(REPLY)).unwrap();
        let mut line = String::new();
        assert_eq!(self.reader.read_line(&mut line).unwrap(), 0, "{line}");
    }
}

type Ws = WebSocket<MaybeTlsStream<TcpStream>>;

fn ws_connect(address: SocketAddr, capabilities: &[&str]) -> Ws {
    let (mut websocket, _) = tungstenite::connect(format!("ws://{address}/se-share")).unwrap();
    websocket.send(Message::Text(hello(capabilities))).unwrap();
    assert_eq!(ws_recv(&mut websocket)["t"], "hello_ok");
    websocket
}

fn ws_timeout(websocket: &Ws, timeout: Duration) {
    if let MaybeTlsStream::Plain(stream) = websocket.get_ref() {
        stream.set_read_timeout(Some(timeout)).unwrap();
    }
}

fn ws_recv(websocket: &mut Ws) -> Value {
    ws_timeout(websocket, REPLY);
    loop {
        match websocket.read().unwrap() {
            Message::Text(text) => return serde_json::from_str(&text).unwrap(),
            Message::Close(_) => panic!("closed"),
            _ => {}
        }
    }
}

fn ws_assert_quiet(websocket: &mut Ws) {
    ws_timeout(websocket, QUIET);
    match websocket.read() {
        Err(WsError::Io(error))
            if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
        other => panic!("expected silence, got {other:?}"),
    }
}

fn ws_assert_closed(websocket: &mut Ws) {
    ws_timeout(websocket, REPLY);
    loop {
        match websocket.read() {
            Ok(Message::Close(_)) => return,
            Ok(other) => panic!("unexpected {other:?}"),
            Err(WsError::Io(error))
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
            {
                panic!("server kept the connection open")
            }
            Err(_) => return,
        }
    }
}

fn ws_close(mut websocket: Ws, server: Server, state: &Arc<Mutex<State>>) {
    websocket.close(None).unwrap();
    finish(server, state);
}

fn finish(server: Server, state: &Arc<Mutex<State>>) {
    server.join().unwrap().unwrap();
    assert!(lock_state(state).clients.is_empty());
}

#[test]
fn android_background_task_session_timing_follows_the_contract() {
    let clock = TestClock::new(START_UNIX);
    let (writer, receiver) = Writer::test_raw_channel(16);
    writer.enable_idle(&clock.timing(Keepalive::default()));
    let tags = || -> Vec<Value> {
        receiver
            .try_iter()
            .map(|message| serde_json::from_slice::<Value>(message.json()).unwrap()["t"].clone())
            .collect()
    };

    // Not idle on raw TCP: 60 s after the last inbound data.
    let start = clock.now();
    let mut session = SignalSession::new(start, Some(ACTIVE_READ_WINDOW));
    assert_eq!(
        session.poll(&writer, start),
        Step::Wait(Some(start + 60 * SECOND))
    );
    clock.advance(59 * SECOND);
    session.inbound(clock.now());
    clock.advance(59 * SECOND);
    assert!(matches!(session.poll(&writer, clock.now()), Step::Wait(_)));
    clock.advance(SECOND);
    assert_eq!(session.poll(&writer, clock.now()), Step::Close);

    // WebSocket follows the same active liveness deadline.
    let mut session = SignalSession::new(clock.now(), Some(ACTIVE_READ_WINDOW));
    clock.advance(60 * SECOND);
    assert_eq!(session.poll(&writer, clock.now()), Step::Close);

    // Idle: fixed K cadence regardless of inbound traffic, 60 s to answer.
    let start = clock.now();
    let mut session = SignalSession::new(start, None);
    writer.set_idle(true, None);
    assert_eq!(tags(), ["idle_ack"]);
    let tick = start + 180 * SECOND;
    clock.advance(100 * SECOND);
    session.inbound(clock.now());
    assert_eq!(session.poll(&writer, clock.now()), Step::Wait(Some(tick)));
    clock.advance(80 * SECOND);
    assert_eq!(
        session.poll(&writer, clock.now()),
        Step::Wait(Some(tick + 60 * SECOND))
    );
    assert_eq!(tags(), ["keepalive"]);
    clock.advance(30 * SECOND);
    session.inbound(clock.now());
    assert_eq!(
        session.poll(&writer, clock.now()),
        Step::Wait(Some(tick + 180 * SECOND))
    );
    clock.advance(150 * SECOND);
    assert!(matches!(session.poll(&writer, clock.now()), Step::Wait(_)));
    assert_eq!(tags(), ["keepalive"]);
    clock.advance(59 * SECOND);
    assert!(matches!(session.poll(&writer, clock.now()), Step::Wait(_)));
    clock.advance(SECOND);
    assert_eq!(session.poll(&writer, clock.now()), Step::Close);
}

#[test]
fn android_background_task_tcp_idle_client_gets_keepalive_and_must_answer() {
    let clock = TestClock::new(START_UNIX);
    let (address, state, server) = spawn_server(clock.timing(Keepalive::default()));
    let (mut client, hello_ok) = TcpClient::connect(address, &[CAPABILITY]);
    assert_eq!(hello_ok["capabilities"], serde_json::json!([CAPABILITY]));
    client.send(r#"{"t":"set_idle","idle":true}"#);
    let ack = client.recv();
    assert_eq!(
        (ack["t"].as_str(), ack["keepalive_secs"].as_u64()),
        (Some("idle_ack"), Some(180))
    );

    clock.advance(179 * SECOND);
    client.assert_quiet();
    clock.advance(SECOND);
    assert_eq!(client.recv()["t"], "keepalive");
    clock.advance(59 * SECOND);
    client.send(r#"{"t":"keepalive_ack"}"#);
    // The pong proves the server read the answer before the clock moves on.
    client.send(r#"{"t":"heartbeat"}"#);
    assert_eq!(client.recv()["t"], "pong");
    // Silent until the next keepalive, the idle client stays connected.
    clock.advance(120 * SECOND);
    client.assert_quiet();
    clock.advance(SECOND);
    assert_eq!(client.recv()["t"], "keepalive");
    clock.advance(59 * SECOND);
    client.assert_quiet();
    clock.advance(SECOND);
    client.assert_closed();
    finish(server, &state);
}

#[test]
fn android_background_task_websocket_idle_client_gets_keepalive_and_must_answer() {
    let clock = TestClock::new(START_UNIX);
    let (address, state, server) = spawn_server(clock.timing(Keepalive::clamped(60)));
    let mut websocket = ws_connect(address, &[CAPABILITY]);
    websocket
        .send(Message::Text(
            r#"{"t":"set_idle","idle":true,"keepalive_secs":45}"#.into(),
        ))
        .unwrap();
    let ack = ws_recv(&mut websocket);
    assert_eq!(
        (ack["t"].as_str(), ack["keepalive_secs"].as_u64()),
        (Some("idle_ack"), Some(45))
    );

    clock.advance(44 * SECOND);
    ws_assert_quiet(&mut websocket);
    clock.advance(SECOND);
    assert_eq!(ws_recv(&mut websocket)["t"], "keepalive");
    clock.advance(30 * SECOND);
    websocket
        .send(Message::Text(r#"{"t":"keepalive_ack"}"#.into()))
        .unwrap();
    // The pong proves the server read the answer before the clock moves on.
    websocket
        .send(Message::Text(r#"{"t":"heartbeat"}"#.into()))
        .unwrap();
    assert_eq!(ws_recv(&mut websocket)["t"], "pong");
    clock.advance(15 * SECOND);
    assert_eq!(ws_recv(&mut websocket)["t"], "keepalive");
    // Unanswered: the next tick still runs, the reply window closes at +60 s.
    clock.advance(45 * SECOND);
    assert_eq!(ws_recv(&mut websocket)["t"], "keepalive");
    clock.advance(14 * SECOND);
    ws_assert_quiet(&mut websocket);
    clock.advance(SECOND);
    ws_assert_closed(&mut websocket);
    finish(server, &state);
}

#[test]
fn android_background_task_tcp_legacy_client_keeps_window_and_partial_lines() {
    let clock = TestClock::new(START_UNIX);
    let (address, state, server) = spawn_server(clock.timing(Keepalive::default()));
    let (mut client, hello_ok) = TcpClient::connect(address, &[]);
    assert!(hello_ok.get("capabilities").is_none());
    // Without the capability `set_idle` is ignored.
    client.send(r#"{"t":"set_idle","idle":true}"#);
    client.send_raw(r#"{"t":"heart"#);
    std::thread::sleep(Duration::from_millis(50));
    client.send_raw("beat\"}\n");
    assert_eq!(client.recv()["t"], "pong");

    clock.advance(59 * SECOND);
    client.send(r#"{"t":"heartbeat"}"#);
    assert_eq!(client.recv()["t"], "pong");
    clock.advance(59 * SECOND);
    client.assert_quiet();
    clock.advance(SECOND);
    client.assert_closed();
    finish(server, &state);
}

/// Silent active WebSockets release registration and connection resources,
/// including after the client switches back from negotiated idle mode.
#[test]
fn review_task_active_websocket_has_an_inbound_deadline() {
    let clock = TestClock::new(START_UNIX);
    let (address, state, server) = spawn_server(clock.timing(Keepalive::default()));
    let mut websocket = ws_connect(address, &[]);
    clock.advance(60 * SECOND);
    ws_assert_closed(&mut websocket);
    finish(server, &state);

    let clock = TestClock::new(START_UNIX);
    let (address, state, server) = spawn_server(clock.timing(Keepalive::default()));
    let mut websocket = ws_connect(address, &[CAPABILITY]);
    for idle in [true, false] {
        let message = serde_json::json!({ "t": "set_idle", "idle": idle }).to_string();
        websocket.send(Message::Text(message)).unwrap();
        assert_eq!(ws_recv(&mut websocket)["idle"], idle);
    }
    clock.advance(60 * SECOND);
    ws_assert_closed(&mut websocket);
    finish(server, &state);
}

#[test]
fn android_background_task_keepalive_setting_is_clamped_to_contract_range() {
    assert_eq!(
        parse_keepalive("180").unwrap(),
        (Keepalive::default(), None)
    );
    let (keepalive, notice) = parse_keepalive(" 90 ").unwrap();
    assert_eq!((keepalive.secs(), notice), (90, None));
    let (keepalive, notice) = parse_keepalive("1800").unwrap();
    assert_eq!(keepalive.secs(), 210);
    assert!(notice.unwrap().contains("using 210 s"));
    assert_eq!(parse_keepalive("0").unwrap().0.secs(), 30);
    assert!(parse_keepalive("three minutes").is_err());
    assert_eq!(Keepalive::default().negotiate(Some(1)).secs(), 30);
    assert_eq!(SignalTiming::default().keepalive.secs(), 180);
}
