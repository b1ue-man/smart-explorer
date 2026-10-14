//! Client side of a terminal session, for the owner's own front-end as for
//! every attached `se shell`: it sends the bytes the user types and the size
//! of the user's terminal, and hands what the session sends (the remote
//! terminal's bytes, notices, the end) to one receiver, in order and without
//! loss. Dropping the last handle leaves the session (the owner ends it, an
//! attached client detaches).
use super::client::handshake;
use super::registry::Entry;
use super::terminal_engine::printable;
use super::types::{SessionKind, TerminalEvent, TerminalInfo, TerminalSize};
use super::wire;
use crossbeam_channel::{Receiver, Sender};
use std::net::{Shutdown, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Events waiting for the front-end. A full queue stops reading the
/// connection, so the session's output queue for this client fills in turn:
/// the owner's front-end slows the remote terminal down, as a slow local
/// terminal slows down `ssh`.
const EVENT_CAPACITY: usize = 256;

#[derive(Clone)]
pub(in crate::cli) struct TerminalClient {
    inner: Arc<Inner>,
    events: Receiver<TerminalEvent>,
    _leave: Arc<LeaveOnDrop>,
}

struct Inner {
    writer: Mutex<TcpStream>,
    /// Closes the connection when the writer is busy with a stuck write.
    closer: TcpStream,
    info: TerminalInfo,
    ended: Mutex<Option<(Option<i32>, String)>>,
    ended_changed: Condvar,
}

struct LeaveOnDrop(Arc<Inner>);

impl Drop for LeaveOnDrop {
    fn drop(&mut self) {
        self.0.leave();
    }
}

impl TerminalClient {
    /// Joins the session `session` of the attach service on `port` with
    /// `token`; `size` is the size of this client's terminal.
    pub(super) fn connect(
        port: u16,
        session: &str,
        token: &str,
        label: &str,
        size: TerminalSize,
    ) -> Result<Self, String> {
        size.validate()?;
        let (stream, welcome) =
            handshake(port, session, token, label, wire::PROTOCOL_VERSION_TERMINAL)?;
        let info: TerminalInfo = wire::decode(&welcome).map_err(|error| error.to_string())?;
        if info.kind != SessionKind::Terminal {
            return Err("the session is not a terminal session".into());
        }
        let receiving = stream.try_clone().map_err(|error| error.to_string())?;
        let closer = stream.try_clone().map_err(|error| error.to_string())?;
        let (sender, events) = crossbeam_channel::bounded(EVENT_CAPACITY);
        let inner = Arc::new(Inner {
            writer: Mutex::new(stream),
            closer,
            info,
            ended: Mutex::new(None),
            ended_changed: Condvar::new(),
        });
        let receiver = inner.clone();
        std::thread::Builder::new()
            .name("se-term-receive".into())
            .spawn(move || receiver.receive(receiving, sender))
            .map_err(|error| format!("cannot start the session client: {error}"))?;
        let client = Self {
            _leave: Arc::new(LeaveOnDrop(inner.clone())),
            inner,
            events,
        };
        client.resize(size)?;
        Ok(client)
    }

    /// Joins the terminal session of another process (or of this one).
    pub(in crate::cli) fn attach(
        entry: &Entry,
        label: &str,
        size: TerminalSize,
    ) -> Result<Self, String> {
        if entry.kind != SessionKind::Terminal {
            return Err(format!(
                "session {} ({}) is not a terminal session",
                entry.name, entry.id
            ));
        }
        Self::connect(entry.port, &entry.id, &entry.token, label, size)
    }

    /// Session facts as of joining.
    #[allow(dead_code)] // Read by the terminal front-end.
    pub(in crate::cli) fn info(&self) -> TerminalInfo {
        self.inner.info.clone()
    }

    /// What the session sends, in order: bytes, notices, and last the end.
    /// Every handle shares one queue, so one reader should take it.
    pub(in crate::cli) fn events(&self) -> Receiver<TerminalEvent> {
        self.events.clone()
    }

    /// Bytes the user typed, for the remote terminal.
    pub(in crate::cli) fn input(&self, bytes: &[u8]) -> Result<(), String> {
        for chunk in bytes.chunks(wire::MAX_DATA) {
            self.inner.send(wire::TERM_INPUT, chunk)?;
        }
        Ok(())
    }

    /// The size of the user's terminal changed (1 to 32767 columns and rows).
    /// The remote terminal takes the size of the client that was active last.
    pub(in crate::cli) fn resize(&self, size: TerminalSize) -> Result<(), String> {
        size.validate()?;
        let payload = serde_json::to_vec(&size).map_err(|error| error.to_string())?;
        self.inner.send(wire::TERM_RESIZE, &payload)
    }

    /// Ends the whole session for every participant. If a write is stuck, the
    /// connection is closed instead (which ends the session for an owner and
    /// detaches an attached client).
    #[allow(dead_code)] // Offered to the terminal front-end.
    pub(in crate::cli) fn cancel(&self) {
        self.inner.send_or_close(wire::CANCEL);
    }

    /// The owner ends the session; an attached client detaches.
    pub(in crate::cli) fn leave(&self) {
        self.inner.leave();
    }

    /// Exit status and message once the session ended or the connection to
    /// its owner was lost.
    #[allow(dead_code)] // Offered to the terminal front-end.
    pub(in crate::cli) fn ended(&self) -> Option<(Option<i32>, String)> {
        self.inner.ended.lock().ok().and_then(|ended| ended.clone())
    }

    #[allow(dead_code)] // Offered to the terminal front-end.
    pub(in crate::cli) fn wait_ended(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let Ok(mut ended) = self.inner.ended.lock() else {
            return false;
        };
        while ended.is_none() {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            ended = match self.inner.ended_changed.wait_timeout(ended, deadline - now) {
                Ok((next, _)) => next,
                Err(_) => return false,
            };
        }
        true
    }
}

impl Inner {
    fn send(&self, tag: u8, payload: &[u8]) -> Result<(), String> {
        let mut writer = self.writer.lock().map_err(|_| poisoned())?;
        wire::write_frame(&mut *writer, tag, payload)
            .map_err(|error| format!("the session connection failed: {error}"))
    }

    /// Sends an empty control frame without waiting for a busy writer.
    fn send_or_close(&self, tag: u8) {
        match self.writer.try_lock() {
            Ok(mut writer) => {
                let _ = wire::write_frame(&mut *writer, tag, &[]);
            }
            Err(_) => {
                let _ = self.closer.shutdown(Shutdown::Both);
            }
        }
    }

    fn leave(&self) {
        match self.writer.try_lock() {
            Ok(mut writer) => {
                let _ = wire::write_frame(&mut *writer, wire::LEAVE, &[]);
                let _ = writer.shutdown(Shutdown::Write);
            }
            Err(_) => {
                let _ = self.closer.shutdown(Shutdown::Both);
            }
        }
    }

    fn receive(self: Arc<Self>, mut stream: TcpStream, events: Sender<TerminalEvent>) {
        let end = loop {
            let Ok((tag, payload)) = wire::read_frame(&mut stream) else {
                break (
                    None,
                    "the connection to the session's owner was lost".to_string(),
                );
            };
            let event = match tag {
                wire::TERM_OUTPUT => TerminalEvent::Output(payload),
                wire::NOTICE => match wire::decode::<wire::Text>(&payload) {
                    Ok(notice) => TerminalEvent::Notice(printable(&notice.text)),
                    Err(_) => break invalid(),
                },
                wire::ENDED => match wire::decode::<wire::Ended>(&payload) {
                    Ok(ended) => break (ended.exit_code, printable(&ended.message)),
                    Err(_) => break invalid(),
                },
                _ => break invalid(),
            };
            if events.send(event).is_err() {
                let _ = stream.shutdown(Shutdown::Both);
                return;
            }
        };
        if let Ok(mut ended) = self.ended.lock() {
            *ended = Some(end.clone());
        }
        self.ended_changed.notify_all();
        let _ = events.send(TerminalEvent::Ended {
            exit_code: end.0,
            message: end.1,
        });
        let _ = stream.shutdown(Shutdown::Both);
    }
}

fn invalid() -> (Option<i32>, String) {
    (None, "the session sent an invalid frame".to_string())
}

fn poisoned() -> String {
    "the session client lock is poisoned".to_string()
}
