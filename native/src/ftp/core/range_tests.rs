//! Ranges over FTP (E13, U48-8, security answers S12 and S6 a): REST and a
//! RETR cut with ABOR after the range, only on connections that are logged in
//! and free already, never a new login for a range; and the stage probe that
//! is skipped for names made from the system's random source. A scripted
//! passive-mode server on loopback serves one 64 KiB file, `/f`.
use super::backend_from_url;
use super::data_overlap_server::{lock as lock_events, Server as ProbeServer};
use super::pool::ranges_possible;
use super::staging::{from_system_random, probe_needed, require_stage_free};
use crate::vfs::{congestion_of, Backend};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const FILE: usize = 64 * 1024;

fn content() -> Vec<u8> {
    (0..FILE).map(|index| (index % 251) as u8).collect()
}

struct Rules {
    rest: bool,
    /// RETR is answered 421 once set.
    refuse: AtomicBool,
    /// The data connection stays open after the last byte until the client
    /// sends ABOR (a file that is still being sent).
    hold: bool,
}

struct Server {
    address: SocketAddr,
    logins: Arc<AtomicUsize>,
    events: Arc<Mutex<Vec<String>>>,
    rules: Arc<Rules>,
    stop: Arc<AtomicBool>,
}

impl Server {
    fn start(rest: bool, hold: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let server = Self {
            address,
            logins: Arc::new(AtomicUsize::new(0)),
            events: Arc::new(Mutex::new(Vec::new())),
            rules: Arc::new(Rules {
                rest,
                refuse: AtomicBool::new(false),
                hold,
            }),
            stop: Arc::new(AtomicBool::new(false)),
        };
        let (logins, events, rules, stop) = (
            server.logins.clone(),
            server.events.clone(),
            server.rules.clone(),
            server.stop.clone(),
        );
        thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(30);
            while !stop.load(Ordering::SeqCst) && Instant::now() < deadline {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (logins, events, rules) =
                            (logins.clone(), events.clone(), rules.clone());
                        thread::spawn(move || {
                            let _ = serve(stream, &logins, &events, &rules);
                        });
                    }
                    Err(_) => thread::sleep(Duration::from_millis(2)),
                }
            }
        });
        server
    }

    fn backend(&self) -> impl Backend {
        backend_from_url(&format!(
            "ftp://test:secret@127.0.0.1:{}/",
            self.address.port()
        ))
        .unwrap()
    }

    fn logins(&self) -> usize {
        self.logins.load(Ordering::SeqCst)
    }

    fn saw(&self, command: &str) -> bool {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .any(|seen| seen == command)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn reply(control: &mut TcpStream, line: &str) -> io::Result<()> {
    control.write_all(format!("{line}\r\n").as_bytes())
}

fn serve(
    stream: TcpStream,
    logins: &AtomicUsize,
    events: &Mutex<Vec<String>>,
    rules: &Rules,
) -> io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut control = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    let mut passive: Option<TcpListener> = None;
    let mut rest = 0usize;
    reply(&mut control, "220 ready")?;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let line = line.trim_end().to_string();
        events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(line.clone());
        let (command, argument) = line.split_once(' ').unwrap_or((line.as_str(), ""));
        match command {
            "USER" => {
                logins.fetch_add(1, Ordering::SeqCst);
                reply(&mut control, "331 password")?
            }
            "PASS" => reply(&mut control, "230 in")?,
            "TYPE" => reply(&mut control, "200 binary")?,
            "PWD" => reply(&mut control, "257 \"/\" is current directory")?,
            "NOOP" => reply(&mut control, "200 alive")?,
            "PASV" => {
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
            "REST" if !rules.rest => reply(&mut control, "502 Command not implemented")?,
            "REST" => {
                rest = argument.parse().unwrap_or(0);
                reply(&mut control, &format!("350 Restarting at {rest}"))?
            }
            "RETR" if rules.refuse.load(Ordering::SeqCst) => {
                // The client sends RETR before it connects to the data port
                // (suppaftp `data_command`). A server that closes the port
                // first resets that connect, and the client never reads the
                // 421. A real server has the port open: take the connection,
                // then refuse, and end the control stream with a FIN.
                let data = accept_data(passive.take())?;
                reply(&mut control, "421 Service not available")?;
                control.shutdown(std::net::Shutdown::Write)?;
                drop(data);
                return Ok(());
            }
            "RETR" if argument == "/f" => {
                let offset = std::mem::take(&mut rest).min(FILE);
                // A range that stops short of the end is still being sent.
                let hold = rules.hold && offset > 0 && offset < FILE - 1_000;
                retrieve(
                    &mut control,
                    &mut reader,
                    events,
                    passive.take(),
                    offset,
                    hold,
                )?
            }
            "RETR" => reply(&mut control, "550 No such file")?,
            "QUIT" => return reply(&mut control, "221 bye"),
            _ => reply(&mut control, "500 unsupported")?,
        }
    }
}

/// The client's data connection on the passive port opened by PASV.
fn accept_data(passive: Option<TcpListener>) -> io::Result<TcpStream> {
    let listener = passive.ok_or_else(|| io::Error::other("no PASV"))?;
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + Duration::from_secs(3);
    let data = loop {
        match listener.accept() {
            Ok((data, _)) => break data,
            Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(2)),
            Err(error) => return Err(error),
        }
    };
    data.set_nonblocking(false)?;
    Ok(data)
}

/// 150, the bytes from `offset`, then 226; with `hold` the data connection
/// stays open until the client's ABOR, answered 426 and 226.
fn retrieve(
    control: &mut TcpStream,
    reader: &mut BufReader<TcpStream>,
    events: &Mutex<Vec<String>>,
    passive: Option<TcpListener>,
    offset: usize,
    hold: bool,
) -> io::Result<()> {
    let mut data = accept_data(passive)?;
    reply(control, "150 opening data connection")?;
    data.write_all(&content()[offset..])?;
    if !hold {
        drop(data);
        return reply(control, "226 done");
    }
    let mut line = String::new();
    reader
        .get_ref()
        .set_read_timeout(Some(Duration::from_secs(3)))?;
    if reader.read_line(&mut line).is_ok() && line.contains("ABOR") {
        events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(line.trim_end().to_string());
        drop(data);
        reply(control, "426 Transfer aborted")?;
        return reply(control, "226 Abort successful");
    }
    drop(data);
    reply(control, "226 done")
}

fn read_range(backend: &impl Backend, offset: u64, len: u64) -> io::Result<Option<Vec<u8>>> {
    let Some(mut reader) = backend.open_read_range("/f", None, offset, len)? else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

/// Two logged-in free pooled connections: two readers open at once, read to
/// the end and dropped.
fn warm(backend: &impl Backend) {
    let mut first = backend.open_read("/f").unwrap();
    let mut second = backend.open_read("/f").unwrap();
    let mut sink = Vec::new();
    first.read_to_end(&mut sink).unwrap();
    second.read_to_end(&mut sink).unwrap();
}

#[test]
fn speed_task_ftp_range_reader_aborts_after_len_and_resyncs() {
    let server = Server::start(true, true);
    let backend = server.backend();
    warm(&backend);
    assert!(backend.supports_range_reads("/f"));
    let logins = server.logins();
    let bytes = read_range(&backend, 1_000, 10_000)
        .unwrap()
        .expect("ranges");
    assert_eq!(bytes, content()[1_000..11_000], "exactly len bytes");
    assert!(server.saw("REST 1000"));
    assert!(server.saw("ABOR"), "the transfer is cut after the range");
    // The connection went back in step (PWD answered 257 after the abort):
    // the next range runs on it without a new login.
    let again = read_range(&backend, 20_000, 5_000)
        .unwrap()
        .expect("ranges");
    assert_eq!(again, content()[20_000..25_000]);
    assert_eq!(server.logins(), logins, "no login for a range");
    // A range that ends where the file ends is finished normally.
    let tail = read_range(&backend, (FILE - 100) as u64, 100)
        .unwrap()
        .unwrap();
    assert_eq!(tail, content()[FILE - 100..]);
}

#[test]
fn speed_task_ftp_range_without_rest_returns_none() {
    let server = Server::start(false, false);
    let backend = server.backend();
    warm(&backend);
    let logins = server.logins();
    assert!(backend.supports_range_reads("/f"));
    assert!(read_range(&backend, 1_000, 100).unwrap().is_none());
    assert_eq!(server.logins(), logins);
}

#[test]
fn speed_task_ftp_range_needs_two_connections() {
    let server = Server::start(true, false);
    let backend = server.backend();
    // Nothing is free yet: no ranges, and asking does not log in.
    assert!(!backend.supports_range_reads("/f"));
    let logins = server.logins();
    let error = read_range(&backend, 0, 100)
        .err()
        .expect("no free connection");
    assert!(congestion_of(&error).is_some(), "{error}");
    assert_eq!(server.logins(), logins, "a range never logs in");
    // One free connection is not enough either.
    let mut single = backend.open_read("/f").unwrap();
    single.read_to_end(&mut Vec::new()).unwrap();
    drop(single);
    assert!(!backend.supports_range_reads("/f"));
    // Two are.
    warm(&backend);
    assert!(backend.supports_range_reads("/f"));
    // The rule as a function: two idle, no refusal, room for two transfers.
    let now = Instant::now();
    assert!(ranges_possible(2, None, None, now));
    assert!(ranges_possible(2, None, Some(2), now));
    assert!(!ranges_possible(1, None, None, now));
    assert!(!ranges_possible(2, None, Some(1), now));
    assert!(!ranges_possible(
        2,
        Some(now + Duration::from_secs(60)),
        None,
        now
    ));
    assert!(ranges_possible(2, Some(now), None, now));
}

#[test]
fn speed_task_ftp_range_421_ends_ranges_without_new_logins() {
    let server = Server::start(true, false);
    let backend = server.backend();
    warm(&backend);
    assert!(backend.supports_range_reads("/f"));
    let logins = server.logins();
    server.rules.refuse.store(true, Ordering::SeqCst);
    // The refusal is not an error of the file: it goes as one stream.
    assert!(read_range(&backend, 1_000, 100).unwrap().is_none());
    // Ranges stay off, and nothing logged in again to try.
    assert!(!backend.supports_range_reads("/f"));
    assert_eq!(server.logins(), logins, "no loop of new logins");
}

#[test]
fn speed_task_ftp_stage_probe_is_skipped_only_for_system_random_names() {
    // Names made from the system's random source: `.se-upload-` and 16
    // lowercase hex digits at the end.
    assert!(from_system_random(
        "/dir/file.bin.se-upload-00ff00ff00ff00ff"
    ));
    assert!(from_system_random("a.se-upload-0123456789abcdef"));
    // The stand-in (starts with `w`), upper case, a wrong length, another
    // purpose and a marker that is not at the end are probed.
    assert!(!from_system_random("/dir/f.se-upload-w0ff00ff00ff00ff"));
    assert!(!from_system_random("/dir/f.se-upload-00FF00FF00FF00FF"));
    assert!(!from_system_random("/dir/f.se-upload-00ff"));
    assert!(!from_system_random("/dir/f.se-sync-00ff00ff00ff00ff"));
    assert!(!from_system_random(
        "/dir/f.se-upload-00ff00ff00ff00ff.part"
    ));
    assert!(!from_system_random("/dir/plain.bin"));
    assert!(!probe_needed(false, "/d/f.se-upload-00ff00ff00ff00ff"));
    assert!(probe_needed(false, "/d/f.se-upload-w0ff00ff00ff00ff"));
    assert!(probe_needed(true, "/d/f.se-upload-00ff00ff00ff00ff"));
    // On the wire: no look at a system-random name, a look at the others.
    let server = ProbeServer::start();
    let connection = server.connection(|address| {
        TcpStream::connect(address).map_err(suppaftp::FtpError::ConnectionError)
    });
    require_stage_free(&connection, "/f.se-upload-00ff00ff00ff00ff").unwrap();
    let looked = |name: &str| {
        lock_events(&server.state.events).iter().any(|line| {
            line.ends_with(name) && (line.starts_with("SIZE") || line.starts_with("MLST"))
        })
    };
    assert!(!looked("/f.se-upload-00ff00ff00ff00ff"), "no probe");
    require_stage_free(&connection, "/f.se-upload-w0ff00ff00ff00ff").unwrap();
    assert!(looked("/f.se-upload-w0ff00ff00ff00ff"), "probed");
    // A taken name is still refused when it is probed.
    lock_events(&server.state.files).insert("/g.se-upload-w000000000000000".into(), vec![1]);
    let error = require_stage_free(&connection, "/g.se-upload-w000000000000000")
        .err()
        .expect("a taken name");
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
}
