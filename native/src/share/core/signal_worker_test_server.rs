//! A fake Share server for the worker tests: TCP lines or WebSocket.

use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use serde_json::Value;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use crate::share::signal_connection::SignalConnection;

pub(super) enum FakeServer {
    Lines {
        stream: TcpStream,
        reader: BufReader<TcpStream>,
        partial: String,
    },
    WebSocket(WebSocket<TcpStream>),
}

impl FakeServer {
    pub(super) fn send(&mut self, json: &str) {
        match self {
            Self::Lines { stream, .. } => {
                stream
                    .write_all(format!("{json}\n").as_bytes())
                    .expect("server write");
                stream.flush().expect("server flush");
            }
            Self::WebSocket(socket) => socket
                .send(Message::Text(json.to_string()))
                .expect("server send"),
        }
    }

    /// The next client message within `timeout`; `None` on silence.
    pub(super) fn next(&mut self, timeout: Duration) -> Option<Value> {
        let deadline = Instant::now() + timeout;
        loop {
            let slice = deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(50));
            if slice.is_zero() {
                return None;
            }
            match self {
                Self::Lines {
                    reader, partial, ..
                } => {
                    reader
                        .get_ref()
                        .set_read_timeout(Some(slice))
                        .expect("timeout");
                    match reader.read_line(partial) {
                        Ok(0) => return None,
                        Ok(_) if partial.ends_with('\n') => {
                            let line = std::mem::take(partial);
                            return Some(serde_json::from_str(&line).expect("client JSON"));
                        }
                        Ok(_) => {}
                        Err(error) if is_timeout(&error) => {}
                        Err(error) => panic!("server read: {error}"),
                    }
                }
                Self::WebSocket(socket) => {
                    socket
                        .get_ref()
                        .set_read_timeout(Some(slice))
                        .expect("timeout");
                    match socket.read() {
                        Ok(Message::Text(text)) => {
                            return Some(serde_json::from_str(&text).expect("client JSON"))
                        }
                        Ok(_) => {}
                        Err(tungstenite::Error::Io(error)) if is_timeout(&error) => {}
                        Err(error) => panic!("server read: {error}"),
                    }
                }
            }
        }
    }

    pub(super) fn expect(&mut self, kind: &str) -> Value {
        let message = self
            .next(Duration::from_secs(10))
            .unwrap_or_else(|| panic!("the client sent no {kind}"));
        assert_eq!(message["t"], kind, "{message}");
        message
    }

    /// Like `expect`, but presence republished meanwhile (a network change
    /// may give the endpoint new routes) is skipped.
    pub(super) fn expect_after_presence(&mut self, kind: &str) -> Value {
        loop {
            let message = self.expect_any();
            if message["t"] != "publish_direct" {
                assert_eq!(message["t"], kind, "{message}");
                return message;
            }
        }
    }

    pub(super) fn expect_any(&mut self) -> Value {
        self.next(Duration::from_secs(10))
            .unwrap_or_else(|| panic!("the client sent nothing"))
    }

    pub(super) fn expect_silence(&mut self, duration: Duration) {
        if let Some(message) = self.next(duration) {
            panic!("unexpected client message {message}");
        }
    }
}

fn is_timeout(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

pub(super) fn tcp_pair() -> (SignalConnection, FakeServer) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let client = TcpStream::connect(listener.local_addr().expect("address")).expect("connect");
    let (server, _) = listener.accept().expect("accept");
    let reader = BufReader::new(server.try_clone().expect("clone"));
    (
        SignalConnection::from_test_tcp(client).expect("client"),
        FakeServer::Lines {
            stream: server,
            reader,
            partial: String::new(),
        },
    )
}

pub(super) fn websocket_pair() -> (SignalConnection, FakeServer) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        tungstenite::accept(stream).expect("server handshake")
    });
    let client = TcpStream::connect(address).expect("connect");
    let (socket, _) =
        tungstenite::client(format!("ws://{address}/"), MaybeTlsStream::Plain(client))
            .expect("client handshake");
    let server = server.join().expect("server thread");
    (
        SignalConnection::from_test_websocket(socket),
        FakeServer::WebSocket(server),
    )
}
