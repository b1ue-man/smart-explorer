//! A scripted passive-mode FTP server for data_overlap_tests.rs. It accepts
//! the data connection 100 ms late and sends `150` only after accepting
//! (vsftpd), counts the transfer commands it read, and notes any command that
//! arrives before the reply to the previous one was read.
use super::connection::{connect_stream, FtpUrl};
use super::io_adapters::{FtpConnection, FtpReconnect};
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};
use suppaftp::{FtpError, FtpResult};

#[derive(Default)]
pub(super) struct State {
    pub(super) files: Mutex<HashMap<String, Vec<u8>>>,
    pub(super) events: Mutex<Vec<String>>,
    /// Transfer commands (STOR, RETR, MLSD) read so far.
    pub(super) transfers: (Mutex<usize>, Condvar),
    pub(super) pipelined: AtomicBool,
    pub(super) stop: AtomicBool,
}

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn reply(control: &mut TcpStream, line: &str) -> io::Result<()> {
    control.write_all(format!("{line}\r\n").as_bytes())
}

impl State {
    fn event(&self, line: &str) {
        lock(&self.events).push(line.to_string());
    }

    pub(super) fn position(&self, line: &str) -> Option<usize> {
        lock(&self.events).iter().position(|seen| seen == line)
    }

    /// A transfer command arrived: the hook of the client may go on.
    fn transfer_seen(&self) {
        *lock(&self.transfers.0) += 1;
        self.transfers.1.notify_all();
    }

    fn data(&self, passive: Option<TcpListener>) -> io::Result<TcpStream> {
        let listener = passive.ok_or_else(|| io::Error::other("no PASV"))?;
        listener.set_nonblocking(true)?;
        thread::sleep(Duration::from_millis(100));
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            match listener.accept() {
                Ok((data, _)) => {
                    data.set_nonblocking(false)?;
                    data.set_read_timeout(Some(Duration::from_secs(5)))?;
                    self.event("data accepted");
                    return Ok(data);
                }
                Err(error) if Instant::now() < deadline => {
                    let _ = error;
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// 150 after accepting, `work` on the data socket, then 226.
    fn transfer(
        &self,
        control: &mut TcpStream,
        passive: Option<TcpListener>,
        work: impl FnOnce(&mut TcpStream) -> io::Result<()>,
    ) -> io::Result<()> {
        let Ok(mut data) = self.data(passive) else {
            return reply(control, "425 Can't open data connection");
        };
        reply(control, "150 opening data connection")?;
        work(&mut data)?;
        drop(data);
        reply(control, "226 done")
    }

    fn serve(&self, stream: TcpStream) -> io::Result<()> {
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut control = stream.try_clone()?;
        let mut reader = BufReader::new(stream);
        let mut passive: Option<TcpListener> = None;
        let mut rename_from: Option<String> = None;
        reply(&mut control, "220 ready")?;
        while !self.stop.load(Ordering::SeqCst) {
            let mut line = String::new();
            if reader.read_line(&mut line)? == 0 {
                return Ok(());
            }
            let line = line.trim_end().to_string();
            self.event(&line);
            let (command, argument) = line.split_once(' ').unwrap_or((line.as_str(), ""));
            if matches!(command, "STOR" | "RETR" | "MLSD") {
                self.transfer_seen();
            }
            // A client that pipelines has sent its next command by now.
            thread::sleep(Duration::from_millis(20));
            reader
                .get_ref()
                .set_read_timeout(Some(Duration::from_millis(1)))?;
            let ahead = !reader.buffer().is_empty()
                || reader.get_ref().peek(&mut [0u8]).is_ok_and(|read| read > 0);
            reader
                .get_ref()
                .set_read_timeout(Some(Duration::from_secs(5)))?;
            if ahead {
                self.pipelined.store(true, Ordering::SeqCst);
            }
            let path = argument.to_string();
            let known = lock(&self.files).get(&path).cloned();
            match (command, known) {
                ("USER", _) => reply(&mut control, "331 password")?,
                ("PASS", _) => reply(&mut control, "230 in")?,
                ("TYPE", _) => reply(&mut control, "200 binary")?,
                ("PWD", _) => reply(&mut control, "257 \"/\" is current directory")?,
                ("FEAT", _) => reply(
                    &mut control,
                    "211-Features:\r\n MLST size*;type*;\r\n211 End",
                )?,
                ("MLST" | "SIZE", None) => reply(&mut control, "550 not found")?,
                ("SIZE", Some(bytes)) => reply(&mut control, &format!("213 {}", bytes.len()))?,
                ("PASV", _) => {
                    let listener = TcpListener::bind("127.0.0.1:0")?;
                    let port = listener.local_addr()?.port();
                    passive = Some(listener);
                    let text = format!(
                        "227 Entering Passive Mode (127,0,0,1,{},{})",
                        port / 256,
                        port % 256
                    );
                    reply(&mut control, &text)?;
                }
                ("STOR", _) => self.transfer(&mut control, passive.take(), |data| {
                    let mut body = Vec::new();
                    data.read_to_end(&mut body)?;
                    lock(&self.files).insert(path.clone(), body);
                    Ok(())
                })?,
                ("RETR", Some(bytes)) => {
                    self.transfer(&mut control, passive.take(), |data| data.write_all(&bytes))?
                }
                ("MLSD", _) => self.transfer(&mut control, passive.take(), |data| {
                    for (name, bytes) in lock(&self.files).iter() {
                        let row = format!("type=file;size={}; {}\r\n", bytes.len(), &name[1..]);
                        data.write_all(row.as_bytes())?;
                    }
                    Ok(())
                })?,
                ("RNFR", Some(_)) => {
                    rename_from = Some(path);
                    reply(&mut control, "350 pending")?
                }
                ("RNTO", _) => match rename_from.take() {
                    Some(from) => {
                        let content = lock(&self.files).remove(&from).unwrap_or_default();
                        lock(&self.files).insert(path, content);
                        reply(&mut control, "250 renamed")?
                    }
                    None => reply(&mut control, "503 RNFR first")?,
                },
                ("QUIT", _) => return reply(&mut control, "221 bye"),
                _ => reply(&mut control, "550 refused")?,
            }
        }
        Ok(())
    }
}

pub(super) struct Server {
    pub(super) address: SocketAddr,
    pub(super) state: Arc<State>,
    pub(super) worker: Option<thread::JoinHandle<()>>,
}

impl Server {
    pub(super) fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the scripted server");
        listener.set_nonblocking(true).expect("non-blocking accept");
        let address = listener.local_addr().expect("server address");
        let state = Arc::new(State::default());
        let shared = state.clone();
        let worker = thread::spawn(move || {
            let mut sessions = Vec::new();
            while !shared.stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let state = shared.clone();
                        sessions.push(thread::spawn(move || {
                            let _ = state.serve(stream);
                        }));
                    }
                    Err(_) => thread::sleep(Duration::from_millis(2)),
                }
            }
            sessions.into_iter().for_each(|session| {
                let _ = session.join();
            });
        });
        Self {
            address,
            state,
            worker: Some(worker),
        }
    }

    pub(super) fn url(&self) -> FtpUrl {
        FtpUrl {
            secure: false,
            user: "test".to_string(),
            password: "secret".to_string(),
            host: "127.0.0.1".to_string(),
            port: self.address.port(),
            root: "/".to_string(),
        }
    }

    /// A control connection whose data sockets are built by `builder`.
    pub(super) fn connection<F>(&self, builder: F) -> Arc<FtpConnection>
    where
        F: Fn(SocketAddr) -> FtpResult<TcpStream> + Send + Sync + 'static,
    {
        let url = self.url();
        let stream = connect_stream(&url)
            .expect("log in")
            .passive_stream_builder(builder);
        let reconnect: FtpReconnect = Arc::new(move || connect_stream(&url));
        FtpConnection::new(stream, reconnect).expect("control connection")
    }

    /// Data sockets that connect only after the server read a transfer
    /// command: a client that connects first waits here and fails.
    pub(super) fn connection_after_command(&self) -> Arc<FtpConnection> {
        let state = self.state.clone();
        self.connection(move |address| {
            let (count, seen) = &state.transfers;
            let (count, timeout) = seen
                .wait_timeout_while(lock(count), Duration::from_secs(3), |count| *count == 0)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if timeout.timed_out() || *count == 0 {
                let error = io::Error::new(io::ErrorKind::TimedOut, "no transfer command yet");
                return Err(FtpError::ConnectionError(error));
            }
            TcpStream::connect(address).map_err(FtpError::ConnectionError)
        })
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.state.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
