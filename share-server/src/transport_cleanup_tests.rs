use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use super::state::{lock_state, State};
use super::transport::handle;

const HELLO: &str = r#"{"t":"hello","protocol_version":3,"device_id":"raw-client","device_name":"Raw","listen_port":0,"lan":[],"public_key":"pk","fingerprint":"fp"}"#;

#[test]
fn pipelined_raw_messages_survive_the_hello_reader_transition() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let state = Arc::new(Mutex::new(State::default()));
    let server_state = Arc::clone(&state);
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        handle(stream, server_state)
    });

    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .write_all(format!("{HELLO}\n{{\"t\":\"heartbeat\"}}\n").as_bytes())
        .unwrap();
    stream.flush().unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert_eq!(message_type(&line), "hello_ok");
    line.clear();
    reader.read_line(&mut line).unwrap();
    assert_eq!(message_type(&line), "pong");

    drop((reader, stream));
    server.join().unwrap().unwrap();
    assert!(lock_state(&state).clients.is_empty());
}

#[test]
fn raw_rate_limit_error_cleans_registration_and_allows_reconnect() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let state = Arc::new(Mutex::new(State::default()));
    let server_state = Arc::clone(&state);
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let first = handle(stream, Arc::clone(&server_state)).unwrap_err();
        assert_eq!(first.kind(), ErrorKind::PermissionDenied, "{first}");
        assert!(lock_state(&server_state).clients.is_empty());

        let (stream, _) = listener.accept().unwrap();
        handle(stream, Arc::clone(&server_state)).unwrap();
        assert!(lock_state(&server_state).clients.is_empty());
    });

    let mut attacker = TcpStream::connect(address).unwrap();
    let heartbeats = "{\"t\":\"heartbeat\"}\n".repeat(140);
    attacker
        .write_all(format!("{HELLO}\n{heartbeats}").as_bytes())
        .unwrap();
    attacker.flush().unwrap();
    drop(attacker);

    let mut replacement = TcpStream::connect(address).unwrap();
    replacement
        .write_all(format!("{HELLO}\n").as_bytes())
        .unwrap();
    replacement.flush().unwrap();
    let mut response = String::new();
    BufReader::new(replacement.try_clone().unwrap())
        .read_line(&mut response)
        .unwrap();
    assert_eq!(message_type(&response), "hello_ok");
    drop(replacement);

    server.join().unwrap();
    assert!(lock_state(&state).clients.is_empty());
}

fn message_type(line: &str) -> String {
    serde_json::from_str::<serde_json::Value>(line).unwrap()["t"]
        .as_str()
        .unwrap()
        .to_string()
}
