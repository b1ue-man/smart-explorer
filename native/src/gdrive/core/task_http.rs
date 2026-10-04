//! Loopback HTTP server for the transfer-engine task tests: keep-alive
//! connections (counted), one thread per connection, answers from a handler.
use super::GDriveBackend;
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Clone, Debug)]
pub(super) struct Request {
    pub(super) method: String,
    pub(super) target: String,
    pub(super) headers: HashMap<String, String>,
    pub(super) body: Vec<u8>,
}

impl Request {
    pub(super) fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or_default()
    }

    /// A query parameter, percent-decoded.
    pub(super) fn query(&self, name: &str) -> Option<String> {
        let query = self.target.split_once('?')?.1;
        query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .find(|(key, _)| *key == name)
            .map(|(_, value)| decode(value))
    }

    pub(super) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

pub(super) struct Answer {
    pub(super) status: u16,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Vec<u8>,
}

impl Answer {
    pub(super) fn json(value: serde_json::Value) -> Self {
        Self::status(200, value)
    }

    pub(super) fn status(status: u16, value: serde_json::Value) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: value.to_string().into_bytes(),
        }
    }

    pub(super) fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    /// The request reached the server, but no acknowledgement reaches Drive.
    pub(super) fn disconnect() -> Self {
        Self { status: 0, headers: Vec::new(), body: Vec::new() }
    }
}

type Handler = dyn Fn(&Request) -> Answer + Send + Sync;

pub(super) struct Server {
    base: String,
    requests: Arc<Mutex<Vec<Request>>>,
    connections: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    acceptor: Option<JoinHandle<()>>,
    workers: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl Server {
    pub(super) fn start(handler: impl Fn(&Request) -> Answer + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handler: Arc<Handler> = Arc::new(handler);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let connections = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let workers = Arc::new(Mutex::new(Vec::new()));
        let acceptor = {
            let (requests, connections, stop) = (
                Arc::clone(&requests),
                Arc::clone(&connections),
                Arc::clone(&stop),
            );
            let workers = Arc::clone(&workers);
            thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            connections.fetch_add(1, Ordering::SeqCst);
                            let (handler, requests, stop) = (
                                Arc::clone(&handler),
                                Arc::clone(&requests),
                                Arc::clone(&stop),
                            );
                            workers.lock().unwrap().push(thread::spawn(move || {
                                serve(stream, &*handler, &requests, &stop)
                            }));
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                        }
                        Err(_) => break,
                    }
                }
            })
        };
        Self {
            base,
            requests,
            connections,
            stop,
            acceptor: Some(acceptor),
            workers,
        }
    }

    /// A test backend (memory-only cache, fake token) against this server.
    pub(super) fn backend(&self) -> GDriveBackend {
        GDriveBackend::test_backend_with_identity(&self.api_base(), Duration::from_secs(3),
            None, None, &self.base, "")
    }

    pub(super) fn api_base(&self) -> String { format!("{}/drive/v3", self.base) }
    pub(super) fn url(&self, path: &str) -> String { format!("{}{}", self.base, path) }

    pub(super) fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }

    pub(super) fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(acceptor) = self.acceptor.take() {
            let _ = acceptor.join();
        }
        let mut failed = false;
        for worker in self.workers.lock().unwrap().drain(..) { failed |= worker.join().is_err(); }
        assert!(!failed || thread::panicking(), "fixture connection worker failed");
    }
}

/// Answer requests on one connection until the client closes it or the
/// server stops (checked while the connection idles in the client's pool).
fn serve(stream: TcpStream, handler: &Handler, requests: &Mutex<Vec<Request>>, stop: &AtomicBool) {
    let _ = stream.set_nonblocking(false);
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    loop {
        let _ = reader
            .get_ref()
            .set_read_timeout(Some(Duration::from_millis(50)));
        match reader.fill_buf() {
            Ok([]) => return,
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                if stop.load(Ordering::SeqCst) {
                    return;
                }
                continue;
            }
            Err(_) => return,
        }
        let _ = reader
            .get_ref()
            .set_read_timeout(Some(Duration::from_secs(10)));
        let Ok(request) = read_request(&mut reader) else {
            return;
        };
        let answer = handler(&request);
        requests.lock().unwrap().push(request);
        if answer.status == 0 { return; }
        let mut head = format!("HTTP/1.1 {} Fixture\r\n", answer.status);
        for (name, value) in &answer.headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str(&format!("Content-Length: {}\r\n\r\n", answer.body.len()));
        if writer.write_all(head.as_bytes()).is_err() || writer.write_all(&answer.body).is_err() {
            return;
        }
    }
}

fn read_request(reader: &mut BufReader<TcpStream>) -> io::Result<Request> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut start = line.split_whitespace();
    let method = start.next().unwrap_or_default().to_string();
    let target = start.next().unwrap_or_default().to_string();
    let mut headers = HashMap::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Request {
        method,
        target,
        headers,
        body,
    })
}

/// Percent-decoding of `cloud_urlenc` output.
pub(super) fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = bytes
            .get(index + 1..index + 3)
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match (bytes[index], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                index += 3;
            }
            (byte, _) => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The metadata JSON and the media bytes of a `multipart/related` body.
pub(super) fn multipart(request: &Request) -> (serde_json::Value, Vec<u8>) {
    let boundary = request
        .header("content-type")
        .and_then(|value| value.split_once("boundary="))
        .map(|(_, boundary)| boundary.to_string())
        .unwrap();
    let body = &request.body;
    let head = format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n");
    let tail = format!("\r\n--{boundary}--\r\n");
    assert!(
        body.starts_with(head.as_bytes()),
        "multipart starts with the metadata part"
    );
    assert!(
        body.ends_with(tail.as_bytes()),
        "multipart ends with the closing boundary"
    );
    let rest = &body[head.len()..body.len() - tail.len()];
    let separator = format!("\r\n--{boundary}\r\nContent-Type: application/octet-stream\r\n\r\n");
    let split = rest
        .windows(separator.len())
        .position(|window| window == separator.as_bytes())
        .unwrap();
    let metadata = serde_json::from_slice(&rest[..split]).unwrap();
    (metadata, rest[split + separator.len()..].to_vec())
}

pub(super) fn query_literal(text: &str) -> String {
    let mut result = String::new();
    let mut escaped = false;
    for c in text.chars() {
        if escaped { result.push(c); escaped = false; }
        else if c == '\\' { escaped = true; }
        else if c == '\'' { break; }
        else { result.push(c); }
    }
    result
}

pub(super) fn list_page(request: &Request, mut files: Vec<serde_json::Value>, configured: usize) -> Answer {
    let query = request.query("q").unwrap_or_default();
    if let Some(rest) = query.split(" and name = '").nth(1) {
        let name = query_literal(rest).to_lowercase();
        files.retain(|file| file["name"].as_str().unwrap().to_lowercase() == name);
    }
    let requested = request.query("pageSize").and_then(|n| n.parse().ok()).unwrap_or(100);
    let size = if configured == 0 { requested } else { requested.min(configured) }.max(1);
    let start = match request.query("pageToken") {
        None => 0,
        Some(token) => match token.strip_prefix("offset:").and_then(|n| n.parse::<usize>().ok()) {
            Some(start) if start <= files.len() => start,
            _ => return Answer::status(400, serde_json::json!({"error":{"message":"page token rejected","errors":[{"reason":"invalid"}]}})),
        },
    };
    let end = start.saturating_add(size).min(files.len());
    let mut page = serde_json::json!({"files": files[start..end], "incompleteSearch": false});
    if end < files.len() { page["nextPageToken"] = serde_json::json!(format!("offset:{end}")); }
    Answer::json(page)
}
