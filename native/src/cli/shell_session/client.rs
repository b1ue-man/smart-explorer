//! Client side of a session, for the owner's own front-end as for every
//! attached `se shell` or `se mcp`: it submits commands and input and
//! dispatches what the host sends, the output and end of its own commands
//! to their sinks and the activity of the other participants to one
//! subscriber. Dropping the last handle leaves the session (the owner closes
//! it, an attached client detaches).
use super::registry::Entry;
use super::types::{
    Activity, CommandOutcome, CommandRequest, CommandSink, OutcomeStatus, SessionInfo, Stream,
};
use super::wire;
use crossbeam_channel::{Receiver, Sender};
use serde::Serialize;
use std::collections::HashMap;
use std::io;
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const WELCOME_TIMEOUT: Duration = Duration::from_secs(10);
/// Activity waiting for the subscriber, which only displays or logs it;
/// more is dropped.
const ACTIVITY_CAPACITY: usize = 1024;

#[derive(Clone)]
pub(in crate::cli) struct SessionClient {
    inner: Arc<Inner>,
    _leave: Arc<LeaveOnDrop>,
}

struct Inner {
    writer: Mutex<TcpStream>,
    sinks: Mutex<Sinks>,
    next_tag: AtomicU64,
    info: Mutex<SessionInfo>,
    ended: Mutex<Option<(Option<i32>, String)>>,
    ended_changed: Condvar,
    activity: Mutex<Option<Sender<Activity>>>,
}

#[derive(Default)]
struct Sinks {
    open: HashMap<u64, Arc<dyn CommandSink>>,
    /// Set once the session is over; no sink is accepted afterwards.
    closed: Option<String>,
}

struct LeaveOnDrop(Arc<Inner>);

impl Drop for LeaveOnDrop {
    fn drop(&mut self) {
        if let Ok(mut writer) = self.0.writer.lock() {
            let _ = wire::write_frame(&mut *writer, wire::LEAVE, &[]);
            let _ = writer.shutdown(Shutdown::Write);
        }
    }
}

impl SessionClient {
    pub(super) fn connect(
        port: u16,
        session: &str,
        token: &str,
        label: &str,
    ) -> Result<Self, String> {
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)
            .map_err(|error| format!("cannot reach the session: {error}"))?;
        let _ = stream.set_nodelay(true);
        let mut writer = &stream;
        wire::write_json(
            &mut writer,
            wire::HELLO,
            &wire::Hello {
                version: wire::PROTOCOL_VERSION,
                session: session.to_string(),
                token: token.to_string(),
                label: label.to_string(),
            },
        )
        .map_err(|error| format!("cannot reach the session: {error}"))?;
        stream
            .set_read_timeout(Some(WELCOME_TIMEOUT))
            .map_err(|error| error.to_string())?;
        let mut reader = &stream;
        let (tag, payload) = wire::read_frame(&mut reader)
            .map_err(|error| format!("the session did not answer: {error}"))?;
        let info: SessionInfo = match tag {
            wire::WELCOME => wire::decode(&payload).map_err(|error| error.to_string())?,
            wire::DENIED => {
                return Err(wire::decode::<wire::Text>(&payload)
                    .map(|text| text.text)
                    .unwrap_or_else(|_| "permission denied".to_string()))
            }
            _ => return Err("the session answered with an unexpected frame".into()),
        };
        stream
            .set_read_timeout(None)
            .map_err(|error| error.to_string())?;
        let receiving = stream.try_clone().map_err(|error| error.to_string())?;
        let inner = Arc::new(Inner {
            writer: Mutex::new(stream),
            sinks: Mutex::new(Sinks::default()),
            next_tag: AtomicU64::new(1),
            info: Mutex::new(info),
            ended: Mutex::new(None),
            ended_changed: Condvar::new(),
            activity: Mutex::new(None),
        });
        let receiver = inner.clone();
        std::thread::Builder::new()
            .name("se-shell-receive".into())
            .spawn(move || receiver.receive(receiving))
            .map_err(|error| format!("cannot start the session client: {error}"))?;
        Ok(Self {
            _leave: Arc::new(LeaveOnDrop(inner.clone())),
            inner,
        })
    }

    /// Joins an attachable session of another process (or of this one).
    pub(in crate::cli) fn attach(entry: &Entry, label: &str) -> Result<Self, String> {
        Self::connect(entry.port, &entry.id, &entry.token, label)
    }

    /// Session facts; `cwd` follows every finished command.
    pub(in crate::cli) fn info(&self) -> SessionInfo {
        match self.inner.info.lock() {
            Ok(info) => info.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Queues `request`; `sink` receives its output and exactly one outcome.
    pub(in crate::cli) fn submit(
        &self,
        request: CommandRequest,
        sink: Arc<dyn CommandSink>,
    ) -> Result<(), String> {
        let tag = self.inner.next_tag.fetch_add(1, Ordering::SeqCst);
        {
            let mut sinks = self.inner.sinks.lock().map_err(|_| poisoned())?;
            if let Some(message) = &sinks.closed {
                return Err(format!("the session has ended: {message}"));
            }
            sinks.open.insert(tag, sink);
        }
        let sent = self.send_json(wire::SUBMIT, &wire::Submit { tag, request });
        if sent.is_err() {
            if let Ok(mut sinks) = self.inner.sinks.lock() {
                sinks.open.remove(&tag);
            }
        }
        sent
    }

    /// Input for this client's own running interactive command.
    pub(in crate::cli) fn input(&self, bytes: &[u8]) -> Result<(), String> {
        for chunk in bytes.chunks(wire::MAX_DATA) {
            self.send(wire::INPUT, chunk)?;
        }
        Ok(())
    }

    /// Ends the whole session for every participant.
    pub(in crate::cli) fn cancel(&self) {
        let _ = self.send(wire::CANCEL, &[]);
    }

    /// The owner closes the session; an attached client detaches.
    pub(in crate::cli) fn leave(&self) {
        let _ = self.send(wire::LEAVE, &[]);
    }

    /// Exit status and message once the session ended or the connection to
    /// its owner was lost.
    pub(in crate::cli) fn ended(&self) -> Option<(Option<i32>, String)> {
        self.inner.ended.lock().ok().and_then(|ended| ended.clone())
    }

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

    /// Activity of the other participants; replaces an earlier subscriber.
    pub(in crate::cli) fn activity(&self) -> Receiver<Activity> {
        let (sender, receiver) = crossbeam_channel::bounded(ACTIVITY_CAPACITY);
        if let Ok(mut slot) = self.inner.activity.lock() {
            *slot = Some(sender);
        }
        receiver
    }

    fn send(&self, tag: u8, payload: &[u8]) -> Result<(), String> {
        let mut writer = self.inner.writer.lock().map_err(|_| poisoned())?;
        wire::write_frame(&mut *writer, tag, payload)
            .map_err(|error| format!("the session connection failed: {error}"))
    }

    fn send_json(&self, tag: u8, value: &impl Serialize) -> Result<(), String> {
        let payload = serde_json::to_vec(value).map_err(|error| error.to_string())?;
        self.send(tag, &payload)
    }
}

impl Inner {
    fn receive(self: Arc<Self>, mut stream: TcpStream) {
        let end = loop {
            let (tag, payload) = match wire::read_frame(&mut stream) {
                Ok(frame) => frame,
                Err(_) => {
                    break (
                        None,
                        "the connection to the session's owner was lost".to_string(),
                    )
                }
            };
            match self.dispatch(tag, &payload) {
                Ok(None) => {}
                Ok(Some(end)) => break end,
                Err(_) => break (None, "the session sent an invalid frame".to_string()),
            }
        };
        self.finish(end);
        let _ = stream.shutdown(Shutdown::Both);
    }

    fn dispatch(&self, tag: u8, payload: &[u8]) -> io::Result<Option<(Option<i32>, String)>> {
        match tag {
            wire::OWN_STDOUT | wire::OWN_STDERR => {
                let (request, bytes) = wire::untag(payload)?;
                let stream = if tag == wire::OWN_STDOUT {
                    Stream::Stdout
                } else {
                    Stream::Stderr
                };
                if let Some(sink) = self.sink(request, false) {
                    sink.output(stream, bytes);
                }
            }
            wire::OWN_DONE => {
                let done: wire::OwnDone = wire::decode(payload)?;
                if let Some(cwd) = &done.outcome.cwd {
                    self.set_cwd(cwd);
                }
                if let Some(sink) = self.sink(done.tag, true) {
                    sink.finished(done.outcome);
                }
            }
            wire::REJECTED => {
                let rejected: wire::Rejected = wire::decode(payload)?;
                if let Some(sink) = self.sink(rejected.tag, true) {
                    sink.finished(CommandOutcome {
                        status: OutcomeStatus::Failed,
                        exit_code: None,
                        cwd: None,
                        session_ended: false,
                        message: Some(rejected.message),
                    });
                }
            }
            wire::ACT_STARTED => {
                let started: wire::ActStarted = wire::decode(payload)?;
                self.notify(Activity::Started {
                    by: started.by,
                    command: started.command,
                });
            }
            wire::ACT_STDOUT | wire::ACT_STDERR => self.notify(Activity::Output {
                stream: if tag == wire::ACT_STDOUT {
                    Stream::Stdout
                } else {
                    Stream::Stderr
                },
                bytes: payload.to_vec(),
            }),
            wire::ACT_DONE => {
                let done: wire::ActDone = wire::decode(payload)?;
                if let Some(cwd) = &done.cwd {
                    self.set_cwd(cwd);
                }
                self.notify(Activity::Done {
                    by: done.by,
                    exit_code: done.exit_code,
                    cwd: done.cwd,
                });
            }
            wire::NOTICE => {
                let notice: wire::Text = wire::decode(payload)?;
                self.notify(Activity::Notice(notice.text));
            }
            wire::ENDED => {
                let ended: wire::Ended = wire::decode(payload)?;
                return Ok(Some((ended.exit_code, ended.message)));
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unexpected frame from the session",
                ))
            }
        }
        Ok(None)
    }

    fn sink(&self, tag: u64, remove: bool) -> Option<Arc<dyn CommandSink>> {
        let mut sinks = self.sinks.lock().ok()?;
        if remove {
            sinks.open.remove(&tag)
        } else {
            sinks.open.get(&tag).cloned()
        }
    }

    fn set_cwd(&self, cwd: &str) {
        if let Ok(mut info) = self.info.lock() {
            info.cwd = Some(cwd.to_string());
        }
    }

    fn notify(&self, activity: Activity) {
        if let Ok(slot) = self.activity.lock() {
            if let Some(sender) = slot.as_ref() {
                let _ = sender.try_send(activity);
            }
        }
    }

    /// The session is over: unfinished sinks fail, waiters wake up.
    fn finish(&self, (exit_code, message): (Option<i32>, String)) {
        let pending = match self.sinks.lock() {
            Ok(mut sinks) => {
                sinks.closed = Some(message.clone());
                sinks.open.drain().map(|(_, sink)| sink).collect::<Vec<_>>()
            }
            Err(_) => Vec::new(),
        };
        for sink in pending {
            sink.finished(CommandOutcome {
                status: OutcomeStatus::Failed,
                exit_code: None,
                cwd: None,
                session_ended: true,
                message: Some(message.clone()),
            });
        }
        if let Ok(mut ended) = self.ended.lock() {
            *ended = Some((exit_code, message.clone()));
        }
        self.ended_changed.notify_all();
        self.notify(Activity::Ended { exit_code, message });
    }
}

fn poisoned() -> String {
    "the session client lock is poisoned".to_string()
}
