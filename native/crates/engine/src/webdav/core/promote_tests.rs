//! Staged promotion on WebDAV against a scripted HTTP server: replacing an
//! existing file is one `MOVE` with `Overwrite: T`, creating a new name one
//! `MOVE` with `Overwrite: F`, and neither ever sends a `DELETE`.
use super::*;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Instant;

#[derive(Debug)]
struct Request {
    method: String,
    path: String,
    headers: HashMap<String, String>,
}

/// Answers PROPFIND for the paths in `files` (and the root) with one regular
/// file, 404 for every other PROPFIND, and 204 for MOVE.
struct Server {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<Vec<Request>>>,
}

impl Server {
    fn start(files: &'static [&'static str]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let shutdown = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            let mut requests = Vec::new();
            let deadline = Instant::now() + Duration::from_secs(20);
            while !shutdown.load(Ordering::SeqCst) && Instant::now() < deadline {
                let stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(_) => break,
                };
                if let Ok(request) = answer(stream, files) {
                    requests.push(request);
                }
            }
            requests
        });
        Self {
            address,
            stop,
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

    fn finish(mut self) -> Vec<Request> {
        self.stop.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join().unwrap()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn answer(stream: TcpStream, files: &[&str]) -> io::Result<Request> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut start = String::new();
    reader.read_line(&mut start)?;
    let mut parts = start.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let mut headers = HashMap::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;

    let (status, body) = match method.as_str() {
        "PROPFIND" if path == "/" => ("207 Multi-Status", folder("/")),
        "PROPFIND" if files.contains(&path.as_str()) => ("207 Multi-Status", file(&path)),
        "PROPFIND" => ("404 Not Found", String::new()),
        "MOVE" => ("204 No Content", String::new()),
        _ => ("400 Bad Request", String::new()),
    };
    let mut stream = stream;
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()?;
    Ok(Request {
        method,
        path,
        headers,
    })
}

fn folder(href: &str) -> String {
    response(href, "<d:resourcetype><d:collection/></d:resourcetype>")
}

fn file(href: &str) -> String {
    response(
        href,
        "<d:resourcetype/><d:getcontentlength>5</d:getcontentlength>\
         <d:getlastmodified>Sat, 26 Sep 2026 10:00:00 GMT</d:getlastmodified>",
    )
}

fn response(href: &str, props: &str) -> String {
    format!(
        r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:"><d:response><d:href>{href}</d:href><d:propstat><d:prop>{props}</d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#
    )
}

fn moves(requests: &[Request]) -> Vec<(&str, &str, &str)> {
    requests
        .iter()
        .filter(|request| request.method == "MOVE")
        .map(|request| {
            let header = |name: &str| request.headers.get(name).map_or("", String::as_str);
            (
                request.path.as_str(),
                header("destination"),
                header("overwrite"),
            )
        })
        .collect()
}

#[test]
fn android_task_webdav_promote_replaces_with_one_move_overwrite_true() {
    let server = Server::start(&["/stage", "/dest"]);
    let backend = server.backend();
    backend.promote_staged("/stage", "/dest").unwrap();
    let requests = server.finish();
    let moves = moves(&requests);
    assert_eq!(moves.len(), 1, "{requests:?}");
    assert_eq!(moves[0].0, "/stage");
    assert!(moves[0].1.ends_with("/dest"), "{requests:?}");
    assert_eq!(moves[0].2, "T");
    assert!(requests.iter().all(|request| request.method != "DELETE"));
}

#[test]
fn android_task_webdav_promote_creates_a_new_name_without_overwrite() {
    let server = Server::start(&["/stage"]);
    let backend = server.backend();
    backend.promote_staged("/stage", "/new").unwrap();
    let requests = server.finish();
    let moves = moves(&requests);
    assert_eq!(moves.len(), 1, "{requests:?}");
    assert!(moves[0].1.ends_with("/new"), "{requests:?}");
    assert_eq!(moves[0].2, "F");
    assert!(requests.iter().all(|request| request.method != "DELETE"));
}
