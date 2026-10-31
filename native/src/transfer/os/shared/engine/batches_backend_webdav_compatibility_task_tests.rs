//! The WebDAV side of the backend compatibility contract: an ordinary folder
//! move needs no binding extension, and the reserved (rebinding) move keeps its
//! strict boundary through the cache. Split from
//! batches_backend_compatibility_task_tests.rs to keep both files small.
use crate::vfs::Backend;
use std::{
    io::{self, Read, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

#[derive(Clone)]
struct HttpRequest {
    method: String,
    path: String,
    headers: std::collections::HashMap<String, String>,
}
struct Dav {
    address: std::net::SocketAddr,
    requests: Arc<std::sync::Mutex<Vec<HttpRequest>>>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Dav {
    fn new(resource_id: bool, rebind_status: u16) -> Self {
        use std::io::BufRead;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (quit, seen) = (stop.clone(), requests.clone());
        let worker = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while !quit.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
                let (stream, _) = match listener.accept() {
                    Ok(stream) => stream,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(2));
                        continue;
                    }
                    Err(_) => return,
                };
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(stream);
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let mut words = line.split_whitespace();
                let method = words.next().unwrap_or("").to_owned();
                let path = words.next().unwrap_or("/").to_owned();
                let mut headers = std::collections::HashMap::new();
                loop {
                    line.clear();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':') {
                        headers.insert(name.to_ascii_lowercase(), value.trim().to_string());
                    }
                }
                let length = headers
                    .get("content-length")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0usize);
                let mut body = vec![0; length];
                if reader.read_exact(&mut body).is_err() {
                    continue;
                }
                seen.lock().unwrap().push(HttpRequest {
                    method: method.clone(),
                    path: path.clone(),
                    headers,
                });
                let status = match method.as_str() {
                    "PROPFIND" => 207,
                    "MOVE" => 201,
                    "REBIND" => rebind_status,
                    _ => 405,
                };
                let properties = if resource_id {
                    "<d:resource-id><d:href>urn:uuid:own-directory</d:href></d:resource-id>"
                } else {
                    ""
                };
                let body = if status == 207 {
                    format!(
                        r#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>{path}</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype>{properties}</d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#
                    )
                } else {
                    String::new()
                };
                let response = format!("HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = reader.get_mut().write_all(response.as_bytes());
            }
        });
        Self {
            address,
            requests,
            stop,
            worker: Some(worker),
        }
    }
    fn backend(&self) -> crate::webdav::WebdavBackend {
        crate::webdav::WebdavBackend::connect(crate::webdav::WebdavConfig {
            https: false,
            host: self.address.ip().to_string(),
            port: self.address.port(),
            user: String::new(),
            password: String::new(),
            root: "/".into(),
        })
        .unwrap()
    }
}
impl Drop for Dav {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

#[test]
fn batches_task_transfer_compat_webdav_ordinary_folder_move_needs_no_binding_extension() {
    let server = Dav::new(false, 405);
    let backend = server.backend();
    backend.rename_no_replace("/old", "/new").unwrap();
    let requests = server.requests.lock().unwrap();
    let moved = requests
        .iter()
        .find(|request| request.method == "MOVE")
        .unwrap();
    assert_eq!(moved.path, "/old");
    assert_eq!(
        moved.headers.get("overwrite").map(String::as_str),
        Some("F")
    );
    assert!(moved.headers.get("destination").unwrap().ends_with("/new"));
    assert!(!requests.iter().any(|request| request.method == "REBIND"));
}

#[test]
fn batches_task_transfer_compat_webdav_reserved_move_keeps_strict_rebind_boundary_through_cache() {
    for (id, status, expected_rebind) in [(false, 405, 0), (true, 405, 1), (true, 201, 1)] {
        let server = Dav::new(id, status);
        let backend = crate::vfs::CachingBackend::new(Arc::new(server.backend()));
        let result = crate::vfs::rename_reserved_directory(&backend, "/owned", "/new");
        if id && status == 201 {
            assert!(result.is_ok());
        } else {
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Unsupported);
        }
        let requests = server.requests.lock().unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.method == "REBIND")
                .count(),
            expected_rebind
        );
        assert!(!requests.iter().any(|request| request.method == "MOVE"));
    }
}
