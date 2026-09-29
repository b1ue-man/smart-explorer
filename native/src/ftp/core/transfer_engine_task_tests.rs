//! Transfer-engine task tests for the FTP connection pool, REST resumes, the
//! streaming stage STOR and single-MKD folders, against a scripted loopback
//! FTP server (plain FTP, passive mode, one thread per control connection).
use super::backend_from_url;
use super::connection::{connect_stream, refusal_code, FtpUrl};
use super::io_adapters::{FtpConnection, FtpReconnect};
use super::pool::FtpPool;
use crate::vfs::{congestion_of, Backend};
use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Refuse {
    /// vsftpd / Pure-FTPd: 421 instead of the greeting.
    Greeting,
    /// ProFTPD `MaxClientsPerUser`: 530 to PASS.
    Login,
}

struct Rules {
    max_connections: usize,
    refuse: Refuse,
    wrong_password: bool,
    rest: bool,
}

impl Rules {
    fn allowing(max_connections: usize, refuse: Refuse) -> Self {
        Self {
            max_connections,
            refuse,
            wrong_password: false,
            rest: true,
        }
    }
}

#[derive(Default)]
struct Store {
    files: HashMap<String, Vec<u8>>,
    dirs: HashSet<String>,
    commands: Vec<String>,
}

fn lock(store: &Mutex<Store>) -> MutexGuard<'_, Store> {
    store
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct Server {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    connections: Arc<AtomicUsize>,
    store: Arc<Mutex<Store>>,
    worker: Option<JoinHandle<()>>,
}

impl Server {
    fn start(rules: Rules, store: Store) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let connections = Arc::new(AtomicUsize::new(0));
        let store = Arc::new(Mutex::new(store));
        let rules = Arc::new(rules);
        let open = Arc::new(AtomicUsize::new(0));
        let worker = {
            let (stop, connections, store) = (stop.clone(), connections.clone(), store.clone());
            thread::spawn(move || {
                let mut handlers = Vec::new();
                let deadline = Instant::now() + Duration::from_secs(30);
                while !stop.load(Ordering::SeqCst) && Instant::now() < deadline {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            connections.fetch_add(1, Ordering::SeqCst);
                            let (rules, store, open) = (rules.clone(), store.clone(), open.clone());
                            handlers
                                .push(thread::spawn(move || serve(stream, &rules, &store, &open)));
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
            connections,
            store,
            worker: Some(worker),
        }
    }

    fn url(&self) -> FtpUrl {
        FtpUrl {
            secure: false,
            user: "test".to_string(),
            password: "secret".to_string(),
            host: "127.0.0.1".to_string(),
            port: self.address.port(),
            root: "/".to_string(),
        }
    }

    fn url_string(&self) -> String {
        format!("ftp://test:secret@127.0.0.1:{}/", self.address.port())
    }

    fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }

    fn file(&self, path: &str) -> Option<Vec<u8>> {
        lock(&self.store).files.get(path).cloned()
    }

    fn has_dir(&self, path: &str) -> bool {
        lock(&self.store).dirs.contains(path)
    }

    fn saw(&self, command: &str) -> bool {
        lock(&self.store)
            .commands
            .iter()
            .any(|seen| seen == command)
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

struct OpenGuard(Arc<AtomicUsize>);

impl Drop for OpenGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn reply(stream: &mut TcpStream, line: &str) -> io::Result<()> {
    stream.write_all(line.as_bytes())?;
    stream.write_all(b"\r\n")?;
    stream.flush()
}

fn child_name<'a>(dir: &str, path: &'a str) -> Option<&'a str> {
    let (parent, name) = path.rsplit_once('/')?;
    (parent == dir && !name.is_empty()).then_some(name)
}

fn listing(store: &Store, dir: &str) -> String {
    let dir = dir.trim_end_matches('/');
    let mut lines = String::new();
    for path in &store.dirs {
        if let Some(name) = child_name(dir, path) {
            lines.push_str(&format!("drwxr-xr-x 1 0 0 0 Jan 1 2020 {name}\r\n"));
        }
    }
    for (path, content) in &store.files {
        if let Some(name) = child_name(dir, path) {
            let size = content.len();
            lines.push_str(&format!("-rw-r--r-- 1 0 0 {size} Jan 1 2020 {name}\r\n"));
        }
    }
    lines
}

/// 150, one passive data connection for `work`, then 226.
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
    data.set_read_timeout(Some(Duration::from_secs(5)))?;
    work(&mut data)?;
    drop(data);
    reply(control, "226 transfer complete")
}

fn serve(mut stream: TcpStream, rules: &Rules, store: &Mutex<Store>, open: &Arc<AtomicUsize>) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let over = open.fetch_add(1, Ordering::SeqCst) + 1 > rules.max_connections;
    let _guard = OpenGuard(open.clone());
    if over && rules.refuse == Refuse::Greeting {
        let _ = reply(
            &mut stream,
            "421 There are too many connections from your internet address.",
        );
        return;
    }
    if reply(&mut stream, "220 script ready").is_err() {
        return;
    }
    let Ok(reader) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(reader);
    let mut passive: Option<TcpListener> = None;
    let mut rest = 0usize;
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let line = line.trim_end_matches(['\r', '\n']).to_string();
        let (command, argument) = match line.split_once(' ') {
            Some((command, argument)) => (command.to_string(), argument.to_string()),
            None => (line.clone(), String::new()),
        };
        lock(store).commands.push(line.clone());
        let answered = match command.as_str() {
            "USER" => reply(&mut stream, "331 password required"),
            "PASS" if rules.wrong_password => reply(&mut stream, "530 Login incorrect."),
            "PASS" if over => reply(
                &mut stream,
                "530 Sorry, the maximum number of clients (1) for this user are already connected.",
            ),
            "PASS" => reply(&mut stream, "230 logged in"),
            "TYPE" => reply(&mut stream, "200 binary"),
            "PWD" => reply(&mut stream, "257 \"/\" is current directory"),
            "NOOP" => reply(&mut stream, "200 alive"),
            "QUIT" => {
                let _ = reply(&mut stream, "221 bye");
                return;
            }
            "MKD" => {
                let created = {
                    let mut store = lock(store);
                    let taken =
                        store.dirs.contains(&argument) || store.files.contains_key(&argument);
                    if !taken {
                        store.dirs.insert(argument.clone());
                    }
                    !taken
                };
                if created {
                    reply(&mut stream, &format!("257 \"{argument}\" created"))
                } else {
                    reply(&mut stream, "550 File exists")
                }
            }
            "PASV" => match TcpListener::bind("127.0.0.1:0") {
                Ok(listener) => {
                    let port = listener.local_addr().map_or(0, |address| address.port());
                    passive = Some(listener);
                    let (high, low) = (port / 256, port % 256);
                    reply(
                        &mut stream,
                        &format!("227 Entering Passive Mode (127,0,0,1,{high},{low})"),
                    )
                }
                Err(error) => Err(error),
            },
            "REST" if !rules.rest => reply(&mut stream, "502 Command not implemented"),
            "REST" => {
                rest = argument.parse().unwrap_or(0);
                reply(&mut stream, &format!("350 Restarting at {rest}"))
            }
            "LIST" => transfer(&mut stream, passive.take(), |data| {
                let lines = listing(&lock(store), &argument);
                data.write_all(lines.as_bytes())
            }),
            "RETR" => {
                let offset = std::mem::take(&mut rest);
                let content = lock(store).files.get(&argument).cloned();
                match content {
                    Some(content) => transfer(&mut stream, passive.take(), |data| {
                        data.write_all(&content[offset.min(content.len())..])
                    }),
                    None => reply(&mut stream, "550 No such file"),
                }
            }
            "STOR" => transfer(&mut stream, passive.take(), |data| {
                let mut body = Vec::new();
                data.read_to_end(&mut body)?;
                lock(store).files.insert(argument.clone(), body);
                Ok(())
            }),
            _ => reply(&mut stream, "500 unsupported"),
        };
        if answered.is_err() {
            return;
        }
    }
}

fn pool_for(server: &Server) -> Arc<FtpPool> {
    let url = server.url();
    let reconnect_url = url.clone();
    let reconnect: FtpReconnect = Arc::new(move || connect_stream(&reconnect_url));
    let primary = FtpConnection::new(connect_stream(&url).unwrap(), reconnect.clone()).unwrap();
    FtpPool::new(primary, reconnect)
}

#[test]
fn transfer_engine_task_ftp_pool_grows_until_refused_and_keeps_browsing_free() {
    let server = Server::start(Rules::allowing(3, Refuse::Greeting), Store::default());
    let pool = pool_for(&server);
    assert_eq!(pool.transfer_capacity(), None);
    let first = pool.lease().unwrap();
    let second = pool.lease().unwrap();
    assert!(!Arc::ptr_eq(first.connection(), pool.primary()));
    assert!(!Arc::ptr_eq(second.connection(), pool.primary()));
    // The fourth login is refused: three connections in all, one of them
    // stays for browsing, so two transfers run at once.
    let refused = pool.lease().err().expect("the server limit must surface");
    assert!(congestion_of(&refused).is_some(), "{refused}");
    assert_eq!(pool.transfer_capacity(), Some(2));
    assert_eq!(server.connections(), 4);
    // A returned connection is reused without a new login.
    drop(first);
    let third = pool.lease().unwrap();
    assert!(!Arc::ptr_eq(third.connection(), pool.primary()));
    assert_eq!(server.connections(), 4);
    // Browsing goes on while both transfer connections are lent.
    let pwd = pool
        .primary()
        .with_stream_read(|stream| {
            stream
                .pwd()
                .map_err(|error| io::Error::other(error.to_string()))
        })
        .unwrap();
    assert_eq!(pwd, "/");
    drop((second, third));
    drop(pool);
}

#[test]
fn transfer_engine_task_ftp_one_connection_server_shares_the_browsing_connection() {
    let server = Server::start(Rules::allowing(1, Refuse::Login), Store::default());
    let pool = pool_for(&server);
    // PASS of the second connection answers 530: after the first login
    // succeeded, that is the per-user limit, not a wrong password.
    let lease = pool.lease().unwrap();
    assert!(Arc::ptr_eq(lease.connection(), pool.primary()));
    assert_eq!(pool.transfer_capacity(), Some(0));
    drop(lease);
    drop(pool);
}

#[test]
fn transfer_engine_task_ftp_530_before_the_first_login_is_a_login_error() {
    let mut rules = Rules::allowing(4, Refuse::Login);
    rules.wrong_password = true;
    let server = Server::start(rules, Store::default());
    let error = backend_from_url(&server.url_string())
        .err()
        .expect("a refused first login must fail the connect");
    assert!(congestion_of(&error).is_none(), "{error}");
    assert!(error.to_string().contains("530"), "{error}");
    assert_eq!(refusal_code(&error), Some(530));
}

#[test]
fn transfer_engine_task_ftp_sized_stage_streams_stor_with_the_exact_length() {
    let server = Server::start(Rules::allowing(8, Refuse::Greeting), Store::default());
    let backend = backend_from_url(&server.url_string()).unwrap();

    let mut writer = backend.open_write_copy_stage_sized("/stage", 5).unwrap();
    writer.write_all(b"hello").unwrap();
    writer.flush().unwrap();
    writer.flush().unwrap();
    drop(writer);
    assert_eq!(server.file("/stage").as_deref(), Some(&b"hello"[..]));
    assert!(server.saw("STOR /stage"));

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

    // An occupied stage name is refused before any STOR.
    assert_eq!(
        backend
            .open_write_copy_stage_sized("/stage", 1)
            .err()
            .map(|error| error.kind()),
        Some(io::ErrorKind::AlreadyExists)
    );
    drop(backend);
}

#[test]
fn transfer_engine_task_ftp_resume_sends_rest_or_reports_none() {
    let content: Vec<u8> = (0..100u8).collect();
    let mut store = Store::default();
    store.files.insert("/data.bin".to_string(), content.clone());
    let server = Server::start(Rules::allowing(8, Refuse::Greeting), store);
    let backend = backend_from_url(&server.url_string()).unwrap();
    let mut resumed = backend
        .open_read_at("/data.bin", None, 40)
        .unwrap()
        .expect("the server supports REST");
    let mut out = Vec::new();
    resumed.read_to_end(&mut out).unwrap();
    assert_eq!(out, content[40..]);
    drop(resumed);
    assert!(server.saw("REST 40"));
    drop(backend);

    let mut store = Store::default();
    store.files.insert("/data.bin".to_string(), content.clone());
    let mut rules = Rules::allowing(8, Refuse::Greeting);
    rules.rest = false;
    let server = Server::start(rules, store);
    let backend = backend_from_url(&server.url_string()).unwrap();
    assert!(backend
        .open_read_at("/data.bin", None, 40)
        .unwrap()
        .is_none());
    // The refused REST left the connection in step.
    let mut whole = Vec::new();
    backend
        .open_read("/data.bin")
        .unwrap()
        .read_to_end(&mut whole)
        .unwrap();
    assert_eq!(whole, content);
    drop(backend);
}

#[test]
fn transfer_engine_task_ftp_folders_take_one_mkd() {
    let mut store = Store::default();
    store.dirs.insert("/old".to_string());
    store.files.insert("/file.txt".to_string(), b"x".to_vec());
    let server = Server::start(Rules::allowing(8, Refuse::Greeting), store);
    let backend = backend_from_url(&server.url_string()).unwrap();
    backend.create_dir("/new").unwrap();
    backend.create_dir("/old").unwrap();
    assert_eq!(
        backend.create_dir_new("/old").unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(
        backend.create_dir("/file.txt").unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    backend.create_dir_new("/fresh").unwrap();
    assert!(server.has_dir("/new"));
    assert!(server.has_dir("/fresh"));
    assert!(server.saw("MKD /new"));
    drop(backend);
}
