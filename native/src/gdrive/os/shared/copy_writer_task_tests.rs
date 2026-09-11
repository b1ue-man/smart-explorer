use super::GDriveBackend;
use crate::vfs::Backend;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const FILES: &str = "/drive/v3/files";
const OWN: &str = "/drive/v3/files/owned-id";
const GENERATE: &str = "/drive/v3/files/generateIds";
const UPLOAD: &str = "/upload/drive/v3/files";
const STAGE: &str = "stage.txt";

fn require_task() {
    assert_eq!(std::env::var("SMART_EXPLORER_COPY_PASTE_TASK").as_deref(), Ok("1"),
        "requires the isolated copy/paste task runner");
}

#[derive(Debug)]
struct Request {
    method: String,
    target: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

enum Reply { Json(Value), Status(u16), Session, Lost }
struct Step { method: &'static str, path: &'static str, reply: Reply }
struct Capture { requests: Vec<Request>, pending: usize, errors: Vec<String> }
struct Fixture {
    base: String,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<Capture>>,
}

impl Fixture {
    fn new(steps: Vec<Step>) -> Self {
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
                        Reply::Lost => return Ok(()), // Close without an acknowledgement.
                        Reply::Json(value) => (200, String::new(), value.to_string()),
                        Reply::Status(status) => (status, String::new(), "{}".into()),
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

    fn backend(&self) -> GDriveBackend {
        // Memory-only fake credentials and disabled persistent cache. All
        // metadata and upload URIs derive from this dynamically bound origin.
        GDriveBackend::test_backend(&format!("{}/drive/v3", self.base))
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

fn step(method: &'static str, path: &'static str, reply: Reply) -> Step {
    Step { method, path, reply }
}

fn object(id: &str) -> Value {
    json!({"id": id, "name": STAGE, "parents": ["root"], "trashed": false,
        "mimeType": "application/octet-stream", "size": "3",
        "md5Checksum": "900150983cd24fb0d6963f7d28e17f72"})
}

fn list(objects: Vec<Value>) -> Step {
    step("GET", FILES, Reply::Json(json!({"files": objects})))
}

fn preflight() -> Vec<Step> {
    vec![list(vec![]), step("GET", GENERATE, Reply::Json(json!({"ids": ["owned-id"]})))]
}

fn upload_steps() -> Vec<Step> {
    let mut steps = preflight();
    steps.push(step("POST", UPLOAD, Reply::Session));
    steps.push(step("PUT", "/session", Reply::Json(json!({"id": "owned-id"}))));
    steps
}

fn assert_owned_create(requests: &[Request], put_count: usize) {
    assert!(requests.iter().all(|request| matches!(request.method.as_str(), "GET" | "POST" | "PUT")),
        "no PATCH, DELETE or other mutation is allowed: {requests:?}");
    let posts: Vec<_> = requests.iter().filter(|request| request.method == "POST").collect();
    assert_eq!(posts.len(), 1, "create must not be replayed");
    assert_eq!(posts[0].target, format!("{UPLOAD}?uploadType=resumable&fields=id"));
    let metadata: Value = serde_json::from_slice(&posts[0].body).unwrap();
    assert_eq!(metadata["id"], "owned-id");
    assert_eq!(metadata["name"], STAGE);
    assert_eq!(metadata["parents"], json!(["root"]));
    assert_eq!(metadata["mimeType"], "application/octet-stream");
    assert_eq!(requests.iter().filter(|request| request.target.starts_with(GENERATE)).count(), 1);
    let puts: Vec<_> = requests.iter().filter(|request| request.method == "PUT").collect();
    assert_eq!(puts.len(), put_count);
    assert!(puts.iter().all(|request| request.target == "/session"));
    assert!(requests.iter().all(|request| request.headers.get("authorization").map(String::as_str) == Some("Bearer test-token")));
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_provider_drive_create_ignores_stale_path_and_pending_ids() {
    require_task();
    let mut steps = upload_steps();
    steps.extend([step("GET", OWN, Reply::Json(object("owned-id"))), list(vec![object("owned-id")])]);
    let fixture = Fixture::new(steps);
    let backend = fixture.backend();
    backend.remember_path(STAGE, "foreign-cache", None).unwrap();
    backend.pending_upload_ids_guard().unwrap().insert(STAGE.into(), "foreign-pending".into());
    assert!(matches!(backend.open_write_new(STAGE), Err(error) if error.kind() == io::ErrorKind::Unsupported));
    let mut writer = backend.open_write_copy_stage(STAGE).unwrap();
    writer.write_all(b"abc").unwrap();
    writer.flush().unwrap();
    writer.flush().unwrap();
    assert!(writer.write(b"late").is_err());
    drop(writer);
    assert_eq!(backend.cached_id(STAGE).unwrap().as_deref(), Some("owned-id"));
    assert_eq!(backend.pending_upload_ids_guard().unwrap().get(STAGE).map(String::as_str), Some("foreign-pending"));
    let requests = fixture.finish();
    assert_owned_create(&requests, 1);
    let put = requests.iter().find(|request| request.method == "PUT").unwrap();
    assert_eq!(put.body, b"abc");
    assert_eq!(put.headers.get("content-range").map(String::as_str), Some("bytes 0-2/3"));
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_provider_drive_occupied_or_duplicate_stage_never_mutates() {
    require_task();
    for objects in [vec![object("foreign")], vec![object("foreign"), object("another")]] {
        let fixture = Fixture::new(vec![list(objects)]);
        let backend = fixture.backend();
        let mut writer = backend.open_write_copy_stage(STAGE).unwrap();
        writer.write_all(b"abc").unwrap();
        assert_eq!(writer.flush().unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        drop(writer);
        let requests = fixture.finish();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "GET");
    }
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_provider_drive_concurrent_name_collision_never_adopts_or_replays() {
    require_task();
    for objects in [vec![object("foreign")], vec![object("owned-id"), object("foreign")]] {
        let mut steps = upload_steps();
        for _ in 0..2 {
            steps.extend([step("GET", OWN, Reply::Json(object("owned-id"))), list(objects.clone())]);
        }
        let fixture = Fixture::new(steps);
        let backend = fixture.backend();
        let mut writer = backend.open_write_copy_stage(STAGE).unwrap();
        writer.write_all(b"abc").unwrap();
        assert_eq!(writer.flush().unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(writer.flush().unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert!(writer.write(b"late").is_err());
        drop(writer);
        assert_eq!(backend.cached_id(STAGE).unwrap(), None);
        assert_owned_create(&fixture.finish(), 1);
    }
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_provider_drive_409_reconciles_only_owned_identity() {
    require_task();
    for correct_identity in [true, false] {
        let mut steps = preflight();
        steps.push(step("POST", UPLOAD, Reply::Status(409)));
        let metadata = object(if correct_identity { "owned-id" } else { "foreign" });
        steps.push(step("GET", OWN, Reply::Json(metadata.clone())));
        if correct_identity { steps.push(list(vec![object("owned-id")])); }
        else { steps.push(step("GET", OWN, Reply::Json(metadata))); }
        let fixture = Fixture::new(steps);
        let backend = fixture.backend();
        let mut writer = backend.open_write_copy_stage(STAGE).unwrap();
        writer.write_all(b"abc").unwrap();
        if correct_identity {
            writer.flush().unwrap();
            writer.flush().unwrap();
        } else {
            assert_eq!(writer.flush().unwrap_err().kind(), io::ErrorKind::InvalidData);
            assert_eq!(writer.flush().unwrap_err().kind(), io::ErrorKind::InvalidData);
        }
        drop(writer);
        assert_owned_create(&fixture.finish(), 0);
    }
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_provider_drive_lost_upload_ack_reconciles_own_content() {
    require_task();
    let mut steps = preflight();
    steps.extend([
        step("POST", UPLOAD, Reply::Session), step("PUT", "/session", Reply::Lost),
        step("PUT", "/session", Reply::Status(404)),
        step("GET", OWN, Reply::Json(object("owned-id"))), list(vec![object("owned-id")]),
    ]);
    let fixture = Fixture::new(steps);
    let backend = fixture.backend();
    let mut writer = backend.open_write_copy_stage(STAGE).unwrap();
    writer.write_all(b"abc").unwrap();
    writer.flush().unwrap();
    writer.flush().unwrap();
    drop(writer);
    let requests = fixture.finish();
    assert_owned_create(&requests, 2);
    let puts: Vec<_> = requests.iter().filter(|request| request.method == "PUT").collect();
    assert_eq!(puts[0].body, b"abc");
    assert!(puts[1].body.is_empty(), "after ACK loss, query rather than replay media");
    assert_eq!(puts[1].headers.get("content-range").map(String::as_str), Some("bytes */3"));
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_provider_drive_wrong_metadata_keeps_pending_read_only() {
    require_task();
    for (field, wrong) in [
        ("name", json!("moved.txt")), ("parents", json!(["other-parent"])),
        ("size", json!("4")), ("md5Checksum", json!("different-content")),
        ("trashed", json!(true)), ("mimeType", json!("application/vnd.google-apps.document")),
    ] {
        let mut metadata = object("owned-id");
        metadata[field] = wrong;
        let mut steps = upload_steps();
        steps.extend([step("GET", OWN, Reply::Json(metadata.clone())), step("GET", OWN, Reply::Json(metadata))]);
        let fixture = Fixture::new(steps);
        let backend = fixture.backend();
        let mut writer = backend.open_write_copy_stage(STAGE).unwrap();
        writer.write_all(b"abc").unwrap();
        assert_eq!(writer.flush().unwrap_err().kind(), io::ErrorKind::InvalidData, "{field}");
        assert_eq!(writer.flush().unwrap_err().kind(), io::ErrorKind::InvalidData, "{field}");
        assert!(writer.write(b"changed payload").is_err());
        drop(writer);
        assert_eq!(backend.cached_id(STAGE).unwrap(), None);
        assert_owned_create(&fixture.finish(), 1);
    }
}
