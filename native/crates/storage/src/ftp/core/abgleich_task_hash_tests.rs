//! `content_digest` over FTP against a scripted control-only server: the
//! digest arrives as one reply to `HASH` or `XSHA256`, and the server is
//! never asked for a data connection (no PASV, EPSV, RETR, LIST, MLSD). The
//! sync's MD5 signature (`content_md5`) is asked the same way through
//! `HASH` with MD5 or `XMD5`.
use super::core_impl::{backend_from_url, FtpBackend};
use crate::vfs::Backend;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const HEX: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
const MD5: &str = "5d41402abc4b2a76b9719d911017c592";
const SIZE: usize = 5;

#[derive(Clone)]
struct Script {
    /// The FEAT lines between `211-` and `211 End`.
    feat: Vec<&'static str>,
    hash: String,
    xsha: String,
    /// The answers to `HASH` after `OPTS HASH MD5` and to `XMD5`.
    md5_hash: String,
    xmd5: String,
    /// MDTM answers this after a hash command ran (a file written meanwhile).
    mdtm_after_hash: &'static str,
}

impl Script {
    fn with_feat(feat: &[&'static str]) -> Self {
        Self {
            feat: feat.to_vec(),
            hash: format!("213 SHA-256 0-{} {HEX} /f.bin", SIZE - 1),
            xsha: format!("250 {HEX}"),
            md5_hash: format!("213 MD5 0-{} {MD5} /f.bin", SIZE - 1),
            xmd5: format!("251 {MD5}"),
            mdtm_after_hash: "213 20260101000000",
        }
    }
}

#[derive(Default)]
struct Log {
    commands: Vec<String>,
    hashed: bool,
}

struct Server {
    port: u16,
    stop: Arc<AtomicBool>,
    log: Arc<Mutex<Log>>,
    worker: Option<JoinHandle<()>>,
}

fn lock(log: &Mutex<Log>) -> MutexGuard<'_, Log> {
    log.lock().unwrap_or_else(PoisonError::into_inner)
}

fn reply(stream: &mut TcpStream, text: &str) -> io::Result<()> {
    stream.write_all(format!("{text}\r\n").as_bytes())
}

fn serve(mut control: TcpStream, script: &Script, log: &Mutex<Log>) -> io::Result<()> {
    control.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(control.try_clone()?);
    reply(&mut control, "220 ready")?;
    let mut selected = "SHA-256";
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let line = line.trim_end().to_string();
        lock(log).commands.push(line.clone());
        let (command, argument) = line.split_once(' ').unwrap_or((line.as_str(), ""));
        match command.to_ascii_uppercase().as_str() {
            "USER" => reply(&mut control, "331 password")?,
            "PASS" => reply(&mut control, "230 in")?,
            "TYPE" => reply(&mut control, "200 binary")?,
            "PWD" => reply(&mut control, "257 \"/\" is current directory")?,
            "FEAT" => {
                let mut text = "211-Features:".to_string();
                for feature in &script.feat {
                    text.push_str(&format!("\r\n {feature}"));
                }
                text.push_str("\r\n211 End");
                reply(&mut control, &text)?
            }
            "OPTS" if argument == "HASH SHA-256" => {
                selected = "SHA-256";
                reply(&mut control, "200 SHA-256")?
            }
            "OPTS" if argument == "HASH MD5" => {
                selected = "MD5";
                reply(&mut control, "200 MD5")?
            }
            "OPTS" => reply(&mut control, "501 unknown")?,
            "HASH" if argument == "/f.bin" => {
                lock(log).hashed = true;
                let text = if selected == "MD5" {
                    &script.md5_hash
                } else {
                    &script.hash
                };
                reply(&mut control, text)?
            }
            "XMD5" if argument == "/f.bin" => {
                lock(log).hashed = true;
                reply(&mut control, &script.xmd5)?
            }
            "XSHA256" if argument == "/f.bin" => {
                lock(log).hashed = true;
                reply(&mut control, &script.xsha)?
            }
            "SIZE" if argument == "/f.bin" => reply(&mut control, &format!("213 {SIZE}"))?,
            "MDTM" if argument == "/f.bin" => {
                let text = if lock(log).hashed {
                    script.mdtm_after_hash
                } else {
                    "213 20260101000000"
                };
                reply(&mut control, text)?
            }
            "NOOP" => reply(&mut control, "200 ok")?,
            "QUIT" => return reply(&mut control, "221 bye"),
            _ => reply(&mut control, "550 refused")?,
        }
    }
}

impl Server {
    fn start(script: Script) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        let log = Arc::new(Mutex::new(Log::default()));
        let worker = {
            let (stop, log) = (stop.clone(), log.clone());
            thread::spawn(move || {
                let mut sessions = Vec::new();
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            if stream.set_nonblocking(false).is_err() {
                                continue;
                            }
                            let (script, log) = (script.clone(), log.clone());
                            sessions.push(thread::spawn(move || {
                                let _ = serve(stream, &script, &log);
                            }));
                        }
                        Err(_) => thread::sleep(Duration::from_millis(2)),
                    }
                }
                sessions.into_iter().for_each(|session| {
                    let _ = session.join();
                });
            })
        };
        Self {
            port,
            stop,
            log,
            worker: Some(worker),
        }
    }

    fn backend(&self) -> FtpBackend {
        backend_from_url(&format!("ftp://test:secret@127.0.0.1:{}/", self.port)).unwrap()
    }

    fn commands(&self) -> Vec<String> {
        lock(&self.log).commands.clone()
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

fn digest(backend: &FtpBackend) -> io::Result<Option<crate::vfs::ContentDigest>> {
    crate::vfs::content_digest(backend, "/f.bin", &AtomicBool::new(false))
}

fn md5(backend: &FtpBackend) -> io::Result<Option<String>> {
    crate::vfs::content_md5(backend, "/f.bin", &AtomicBool::new(false))
}

/// No command moved content or opened a data connection.
fn assert_no_data(commands: &[String]) {
    for command in commands {
        let word = command.split(' ').next().unwrap_or("").to_ascii_uppercase();
        assert!(
            !matches!(
                word.as_str(),
                "PASV" | "EPSV" | "PORT" | "EPRT" | "RETR" | "STOR" | "LIST" | "MLSD" | "NLST"
            ),
            "a data command was sent: {command}"
        );
    }
}

#[test]
fn abgleich_task_ftp_hash_command_gives_the_digest_without_a_data_connection() {
    let cases: [(&str, fn()); 2] = [
        (
            "abgleich_task_ftp_hash_command_gives_the_digest_without_a_data_connection",
            || {
                let server =
                    Server::start(Script::with_feat(&["SIZE", "HASH SHA-256;SHA-512*;MD5"]));
                let backend = server.backend();
                assert!(backend.extensions().is_some());
                let found = digest(&backend).unwrap().expect("the server hashed");
                assert_eq!(found.sha256, HEX);
                assert_eq!(found.size, SIZE as u64);
                let commands = server.commands();
                assert_no_data(&commands);
                let opts = commands
                    .iter()
                    .position(|c| c == "OPTS HASH SHA-256")
                    .expect("the algorithm was selected");
                let hash = commands.iter().position(|c| c == "HASH /f.bin").unwrap();
                assert!(opts < hash, "{commands:?}");
            },
        ),
        (
            "abgleich_task_ftp_md5_by_hash_command_without_a_data_connection",
            || {
                let server = Server::start(Script::with_feat(&["SIZE", "HASH SHA-256;MD5*"]));
                let backend = server.backend();
                assert_eq!(md5(&backend).unwrap().as_deref(), Some(MD5));
                let commands = server.commands();
                assert_no_data(&commands);
                let opts = commands
                    .iter()
                    .position(|c| c == "OPTS HASH MD5")
                    .expect("the algorithm was selected");
                let hash = commands.iter().position(|c| c == "HASH /f.bin").unwrap();
                assert!(opts < hash, "{commands:?}");
                // The SHA-256 question after it selects its own algorithm again.
                assert_eq!(digest(&backend).unwrap().expect("hashed").sha256, HEX);
            },
        ),
    ];
    for (case, check) in cases {
        std::panic::catch_unwind(check).unwrap_or_else(|_| panic!("{case}"));
    }
}

#[test]
fn abgleich_task_ftp_xsha256_command_gives_the_digest_without_a_data_connection() {
    let cases: [(&str, fn()); 2] = [
        (
            "abgleich_task_ftp_xsha256_command_gives_the_digest_without_a_data_connection",
            || {
                let server = Server::start(Script::with_feat(&["SIZE", "XCRC", "XSHA256"]));
                let backend = server.backend();
                let found = digest(&backend).unwrap().expect("the server hashed");
                assert_eq!(found.sha256, HEX);
                let commands = server.commands();
                assert_no_data(&commands);
                assert!(commands.iter().any(|c| c == "XSHA256 /f.bin"));
                assert!(!commands.iter().any(|c| c.starts_with("HASH")));
            },
        ),
        (
            "abgleich_task_ftp_md5_by_xmd5_command_takes_251_250_and_213",
            || {
                for reply in [
                    format!("251 {MD5}"),
                    format!("250 {}", MD5.to_ascii_uppercase()),
                    format!("213 {MD5} f.bin"),
                ] {
                    let mut script = Script::with_feat(&["SIZE", "XCRC", "XMD5"]);
                    script.xmd5 = reply.clone();
                    let server = Server::start(script);
                    assert_eq!(
                        md5(&server.backend()).unwrap().as_deref(),
                        Some(MD5),
                        "{reply}"
                    );
                    let commands = server.commands();
                    assert_no_data(&commands);
                    assert!(commands.iter().any(|c| c == "XMD5 /f.bin"));
                    assert!(!commands.iter().any(|c| c.starts_with("HASH")));
                }
            },
        ),
    ];
    for (case, check) in cases {
        std::panic::catch_unwind(check).unwrap_or_else(|_| panic!("{case}"));
    }
}

#[test]
fn abgleich_task_ftp_server_without_hash_commands_leaves_the_content_to_the_caller() {
    let cases: [(&str, fn()); 2] = [
        (
            "abgleich_task_ftp_server_without_hash_commands_leaves_the_content_to_the_caller",
            || {
                let server = Server::start(Script::with_feat(&[
                    "SIZE",
                    "MDTM",
                    "XCRC",
                    "XMD5",
                    "HASH MD5;SHA-1*",
                ]));
                let backend = server.backend();
                assert_eq!(digest(&backend).unwrap(), None);
                let commands = server.commands();
                assert!(
                    !commands.iter().any(|c| c.starts_with("HASH")
                        || c.starts_with("XSHA256")
                        || c.starts_with("OPTS")),
                    "{commands:?}"
                );
            },
        ),
        (
            "abgleich_task_ftp_md5_without_an_md5_command_leaves_the_content_to_the_caller",
            || {
                let server = Server::start(Script::with_feat(&[
                    "SIZE",
                    "MDTM",
                    "XSHA256",
                    "HASH SHA-256;SHA-1*",
                ]));
                assert_eq!(md5(&server.backend()).unwrap(), None);
                let commands = server.commands();
                assert!(
                    !commands.iter().any(|c| c.starts_with("HASH")
                        || c.starts_with("XMD5")
                        || c.starts_with("XSHA256")
                        || c.starts_with("OPTS")),
                    "{commands:?}"
                );
            },
        ),
    ];
    for (case, check) in cases {
        std::panic::catch_unwind(check).unwrap_or_else(|_| panic!("{case}"));
    }
}

#[test]
fn abgleich_task_ftp_digest_of_another_algorithm_or_range_is_not_taken() {
    let cases: [(&str, fn()); 2] = [
        (
            "abgleich_task_ftp_digest_of_another_algorithm_or_range_is_not_taken",
            || {
                let features = ["SIZE", "HASH SHA-256"];
                for reply in [
                    format!("213 SHA-512 0-4 {HEX} /f.bin"),
                    format!("213 SHA-256 0-2 {HEX} /f.bin"),
                    format!("213 SHA-256 1-4 {HEX} /f.bin"),
                    "213 SHA-256 0-4 nothex /f.bin".to_string(),
                    "550 no such file".to_string(),
                    "556 refused for policy reasons".to_string(),
                ] {
                    let mut script = Script::with_feat(&features);
                    script.hash = reply.clone();
                    let server = Server::start(script);
                    let backend = server.backend();
                    assert_eq!(digest(&backend).unwrap(), None, "{reply}");
                    assert_no_data(&server.commands());
                }
            },
        ),
        (
            "abgleich_task_ftp_md5_of_another_algorithm_or_range_is_not_taken",
            || {
                for (features, field, reply) in [
                    (
                        ["SIZE", "HASH MD5"],
                        0,
                        format!("213 SHA-1 0-4 {MD5} /f.bin"),
                    ),
                    (["SIZE", "HASH MD5"], 0, format!("213 MD5 0-2 {MD5} /f.bin")),
                    (["SIZE", "HASH MD5"], 0, format!("213 MD5 0-4 {HEX} /f.bin")),
                    (
                        ["SIZE", "HASH MD5"],
                        0,
                        "213 MD5 0-4 nothex /f.bin".to_string(),
                    ),
                    (
                        ["SIZE", "HASH MD5"],
                        0,
                        "556 refused for policy reasons".to_string(),
                    ),
                    (["SIZE", "XMD5"], 1, format!("251 {HEX}")),
                    (["SIZE", "XMD5"], 1, format!("251 {MD5} {MD5}")),
                    (["SIZE", "XMD5"], 1, "550 no such file".to_string()),
                    (["SIZE", "XMD5"], 1, format!("500 {MD5}")),
                ] {
                    let mut script = Script::with_feat(&features);
                    if field == 0 {
                        script.md5_hash = reply.clone();
                    } else {
                        script.xmd5 = reply.clone();
                    }
                    let server = Server::start(script);
                    assert_eq!(md5(&server.backend()).unwrap(), None, "{reply}");
                    assert_no_data(&server.commands());
                }
            },
        ),
    ];
    for (case, check) in cases {
        std::panic::catch_unwind(check).unwrap_or_else(|_| panic!("{case}"));
    }
}

#[test]
fn abgleich_task_ftp_hash_range_may_name_the_size_itself() {
    let mut script = Script::with_feat(&["SIZE", "HASH SHA-256"]);
    script.hash = format!("213 SHA-256 0-{SIZE} {HEX} /f.bin");
    let server = Server::start(script);
    assert!(digest(&server.backend()).unwrap().is_some());
}

#[test]
fn abgleich_task_ftp_file_written_while_hashed_is_an_error_not_a_digest() {
    let cases: [(&str, fn()); 2] = [
        (
            "abgleich_task_ftp_file_written_while_hashed_is_an_error_not_a_digest",
            || {
                let mut script = Script::with_feat(&["SIZE", "HASH SHA-256"]);
                script.mdtm_after_hash = "213 20260101000100";
                let server = Server::start(script);
                let error = digest(&server.backend()).unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            },
        ),
        (
            "abgleich_task_ftp_md5_of_a_file_written_while_hashed_is_an_error",
            || {
                let mut script = Script::with_feat(&["SIZE", "HASH MD5"]);
                script.mdtm_after_hash = "213 20260101000100";
                let server = Server::start(script);
                let error = md5(&server.backend()).unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            },
        ),
    ];
    for (case, check) in cases {
        std::panic::catch_unwind(check).unwrap_or_else(|_| panic!("{case}"));
    }
}

#[test]
fn abgleich_task_ftp_digest_replies_parse_strictly() {
    let cases: [(&str, fn()); 2] = [
        ("abgleich_task_ftp_digest_replies_parse_strictly", || {
            use super::content_digest::{parse_bare_reply, parse_hash_reply};
            assert_eq!(
                parse_hash_reply(
                    &format!("213 sha-256 0-9 {} x y", HEX.to_ascii_uppercase()),
                    10
                )
                .as_deref(),
                Some(HEX)
            );
            assert_eq!(
                parse_hash_reply(&format!("213 SHA-256 0-0 {HEX} e"), 0).as_deref(),
                Some(HEX)
            );
            assert_eq!(
                parse_hash_reply(&format!("250 SHA-256 0-9 {HEX} x"), 10),
                None
            );
            assert_eq!(
                parse_bare_reply(&format!("250 {HEX}")).as_deref(),
                Some(HEX)
            );
            assert_eq!(
                parse_bare_reply(&format!("213 {HEX} file")).as_deref(),
                Some(HEX)
            );
            assert_eq!(
                parse_bare_reply(&format!("250 {HEX} {HEX}")),
                None,
                "two digests are none"
            );
            assert_eq!(
                parse_bare_reply("250 d41d8cd98f00b204e9800998ecf8427e"),
                None,
                "an MD5 is no SHA-256"
            );
            assert_eq!(parse_bare_reply(&format!("500 {HEX}")), None);
        }),
        ("abgleich_task_ftp_md5_replies_parse_strictly", || {
            use super::content_digest::{
                parse_bare_reply_for, parse_hash_reply_for, Algorithm::Md5,
            };
            assert_eq!(
                parse_hash_reply_for(&format!("213 md5 0-9 {MD5} x"), 10, Md5).as_deref(),
                Some(MD5)
            );
            assert_eq!(
                parse_hash_reply_for(&format!("213 SHA-256 0-9 {MD5} x"), 10, Md5),
                None
            );
            assert_eq!(
                parse_bare_reply_for(&format!("251 {MD5}"), Md5).as_deref(),
                Some(MD5)
            );
            assert_eq!(parse_bare_reply_for(&format!("250 {HEX}"), Md5), None);
            assert_eq!(parse_bare_reply_for(&format!("500 {MD5}"), Md5), None);
            assert_eq!(
                parse_bare_reply_for(&format!("251 {MD5} {MD5}"), Md5),
                None,
                "two digests are none"
            );
        }),
    ];
    for (case, check) in cases {
        std::panic::catch_unwind(check).unwrap_or_else(|_| panic!("{case}"));
    }
}
