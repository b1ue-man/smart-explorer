//! Scripted loopback Drive API, used only by the combined remote task suite.
use super::GDriveBackend;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub(crate) struct Request {
    pub(crate) method: String,
    pub(crate) target: String,
    headers: HashMap<String, String>,
    pub(crate) body: Vec<u8>,
}

pub(crate) enum Reply { Json(Value), Status(u16), HttpError(u16, Value), Session, Bytes(String) }
pub(crate) struct Step { method: &'static str, path: &'static str, reply: Reply }
struct Capture { requests: Vec<Request>, pending: usize, errors: Vec<String> }
pub(crate) struct Fixture {
    base: String,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<Capture>>,
}

impl Fixture {
    pub(crate) fn new(steps: Vec<Step>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server_base = base.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let shutdown = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            let mut steps = VecDeque::from(steps);
            let mut capture = Capture { requests: Vec::new(), pending: 0, errors: Vec::new() };
            let deadline = Instant::now() + Duration::from_secs(20);
            while !shutdown.load(Ordering::SeqCst) && Instant::now() < deadline {
                let mut stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => { capture.errors.push(error.to_string()); break; }
                };
                let result = (|| -> io::Result<()> {
                    let request = receive(&mut stream)?;
                    let step = steps.pop_front();
                    let reply = match step {
                        Some(step) if request.method == step.method
                            && request.target.split('?').next() == Some(step.path) => step.reply,
                        _ => {
                            capture.errors.push(format!("unexpected request: {request:?}"));
                            Reply::Status(400)
                        }
                    };
                    capture.requests.push(request);
                    let (status, headers, body) = match reply {
                        Reply::Bytes(body) => (200, String::new(), body),
                        Reply::Json(value) => (200, String::new(), value.to_string()),
                        Reply::Status(status) => (status, String::new(), "{}".into()),
                        Reply::HttpError(status, value) => (status, String::new(), value.to_string()),
                        Reply::Session => (200, format!("Location: {server_base}/session\r\n"), String::new()),
                    };
                    let response = format!(
                        "HTTP/1.1 {status} Fixture\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    stream.write_all(response.as_bytes())
                })();
                if let Err(error) = result { capture.errors.push(error.to_string()); }
            }
            capture.pending = steps.len();
            capture
        });
        Self { base, stop, worker: Some(worker) }
    }

    pub(crate) fn backend(&self) -> GDriveBackend {
        // Memory-only fake credentials and disabled persistent cache. All
        // metadata and upload URIs derive from this dynamically bound origin.
        GDriveBackend::test_backend(&format!("{}/drive/v3", self.base))
    }

    pub(crate) fn finish(mut self) -> Vec<Request> {
        self.stop.store(true, Ordering::SeqCst);
        let capture = self.worker.take().unwrap().join().unwrap();
        assert!(capture.errors.is_empty(), "{:?}", capture.errors);
        assert_eq!(capture.pending, 0, "expected requests were not sent");
        capture.requests
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Nonblocking accept and a total receive deadline bound unwind cleanup.
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}

fn receive(stream: &mut TcpStream) -> io::Result<Request> {
    stream.set_read_timeout(Some(Duration::from_millis(100)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut raw = Vec::new();
    while !raw.ends_with(b"\r\n\r\n") {
        if raw.len() >= 16 * 1024 { return Err(io::Error::other("oversized HTTP headers")); }
        let mut byte = [0];
        read_until(stream, &mut byte, deadline)?;
        raw.push(byte[0]);
    }
    let text = std::str::from_utf8(&raw).map_err(io::Error::other)?;
    let mut lines = text.lines();
    let mut start = lines.next().unwrap_or_default().split_whitespace();
    let method = start.next().unwrap_or_default().to_string();
    let target = start.next().unwrap_or_default().to_string();
    let headers: HashMap<_, _> = lines.filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_string())).collect();
    let length = headers.get("content-length").map(|value| value.parse::<usize>())
        .transpose().map_err(io::Error::other)?.unwrap_or(0);
    if length > 1024 * 1024 { return Err(io::Error::other("oversized fixture body")); }
    let mut body = vec![0; length];
    read_until(stream, &mut body, deadline)?;
    Ok(Request { method, target, headers, body })
}

fn read_until(stream: &mut TcpStream, mut target: &mut [u8], deadline: Instant) -> io::Result<()> {
    while !target.is_empty() {
        if Instant::now() >= deadline { return Err(io::Error::new(io::ErrorKind::TimedOut, "fixture receive deadline")); }
        match stream.read(target) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "incomplete HTTP request")),
            Ok(count) => { target = &mut target[count..]; }
            Err(error) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub(crate) fn step(method: &'static str, path: &'static str, reply: Reply) -> Step {
    Step { method, path, reply }
}

