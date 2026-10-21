//! A scripted WebDAV server for request_count_tests.rs: files in memory,
//! every request counted by method with its headers.
use super::{WebdavBackend, WebdavConfig};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub(super) const COLLECTION: &str = r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:"><d:response><d:href>/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;

pub(super) struct Request {
    pub(super) method: String,
    pub(super) path: String,
    pub(super) headers: HashMap<String, String>,
}

#[derive(Default)]
pub(super) struct Dav {
    /// path -> (bytes, modification time in seconds)
    pub(super) files: Mutex<HashMap<String, (Vec<u8>, i64)>>,
    pub(super) log: Mutex<Vec<Request>>,
    /// MOVE answers `X-OC-Mtime: accepted`.
    pub(super) move_proof: AtomicBool,
    /// MOVE gives the moved file another time.
    pub(super) move_forgets_time: AtomicBool,
    pub(super) stop: AtomicBool,
}

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Dav {
    fn answer(
        &self,
        request: &Request,
        body: Vec<u8>,
    ) -> (u16, Vec<(&'static str, String)>, String) {
        let header = |name: &str| request.headers.get(name).cloned().unwrap_or_default();
        let mut files = lock(&self.files);
        match request.method.as_str() {
            "PUT" if files.contains_key(&request.path) => (412, vec![], String::new()),
            "PUT" => {
                let seconds = header("x-oc-mtime").parse().unwrap_or(1_600_000_000);
                files.insert(request.path.clone(), (body, seconds));
                (201, vec![], String::new())
            }
            // The connect looks at the root once.
            "PROPFIND" if request.path == "/" => (207, vec![], COLLECTION.to_string()),
            "PROPFIND" => match files.get(&request.path) {
                Some((bytes, seconds)) => (
                    207,
                    vec![],
                    multistatus(&request.path, bytes.len(), *seconds),
                ),
                None => (404, vec![], String::new()),
            },
            "MOVE" => {
                let target = header("destination");
                let target = target.split_once("//").map_or("", |(_, rest)| rest);
                let target = target.find('/').map_or("", |at| &target[at..]).to_string();
                if header("overwrite") == "F" && files.contains_key(&target) {
                    return (412, vec![], String::new());
                }
                let Some((bytes, seconds)) = files.remove(&request.path) else {
                    return (404, vec![], String::new());
                };
                let seconds = if self.move_forgets_time.load(Ordering::SeqCst) {
                    1
                } else {
                    seconds
                };
                files.insert(target, (bytes, seconds));
                let proof = self.move_proof.load(Ordering::SeqCst);
                (
                    201,
                    if proof {
                        vec![("X-OC-Mtime", "accepted".to_string())]
                    } else {
                        vec![]
                    },
                    String::new(),
                )
            }
            "MKCOL" => (201, vec![], String::new()),
            _ => (500, vec![], String::new()),
        }
    }

    fn serve(&self, stream: TcpStream) {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let Ok(read_half) = stream.try_clone() else {
            return;
        };
        let (mut reader, mut writer) = (BufReader::new(read_half), stream);
        while let Some(request) = read_request(&mut reader) {
            let (request, body) = request;
            let (status, headers, text) = self.answer(&request, body);
            lock(&self.log).push(request);
            let mut head = format!(
                "HTTP/1.1 {status} Scripted\r\nContent-Length: {}\r\n",
                text.len()
            );
            for (name, value) in headers {
                head.push_str(&format!("{name}: {value}\r\n"));
            }
            head.push_str("\r\n");
            if writer.write_all(head.as_bytes()).is_err()
                || writer.write_all(text.as_bytes()).is_err()
            {
                return;
            }
        }
    }

    pub(super) fn count(&self, method: &str) -> usize {
        lock(&self.log)
            .iter()
            .filter(|seen| seen.method == method)
            .count()
    }

    pub(super) fn requests(&self, method: &str) -> Vec<(String, HashMap<String, String>)> {
        let log = lock(&self.log);
        let wanted = log.iter().filter(|seen| seen.method == method);
        wanted
            .map(|seen| (seen.path.clone(), seen.headers.clone()))
            .collect()
    }
}

pub(super) fn multistatus(path: &str, size: usize, seconds: i64) -> String {
    let modified = chrono::DateTime::from_timestamp(seconds, 0)
        .map(|time| time.format("%a, %d %b %Y %H:%M:%S GMT").to_string())
        .unwrap_or_default();
    format!(
        r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:"><d:response><d:href>{path}</d:href><d:propstat><d:prop><d:resourcetype/><d:getcontentlength>{size}</d:getcontentlength><d:getlastmodified>{modified}</d:getlastmodified><d:getetag>"e{size}"</d:getetag></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#
    )
}

pub(super) fn read_request(reader: &mut BufReader<TcpStream>) -> Option<(Request, Vec<u8>)> {
    let mut start = String::new();
    if reader.read_line(&mut start).ok()? == 0 {
        return None;
    }
    let mut parts = start.split_whitespace();
    let (method, path) = (parts.next()?.to_string(), parts.next()?.to_string());
    let mut headers = HashMap::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':')?;
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
    }
    let length = headers
        .get("content-length")
        .and_then(|value| value.parse().ok());
    let mut body = vec![0; length.unwrap_or(0)];
    reader.read_exact(&mut body).ok()?;
    Some((
        Request {
            method,
            path,
            headers,
        },
        body,
    ))
}

pub(super) struct Server {
    pub(super) dav: Arc<Dav>,
    pub(super) address: SocketAddr,
    pub(super) worker: Option<JoinHandle<()>>,
}

impl Server {
    pub(super) fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the scripted server");
        listener.set_nonblocking(true).expect("non-blocking accept");
        let address = listener.local_addr().expect("server address");
        let dav = Arc::new(Dav::default());
        let shared = dav.clone();
        let worker = thread::spawn(move || {
            let mut sessions = Vec::new();
            while !shared.stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let dav = shared.clone();
                        let _ = stream.set_nonblocking(false);
                        sessions.push(thread::spawn(move || dav.serve(stream)));
                    }
                    Err(_) => thread::sleep(Duration::from_millis(2)),
                }
            }
            sessions.into_iter().for_each(|session| {
                let _ = session.join();
            });
        });
        Self {
            dav,
            address,
            worker: Some(worker),
        }
    }

    pub(super) fn backend(&self) -> WebdavBackend {
        WebdavBackend::connect(WebdavConfig {
            https: false,
            host: self.address.ip().to_string(),
            port: self.address.port(),
            user: String::new(),
            password: String::new(),
            root: "/".into(),
        })
        .expect("connect to the scripted server")
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.dav.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
