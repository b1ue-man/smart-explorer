//! The worker proof of the daemon IPC (r-lokal-1, L-B1; suite expectations
//! lok-1a to lok-1e of the review 2026-11-07). A foreign process behind the
//! published loopback port gets a nonce and nothing else; only the real
//! listener (`ipc_listener::serve_for_test`: the accept loop, the capability
//! prelude and the client handling of the worker) is proven and then served.
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::{
    connect_token, connect_with_token, hex, parse_request, reply_line, ProofFailure, ProofLine,
};
use crate::daemon::ipc_host::ShareHost;
use crate::daemon::ipc_listener::serve_for_test;
use crate::daemon::ipc_protocol::{read_response, write_request, IpcRequest, IpcResponse};

const TOKEN: &str = "lokal-task-daemon-token-0123456789abcdef";
const LIMIT: Duration = Duration::from_secs(10);
const DIAL: Duration = Duration::from_millis(700);

/// What the process behind the published port does with the proof request.
#[derive(Clone)]
enum Stand {
    /// Reads the request, then closes the connection.
    Close,
    /// Reads the request, then stays silent until the client gives up.
    Silent,
    /// Answers with the proof of `token` computed for the address `bound_to`.
    Answer {
        token: &'static str,
        bound_to: SocketAddr,
    },
    /// Answers with these bytes.
    Reply(Vec<u8>),
    /// Hands the request to the real worker at the address and returns its answer.
    Relay(SocketAddr),
}

struct StandIn {
    addr: SocketAddr,
    thread: JoinHandle<Vec<u8>>,
}

impl StandIn {
    /// Everything the stand-in received on its one connection.
    fn received(self) -> Vec<u8> {
        self.thread.join().expect("the stand-in thread ended")
    }
}

fn stand_in(behaviour: Stand) -> StandIn {
    let listener = TcpListener::bind("127.0.0.1:0").expect("stand-in listener");
    listener
        .set_nonblocking(true)
        .expect("nonblocking listener");
    let addr = listener.local_addr().expect("stand-in address");
    let thread = std::thread::spawn(move || run_stand_in(&listener, behaviour));
    StandIn { addr, thread }
}

fn accept_within(listener: &TcpListener, limit: Duration) -> Option<TcpStream> {
    let end = Instant::now() + limit;
    while Instant::now() < end {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).ok()?;
                return Some(stream);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => return None,
        }
    }
    None
}

fn read_until_newline(stream: &mut TcpStream, received: &mut Vec<u8>) {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("read timeout");
    let mut chunk = [0u8; 256];
    while !received.contains(&b'\n') {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(count) => received.extend_from_slice(&chunk[..count]),
        }
    }
}

/// Reads what is still sent until the peer closes or `grace` has passed.
fn drain(stream: &mut TcpStream, received: &mut Vec<u8>, grace: Duration) {
    let end = Instant::now() + grace;
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .expect("read timeout");
    let mut chunk = [0u8; 1024];
    while Instant::now() < end {
        match stream.read(&mut chunk) {
            Ok(0) => return,
            Ok(count) => received.extend_from_slice(&chunk[..count]),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return,
        }
    }
}

fn run_stand_in(listener: &TcpListener, behaviour: Stand) -> Vec<u8> {
    let mut received = Vec::new();
    let Some(mut stream) = accept_within(listener, LIMIT) else {
        return received;
    };
    read_until_newline(&mut stream, &mut received);
    let nonce = match parse_request(&received) {
        ProofLine::Complete(_, nonce) => nonce,
        _ => return received,
    };
    match behaviour {
        Stand::Close => return received,
        Stand::Silent => {}
        Stand::Answer { token, bound_to } => {
            let reply = reply_line(token, &nonce, &bound_to).expect("reply line");
            let _ = stream.write_all(&reply);
        }
        Stand::Reply(bytes) => {
            let _ = stream.write_all(&bytes);
        }
        Stand::Relay(worker) => {
            let mut upstream =
                TcpStream::connect_timeout(&worker, Duration::from_secs(2)).expect("real worker");
            upstream
                .write_all(&received)
                .expect("request to the worker");
            let mut reply = Vec::new();
            read_until_newline(&mut upstream, &mut reply);
            let _ = stream.write_all(&reply);
        }
    }
    // What the client sends after it has judged the answer (the token, if it
    // trusted the stand-in) arrives here until it closes the connection.
    drain(&mut stream, &mut received, Duration::from_secs(2));
    received
}

/// The stand-in received exactly the nonce line and nothing carrying `token`.
fn assert_only_the_nonce(received: &[u8], token: &str) {
    let text = std::str::from_utf8(received).expect("the stand-in received UTF-8");
    assert_eq!(
        text.matches('\n').count(),
        1,
        "exactly one line expected: {text:?}"
    );
    assert!(!text.contains(token), "the token reached the stand-in");
    let line: serde_json::Value = serde_json::from_str(text.trim_end()).expect("a JSON line");
    let fields = line.as_object().expect("a JSON object");
    assert_eq!(fields.len(), 2, "the line has the fields t and nonce only");
    assert_eq!(line["t"], "ipc_proof");
    let nonce = line["nonce"].as_str().expect("a nonce");
    assert_eq!(nonce.len(), 64);
    assert!(nonce.bytes().all(|byte| byte.is_ascii_hexdigit()));
}

enum Outcome {
    Proven(TcpStream),
    Unproven,
    Unreachable(std::io::ErrorKind),
}

fn outcome(result: Result<TcpStream, ProofFailure>) -> Outcome {
    match result {
        Ok(stream) => Outcome::Proven(stream),
        Err(ProofFailure::Unproven) => Outcome::Unproven,
        Err(ProofFailure::Unreachable(error)) => Outcome::Unreachable(error.kind()),
    }
}

fn is_timeout(kind: std::io::ErrorKind) -> bool {
    matches!(
        kind,
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    )
}

/// A listener of the worker for `token`, with its host.
fn worker(token: &str) -> (crate::daemon::ipc_listener::IpcListener, ShareHost) {
    let host = ShareHost::new("c".repeat(32));
    let listener = serve_for_test(host.clone(), token.into()).expect("worker listener");
    (listener, host)
}

fn other_port() -> SocketAddr {
    "127.0.0.1:9".parse().expect("an address")
}

/// lok-1a and the unanswered case of L-B1: the process behind the published
/// port gets the nonce and nothing else, whatever it answers.
#[test]
fn lokal_task_foreign_listener_gets_only_the_nonce_and_never_the_token() {
    let cases: Vec<(&str, Stand, bool)> = vec![
        ("closes without an answer", Stand::Close, false),
        (
            "answers with the proof of another token",
            Stand::Answer {
                token: "another-daemon-token-0123456789abcdef",
                bound_to: "127.0.0.1:1".parse().expect("an address"),
            },
            false,
        ),
        (
            "answers with a proof bound to another port",
            Stand::Answer {
                token: TOKEN,
                bound_to: other_port(),
            },
            false,
        ),
        (
            "answers with a line that is no proof",
            Stand::Reply(b"HTTP/1.1 400 Bad Request\r\n".to_vec()),
            false,
        ),
        (
            "answers with a proof of the wrong kind",
            Stand::Reply(format!("{{\"t\":\"pong\",\"proof\":\"{}\"}}\n", hex(&[1u8; 32])).into()),
            false,
        ),
        (
            "answers with a line longer than a proof",
            Stand::Reply([vec![b'a'; 300], b"\n".to_vec()].concat()),
            false,
        ),
        ("reads the request and stays silent", Stand::Silent, true),
    ];
    for (case, behaviour, silent) in cases {
        // The typed result `probe_worker` maps: Unproven is Stale, a missing
        // answer in time is Missing.
        let typed = stand_in(behaviour.clone());
        let result = outcome(connect_with_token(&typed.addr, DIAL, TOKEN));
        match (silent, result) {
            (false, Outcome::Unproven) => {}
            (true, Outcome::Unreachable(kind)) if is_timeout(kind) => {}
            (_, Outcome::Proven(_)) => panic!("{case}: the stand-in was proven"),
            (_, Outcome::Unproven) => panic!("{case}: unproven instead of a timeout"),
            (_, Outcome::Unreachable(kind)) => panic!("{case}: unreachable ({kind:?})"),
        }
        assert_only_the_nonce(&typed.received(), TOKEN);

        // The error every client file returns (`connect`, `connect_token`,
        // `exec_lifetime::connect_control`): PermissionDenied, and a silent
        // process the timeout of the dial.
        let plain = stand_in(behaviour);
        let error = connect_token(&plain.addr, DIAL, TOKEN)
            .err()
            .unwrap_or_else(|| panic!("{case}: connect_token returned a stream"));
        if silent {
            assert!(is_timeout(error.kind()), "{case}: {:?}", error.kind());
        } else {
            assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied, "{case}");
        }
        assert_only_the_nonce(&plain.received(), TOKEN);
    }
}

/// lok-1b: a process on the old port that relays the nonce to the real
/// worker on its new port cannot pass: the proof names the worker's port.
#[test]
fn lokal_task_relayed_nonce_does_not_prove_the_squatter() {
    let (listener, _host) = worker(TOKEN);
    let real = listener.addr();
    let squatter = stand_in(Stand::Relay(real));
    match outcome(connect_with_token(
        &squatter.addr,
        Duration::from_secs(3),
        TOKEN,
    )) {
        Outcome::Unproven => {}
        Outcome::Proven(_) => panic!("the relaying squatter was proven"),
        Outcome::Unreachable(kind) => panic!("the relay was unreachable ({kind:?})"),
    }
    assert_only_the_nonce(&squatter.received(), TOKEN);
    // The real worker answers for its own address.
    assert!(matches!(
        outcome(connect_with_token(&real, Duration::from_secs(3), TOKEN)),
        Outcome::Proven(_)
    ));
}

/// lok-1c: the real listener is proven; a request with the token is served
/// on the proven stream. A client with another token refuses the worker.
#[test]
fn lokal_task_real_worker_is_proven_and_then_serves_the_request() {
    let (listener, host) = worker(TOKEN);
    let addr = listener.addr();
    let mut stream = match outcome(connect_with_token(&addr, Duration::from_secs(3), TOKEN)) {
        Outcome::Proven(stream) => stream,
        Outcome::Unproven => panic!("the real worker was not proven"),
        Outcome::Unreachable(kind) => panic!("the real worker was unreachable ({kind:?})"),
    };
    stream.set_read_timeout(Some(LIMIT)).expect("read timeout");
    stream
        .set_write_timeout(Some(LIMIT))
        .expect("write timeout");
    write_request(
        &mut stream,
        &IpcRequest::Ping {
            token: TOKEN.into(),
        },
    )
    .expect("ping");
    match read_response(&mut stream).expect("pong") {
        IpcResponse::Pong {
            version,
            generation,
            initialized,
        } => {
            // What `probe_worker` requires of a ready worker.
            assert_eq!(version, env!("CARGO_PKG_VERSION"));
            assert_eq!(generation, host.generation());
            assert_eq!(generation.len(), 32);
            assert!(!initialized);
        }
        _ => panic!("the worker did not answer the ping with a pong"),
    }
    drop(stream);

    let error = connect_token(&addr, Duration::from_secs(3), "another-daemon-token")
        .err()
        .expect("a client with another token proved the worker");
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
}

/// lok-1d: a client of an older version sends `{"t":"ping","token":T}` as its
/// first line without a proof request and is answered as before; a wrong
/// token is still refused with an explicit answer.
#[test]
fn lokal_task_old_clients_without_a_proof_request_are_still_served() {
    let (listener, _host) = worker(TOKEN);
    let addr = listener.addr();
    let ask = |token: &str| {
        let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3)).expect("dial");
        stream.set_read_timeout(Some(LIMIT)).expect("read timeout");
        stream
            .write_all(format!("{{\"t\":\"ping\",\"token\":\"{token}\"}}\n").as_bytes())
            .expect("old-form ping");
        read_response(&mut stream).expect("an answer")
    };
    assert!(matches!(ask(TOKEN), IpcResponse::Pong { .. }));
    assert!(matches!(ask("wrong-daemon-token"), IpcResponse::Err { .. }));
}

/// lok-1e: one proof per connection. A second proof request, sent apart or
/// pipelined behind the first, is dropped: the connection closes unanswered.
#[test]
fn lokal_task_second_proof_on_one_connection_is_dropped() {
    let (listener, _host) = worker(TOKEN);
    let addr = listener.addr();
    let request = format!(
        "{{\"t\":\"ipc_proof\",\"nonce\":\"{}\"}}\n",
        hex(&[5u8; 32])
    );

    // Apart: proven by the client first, then a second request.
    let mut stream = match outcome(connect_with_token(&addr, Duration::from_secs(3), TOKEN)) {
        Outcome::Proven(stream) => stream,
        _ => panic!("the real worker was not proven"),
    };
    stream.set_read_timeout(Some(LIMIT)).expect("read timeout");
    stream.write_all(request.as_bytes()).expect("second proof");
    assert_closed_without_answer(&mut stream, "second proof sent apart");

    // Pipelined: both requests in one write; one reply line, then the close.
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3)).expect("dial");
    stream.set_read_timeout(Some(LIMIT)).expect("read timeout");
    stream
        .write_all(format!("{request}{request}").as_bytes())
        .expect("two proofs");
    let mut first = Vec::new();
    read_until_newline(&mut stream, &mut first);
    let reply: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&first).trim_end()).expect("reply");
    assert_eq!(reply["t"], "ipc_proof");
    assert_eq!(reply["proof"].as_str().map(str::len), Some(64));
    assert_closed_without_answer(&mut stream, "second proof pipelined");
}

fn assert_closed_without_answer(stream: &mut TcpStream, case: &str) {
    stream.set_read_timeout(Some(LIMIT)).expect("read timeout");
    let mut byte = [0u8; 1];
    match stream.read(&mut byte) {
        Ok(0) => {}
        Ok(_) => panic!("{case}: the worker answered the second proof"),
        Err(error) if is_timeout(error.kind()) => {
            panic!("{case}: the connection stayed open")
        }
        // A reset instead of an end of stream: the connection is closed too.
        Err(_) => {}
    }
}

/// L-B1, the call site: `exec_lifetime::connect_control` is reachable only
/// inside `exec_ipc`, so the test checks its source. It must dial through
/// `ipc_proof::connect_token` (tested above with the same arguments) before it
/// writes `ExecControl`, and no production file of the worker IPC that reads
/// the daemon token or the published address may open its own TCP connection.
#[test]
fn lokal_task_every_daemon_token_sender_dials_through_the_proof() {
    let source = include_str!("exec_lifetime.rs");
    let start = source
        .find("fn connect_control(")
        .expect("connect_control exists");
    let body = &source[start..];
    let body = &body[..body.find("write_request(").expect("it writes the request")];
    let compact = body.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        compact.contains("ipc_proof::connect_token(&address,"),
        "connect_control does not dial through the proof: {compact}"
    );
    assert!(!compact.contains("TcpStream::connect"));

    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/daemon/os/shared");
    let mut checked = 0usize;
    let mut direct = Vec::new();
    for entry in std::fs::read_dir(&directory).expect("the worker IPC sources") {
        let path = entry.expect("a directory entry").path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let production = name.ends_with(".rs")
            && !name.ends_with("_tests.rs")
            && !name.ends_with("_fixture.rs")
            && name != "ipc_proof.rs";
        if !production {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("a source file");
        if !(text.contains("read_token(") || text.contains("read_ipc_addr(")) {
            continue;
        }
        checked += 1;
        if text.contains("TcpStream::connect") {
            direct.push(name.to_string());
        }
    }
    assert!(checked >= 10, "only {checked} token readers found");
    assert!(
        direct.is_empty(),
        "these files read the daemon token or the published address and dial without the proof: {direct:?}"
    );
}
