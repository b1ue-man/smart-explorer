//! Scripted loopback FTP server of the gap-closing batch's FTP tests
//! (luecken_task_ftp_tests.rs): plain FTP, passive mode, one thread per
//! control connection; records every command line as the server read it.
use super::connection::{connect_stream, FtpUrl};
use super::io_adapters::{FtpConnection, FtpReconnect};
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// RETR of this path streams until the client goes away.
pub(super) const ENDLESS: &str = "/endless";

#[derive(Clone)]
pub(super) struct Script {
    /// The whole PWD reply line.
    pub(super) pwd: String,
    /// FEAT offers MLST/MLSD, but MLSD and `LIST -a` are refused.
    pub(super) refuse_listings: bool,
}

impl Default for Script {
    fn default() -> Self {
        Self {
            pwd: "257 \"/\" is current directory".to_string(),
            refuse_listings: false,
        }
    }
}

#[derive(Default)]
struct State {
    files: HashMap<String, Vec<u8>>,
    commands: Vec<String>,
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(super) struct Server {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    accepts: Arc<AtomicUsize>,
    state: Arc<Mutex<State>>,
    worker: Option<JoinHandle<()>>,
}

impl Server {
    pub(super) fn start(script: Script, files: &[(&str, &[u8])]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let accepts = Arc::new(AtomicUsize::new(0));
        let mut state = State::default();
        for (path, bytes) in files {
            state.files.insert(path.to_string(), bytes.to_vec());
        }
        let state = Arc::new(Mutex::new(state));
        let worker = {
            let (stop, accepts, state) = (stop.clone(), accepts.clone(), state.clone());
            thread::spawn(move || {
                let mut handlers = Vec::new();
                let deadline = Instant::now() + Duration::from_secs(60);
                while !stop.load(Ordering::SeqCst) && Instant::now() < deadline {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            accepts.fetch_add(1, Ordering::SeqCst);
                            let (script, state) = (script.clone(), state.clone());
                            handlers.push(thread::spawn(move || serve(stream, &script, &state)));
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
            accepts,
            state,
            worker: Some(worker),
        }
    }

    pub(super) fn url(&self) -> String {
        format!("ftp://test:secret@127.0.0.1:{}/", self.address.port())
    }

    /// One control connection of its own, without keepalive NOOPs.
    pub(super) fn connection(&self) -> Arc<FtpConnection> {
        let config = FtpUrl {
            secure: false,
            user: "test".to_string(),
            password: "secret".to_string(),
            host: "127.0.0.1".to_string(),
            port: self.address.port(),
            root: "/".to_string(),
        };
        let again = config.clone();
        let reconnect: FtpReconnect = Arc::new(move || connect_stream(&again));
        let stream = connect_stream(&config).unwrap();
        let idle = Duration::from_secs(600);
        FtpConnection::new_with_timing(stream, reconnect, idle, Duration::from_secs(5)).unwrap()
    }

    pub(super) fn accepts(&self) -> usize {
        self.accepts.load(Ordering::SeqCst)
    }

    pub(super) fn commands(&self) -> Vec<String> {
        lock(&self.state).commands.clone()
    }

    pub(super) fn file(&self, path: &str) -> Option<Vec<u8>> {
        lock(&self.state).files.get(path).cloned()
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

fn reply(stream: &mut TcpStream, line: &str) -> io::Result<()> {
    stream.write_all(line.as_bytes())?;
    stream.write_all(b"\r\n")?;
    stream.flush()
}

/// 150, the passive data connection for `work`, then 226 (or 426 when the
/// client went away during `work`).
fn transfer(
    control: &mut TcpStream,
    passive: Option<TcpListener>,
    work: impl FnOnce(&mut TcpStream) -> io::Result<()>,
) -> io::Result<()> {
    let Some(listener) = passive else {
        return reply(control, "425 Use PASV first");
    };
    reply(control, "150 opening data connection")?;
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut data = loop {
        match listener.accept() {
            Ok((data, _)) => break data,
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(2));
            }
            Err(error) => return Err(error),
        }
    };
    data.set_nonblocking(false)?;
    data.set_read_timeout(Some(Duration::from_secs(10)))?;
    data.set_write_timeout(Some(Duration::from_secs(10)))?;
    let done = work(&mut data);
    drop(data);
    match done {
        Ok(()) => reply(control, "226 transfer complete"),
        Err(_) => reply(control, "426 data connection closed; transfer aborted"),
    }
}

fn listing(state: &State, folder: &str) -> String {
    let prefix = format!("{}/", folder.trim_end_matches('/'));
    let mut rows = String::new();
    for (path, bytes) in &state.files {
        let Some(name) = path
            .strip_prefix(&prefix)
            .filter(|name| !name.contains('/'))
        else {
            continue;
        };
        rows.push_str(&format!(
            "-rw-r--r-- 1 0 0 {} Jan 1 2020 {name}\r\n",
            bytes.len()
        ));
    }
    rows
}

fn serve(mut control: TcpStream, script: &Script, state: &Mutex<State>) {
    let _ = control.set_nonblocking(false);
    let _ = control.set_read_timeout(Some(Duration::from_secs(20)));
    if reply(&mut control, "220 luecken script ready").is_err() {
        return;
    }
    let Ok(reader) = control.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(reader);
    let mut passive: Option<TcpListener> = None;
    let mut current = "/".to_string();
    loop {
        let mut raw = Vec::new();
        match reader.read_until(b'\n', &mut raw) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let text = String::from_utf8_lossy(&raw);
        let line = text.trim_end_matches(['\r', '\n']).to_string();
        lock(state).commands.push(line.clone());
        let (command, argument) = line.split_once(' ').unwrap_or((line.as_str(), ""));
        let answered = match command {
            "USER" => reply(&mut control, "331 password required"),
            "PASS" => reply(&mut control, "230 logged in"),
            "TYPE" => reply(&mut control, "200 binary"),
            "NOOP" => reply(&mut control, "200 alive"),
            "PWD" => reply(&mut control, &script.pwd),
            "CWD" => {
                current = argument.to_string();
                reply(&mut control, "250 directory changed")
            }
            "FEAT" if script.refuse_listings => reply(
                &mut control,
                "211-Features:\r\n MLST type*;size*;modify*;\r\n MLSD\r\n211 End",
            ),
            "SIZE" => {
                let size = lock(state).files.get(argument).map(Vec::len);
                match size {
                    Some(size) => reply(&mut control, &format!("213 {size}")),
                    None => reply(&mut control, "550 not a regular file"),
                }
            }
            "DELE" => {
                let removed = lock(state).files.remove(argument).is_some();
                if removed {
                    reply(&mut control, "250 deleted")
                } else {
                    reply(&mut control, "550 no such file")
                }
            }
            "ABOR" => reply(&mut control, "226 ABOR command successful"),
            "PASV" => match TcpListener::bind("127.0.0.1:0") {
                Ok(listener) => {
                    let port = listener.local_addr().map_or(0, |address| address.port());
                    passive = Some(listener);
                    let (high, low) = (port / 256, port % 256);
                    let answer = format!("227 Entering Passive Mode (127,0,0,1,{high},{low})");
                    reply(&mut control, &answer)
                }
                Err(error) => Err(error),
            },
            "MLSD" if script.refuse_listings => reply(&mut control, "500 MLSD not understood"),
            "LIST" if script.refuse_listings && argument == "-a" => {
                reply(&mut control, "501 no options")
            }
            "LIST" => {
                let rows = listing(&lock(state), &current);
                transfer(&mut control, passive.take(), |data| {
                    data.write_all(rows.as_bytes())
                })
            }
            "RETR" if argument == ENDLESS => transfer(&mut control, passive.take(), |data| {
                let chunk = [0x5a; 64 * 1024];
                for _ in 0..4096 {
                    data.write_all(&chunk)?;
                }
                Ok(())
            }),
            "RETR" => {
                let bytes = lock(state).files.get(argument).cloned();
                match bytes {
                    Some(bytes) => {
                        transfer(&mut control, passive.take(), |data| data.write_all(&bytes))
                    }
                    None => reply(&mut control, "550 no such file"),
                }
            }
            "QUIT" => {
                let _ = reply(&mut control, "221 bye");
                return;
            }
            _ => reply(&mut control, "500 unsupported"),
        };
        if answered.is_err() {
            return;
        }
    }
}
