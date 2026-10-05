//! LIST-only servers must expose the same fresh signature as standalone stat.
use super::backend_from_url;
use crate::vfs::{Backend, BackendExtensions, MtimePrecision};
use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const NAME: &str = "literal%20-file.txt";
const LIST: &str = "-rw-r--r-- 1 user group 22 Oct 05 05:33 literal%20-file.txt\r\n";

struct Server {
    address: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    control: Arc<Mutex<Option<TcpStream>>>,
    worker: Option<JoinHandle<()>>,
}

fn accept(listener: &TcpListener, stop: &AtomicBool) -> Option<TcpStream> {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !stop.load(Ordering::Acquire) && Instant::now() < deadline {
        match listener.accept() {
            Ok((stream, _)) => return Some(stream),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(_) => return None,
        }
    }
    None
}

impl Server {
    fn start(mdtm: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let control = Arc::new(Mutex::new(None));
        let owned_stop = stop.clone();
        let owned_control = control.clone();
        let worker = thread::spawn(move || {
            let Some(stream) = accept(&listener, &owned_stop) else {
                return;
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            *owned_control.lock().unwrap() = Some(stream.try_clone().unwrap());
            let mut control = BufReader::new(stream);
            writeln!(control.get_mut(), "220 fixture ready\r").unwrap();
            let mut passive = None;
            loop {
                let mut line = String::new();
                if control.read_line(&mut line).unwrap_or(0) == 0 {
                    return;
                }
                let line = line.trim_end_matches(['\r', '\n']);
                let (command, path) = line.split_once(' ').unwrap_or((line, ""));
                let response = match command {
                    "USER" => "331 password required".to_string(),
                    "PASS" => "230 logged in".to_string(),
                    "TYPE" | "NOOP" => "200 OK".to_string(),
                    "FEAT" => "500 unsupported".to_string(),
                    "PWD" => "257 \"/\" current directory".to_string(),
                    "CWD" => "250 directory changed".to_string(),
                    "PASV" => {
                        let data = TcpListener::bind("127.0.0.1:0").unwrap();
                        let port = data.local_addr().unwrap().port();
                        passive = Some(data);
                        format!(
                            "227 Entering Passive Mode (127,0,0,1,{},{})",
                            port / 256,
                            port % 256
                        )
                    }
                    "LIST" => {
                        writeln!(control.get_mut(), "150 listing\r").unwrap();
                        let Some(mut data) =
                            passive.take().and_then(|data| accept(&data, &owned_stop))
                        else {
                            return;
                        };
                        data.set_write_timeout(Some(Duration::from_secs(5)))
                            .unwrap();
                        data.write_all(LIST.as_bytes()).unwrap();
                        drop(data);
                        "226 complete".to_string()
                    }
                    "SIZE" if path == format!("/{NAME}") => "213 22".to_string(),
                    "MDTM" if mdtm && path == format!("/{NAME}") => {
                        "213 20261005053353".to_string()
                    }
                    "SIZE" | "MDTM" => "550 unavailable or denied".to_string(),
                    "QUIT" => return,
                    _ => "502 unsupported".to_string(),
                };
                if writeln!(control.get_mut(), "{response}\r").is_err() {
                    return;
                }
            }
        });
        Self {
            address,
            stop,
            control,
            worker: Some(worker),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(control) = self.control.lock().unwrap().as_ref() {
            let _ = control.shutdown(Shutdown::Both);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[test]
fn sync_reliability_task_provider_ftp_list_and_stat_exact_signature() {
    let server = Server::start(true);
    let backend = backend_from_url(&format!("ftp://user:password@{}/", server.address)).unwrap();
    let listing = backend.list_dir_tolerant("/").unwrap();
    assert!(listing.omitted.is_empty());
    let listed = &listing.entries[0];
    let fresh = backend.stat(&format!("/{NAME}")).unwrap();
    assert_eq!(listed.name, NAME);
    assert_eq!(listed.size, 22);
    assert_eq!(
        listed.mtime_ms,
        super::metadata::parse_time("20261005053353").unwrap()
    );
    assert_eq!((listed.size, listed.mtime_ms), (fresh.size, fresh.mtime_ms));
    assert_eq!(
        backend.target_limits("/").mtime_precision,
        MtimePrecision::Seconds
    );
    assert_eq!(
        backend.stat("/missing").unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
}

#[test]
fn sync_reliability_task_provider_ftp_denied_mdtm_keeps_parent_listing_signature() {
    let server = Server::start(false);
    let backend = backend_from_url(&format!("ftp://user:password@{}/", server.address)).unwrap();
    let listing = backend.list_dir_tolerant("/").unwrap();
    assert!(listing.omitted.is_empty());
    let listed = &listing.entries[0];
    let fresh = backend.stat(&format!("/{NAME}")).unwrap();
    assert_eq!(listed.name, NAME);
    assert_eq!((listed.size, listed.mtime_ms), (fresh.size, fresh.mtime_ms));
    assert_eq!(
        backend.target_limits("/").mtime_precision,
        MtimePrecision::Days
    );
}
