use crate::vfs::Backend;
use crate::webdav::{WebdavBackend, WebdavConfig};
use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

fn require_task() {
    assert_eq!(std::env::var("SMART_EXPLORER_COPY_PASTE_TASK").as_deref(), Ok("1"),
        "requires the isolated copy/paste task runner");
}

#[derive(Debug)]
struct Request {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

struct Capture {
    requests: Vec<Request>,
    pending: usize,
    errors: Vec<String>,
}

struct Fixture {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<Capture>>,
}

impl Fixture {
    fn new(put_status: u16) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let shutdown = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            let mut steps = VecDeque::from([("PROPFIND", "/", 207), ("PUT", "/stage", put_status)]);
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
                    let status = match step {
                        Some((method, path, status)) if request.method == method && request.path == path => status,
                        _ => {
                            capture.errors.push(format!("unexpected request: {request:?}"));
                            400
                        }
                    };
                    capture.requests.push(request);
                    let body = if status == 207 {
                        r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:"></d:multistatus>"#
                    } else { "" };
                    let response = format!(
                        "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    stream.write_all(response.as_bytes())
                })();
                if let Err(error) = result { capture.errors.push(error.to_string()); }
            }
            capture.pending = steps.len();
            capture
        });
        Self { address, stop, worker: Some(worker) }
    }

    fn backend(&self) -> WebdavBackend {
        WebdavBackend::connect(WebdavConfig {
            https: false, host: self.address.ip().to_string(), port: self.address.port(),
            user: String::new(), password: String::new(), root: "/".into(),
        }).unwrap()
    }

    fn finish(mut self) -> Vec<Request> {
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
        // Nonblocking accept and a total receive deadline bound this join,
        // including when the test unwinds before finish(). No detached worker.
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
    let path = start.next().unwrap_or_default().to_string();
    let headers: HashMap<_, _> = lines.filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_string())).collect();
    let length = headers.get("content-length").map(|value| value.parse::<usize>())
        .transpose().map_err(io::Error::other)?.unwrap_or(0);
    if length > 1024 * 1024 { return Err(io::Error::other("oversized fixture body")); }
    let mut body = vec![0; length];
    read_until(stream, &mut body, deadline)?;
    Ok(Request { method, path, headers, body })
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

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_provider_webdav_conditional_create_and_abort() {
    require_task();
    let fixture = Fixture::new(201);
    let backend = fixture.backend();
    {
        let mut aborted = backend.open_write_new("/aborted").unwrap();
        aborted.write_all(b"unpublished").unwrap();
    }
    let mut writer = backend.open_write_copy_stage("/stage").unwrap();
    writer.write_all(b"payload").unwrap();
    writer.flush().unwrap();
    writer.flush().unwrap();
    assert!(writer.write_all(b"late").is_err());
    drop(writer);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].headers.get("if-none-match").map(String::as_str), Some("*"));
    assert_eq!(requests[1].body, b"payload");
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_provider_webdav_conflict_kind_survives_repeat_flush() {
    require_task();
    let fixture = Fixture::new(412);
    let backend = fixture.backend();
    let mut writer = backend.open_write_new("/stage").unwrap();
    writer.write_all(b"must not replace").unwrap();
    assert_eq!(writer.flush().unwrap_err().kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(writer.flush().unwrap_err().kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(writer.write(b"late").unwrap_err().kind(), io::ErrorKind::AlreadyExists);
    drop(writer);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2, "no retry or DELETE after a conflict");
    assert_eq!(requests[1].headers.get("if-none-match").map(String::as_str), Some("*"));
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_provider_webdav_edit_put_remains_unconditional() {
    require_task();
    let fixture = Fixture::new(204);
    let backend = fixture.backend();
    let mut writer = backend.open_write("/stage").unwrap();
    writer.write_all(b"edited").unwrap();
    writer.flush().unwrap();
    writer.flush().unwrap();
    drop(writer);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    assert!(!requests[1].headers.contains_key("if-none-match"));
    assert_eq!(requests[1].body, b"edited");
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_provider_webdav_pending_status_is_not_commit() {
    require_task();
    let fixture = Fixture::new(202);
    let backend = fixture.backend();
    let mut writer = backend.open_write_new("/stage").unwrap();
    writer.write_all(b"pending").unwrap();
    assert!(writer.flush().is_err());
    assert!(writer.flush().is_err());
    drop(writer);
    assert_eq!(fixture.finish().len(), 2);
}
