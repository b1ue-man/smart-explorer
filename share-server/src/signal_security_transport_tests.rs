//! TLS, challenge login and transport-policy acceptance signals for the remote suite.

use std::collections::HashSet;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh_base::SecretKey;
use rustls::{ClientConfig, ClientConnection, RootCertStore, ServerConfig, StreamOwned};
use serde_json::{json, Value};
use tungstenite::{Message, WebSocket};

use crate::idle::SignalTiming;
use crate::limits::SourceKey;
use crate::state::{ClientIdentity, State};
use crate::{hello_session::HelloSession, login, server_tls, transport, Out};

type TlsWs = WebSocket<StreamOwned<ClientConnection, TcpStream>>;

struct Certificates {
    directory: PathBuf,
    first: rcgen::CertifiedKey<rcgen::KeyPair>,
    second: rcgen::CertifiedKey<rcgen::KeyPair>,
}

impl Certificates {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "se-signal-tls-{}-{}",
            std::process::id(),
            login::hex(&login::nonce().unwrap())
        ));
        std::fs::create_dir(&directory).unwrap();
        let first = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let second =
            rcgen::generate_simple_self_signed(vec!["localhost".into(), "renewal.example".into()])
                .unwrap();
        let files = Self {
            directory,
            first,
            second,
        };
        files.write(&files.first);
        files
    }

    fn cert(&self) -> PathBuf {
        self.directory.join("cert.pem")
    }
    fn key(&self) -> PathBuf {
        self.directory.join("key.pem")
    }
    fn write(&self, certificate: &rcgen::CertifiedKey<rcgen::KeyPair>) {
        std::fs::write(self.cert(), certificate.cert.pem()).unwrap();
        std::fs::write(self.key(), certificate.signing_key.serialize_pem()).unwrap();
    }
}

impl Drop for Certificates {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn hello(secret: &SecretKey) -> Value {
    json!({
        "t": "hello", "protocol_version": 3, "device_id": "verified-device",
        "device_name": "", "listen_port": 0, "lan": [],
        "public_key": secret.public().to_string(), "fingerprint": "",
        "capabilities": [login::CAPABILITY],
    })
}

fn auth(challenge: Value, secret: &SecretKey) -> Value {
    assert_eq!(challenge["t"], "hello_challenge");
    let nonce = login::hex_decode(challenge["nonce"].as_str().unwrap()).unwrap();
    let key = secret.public().to_string();
    json!({ "t": "hello_auth", "signature": login::hex(&secret.sign(
        &login::digest(&nonce, "verified-device", &key)
    ).to_bytes()) })
}

fn read_ws(socket: &mut TlsWs) -> Value {
    loop {
        match socket.read().unwrap() {
            Message::Text(text) => return serde_json::from_str(&text).unwrap(),
            Message::Close(_) => panic!("unexpected close"),
            _ => {}
        }
    }
}

fn run_wss(config: Arc<ServerConfig>, certificate: &rcgen::Certificate) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let state = Arc::new(Mutex::new(State::default()));
    let server_state = state.clone();
    let server = std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        transport::handle_with_security(
            socket,
            server_state,
            SourceKey::from_socket(address),
            &SignalTiming::default(),
            Some(config),
            false,
        )
    });
    let mut roots = RootCertStore::empty();
    roots.add(certificate.der().clone()).unwrap();
    let client_config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
    let socket = TcpStream::connect(address).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    socket
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let client =
        ClientConnection::new(Arc::new(client_config), "localhost".try_into().unwrap()).unwrap();
    let (mut websocket, _) = tungstenite::client(
        format!("wss://localhost:{}/se-share", address.port()),
        StreamOwned::new(client, socket),
    )
    .unwrap();
    let secret = SecretKey::from_bytes(&[21; 32]);
    websocket
        .send(Message::Text(hello(&secret).to_string()))
        .unwrap();
    let response = auth(read_ws(&mut websocket), &secret);
    websocket.send(Message::Text(response.to_string())).unwrap();
    assert_eq!(
        read_ws(&mut websocket)["capabilities"],
        json!([login::CAPABILITY])
    );
    let writer = {
        let state = state.lock().unwrap();
        let client = state.clients.values().next().unwrap();
        assert_eq!(client.identity, ClientIdentity::Proven(secret.public()));
        client.writer.clone()
    };
    // No inbound heartbeat is needed to flush an asynchronously queued output.
    assert!(writer.try_send(&Out::DirectOffline {
        lookup_id: "queued-wake".into()
    }));
    assert_eq!(read_ws(&mut websocket)["lookup_id"], "queued-wake");
    websocket.close(None).unwrap();
    server.join().unwrap().unwrap();
    assert!(state.lock().unwrap().clients.is_empty());
}

#[test]
fn review_task_wss_login_output_wake_and_certificate_reload() {
    let files = Certificates::new();
    let config = server_tls::load(&files.cert(), &files.key()).unwrap();
    run_wss(config.clone(), &files.first.cert);
    // A half-replaced pair must retain the previously validated certificate.
    std::fs::write(files.cert(), files.second.cert.pem()).unwrap();
    std::fs::write(files.key(), "bad key").unwrap();
    assert!(server_tls::load(&files.cert(), &files.key()).is_err());
    run_wss(config.clone(), &files.first.cert);
    files.write(&files.second);
    run_wss(config, &files.second.cert);
}

#[test]
fn review_task_plaintext_is_refused_before_registration() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let state = Arc::new(Mutex::new(State::default()));
    let server_state = state.clone();
    let server = std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        transport::handle_with_security(
            socket,
            server_state,
            SourceKey::from_socket(address),
            &SignalTiming::default(),
            None,
            false,
        )
    });
    let mut client = TcpStream::connect(address).unwrap();
    writeln!(client, "{}", hello(&SecretKey::from_bytes(&[21; 32]))).unwrap();
    assert_eq!(
        server.join().unwrap().unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert!(state.lock().unwrap().clients.is_empty());
}

#[test]
fn review_task_raw_tcp_key_login_rejects_another_signer() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let state = Arc::new(Mutex::new(State::default()));
    let server_state = state.clone();
    let server =
        std::thread::spawn(move || transport::handle(listener.accept().unwrap().0, server_state));
    let mut client = TcpStream::connect(address).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut reader = BufReader::new(client.try_clone().unwrap());
    let secret = SecretKey::from_bytes(&[21; 32]);
    writeln!(client, "{}", hello(&secret)).unwrap();
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let challenge = serde_json::from_str(&line).unwrap();
    writeln!(
        client,
        "{}",
        auth(challenge, &SecretKey::from_bytes(&[22; 32]))
    )
    .unwrap();
    line.clear();
    reader.read_line(&mut line).unwrap();
    let rejection: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(rejection["msg"], login::LOGIN_FAILED);
    server.join().unwrap().unwrap();
    assert!(state.lock().unwrap().clients.is_empty());
}

#[test]
fn review_task_login_challenge_is_connection_bound_and_single_use() {
    let secret = SecretKey::from_bytes(&[21; 32]);
    let mut first = HelloSession::begin(serde_json::from_value(hello(&secret)).unwrap()).unwrap();
    let mut second = HelloSession::begin(serde_json::from_value(hello(&secret)).unwrap()).unwrap();
    let challenge = serde_json::to_value(first.challenge().unwrap()).unwrap();
    let signed = auth(challenge, &secret);
    assert!(second
        .answer(serde_json::from_value(signed.clone()).unwrap())
        .is_err());
    first
        .answer(serde_json::from_value(signed.clone()).unwrap())
        .unwrap();
    assert_eq!(first.identity, ClientIdentity::Proven(secret.public()));
    assert!(first
        .answer(serde_json::from_value(signed).unwrap())
        .is_err());
    assert_eq!(
        first.capabilities,
        HashSet::from([login::CAPABILITY.to_string()])
    );
    let mut legacy = hello(&secret);
    legacy["capabilities"] = json!([]);
    let legacy = HelloSession::begin(serde_json::from_value(legacy).unwrap()).unwrap();
    assert_eq!(
        legacy.identity,
        ClientIdentity::LegacyClaimed(secret.public())
    );
    assert!(legacy.challenge().is_none());
}
