//! The hello of a TCP client with and without capabilities (`transport::handle`):
//! only `tracked_direct_v1` is negotiated, and a hello without capabilities gets
//! the reply of earlier versions.
use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::state::State;
use super::tracked_direct::CAPABILITY;
use super::transport::handle;

#[test]
fn capable_tcp_hello_negotiates_only_tracked_direct_v1() {
    // Rows: a hello that offers the tracked-direct capability next to an unknown
    // one negotiates only that capability; a hello without capabilities gets the
    // response of earlier versions, unchanged. Each row compares the whole reply.
    let cases = [
        (
            "capable hello",
            Some(&[CAPABILITY, "unsupported"][..]),
            serde_json::json!({"t": "hello_ok", "capabilities": [CAPABILITY]}),
        ),
        ("legacy hello", None, serde_json::json!({"t": "hello_ok"})),
    ];
    for (case, capabilities, expected) in cases {
        assert_eq!(tcp_hello(capabilities), expected, "{case}");
    }
}

fn tcp_hello(capabilities: Option<&[&str]>) -> serde_json::Value {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        handle(stream, Arc::new(Mutex::new(State::default()))).unwrap();
    });
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut hello = serde_json::json!({
        "t": "hello",
        "protocol_version": 3,
        "device_id": "requester",
        "device_name": "Requester",
        "listen_port": 0,
        "lan": [],
        "public_key": "pk",
        "fingerprint": "fp"
    });
    if let Some(capabilities) = capabilities {
        hello["capabilities"] = serde_json::json!(capabilities);
    }
    writeln!(stream, "{hello}").unwrap();
    stream.flush().unwrap();

    let mut line = String::new();
    BufReader::new(stream.try_clone().unwrap())
        .read_line(&mut line)
        .unwrap();
    stream.shutdown(Shutdown::Both).unwrap();
    server.join().unwrap();
    serde_json::from_str(line.trim()).unwrap()
}
