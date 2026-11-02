//! A sync comparison in checksum mode (`CompareMode::Checksum`, so
//! `HashMode::FullFresh` on both sides) against an FTP server that forms the
//! MD5 itself (`HASH` after `OPTS HASH MD5`): every remote file is keyed by
//! one control reply and none is read (no `RETR`), while the comparison still
//! tells the equal files from the one that differs. The scripted server
//! answers MLST and MLSD (passive data connections), SIZE and MDTM, and
//! records every command line of every control connection.
use super::core_impl::backend_from_url;
use crate::bisync::{self, Action, BisyncOptions, CompareMode, WalkFilter};
use crate::vfs::LocalBackend;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// The time of every remote entry (MLST/MLSD `modify`, MDTM).
const MODIFY: &str = "20260101000000";

/// Relative path, remote content below `/sum`, local content: two equal
/// files (one in a subfolder) and one of the same size that differs.
const FILES: [(&str, &[u8], &[u8]); 3] = [
    ("equal.bin", b"the same content", b"the same content"),
    ("differs.bin", b"remote content!!", b"local content!!!"),
    ("s/nested.bin", b"nested and equal", b"nested and equal"),
];

#[derive(Default)]
struct State {
    files: BTreeMap<String, Vec<u8>>,
    commands: Vec<String>,
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Server {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<State>>,
    worker: Option<JoinHandle<()>>,
}

impl Server {
    fn start(files: &[(String, Vec<u8>)]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let mut state = State::default();
        for (path, bytes) in files {
            state.files.insert(path.clone(), bytes.clone());
        }
        let state = Arc::new(Mutex::new(state));
        let worker = {
            let (stop, state) = (stop.clone(), state.clone());
            thread::spawn(move || {
                let mut sessions = Vec::new();
                let deadline = Instant::now() + Duration::from_secs(120);
                while !stop.load(Ordering::SeqCst) && Instant::now() < deadline {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            // Windows hands the listener's non-blocking mode
                            // on to accepted sockets.
                            if stream.set_nonblocking(false).is_err() {
                                continue;
                            }
                            let state = state.clone();
                            sessions.push(thread::spawn(move || serve(stream, &state)));
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                        }
                        Err(_) => break,
                    }
                }
                for session in sessions {
                    let _ = session.join();
                }
            })
        };
        Self {
            address,
            stop,
            state,
            worker: Some(worker),
        }
    }

    fn url(&self) -> String {
        format!("ftp://test:secret@127.0.0.1:{}/", self.address.port())
    }

    fn commands(&self) -> Vec<String> {
        lock(&self.state).commands.clone()
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

/// 150, the passive data connection for `work`, then 226.
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
    data.set_write_timeout(Some(Duration::from_secs(10)))?;
    let done = work(&mut data);
    drop(data);
    match done {
        Ok(()) => reply(control, "226 transfer complete"),
        Err(_) => reply(control, "426 transfer aborted"),
    }
}

fn is_dir(state: &State, path: &str) -> bool {
    let prefix = format!("{}/", path.trim_end_matches('/'));
    path == "/" || state.files.keys().any(|file| file.starts_with(&prefix))
}

/// The MLSx facts of `path`; `None` when it does not exist.
fn facts(state: &State, path: &str) -> Option<String> {
    if let Some(bytes) = state.files.get(path) {
        return Some(format!("type=file;size={};modify={MODIFY};", bytes.len()));
    }
    is_dir(state, path).then(|| format!("type=dir;modify={MODIFY};"))
}

/// The MLSD rows of the folder `path`; `None` when it is no folder.
fn listing(state: &State, path: &str) -> Option<String> {
    if !is_dir(state, path) {
        return None;
    }
    let prefix = format!("{}/", path.trim_end_matches('/'));
    let mut folders = BTreeSet::new();
    let mut rows = String::new();
    for (file, bytes) in &state.files {
        let Some(rest) = file.strip_prefix(&prefix) else {
            continue;
        };
        match rest.split_once('/') {
            Some((folder, _)) => {
                folders.insert(folder.to_string());
            }
            None => rows.push_str(&format!(
                "type=file;size={};modify={MODIFY}; {rest}\r\n",
                bytes.len()
            )),
        }
    }
    for folder in folders {
        rows.push_str(&format!("type=dir;modify={MODIFY}; {folder}\r\n"));
    }
    Some(rows)
}

/// The reply to `HASH path` (draft-bryan-ftpext-hash-02, inclusive end).
fn hash_reply(algorithm: &str, path: &str, bytes: &[u8]) -> String {
    let hex = if algorithm == "MD5" {
        format!("{:x}", md5::compute(bytes))
    } else {
        format!("{:x}", Sha256::digest(bytes))
    };
    format!("213 {algorithm} 0-{} {hex} {path}", bytes.len() - 1)
}

fn serve(mut control: TcpStream, state: &Mutex<State>) {
    let _ = control.set_read_timeout(Some(Duration::from_secs(10)));
    if reply(&mut control, "220 hashing script ready").is_err() {
        return;
    }
    let Ok(reader) = control.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(reader);
    let mut passive: Option<TcpListener> = None;
    let mut algorithm = "SHA-256";
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
        let answered = match command.to_ascii_uppercase().as_str() {
            "USER" => reply(&mut control, "331 password required"),
            "PASS" => reply(&mut control, "230 logged in"),
            "TYPE" => reply(&mut control, "200 binary"),
            "NOOP" => reply(&mut control, "200 alive"),
            "PWD" => reply(&mut control, "257 \"/\" is current directory"),
            "FEAT" => reply(
                &mut control,
                "211-Features:\r\n MLST type*;size*;modify*;\r\n MLSD\r\n SIZE\r\n MDTM\r\n HASH SHA-256*;MD5\r\n211 End",
            ),
            "OPTS" => match argument.to_ascii_uppercase().as_str() {
                "HASH MD5" => {
                    algorithm = "MD5";
                    reply(&mut control, "200 MD5")
                }
                "HASH SHA-256" => {
                    algorithm = "SHA-256";
                    reply(&mut control, "200 SHA-256")
                }
                _ => reply(&mut control, "501 option not understood"),
            },
            "HASH" => {
                let bytes = lock(state).files.get(argument).cloned();
                match bytes {
                    Some(bytes) => reply(&mut control, &hash_reply(algorithm, argument, &bytes)),
                    None => reply(&mut control, "550 not a regular file"),
                }
            }
            "MLST" => {
                let found = facts(&lock(state), argument);
                match found {
                    Some(facts) => reply(
                        &mut control,
                        &format!("250-Listing {argument}\r\n {facts} {argument}\r\n250 End"),
                    ),
                    None => reply(&mut control, "550 not found"),
                }
            }
            "SIZE" => {
                let size = lock(state).files.get(argument).map(Vec::len);
                match size {
                    Some(size) => reply(&mut control, &format!("213 {size}")),
                    None => reply(&mut control, "550 not a regular file"),
                }
            }
            "MDTM" => {
                let known = lock(state).files.contains_key(argument);
                if known {
                    reply(&mut control, &format!("213 {MODIFY}"))
                } else {
                    reply(&mut control, "550 not a regular file")
                }
            }
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
            "MLSD" => {
                let rows = listing(&lock(state), argument);
                match rows {
                    Some(rows) => transfer(&mut control, passive.take(), |data| {
                        data.write_all(rows.as_bytes())
                    }),
                    None => reply(&mut control, "550 no such folder"),
                }
            }
            // Served, so a read shows up as a recorded RETR and not as a
            // failed comparison.
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
            _ => reply(&mut control, "502 not implemented"),
        };
        if answered.is_err() {
            return;
        }
    }
}

fn rel_of(action: &Action) -> &str {
    match action {
        Action::CopyAtoB(rel)
        | Action::CopyBtoA(rel)
        | Action::FinalizeMoveAtoB(rel)
        | Action::FinalizeMoveBtoA(rel)
        | Action::DeleteA(rel)
        | Action::DeleteB(rel)
        | Action::KeepBothAtoB(rel)
        | Action::KeepBothBtoA(rel) => rel,
    }
}

#[test]
fn abgleich_task_ftp_checksum_compare_keys_every_file_by_hash_and_reads_none() {
    let remote_files: Vec<(String, Vec<u8>)> = FILES
        .iter()
        .map(|(rel, remote, _)| (format!("/sum/{rel}"), remote.to_vec()))
        .collect();
    let server = Server::start(&remote_files);
    let directory = tempfile::tempdir().unwrap();
    for (rel, _, local) in FILES {
        let path = directory.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, local).unwrap();
    }
    let root = directory.path().to_string_lossy().replace('\\', "/");
    let local = LocalBackend::new(&root);
    let remote = backend_from_url(&server.url()).unwrap();
    let ignore = bisync::empty_globset();
    let opts = BisyncOptions {
        compare: CompareMode::Checksum,
        ..BisyncOptions::default()
    };
    let preview = bisync::preview(
        &local,
        &root,
        &remote,
        "/sum",
        opts,
        &AtomicBool::new(false),
        &WalkFilter::basic(true, &ignore),
    );
    let commands = server.commands();
    assert_eq!(preview.error, None, "{commands:?}");
    assert_eq!((preview.a_files, preview.b_files), (3, 3));

    // The content keys decided: only the differing file is reported.
    let mut reported: Vec<&str> = preview
        .conflicts
        .iter()
        .map(|conflict| conflict.rel.as_str())
        .chain(preview.actions.iter().map(rel_of))
        .collect();
    reported.sort_unstable();
    reported.dedup();
    assert_eq!(reported, ["differs.bin"], "{:?}", preview.actions);

    // No remote file was read and nothing was written.
    let upper: Vec<String> = commands.iter().map(|c| c.to_ascii_uppercase()).collect();
    for word in [
        "RETR", "STOR", "APPE", "DELE", "MKD", "RMD", "RNFR", "RNTO", "MFMT",
    ] {
        assert!(
            !upper.iter().any(|c| c.split(' ').next() == Some(word)),
            "the checksum comparison sent {word}: {commands:?}"
        );
    }
    // Every remote file was keyed by the server's MD5.
    let selected = commands
        .iter()
        .position(|c| c == "OPTS HASH MD5")
        .unwrap_or_else(|| panic!("MD5 was never selected: {commands:?}"));
    for (rel, _, _) in FILES {
        let asked = format!("HASH /sum/{rel}");
        let at = commands
            .iter()
            .position(|c| *c == asked)
            .unwrap_or_else(|| panic!("{asked} was never sent: {commands:?}"));
        assert!(selected < at, "{commands:?}");
    }
    drop(remote);
}
