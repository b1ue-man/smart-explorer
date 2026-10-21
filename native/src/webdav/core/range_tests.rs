//! Ranges over WebDAV (E13, U48-6): the `Range` header of a bounded read, and
//! what the backend makes of each kind of answer. A scripted keep-alive HTTP
//! server on loopback records the header it was sent.
use super::{WebdavBackend, WebdavConfig};
use crate::vfs::Backend;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// What the server answers to one GET.
struct Reply {
    status: u16,
    content_range: Option<String>,
    body: Vec<u8>,
}

struct Server {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    /// The `Range` header of every GET, in arrival order.
    ranges: Arc<Mutex<Vec<Option<String>>>>,
}

impl Server {
    fn start(route: impl Fn(Option<&str>) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let ranges = Arc::new(Mutex::new(Vec::new()));
        let route = Arc::new(route);
        let (flag, seen) = (stop.clone(), ranges.clone());
        thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(30);
            while !flag.load(Ordering::SeqCst) && Instant::now() < deadline {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (route, seen) = (route.clone(), seen.clone());
                        thread::spawn(move || serve(stream, route.as_ref(), &seen));
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            address,
            stop,
            ranges,
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

    fn ranges(&self) -> Vec<Option<String>> {
        self.ranges
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn serve(
    stream: TcpStream,
    route: &(dyn Fn(Option<&str>) -> Reply + Send + Sync),
    seen: &Mutex<Vec<Option<String>>>,
) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let Ok(read_half) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(read_half);
    let mut writer = stream;
    loop {
        let mut start = String::new();
        if reader.read_line(&mut start).unwrap_or(0) == 0 {
            return;
        }
        let mut range = None;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                if name.trim().eq_ignore_ascii_case("range") {
                    range = Some(value.trim().to_string());
                }
            }
        }
        seen.lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(range.clone());
        let reply = route(range.as_deref());
        let mut head = format!(
            "HTTP/1.1 {} Scripted\r\nContent-Length: {}\r\nConnection: keep-alive\r\n",
            reply.status,
            reply.body.len()
        );
        if let Some(value) = &reply.content_range {
            head.push_str(&format!("Content-Range: {value}\r\n"));
        }
        head.push_str("\r\n");
        if writer.write_all(head.as_bytes()).is_err() || writer.write_all(&reply.body).is_err() {
            return;
        }
        let _ = writer.flush();
    }
}

fn file(len: usize) -> Vec<u8> {
    (0..len).map(|index| (index % 251) as u8).collect()
}

/// A server that honours `bytes=a-b` on a file of `len` bytes.
fn honouring(len: usize) -> Server {
    Server::start(move |range| {
        let content = file(len);
        let Some((first, last)) = range
            .and_then(|value| value.strip_prefix("bytes="))
            .and_then(|value| value.split_once('-'))
        else {
            return Reply {
                status: 200,
                content_range: None,
                body: content,
            };
        };
        let first: usize = first.parse().unwrap_or(0);
        let last: usize = last.parse().unwrap_or(len - 1).min(len - 1);
        Reply {
            status: 206,
            content_range: Some(format!("bytes {first}-{last}/{len}")),
            body: content[first..=last].to_vec(),
        }
    })
}

#[test]
fn speed_task_webdav_range_reader_sends_bounded_range_header() {
    let server = honouring(1000);
    let backend = server.backend();
    assert!(backend.supports_range_reads("/f"));
    let mut bytes = Vec::new();
    backend
        .open_read_range("/f", None, 100, 100)
        .unwrap()
        .expect("the server honours ranges")
        .read_to_end(&mut bytes)
        .unwrap();
    assert_eq!(bytes, file(1000)[100..200], "exactly the range");
    // From the start of the file the range is still sent, with its end.
    let mut head = Vec::new();
    backend
        .open_read_range("/f", None, 0, 64)
        .unwrap()
        .expect("ranges from offset 0")
        .read_to_end(&mut head)
        .unwrap();
    assert_eq!(head, file(1000)[..64]);
    assert_eq!(
        server.ranges(),
        vec![Some("bytes=100-199".into()), Some("bytes=0-63".into())],
        "the last byte is included in the header"
    );
    // A resume still asks open-ended; a whole file asks for no range.
    let mut tail = Vec::new();
    backend
        .open_read_at("/f", None, 900)
        .unwrap()
        .expect("resume")
        .read_to_end(&mut tail)
        .unwrap();
    assert_eq!(tail, file(1000)[900..]);
    backend.open_read("/f").unwrap();
    let ranges = server.ranges();
    assert_eq!(ranges[2].as_deref(), Some("bytes=900-"));
    assert_eq!(ranges[3], None);
}

#[test]
fn speed_task_webdav_range_200_response_means_no_ranges() {
    let server = Server::start(|_| Reply {
        status: 200,
        content_range: None,
        body: file(500),
    });
    let backend = server.backend();
    let opened = backend.open_read_range("/f", None, 100, 100).unwrap();
    assert!(
        opened.is_none(),
        "a server that ignores Range sends the whole file: no ranges"
    );
    let opened = backend.open_read_range("/f", None, 0, 100).unwrap();
    assert!(opened.is_none());
}

#[test]
fn speed_task_webdav_range_416_is_invalid_data() {
    let server = Server::start(|_| Reply {
        status: 416,
        content_range: Some("bytes */50".into()),
        body: Vec::new(),
    });
    let backend = server.backend();
    let error = backend
        .open_read_range("/f", None, 100, 100)
        .err()
        .expect("416 is an error");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn speed_task_webdav_range_content_range_must_match_offset() {
    // 206, but from another start than asked: the bytes would be wrong.
    let server = Server::start(|_| Reply {
        status: 206,
        content_range: Some("bytes 0-99/1000".into()),
        body: file(100),
    });
    let backend = server.backend();
    let error = backend
        .open_read_range("/f", None, 100, 100)
        .err()
        .expect("a mismatching start is an error");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    // 206 without any Content-Range cannot be checked either.
    let server = Server::start(|_| Reply {
        status: 206,
        content_range: None,
        body: file(100),
    });
    let error = server
        .backend()
        .open_read_range("/f", None, 100, 100)
        .err()
        .expect("no Content-Range is an error");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    // A server that sends more than asked is cut at the range.
    let server = Server::start(|_| Reply {
        status: 206,
        content_range: Some("bytes 100-499/1000".into()),
        body: file(400),
    });
    let mut bytes = Vec::new();
    server
        .backend()
        .open_read_range("/f", None, 100, 50)
        .unwrap()
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    assert_eq!(bytes.len(), 50);
}
