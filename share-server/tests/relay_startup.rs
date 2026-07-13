use std::{
    net::{SocketAddr, TcpListener, TcpStream},
    process::{Child, Command, Stdio},
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
