//! Transfer-engine task tests for the WebDAV backend against a scripted
//! keep-alive HTTP server on loopback: the streaming stage PUT with its exact
//! length, pooled mutations (and an unpooled DELETE), overload as congestion,
//! one-request folders, ranged resumes and the server-side COPY.
use super::status::{range_start, retry_after};
use super::{WebdavBackend, WebdavConfig};
use crate::vfs::{congestion_of, Backend};
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

struct Seen {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
    connection: usize,
}

struct Answer {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
}

impl Answer {
    fn status(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    fn with(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }

    fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }
}

fn propfind(path: &str, size: Option<u64>) -> Answer {
    let props = match size {
        Some(size) => format!("<d:resourcetype/><d:getcontentlength>{size}</d:getcontentlength>"),
        None => "<d:resourcetype><d:collection/></d:resourcetype>".to_string(),
    };
    Answer::status(207).body(format!(
        r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:"><d:response><d:href>{path}</d:href><d:propstat><d:prop>{props}</d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#
    ))
}

type Route = dyn Fn(&Seen) -> Answer + Send + Sync;

struct Http {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    seen: Arc<Mutex<Vec<Seen>>>,
    worker: Option<JoinHandle<()>>,
}

fn lock(seen: &Mutex<Vec<Seen>>) -> MutexGuard<'_, Vec<Seen>> {
    seen.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Http {
    fn start(route: impl Fn(&Seen) -> Answer + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let route: Arc<Route> = Arc::new(route);
        let worker = {
            let (stop, seen) = (stop.clone(), seen.clone());
            thread::spawn(move || {
                let mut handlers = Vec::new();
                let deadline = Instant::now() + Duration::from_secs(30);
                let mut connection = 0;
                while !stop.load(Ordering::SeqCst) && Instant::now() < deadline {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let (route, seen) = (route.clone(), seen.clone());
                            let index = connection;
                            connection += 1;
                            handlers.push(thread::spawn(move || {
                                handle(stream, index, route.as_ref(), &seen)
                            }));
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                        }
                        Err(_) => break,
                    }
                }
                for handler in handlers {
                    let _ = handler.join();
                }
            })
        };
        Self {
            address,
            stop,
            seen,
            worker: Some(worker),
        }
    }

    fn backend(&self) -> WebdavBackend {
        WebdavBackend::connect(WebdavConfig {
            https: false,
            host: self.address.ip().to_string(),
            port: self.address.port(),
            user: String::new(),
            password: String::new(),
            root: "/".into(),
        })
        .unwrap()
    }

    /// Connection index of every complete request `method path`.
    fn connections(&self, method: &str, path: &str) -> Vec<usize> {
        lock(&self.seen)
            .iter()
            .filter(|seen| seen.method == method && seen.path == path)
            .map(|seen| seen.connection)
            .collect()
    }

    fn request(&self, method: &str, path: &str) -> Option<(HashMap<String, String>, Vec<u8>)> {
        lock(&self.seen)
            .iter()
            .find(|seen| seen.method == method && seen.path == path)
            .map(|seen| (seen.headers.clone(), seen.body.clone()))
    }
}

impl Drop for Http {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn read_request(reader: &mut BufReader<TcpStream>, connection: usize) -> Option<Seen> {
    let mut start = String::new();
    if reader.read_line(&mut start).ok()? == 0 {
        return None;
    }
    let mut parts = start.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let mut headers = HashMap::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end_matches(['\r', '\n']);
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
    reader.read_exact(&mut body).ok()?;
    Some(Seen {
        method,
        path,
        headers,
        body,
        connection,
    })
}

fn handle(stream: TcpStream, connection: usize, route: &Route, seen: &Mutex<Vec<Seen>>) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let Ok(read_half) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(read_half);
    let mut writer = stream;
    while let Some(request) = read_request(&mut reader, connection) {
        let answer = route(&request);
        lock(seen).push(request);
        let mut head = format!(
            "HTTP/1.1 {} Scripted\r\nContent-Length: {}\r\nConnection: keep-alive\r\n",
            answer.status,
            answer.body.len()
        );
        for (name, value) in &answer.headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("\r\n");
        if writer.write_all(head.as_bytes()).is_err() || writer.write_all(&answer.body).is_err() {
            return;
        }
        let _ = writer.flush();
    }
}

#[test]
fn transfer_engine_task_webdav_stage_put_streams_the_exact_length() {
    let http = Http::start(|seen| match seen.method.as_str() {
        "PROPFIND" => propfind(&seen.path, None),
        "PUT" => Answer::status(201),
        _ => Answer::status(500),
    });
    let backend = http.backend();

    let mut writer = backend.open_write_copy_stage_sized("/stage", 11).unwrap();
    writer.write_all(b"hello ").unwrap();
    writer.write_all(b"world").unwrap();
    writer.flush().unwrap();
    writer.flush().unwrap();
    assert!(writer.write_all(b"late").is_err());
    drop(writer);
    let (headers, body) = http.request("PUT", "/stage").expect("the PUT arrived");
    assert_eq!(body, b"hello world");
    assert_eq!(
        headers.get("content-length").map(String::as_str),
        Some("11")
    );
    assert_eq!(headers.get("if-none-match").map(String::as_str), Some("*"));
    assert!(!headers.contains_key("transfer-encoding"));

    let mut short = backend.open_write_copy_stage_sized("/short", 10).unwrap();
    short.write_all(b"abc").unwrap();
    assert_eq!(
        short.flush().unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    drop(short);

    let mut long = backend.open_write_copy_stage_sized("/long", 2).unwrap();
    assert_eq!(
        long.write_all(b"abc").unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    drop(long);
    drop(backend);
    // A body that ended early never completes its PUT.
    assert!(http.request("PUT", "/short").is_none());
    assert!(http.request("PUT", "/long").is_none());
}

#[test]
fn transfer_engine_task_webdav_mutations_reuse_pooled_connections() {
    let http = Http::start(|seen| match seen.method.as_str() {
        "PROPFIND" => propfind(&seen.path, None),
        "MKCOL" | "MOVE" => Answer::status(201),
        "DELETE" => Answer::status(204),
        _ => Answer::status(500),
    });
    let backend = http.backend();
    backend.create_dir("/a").unwrap();
    backend.create_dir_new("/b").unwrap();
    backend.rename("/a", "/c").unwrap();
    backend.remove_file("/c").unwrap();
    backend.remove_file("/b").unwrap();
    drop(backend);

    let first = http.connections("MKCOL", "/a");
    let second = http.connections("MKCOL", "/b");
    let moved = http.connections("MOVE", "/a");
    assert_eq!(first.len(), 1);
    assert_eq!(first, second, "MKCOL reuses the pooled connection");
    assert_eq!(first, moved, "MOVE reuses the pooled connection");
    // DELETE is replayed by ureq on a recycled socket, so it never pools.
    let deletes = [
        http.connections("DELETE", "/c"),
        http.connections("DELETE", "/b"),
    ];
    assert_ne!(deletes[0], deletes[1]);
    assert!(!deletes.iter().any(|delete| delete == &first));
}

#[test]
fn transfer_engine_task_webdav_overload_is_congestion_with_retry_after() {
    let http = Http::start(|seen| match (seen.method.as_str(), seen.path.as_str()) {
        ("PROPFIND", path) => propfind(path, None),
        ("MKCOL", "/busy") => Answer::status(503).with("Retry-After", "7"),
        ("MKCOL", "/limited") => Answer::status(429),
        ("MKCOL", "/full") => Answer::status(507),
        _ => Answer::status(500),
    });
    let backend = http.backend();

    let busy = backend.create_dir("/busy").unwrap_err();
    let congestion = congestion_of(&busy).expect("503 is congestion");
    assert_eq!(congestion.retry_after, Some(Duration::from_secs(7)));
    let limited = backend.create_dir_new("/limited").unwrap_err();
    assert_eq!(
        congestion_of(&limited).map(|congestion| congestion.retry_after),
        Some(None)
    );
    let full = backend.create_dir("/full").unwrap_err();
    assert!(congestion_of(&full).is_none());
    assert_eq!(full.kind(), io::ErrorKind::StorageFull);
    drop(backend);

    assert_eq!(retry_after(" 120 "), Some(Duration::from_secs(120)));
    assert_eq!(
        retry_after("Wed, 21 Oct 2015 07:28:00 GMT"),
        Some(Duration::ZERO)
    );
    assert_eq!(retry_after("soon"), None);
}

#[test]
fn transfer_engine_task_webdav_folders_take_one_mkcol() {
    let http = Http::start(|seen| match (seen.method.as_str(), seen.path.as_str()) {
        ("PROPFIND", "/file") => propfind("/file", Some(1)),
        ("PROPFIND", path) => propfind(path, None),
        ("MKCOL", "/exists") | ("MKCOL", "/file") => Answer::status(405),
        ("MKCOL", _) => Answer::status(201),
        _ => Answer::status(500),
    });
    let backend = http.backend();
    backend.create_dir("/new").unwrap();
    backend.create_dir_new("/fresh").unwrap();
    backend.create_dir("/exists").unwrap();
    assert_eq!(
        backend.create_dir_new("/exists").unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(
        backend.create_dir("/file").unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    drop(backend);
    // A free name costs exactly one request.
    assert_eq!(http.connections("PROPFIND", "/new").len(), 0);
    assert_eq!(http.connections("MKCOL", "/new").len(), 1);
}

#[test]
fn transfer_engine_task_webdav_resume_sends_a_range() {
    let content: Vec<u8> = (0..100u8).collect();
    let served = content.clone();
    let http = Http::start(move |seen| match seen.method.as_str() {
        "PROPFIND" => propfind(&seen.path, None),
        "GET" => match seen.headers.get("range").map(String::as_str) {
            Some("bytes=40-") => Answer::status(206)
                .with("Content-Range", "bytes 40-99/100")
                .body(served[40..].to_vec()),
            Some("bytes=500-") => Answer::status(416),
            _ => Answer::status(200).body(served.clone()),
        },
        _ => Answer::status(500),
    });
    let backend = http.backend();
    let mut resumed = backend
        .open_read_at("/data", None, 40)
        .unwrap()
        .expect("the server answers the range");
    let mut out = Vec::new();
    resumed.read_to_end(&mut out).unwrap();
    assert_eq!(out, content[40..]);
    drop(resumed);
    assert_eq!(
        backend
            .open_read_at("/data", None, 500)
            .err()
            .map(|error| error.kind()),
        Some(io::ErrorKind::InvalidData)
    );
    drop(backend);
    assert_eq!(range_start("bytes 40-99/100"), Some(40));
    assert_eq!(range_start("items 1-2/3"), None);

    let whole = Http::start(move |seen| match seen.method.as_str() {
        "PROPFIND" => propfind(&seen.path, None),
        _ => Answer::status(200).body(content.clone()),
    });
    let backend = whole.backend();
    assert!(backend.open_read_at("/data", None, 40).unwrap().is_none());
    drop(backend);
}

#[test]
fn transfer_engine_task_webdav_copy_into_the_stage_stays_on_the_server() {
    let http = Http::start(|seen| match (seen.method.as_str(), seen.path.as_str()) {
        ("PROPFIND", "/stage") => propfind("/stage", Some(5)),
        ("PROPFIND", path) => propfind(path, None),
        ("COPY", "/src") => Answer::status(201),
        ("COPY", _) => Answer::status(501),
        _ => Answer::status(500),
    });
    let backend = http.backend();
    assert_eq!(
        backend.server_copy_to_stage("/src", "/stage", 5).unwrap(),
        Some(5)
    );
    assert_eq!(
        backend
            .server_copy_to_stage("/other", "/stage2", 5)
            .unwrap(),
        None
    );
    drop(backend);
    let (headers, _) = http.request("COPY", "/src").expect("the COPY arrived");
    assert!(headers
        .get("destination")
        .is_some_and(|destination| destination.ends_with("/stage")));
    assert_eq!(headers.get("overwrite").map(String::as_str), Some("F"));
}
