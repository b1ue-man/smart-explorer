//! Signaling policy of the server process: key login is the default (E18,
//! M23) and connections that wait for their registration cannot take the
//! places of registered clients (R-SERVER-04).

use std::{
    io::{BufRead, BufReader, ErrorKind, Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use iroh_base::SecretKey;
use serde_json::{json, Value};

const SERVER_BINARY: &str = env!("CARGO_BIN_EXE_se-share-server");
const LOGIN_DOMAIN: &[u8] = b"se-signal-login-v1\0";
/// Settings of the operator that these tests must start from.
const SERVER_VARIABLES: &[&str] = &[
    "SE_SHARE_BIND",
    "SE_SHARE_TLS_CERT",
    "SE_SHARE_TLS_KEY",
    "SE_SHARE_ALLOW_PLAINTEXT",
    "SE_SHARE_ALLOW_LEGACY_CLIENTS",
    "SE_SHARE_REQUIRE_KEY_LOGIN",
    "SE_SHARE_STATE_FILE",
    "SE_SHARE_TRUSTED_PROXY_IPS",
    "SE_SHARE_MAX_CONNECTIONS",
    "SE_SHARE_MAX_CLIENTS",
    "SE_SHARE_MAX_HANDSHAKES",
    "SE_SHARE_MAX_HANDSHAKES_PER_SOURCE",
    "SE_SHARE_MAX_HANDSHAKES_PER_NETWORK",
    "SE_SHARE_MAX_CLIENTS_PER_SOURCE",
];

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A signaling-only server on loopback whose standard error is collected.
struct Server {
    _child: ChildGuard,
    address: SocketAddr,
    _log: Arc<Mutex<String>>,
}

impl Server {
    fn start(arguments: &[&str], variables: &[(&str, &str)]) -> Self {
        let address = unused_loopback_address();
        let mut command = Command::new(SERVER_BINARY);
        command
            .arg(address.to_string())
            .args(arguments)
            .env("SE_IROH_RELAY_DISABLE", "true")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        for name in SERVER_VARIABLES {
            command.env_remove(name);
        }
        for (name, value) in variables {
            command.env(name, value);
        }
        let mut child = command.spawn().unwrap();
        let stderr = child.stderr.take().unwrap();
        let log = Arc::new(Mutex::new(String::new()));
        let sink = log.clone();
        let (ready_sender, ready) = std::sync::mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if line.contains("se-share-server signaling on") {
                    let _ = ready_sender.send(());
                }
                let mut log = sink.lock().unwrap_or_else(|error| error.into_inner());
                log.push_str(&line);
                log.push('\n');
            }
        });
        let child = ChildGuard(child);
        if ready.recv_timeout(Duration::from_secs(20)).is_err() {
            let log = log.lock().unwrap_or_else(|error| error.into_inner());
            panic!("the server did not start signaling:\n{log}");
        }
        Self {
            _child: child,
            address,
            _log: log,
        }
    }
}

fn unused_loopback_address() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap()
}

/// One signaling connection that speaks JSON lines.
struct Connection {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

impl Connection {
    fn new(stream: TcpStream) -> Self {
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        Self {
            reader: BufReader::new(stream.try_clone().unwrap()),
            writer: stream,
        }
    }

    fn open(address: SocketAddr) -> Self {
        Self::new(TcpStream::connect(address).unwrap())
    }

    fn send(&mut self, message: &Value) {
        writeln!(self.writer, "{message}").unwrap();
        self.writer.flush().unwrap();
    }

    fn receive(&mut self) -> Value {
        let mut line = String::new();
        let read = self.reader.read_line(&mut line).unwrap();
        assert!(read > 0, "the server closed the connection");
        serde_json::from_str(&line).unwrap()
    }
}

/// A TCP connection that binds the local address `local` before it connects.
fn connect_from(local: Ipv4Addr, server: SocketAddr) -> TcpStream {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let stream = runtime.block_on(async {
        let socket = tokio::net::TcpSocket::new_v4().unwrap();
        socket.bind(SocketAddr::new(IpAddr::V4(local), 0)).unwrap();
        socket.connect(server).await.unwrap()
    });
    let stream = stream.into_std().unwrap();
    stream.set_nonblocking(false).unwrap();
    stream
}

/// The login digest the app computes (`signal_handshake::login_digest`).
fn login_digest(nonce: &[u8], device_id: &str, public_key: &str) -> [u8; 32] {
    let mut context = ring::digest::Context::new(&ring::digest::SHA256);
    context.update(LOGIN_DOMAIN);
    for field in [nonce, device_id.as_bytes(), public_key.as_bytes()] {
        let length = u32::try_from(field.len()).unwrap();
        context.update(&length.to_be_bytes());
        context.update(field);
    }
    let mut digest = [0_u8; 32];
    digest.copy_from_slice(context.finish().as_ref());
    digest
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hex_decode(text: &str) -> Vec<u8> {
    (0..text.len() / 2)
        .map(|index| u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).unwrap())
        .collect()
}

fn hello(device_id: &str, public_key: &str, capabilities: &[&str]) -> Value {
    json!({
        "t": "hello",
        "protocol_version": 3,
        "device_id": device_id,
        "device_name": "test device",
        "listen_port": 0,
        "lan": [],
        "public_key": public_key,
        "fingerprint": "fp",
        "capabilities": capabilities,
    })
}

/// The whole key login of a Hello; returns the answer after the signature.
fn key_login(connection: &mut Connection, device_id: &str, secret: &SecretKey) -> Value {
    let public_key = secret.public().to_string();
    connection.send(&hello(device_id, &public_key, &["key_login_v1"]));
    let challenge = connection.receive();
    assert_eq!(challenge["t"], "hello_challenge", "{challenge}");
    let nonce = hex_decode(challenge["nonce"].as_str().unwrap());
    let signature = secret.sign(&login_digest(&nonce, device_id, &public_key));
    connection.send(&json!({
        "t": "hello_auth",
        "signature": hex(&signature.to_bytes()),
    }));
    connection.receive()
}

/// Whether the server shut the connection down within `limit`; an open
/// connection stays silent until the read times out.
fn closed_within(stream: &mut TcpStream, limit: Duration) -> bool {
    stream.set_read_timeout(Some(limit)).unwrap();
    let mut byte = [0_u8; 1];
    match stream.read(&mut byte) {
        Ok(0) => true,
        Ok(_) => false,
        Err(error) => !matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut),
    }
}

#[test]
fn legacy_clients_need_the_operator_switch_while_key_login_always_works() {
    let cases: [(Vec<&str>, bool); 2] = [(vec![], false), (vec!["--allow-legacy-clients"], true)];
    for (arguments, legacy_admitted) in cases {
        let server = Server::start(&arguments, &[]);

        let mut legacy = Connection::open(server.address);
        legacy.send(&hello("legacy-device", "pk", &[]));
        let answer = legacy.receive();
        if legacy_admitted {
            assert_eq!(answer["t"], "hello_ok", "{arguments:?}: {answer}");
        } else {
            assert_eq!(answer["t"], "error", "{arguments:?}: {answer}");
            let message = answer["msg"].as_str().unwrap();
            assert!(
                message.contains("this server requires key login"),
                "{message}"
            );
            assert!(message.contains("--allow-legacy-clients"), "{message}");
        }

        let mut keyed = Connection::open(server.address);
        let secret = SecretKey::from_bytes(&[51; 32]);
        let answer = key_login(&mut keyed, "keyed-device", &secret);
        assert_eq!(answer["t"], "hello_ok", "{arguments:?}: {answer}");
    }
}

#[test]
fn handshake_pool_closes_the_oldest_silent_connection_and_spares_registered_clients() {
    let server = Server::start(&[], &[("SE_SHARE_MAX_HANDSHAKES_PER_SOURCE", "4")]);
    let mut registered = Connection::open(server.address);
    let answer = key_login(
        &mut registered,
        "registered",
        &SecretKey::from_bytes(&[41; 32]),
    );
    assert_eq!(answer["t"], "hello_ok", "{answer}");

    // Five silent connections from one source against a limit of four.
    let started = Instant::now();
    let mut silent = Vec::new();
    for _ in 0..5 {
        silent.push(TcpStream::connect(server.address).unwrap());
        thread::sleep(Duration::from_millis(50));
    }
    assert!(
        closed_within(&mut silent[0], Duration::from_secs(5)),
        "the oldest silent connection must be closed by the server"
    );
    for stream in &mut silent[1..] {
        assert!(!closed_within(stream, Duration::from_millis(300)));
    }
    assert!(
        started.elapsed() < Duration::from_secs(9),
        "the closing came from the pool, not from the 10 s registration deadline"
    );

    // The registered client was not touched and still answers.
    registered.send(&json!({ "t": "heartbeat" }));
    assert_eq!(registered.receive()["t"], "pong");

    // Another source finds its place and registers while the first is full.
    let stream = connect_from(Ipv4Addr::new(127, 0, 0, 2), server.address);
    let mut other = Connection::new(stream);
    let answer = key_login(
        &mut other,
        "other-source",
        &SecretKey::from_bytes(&[42; 32]),
    );
    assert_eq!(answer["t"], "hello_ok", "{answer}");
    registered.send(&json!({ "t": "heartbeat" }));
    assert_eq!(registered.receive()["t"], "pong");
}
