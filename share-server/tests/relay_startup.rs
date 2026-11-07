use std::{
    io::{BufRead, BufReader},
    net::{SocketAddr, TcpListener, TcpStream, UdpSocket},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

const SERVER_BINARY: &str = env!("CARGO_BIN_EXE_se-share-server");

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn invalid_expected_relay_configuration_exits_nonzero() {
    let output = Command::new(SERVER_BINARY)
        .arg("127.0.0.1:0")
        .env_remove("SE_IROH_RELAY_DISABLE")
        .env("SE_IROH_RELAY_BIND", "not-a-socket-address")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalid Iroh relay bind"), "{stderr}");
}

#[test]
fn explicit_relay_disable_still_starts_signaling() {
    let address = unused_loopback_address();
    let child = Command::new(SERVER_BINARY)
        .arg(address.to_string())
        .env("SE_IROH_RELAY_DISABLE", "true")
        .env("SE_IROH_RELAY_BIND", "not-a-socket-address")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut child = ChildGuard(child);

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if TcpStream::connect_timeout(&address, Duration::from_millis(50)).is_ok() {
            break;
        }
        if let Some(status) = child.0.try_wait().unwrap() {
            panic!("signaling-only server exited early with {status}");
        }
        assert!(
            Instant::now() < deadline,
            "signaling listener did not start"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn unused_loopback_address() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap()
}

/// A temporary directory with a self-signed certificate and key (PEM) for `localhost`.
struct Certificate {
    directory: PathBuf,
}

impl Certificate {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "se-share-relay-startup-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let certified = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        std::fs::write(directory.join("cert.pem"), certified.cert.pem()).unwrap();
        std::fs::write(
            directory.join("key.pem"),
            certified.signing_key.serialize_pem(),
        )
        .unwrap();
        Self { directory }
    }

    fn cert(&self) -> String {
        self.directory.join("cert.pem").display().to_string()
    }

    fn key(&self) -> String {
        self.directory.join("key.pem").display().to_string()
    }
}

impl Drop for Certificate {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

/// A server process with TLS whose standard error is collected.
struct TlsServer {
    child: ChildGuard,
    log: Arc<Mutex<String>>,
    signaling: SocketAddr,
    _certificate: Certificate,
}

impl TlsServer {
    /// Starts the server on loopback with a TLS relay and the QAD settings
    /// `qad_bind` and `qad_disable` (the variables are removed when `None`).
    fn start(qad_bind: Option<SocketAddr>, qad_disable: Option<&str>) -> Self {
        let certificate = Certificate::new();
        let [signaling, relay] = unused_loopback_addresses::<2>();
        let mut command = Command::new(SERVER_BINARY);
        command
            .arg(signaling.to_string())
            .arg("--tls-cert")
            .arg(certificate.cert())
            .arg("--tls-key")
            .arg(certificate.key())
            .env_remove("SE_IROH_RELAY_DISABLE")
            .env_remove("SE_IROH_QAD_BIND")
            .env_remove("SE_IROH_QAD_DISABLE")
            .env("SE_IROH_RELAY_BIND", relay.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if let Some(bind) = qad_bind {
            command.env("SE_IROH_QAD_BIND", bind.to_string());
        }
        if let Some(disable) = qad_disable {
            command.env("SE_IROH_QAD_DISABLE", disable);
        }
        let mut child = command.spawn().unwrap();
        let stderr = child.stderr.take().unwrap();
        let log = Arc::new(Mutex::new(String::new()));
        let sink = log.clone();
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let mut log = sink.lock().unwrap_or_else(|error| error.into_inner());
                log.push_str(&line);
                log.push('\n');
            }
        });
        Self {
            child: ChildGuard(child),
            log,
            signaling,
            _certificate: certificate,
        }
    }

    fn log(&self) -> String {
        self.log
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// Waits until the standard error holds `text`; the server must keep running.
    fn wait_for(&mut self, text: &str) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if self.log().contains(text) {
                return;
            }
            if let Some(status) = self.child.0.try_wait().unwrap() {
                panic!(
                    "the server exited with {status} before {text:?}:\n{}",
                    self.log()
                );
            }
            assert!(Instant::now() < deadline, "no {text:?} in:\n{}", self.log());
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Waits for the signaling line (printed after the relay lines) and checks
    /// that the signaling port takes TCP connections.
    fn wait_until_serving(&mut self) {
        self.wait_for("se-share-server signaling on");
        TcpStream::connect_timeout(&self.signaling, Duration::from_secs(2)).unwrap();
    }
}

#[test]
fn qad_listening_line_is_logged_with_tls() {
    let [qad] = unused_udp_addresses::<1>();
    let mut server = TlsServer::start(Some(qad), None);
    server.wait_for(&format!("QAD listening on {qad} (UDP)"));
    server.wait_until_serving();
    assert!(server.log().contains("iroh relay listening on https://"));
}

#[test]
fn qad_busy_udp_port_keeps_relay_running() {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let busy = socket.local_addr().unwrap();
    let mut server = TlsServer::start(Some(busy), None);
    server.wait_until_serving();
    // The relay and the signaling keep running without QAD.
    thread::sleep(Duration::from_secs(3));
    assert!(server.child.0.try_wait().unwrap().is_none());
    TcpStream::connect_timeout(&server.signaling, Duration::from_secs(2)).unwrap();
    let log = server.log();
    assert!(log.contains("QAD off: cannot bind UDP"), "{log}");
    assert!(log.contains("iroh relay listening on https://"), "{log}");
    assert!(!log.contains("QAD listening"), "{log}");
    drop(socket);
}

#[test]
fn qad_disabled_by_env_keeps_relay_running() {
    let [qad] = unused_udp_addresses::<1>();
    let mut server = TlsServer::start(Some(qad), Some("1"));
    server.wait_until_serving();
    thread::sleep(Duration::from_secs(1));
    assert!(server.child.0.try_wait().unwrap().is_none());
    let log = server.log();
    assert!(log.contains("QAD disabled"), "{log}");
    assert!(log.contains("iroh relay listening on https://"), "{log}");
    assert!(!log.contains("QAD listening"), "{log}");
}

/// Free loopback TCP addresses; all listeners are held until every address is
/// known, so the addresses differ.
fn unused_loopback_addresses<const COUNT: usize>() -> [SocketAddr; COUNT] {
    let listeners: Vec<TcpListener> = (0..COUNT)
        .map(|_| TcpListener::bind("127.0.0.1:0").unwrap())
        .collect();
    let mut addresses = [SocketAddr::from(([127, 0, 0, 1], 0)); COUNT];
    for (address, listener) in addresses.iter_mut().zip(&listeners) {
        *address = listener.local_addr().unwrap();
    }
    addresses
}

/// Free loopback UDP addresses, held together like the TCP ones.
fn unused_udp_addresses<const COUNT: usize>() -> [SocketAddr; COUNT] {
    let sockets: Vec<UdpSocket> = (0..COUNT)
        .map(|_| UdpSocket::bind("127.0.0.1:0").unwrap())
        .collect();
    let mut addresses = [SocketAddr::from(([127, 0, 0, 1], 0)); COUNT];
    for (address, socket) in addresses.iter_mut().zip(&sockets) {
        *address = socket.local_addr().unwrap();
    }
    addresses
}
