//! The loopback attach service of one owner process. Every client of a
//! hosted session, the owner's own front-end included, connects here and
//! proves the session's owner token or attach token in its first frame, a
//! hello of at most 4 KiB that must arrive completely within five seconds;
//! anything else is refused with one generic answer. Only 127.0.0.1 is
//! bound. The owner token never leaves this process, the attach token only
//! reaches the owner-only session entry of an attachable session.
use super::engine::{Engine, Outbound, OUTBOUND_CAPACITY};
use super::types::{Activity, OwnEvent, Role, Stream};
use super::wire;
use crossbeam_channel::{Receiver, Sender};
use std::collections::HashMap;
use std::io::{self, Read};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// Unauthenticated connections served at once; more are closed at once.
const MAX_HANDSHAKES: usize = 16;
/// A write stall after which an attached client's connection is closed.
const ATTACHED_WRITE_TIMEOUT: Duration = Duration::from_secs(30);
/// Longest participant label shown to others.
const MAX_LABEL_CHARS: usize = 64;

pub(super) struct AttachServer {
    port: u16,
    hosted: Mutex<HashMap<String, Hosted>>,
    handshakes: AtomicUsize,
}

struct Hosted {
    engine: Arc<Engine>,
    owner_token: String,
    attach_token: String,
}

static SERVER: Mutex<Option<Arc<AttachServer>>> = Mutex::new(None);

/// The attach service of this process, started on first use.
pub(super) fn shared() -> Result<Arc<AttachServer>, String> {
    let mut slot = SERVER
        .lock()
        .map_err(|_| "the attach service lock is poisoned".to_string())?;
    if let Some(server) = slot.as_ref() {
        return Ok(server.clone());
    }
    let server = AttachServer::start()?;
    *slot = Some(server.clone());
    Ok(server)
}

impl AttachServer {
    fn start() -> Result<Arc<Self>, String> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|error| format!("cannot open the session attach service: {error}"))?;
        let port = listener
            .local_addr()
            .map_err(|error| format!("cannot open the session attach service: {error}"))?
            .port();
        let server = Arc::new(Self {
            port,
            hosted: Mutex::new(HashMap::new()),
            handshakes: AtomicUsize::new(0),
        });
        let accepting = server.clone();
        std::thread::Builder::new()
            .name("se-shell-attach".into())
            .spawn(move || accepting.accept(listener))
            .map_err(|error| format!("cannot start the session attach service: {error}"))?;
        Ok(server)
    }

    pub(super) fn port(&self) -> u16 {
        self.port
    }

    /// Serves `engine` until its session ends; returns the owner token and
    /// the attach token.
    pub(super) fn host(self: &Arc<Self>, engine: &Arc<Engine>) -> Result<(String, String), String> {
        let owner_token = token()?;
        let attach_token = token()?;
        self.hosted
            .lock()
            .map_err(|_| "the attach service lock is poisoned".to_string())?
            .insert(
                engine.meta.id.clone(),
                Hosted {
                    engine: engine.clone(),
                    owner_token: owner_token.clone(),
                    attach_token: attach_token.clone(),
                },
            );
        let server = self.clone();
        let id = engine.meta.id.clone();
        engine.on_end(Box::new(move || {
            if let Ok(mut hosted) = server.hosted.lock() {
                hosted.remove(&id);
            }
        }));
        Ok((owner_token, attach_token))
    }

    fn accept(self: Arc<Self>, listener: TcpListener) {
        for stream in listener.incoming() {
            let Ok(stream) = stream else {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            };
            if self.handshakes.fetch_add(1, Ordering::SeqCst) >= MAX_HANDSHAKES {
                self.handshakes.fetch_sub(1, Ordering::SeqCst);
                let _ = stream.shutdown(Shutdown::Both);
                continue;
            }
            let server = self.clone();
            let spawned = std::thread::Builder::new()
                .name("se-shell-client".into())
                .spawn(move || server.serve(stream));
            if spawned.is_err() {
                self.handshakes.fetch_sub(1, Ordering::SeqCst);
            }
        }
    }

    fn serve(&self, stream: TcpStream) {
        let admitted = self.handshake(&stream);
        self.handshakes.fetch_sub(1, Ordering::SeqCst);
        match admitted {
            Some((engine, role, label)) => {
                let _ = connection(engine, role, &label, stream);
            }
            None => {
                let _ = stream.shutdown(Shutdown::Both);
            }
        }
    }

    fn handshake(&self, stream: &TcpStream) -> Option<(Arc<Engine>, Role, String)> {
        let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
        stream.set_nodelay(true).ok()?;
        stream.set_write_timeout(Some(HANDSHAKE_TIMEOUT)).ok()?;
        let mut reader = DeadlineReader { stream, deadline };
        let payload = wire::read_hello(&mut reader).ok()?;
        let hello: wire::Hello = wire::decode(&payload).ok()?;
        if hello.version != wire::PROTOCOL_VERSION {
            return deny(stream, "unsupported attach protocol version");
        }
        let admitted = {
            let hosted = self.hosted.lock().ok()?;
            match hosted.get(&hello.session) {
                Some(entry) if constant_time_eq(&entry.owner_token, &hello.token) => {
                    Some((entry.engine.clone(), Role::Owner))
                }
                Some(entry)
                    if entry.engine.meta.attachable
                        && constant_time_eq(&entry.attach_token, &hello.token) =>
                {
                    Some((entry.engine.clone(), Role::Attached))
                }
                _ => None,
            }
        };
        let Some((engine, role)) = admitted else {
            return deny(stream, "permission denied");
        };
        stream.set_read_timeout(None).ok()?;
        Some((engine, role, label(&hello.label)))
    }
}

/// Reads the first frame under one deadline for the whole frame: every read
/// waits at most for the time left, so a client that sends slowly cannot
/// hold a handshake slot beyond the deadline.
struct DeadlineReader<'a> {
    stream: &'a TcpStream,
    deadline: Instant,
}

impl Read for DeadlineReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let left = self
            .deadline
            .checked_duration_since(Instant::now())
            .filter(|rest| !rest.is_zero())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::TimedOut, "the attach handshake timed out")
            })?;
        self.stream.set_read_timeout(Some(left))?;
        let mut stream = self.stream;
        stream.read(buffer)
    }
}

/// Serves one admitted client until it leaves or its connection fails. The
/// owner leaving closes the session; an attached client detaches.
fn connection(engine: Arc<Engine>, role: Role, label: &str, stream: TcpStream) -> io::Result<()> {
    let (sender, outbound) = crossbeam_channel::bounded(OUTBOUND_CAPACITY);
    let replies = sender.clone();
    let id = match engine.join(role, label, sender) {
        Ok(id) => id,
        Err(message) => {
            let _ = deny::<()>(&stream, &message);
            let _ = stream.shutdown(Shutdown::Both);
            return Ok(());
        }
    };
    let result = serve_admitted(&engine, id, role, label, &stream, replies, outbound);
    match role {
        Role::Owner => engine.close(),
        Role::Attached => engine.leave(id),
    }
    let _ = stream.shutdown(Shutdown::Read);
    result
}

fn serve_admitted(
    engine: &Arc<Engine>,
    id: u64,
    role: Role,
    label: &str,
    stream: &TcpStream,
    replies: Sender<Outbound>,
    outbound: Receiver<Outbound>,
) -> io::Result<()> {
    let mut writer = stream.try_clone()?;
    writer.set_write_timeout(match role {
        Role::Owner => None,
        Role::Attached => Some(ATTACHED_WRITE_TIMEOUT),
    })?;
    wire::write_json(&mut writer, wire::WELCOME, &engine.info(role))?;
    std::thread::Builder::new()
        .name("se-shell-forward".into())
        .spawn(move || forward(writer, outbound))?;
    let mut reader = stream;
    loop {
        let (tag, payload) = wire::read_frame(&mut reader)?;
        match tag {
            wire::SUBMIT => {
                let submit: wire::Submit = wire::decode(&payload)?;
                if let Err(message) = engine.submit(id, label, submit.tag, submit.request) {
                    let rejected = Outbound::Rejected {
                        tag: submit.tag,
                        message,
                    };
                    if replies
                        .send_timeout(rejected, ATTACHED_WRITE_TIMEOUT)
                        .is_err()
                    {
                        return Ok(());
                    }
                }
            }
            // Input that no own interactive command reads is dropped.
            wire::INPUT => {
                let _ = engine.input(id, &payload);
            }
            wire::CANCEL if payload.is_empty() => engine.cancel(label),
            wire::LEAVE if payload.is_empty() => return Ok(()),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unexpected attach frame",
                ))
            }
        }
    }
}

fn forward(mut writer: TcpStream, outbound: Receiver<Outbound>) {
    for message in outbound {
        if write_outbound(&mut writer, message).is_err() {
            break;
        }
    }
    let _ = writer.shutdown(Shutdown::Both);
}

fn write_outbound(writer: &mut TcpStream, message: Outbound) -> io::Result<()> {
    match message {
        Outbound::Own(OwnEvent::Output { tag, stream, bytes }) => {
            let frame = match stream {
                Stream::Stdout => wire::OWN_STDOUT,
                Stream::Stderr => wire::OWN_STDERR,
            };
            for chunk in bytes.chunks(wire::MAX_DATA) {
                wire::write_frame(writer, frame, &wire::tagged(tag, chunk))?;
            }
            Ok(())
        }
        Outbound::Own(OwnEvent::Done { tag, outcome }) => {
            wire::write_json(writer, wire::OWN_DONE, &wire::OwnDone { tag, outcome })
        }
        Outbound::Rejected { tag, message } => {
            wire::write_json(writer, wire::REJECTED, &wire::Rejected { tag, message })
        }
        Outbound::Activity(Activity::Started { by, command }) => {
            wire::write_json(writer, wire::ACT_STARTED, &wire::ActStarted { by, command })
        }
        Outbound::Activity(Activity::Output { stream, bytes }) => {
            let frame = match stream {
                Stream::Stdout => wire::ACT_STDOUT,
                Stream::Stderr => wire::ACT_STDERR,
            };
            for chunk in bytes.chunks(wire::MAX_DATA) {
                wire::write_frame(writer, frame, chunk)?;
            }
            Ok(())
        }
        Outbound::Activity(Activity::Done { by, exit_code, cwd }) => wire::write_json(
            writer,
            wire::ACT_DONE,
            &wire::ActDone { by, exit_code, cwd },
        ),
        Outbound::Activity(Activity::Notice(text)) => {
            wire::write_json(writer, wire::NOTICE, &wire::Text { text })
        }
        Outbound::Activity(Activity::Ended { exit_code, message }) => {
            wire::write_json(writer, wire::ENDED, &wire::Ended { exit_code, message })
        }
    }
}

fn deny<T>(stream: &TcpStream, message: &str) -> Option<T> {
    let mut writer = stream;
    let _ = wire::write_json(
        &mut writer,
        wire::DENIED,
        &wire::Text {
            text: message.to_string(),
        },
    );
    None
}

/// The label others see: printable characters only, bounded.
fn label(given: &str) -> String {
    let text: String = given
        .chars()
        .filter(|ch| !ch.is_control())
        .take(MAX_LABEL_CHARS)
        .collect();
    if text.trim().is_empty() {
        "client".to_string()
    } else {
        text
    }
}

fn token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| format!("cannot create a session token: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .bytes()
            .zip(right.bytes())
            .fold(0u8, |difference, (left, right)| difference | (left ^ right))
            == 0
}

#[cfg(test)]
mod tests {
    use super::{constant_time_eq, label};

    #[test]
    fn shell_session_task_tokens_compare_exactly_and_labels_are_printable() {
        assert!(constant_time_eq("abcd", "abcd"));
        assert!(!constant_time_eq("abcd", "abce"));
        assert!(!constant_time_eq("abcd", "abc"));
        assert_eq!(label("se shell\x1b[2J (pid 1)"), "se shell[2J (pid 1)");
        assert_eq!(label("\x07"), "client");
        assert_eq!(label(&"x".repeat(100)).len(), 64);
    }
}
