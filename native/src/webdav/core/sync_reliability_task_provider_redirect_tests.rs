//! Owned HTTP fixtures for PROPFIND-only collection canonicalization.
use super::{WebdavBackend, WebdavConfig};
use crate::vfs::Backend;
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
struct Request {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

struct Http {
    port: u16,
    stop: Arc<AtomicBool>,
    seen: Arc<Mutex<Vec<Request>>>,
    worker: Option<JoinHandle<()>>,
}

impl Http {
    fn start(route: impl Fn(&Request) -> (u16, Option<String>, String) + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let worker = {
            let stop = stop.clone();
            let seen = seen.clone();
            thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(30);
                while !stop.load(Ordering::SeqCst) && Instant::now() < deadline {
                    let stream = match listener.accept() {
                        Ok((stream, _)) => stream,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                            continue;
                        }
                        Err(error) => panic!("owned HTTP accept: {error}"),
                    };
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    stream
                        .set_write_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap() == 0 {
                        continue;
                    }
                    let mut words = line.split_whitespace();
                    let method = words.next().unwrap().to_string();
                    let path = words.next().unwrap().to_string();
                    let mut headers = HashMap::new();
                    loop {
                        line.clear();
                        reader.read_line(&mut line).unwrap();
                        if line == "\r\n" {
                            break;
                        }
                        let (name, value) = line.split_once(':').unwrap();
                        headers.insert(name.to_ascii_lowercase(), value.trim().to_string());
                    }
                    let length = headers
                        .get("content-length")
                        .map(|value| value.parse::<usize>().unwrap())
                        .unwrap_or(0);
                    let mut body = vec![0; length];
                    reader.read_exact(&mut body).unwrap();
                    let request = Request {
                        method,
                        path,
                        headers,
                        body,
                    };
                    let (status, location, body) = route(&request);
                    seen.lock().unwrap().push(request);
                    let location = location
                        .map(|value| format!("Location: {value}\r\n"))
                        .unwrap_or_default();
                    write!(
                        reader.get_mut(),
                        "HTTP/1.1 {status} Fixture\r\n{location}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .unwrap();
                }
            })
        };
        Self {
            port,
            stop,
            seen,
            worker: Some(worker),
        }
    }

    fn connect(&self, root: &str) -> io::Result<WebdavBackend> {
        WebdavBackend::connect(WebdavConfig {
            https: false,
            host: "127.0.0.1".into(),
            port: self.port,
            user: "fixture".into(),
            password: "owned-only".into(),
            root: root.into(),
        })
    }

    fn requests(&self) -> Vec<Request> {
        self.seen.lock().unwrap().clone()
    }
}

impl Drop for Http {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let result = worker.join();
            if !thread::panicking() {
                result.unwrap();
            }
        }
    }
}

fn collection(path: &str) -> String {
    format!(
        r#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>{path}</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#
    )
}

#[test]
fn sync_reliability_task_provider_dav_collection_redirect_preserves_metadata_and_identity() {
    for status in [301, 302, 307, 308] {
        let http = Http::start(move |request| match request.method.as_str() {
            "PROPFIND" if !request.path.ends_with('/') => {
                (status, Some(format!("{}/", request.path)), String::new())
            }
            "PROPFIND" => (207, None, collection(&request.path)),
            "GET" if request.path == "/file" => (302, Some("/read".into()), String::new()),
            "GET" => (200, None, "actual bytes".into()),
            "MKCOL" => (301, Some("/unexpected/".into()), String::new()),
            _ => panic!("unexpected request: {request:?}"),
        });
        let root = "/literal%20-folder";
        let backend = http.connect(root).unwrap();
        assert_eq!(backend.root_display(), root);
        let identity = backend.state_identity();
        assert!(identity.ends_with(&format!(":root={root}")));
        assert!(backend.list_dir(root).unwrap().is_empty());
        assert_eq!(backend.state_identity(), identity);
        let requests = http.requests();
        assert_eq!(requests.len(), 4);
        for (pair, depth) in requests.chunks_exact(2).zip(["0", "1"]) {
            assert_eq!(pair[0].path, "/literal%2520-folder");
            assert_eq!(pair[1].path, "/literal%2520-folder/");
            assert_eq!(pair[0].body, pair[1].body);
            assert!(!pair[0].body.is_empty());
            for request in pair {
                assert_eq!(request.method, "PROPFIND");
                assert_eq!(request.headers["depth"], depth);
                assert_eq!(request.headers["content-type"], "application/xml");
                assert_eq!(
                    request.headers["authorization"],
                    pair[0].headers["authorization"]
                );
                assert!(request.headers["authorization"].starts_with("Basic "));
            }
        }
        let mut bytes = Vec::new();
        backend
            .open_read("/file")
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(bytes, b"actual bytes");
        assert!(backend.create_dir("/new").is_err());
        let requests = http.requests();
        assert_eq!(requests.len(), 7);
        assert_eq!(requests[4].method, "GET");
        assert_eq!(requests[5].method, "GET");
        assert_eq!(requests[6].method, "MKCOL");
        assert_eq!(requests[6].path, "/new/");
    }
}

#[test]
fn sync_reliability_task_provider_dav_collection_redirect_rejects_authority_and_root_changes() {
    let foreign = Http::start(|_| (200, None, String::new()));
    let foreign_port = foreign.port;
    for case in 0..6 {
        let http = Http::start(move |request| {
            let location = match case {
                0 => format!("http://127.0.0.1:{foreign_port}/root/"),
                1 => format!("https://127.0.0.1:{foreign_port}/root/"),
                2 => "/sibling/".into(),
                3 => "/root/?query=1".into(),
                4 => "/root/#fragment".into(),
                _ => format!("http://fixture:secret@{}/root/", request.headers["host"]),
            };
            (301, Some(location), String::new())
        });
        assert!(
            http.connect("/root").is_err(),
            "accepted redirect case {case}"
        );
        assert_eq!(http.requests().len(), 1);
    }
    assert!(
        foreign.requests().is_empty(),
        "foreign authority received a request"
    );
    let http = Http::start(|request| (301, Some(format!("{}/", request.path)), String::new()));
    assert!(http.connect("/root").is_err());
    assert_eq!(http.requests().len(), 2, "redirect chain was followed");
    let http = Http::start(|_| (303, Some("/root/".into()), String::new()));
    assert!(http.connect("/root").is_err());
    assert_eq!(http.requests().len(), 1);
}
