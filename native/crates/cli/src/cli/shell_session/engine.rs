//! One running session in its owner process: the transport's reader and
//! writer threads, the participants (the owner's front-end and attached
//! clients), and the deliveries of `EngineState` in state order. Lock order:
//! `state` before `fanout`; deliveries happen under `fanout` only, so a slow
//! participant delays output but never blocks a state change.
use super::engine_state::EngineState;
use super::marker;
use super::transports::{Transport, TransportEvent, TransportInput};
use super::types::{
    Activity, CommandRequest, Delivery, Dialect, OwnEvent, Role, SessionInfo, SessionKind, Stream,
};
use crossbeam_channel::{Sender, TrySendError};
use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Startup: the login profile plus the first marker; a Share peer's
/// connection setup alone may take up to 45 s before that.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
/// An idle session asked to close gets this long for `exit`.
const CLOSE_GRACE: Duration = Duration::from_secs(5);
/// Joined clients besides the owner; each gets a copy of all activity.
const MAX_ATTACHED: usize = 8;
/// Events waiting for one participant's connection.
pub(super) const OUTBOUND_CAPACITY: usize = 256;
/// How long an attached client may stall the output of its own command
/// before it is dropped. The owner's front-end lives in this process and is
/// never dropped: its stall is the session's flow control.
const ATTACHED_STALL: Duration = Duration::from_secs(30);
/// How long the end of the session waits for each participant's queue.
const END_NOTICE_WAIT: Duration = Duration::from_secs(5);

pub(super) enum Outbound {
    Own(OwnEvent),
    Rejected { tag: u64, message: String },
    Activity(Activity),
}

pub(super) struct Meta {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) target: String,
    pub(super) dialect: Dialect,
    pub(super) owner: String,
    pub(super) attachable: bool,
}

struct Participant {
    role: Role,
    label: String,
    sender: Sender<Outbound>,
    skipped: u64,
}

struct Fanout {
    participants: BTreeMap<u64, Participant>,
    next_id: u64,
}

enum WriteOp {
    Bytes(Vec<u8>),
    Eof,
}

type EndHook = Box<dyn FnOnce() + Send>;

pub(super) struct Engine {
    pub(super) meta: Meta,
    state: Mutex<EngineState>,
    changed: Condvar,
    fanout: Mutex<Fanout>,
    writer: Sender<WriteOp>,
    input: Arc<dyn TransportInput>,
    end_hooks: Mutex<Vec<EndHook>>,
}

impl Engine {
    /// Runs `startup` in the new shell and returns once its marker arrived.
    pub(super) fn start(
        transport: Box<dyn Transport>,
        meta: Meta,
        startup: &CommandRequest,
    ) -> Result<Arc<Self>, String> {
        let input = transport.input();
        let created =
            marker::nonce().and_then(|nonce| EngineState::new(meta.dialect, nonce, startup));
        let (state, first) = match created {
            Ok(created) => created,
            Err(error) => {
                input.cancel();
                return Err(error);
            }
        };
        let (writer, writes) = crossbeam_channel::unbounded::<WriteOp>();
        let writer_input = input.clone();
        std::thread::Builder::new()
            .name("se-shell-writer".into())
            .spawn(move || {
                for operation in writes {
                    let written = match operation {
                        WriteOp::Bytes(bytes) => writer_input.stdin(&bytes),
                        WriteOp::Eof => writer_input.eof(),
                    };
                    if written.is_err() {
                        return;
                    }
                }
            })
            .map_err(|error| {
                input.cancel();
                format!("cannot start the session writer: {error}")
            })?;
        let engine = Arc::new(Self {
            meta,
            state: Mutex::new(state),
            changed: Condvar::new(),
            fanout: Mutex::new(Fanout {
                participants: BTreeMap::new(),
                next_id: 1,
            }),
            writer,
            input,
            end_hooks: Mutex::new(Vec::new()),
        });
        for delivery in first {
            if let Delivery::Write(bytes) = delivery {
                let _ = engine.writer.send(WriteOp::Bytes(bytes));
            }
        }
        let reader = engine.clone();
        std::thread::Builder::new()
            .name(format!("se-shell-{}", engine.meta.id))
            .spawn(move || reader.read_loop(transport))
            .map_err(|error| {
                engine.input.cancel();
                format!("cannot start the session reader: {error}")
            })?;
        engine.wait_ready()?;
        Ok(engine)
    }

    pub(super) fn info(&self, role: Role) -> SessionInfo {
        let cwd = self.state.lock().ok().and_then(|state| state.cwd.clone());
        SessionInfo {
            kind: SessionKind::Commands,
            id: self.meta.id.clone(),
            name: self.meta.name.clone(),
            target: self.meta.target.clone(),
            dialect: self.meta.dialect,
            owner: self.meta.owner.clone(),
            attachable: self.meta.attachable,
            cwd,
            role,
        }
    }

    /// Adds a participant whose events go to `sender`.
    pub(super) fn join(
        &self,
        role: Role,
        label: &str,
        sender: Sender<Outbound>,
    ) -> Result<u64, String> {
        let state = self.state.lock().map_err(|_| poisoned())?;
        if let Some((_, message)) = &state.ended {
            return Err(format!("the session has ended: {message}"));
        }
        let mut fanout = self.fanout.lock().map_err(|_| poisoned())?;
        drop(state);
        match role {
            Role::Owner if fanout.participants.values().any(|p| p.role == Role::Owner) => {
                return Err("the session already has its owner".into());
            }
            Role::Attached
                if fanout
                    .participants
                    .values()
                    .filter(|p| p.role == Role::Attached)
                    .count()
                    >= MAX_ATTACHED =>
            {
                return Err(format!(
                    "{MAX_ATTACHED} clients are already attached to this session"
                ));
            }
            _ => {}
        }
        if role == Role::Attached {
            fanout.deliver(vec![Delivery::Activity {
                except: None,
                activity: Activity::Notice(format!("{label} attached")),
            }]);
        }
        let id = fanout.next_id;
        fanout.next_id += 1;
        fanout.participants.insert(
            id,
            Participant {
                role,
                label: label.to_string(),
                sender,
                skipped: 0,
            },
        );
        Ok(id)
    }

    /// A participant left: its waiting commands are dropped.
    pub(super) fn leave(&self, id: u64) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.forget(id);
        let Ok(mut fanout) = self.fanout.lock() else {
            return;
        };
        drop(state);
        if let Some(participant) = fanout.participants.remove(&id) {
            fanout.deliver(vec![Delivery::Activity {
                except: None,
                activity: Activity::Notice(format!("{} detached", participant.label)),
            }]);
        }
    }

    pub(super) fn submit(
        &self,
        id: u64,
        label: &str,
        tag: u64,
        request: CommandRequest,
    ) -> Result<(), String> {
        self.apply(|state| state.submit(id, label, tag, request))
    }

    pub(super) fn input(&self, id: u64, bytes: &[u8]) -> Result<(), String> {
        self.apply(|state| state.input(id, bytes))
    }

    /// Ends the whole session: the exec stream has no signal path to a
    /// single command.
    pub(super) fn cancel(&self, by: &str) {
        if let Ok(mut state) = self.state.lock() {
            state.cancelled(by);
        }
        self.input.cancel();
    }

    /// The owner closes: `exit` when idle (cancelled after `CLOSE_GRACE`),
    /// cancel while a command runs.
    pub(super) fn close(self: &Arc<Self>) {
        let exit = match self.state.lock() {
            Ok(mut state) if state.ended.is_none() => state.close_input(),
            _ => return,
        };
        let Some(exit) = exit else {
            self.cancel(&self.meta.owner);
            return;
        };
        let _ = self.writer.send(WriteOp::Bytes(exit));
        let _ = self.writer.send(WriteOp::Eof);
        let engine = self.clone();
        let _ = std::thread::Builder::new()
            .name("se-shell-close".into())
            .spawn(move || {
                let deadline = Instant::now() + CLOSE_GRACE;
                let Ok(mut state) = engine.state.lock() else {
                    return;
                };
                while state.ended.is_none() {
                    let now = Instant::now();
                    if now >= deadline {
                        drop(state);
                        engine.cancel(&engine.meta.owner);
                        return;
                    }
                    state = match engine.changed.wait_timeout(state, deadline - now) {
                        Ok((next, _)) => next,
                        Err(_) => return,
                    };
                }
            });
    }

    /// Runs `hook` once the session ended (at once if it already has).
    pub(super) fn on_end(&self, hook: EndHook) {
        let Ok(mut hooks) = self.end_hooks.lock() else {
            hook();
            return;
        };
        let ended = self
            .state
            .lock()
            .map(|state| state.ended.is_some())
            .unwrap_or(true);
        if ended {
            drop(hooks);
            hook();
        } else {
            hooks.push(hook);
        }
    }

    fn wait_ready(&self) -> Result<(), String> {
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        loop {
            if let Some(failure) = state.startup_failure() {
                drop(state);
                self.input.cancel();
                return Err(failure);
            }
            if state.ready() {
                return Ok(());
            }
            let now = Instant::now();
            if now >= deadline {
                drop(state);
                self.input.cancel();
                return Err(format!(
                    "the remote shell did not become ready within {} seconds (does a login profile wait for input?)",
                    STARTUP_TIMEOUT.as_secs()
                ));
            }
            state = self
                .changed
                .wait_timeout(state, deadline - now)
                .map_err(|_| poisoned())?
                .0;
        }
    }

    fn read_loop(self: Arc<Self>, mut transport: Box<dyn Transport>) {
        loop {
            match transport.next_event() {
                TransportEvent::Stdout(bytes) => {
                    let _ = self.apply(|state| Ok(state.output(Stream::Stdout, &bytes)));
                }
                TransportEvent::Stderr(bytes) => {
                    let _ = self.apply(|state| Ok(state.output(Stream::Stderr, &bytes)));
                }
                TransportEvent::Ended(end) => {
                    let _ = self.apply(move |state| Ok(state.ended(end)));
                    break;
                }
            }
        }
        let hooks = self
            .end_hooks
            .lock()
            .map(|mut hooks| std::mem::take(&mut *hooks))
            .unwrap_or_default();
        for hook in hooks {
            hook();
        }
        // Dropping the senders ends every connection once its queue is written.
        if let Ok(mut fanout) = self.fanout.lock() {
            fanout.participants.clear();
        }
    }

    /// Applies one state change and delivers its results in order.
    fn apply(
        &self,
        change: impl FnOnce(&mut EngineState) -> Result<Vec<Delivery>, String>,
    ) -> Result<(), String> {
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        let deliveries = change(&mut state)?;
        let mut rest = Vec::with_capacity(deliveries.len());
        for delivery in deliveries {
            match delivery {
                Delivery::Write(bytes) => {
                    let _ = self.writer.send(WriteOp::Bytes(bytes));
                }
                other => rest.push(other),
            }
        }
        self.changed.notify_all();
        let mut fanout = self.fanout.lock().map_err(|_| poisoned())?;
        drop(state);
        fanout.deliver(rest);
        Ok(())
    }
}

impl Fanout {
    fn deliver(&mut self, deliveries: Vec<Delivery>) {
        for delivery in deliveries {
            match delivery {
                Delivery::Write(_) => {}
                Delivery::Own { to, event } => {
                    let Some(participant) = self.participants.get(&to) else {
                        continue;
                    };
                    let sent = match participant.role {
                        Role::Owner => participant.sender.send(Outbound::Own(event)).is_ok(),
                        Role::Attached => participant
                            .sender
                            .send_timeout(Outbound::Own(event), ATTACHED_STALL)
                            .is_ok(),
                    };
                    if !sent {
                        self.participants.remove(&to);
                    }
                }
                Delivery::Activity { except, activity } => self.broadcast(except, &activity),
            }
        }
    }

    fn broadcast(&mut self, except: Option<u64>, activity: &Activity) {
        let size = match activity {
            Activity::Output { bytes, .. } => bytes.len() as u64,
            Activity::Started { command, .. } => command.len() as u64,
            _ => 1,
        };
        let mut gone = Vec::new();
        for (id, participant) in self.participants.iter_mut() {
            if Some(*id) == except {
                continue;
            }
            if let Activity::Ended { .. } = activity {
                // Everyone must learn that the session is over.
                if participant
                    .sender
                    .send_timeout(Outbound::Activity(activity.clone()), END_NOTICE_WAIT)
                    .is_err()
                {
                    gone.push(*id);
                }
                continue;
            }
            if participant.skipped > 0 {
                let notice = Activity::Notice(format!(
                    "{} bytes of activity were skipped because this client read too slowly",
                    participant.skipped
                ));
                match participant.sender.try_send(Outbound::Activity(notice)) {
                    Ok(()) => participant.skipped = 0,
                    Err(TrySendError::Full(_)) => {
                        participant.skipped += size;
                        continue;
                    }
                    Err(TrySendError::Disconnected(_)) => {
                        gone.push(*id);
                        continue;
                    }
                }
            }
            match participant
                .sender
                .try_send(Outbound::Activity(activity.clone()))
            {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => participant.skipped += size,
                Err(TrySendError::Disconnected(_)) => gone.push(*id),
            }
        }
        for id in gone {
            self.participants.remove(&id);
        }
    }
}

fn poisoned() -> String {
    "the session state lock is poisoned".to_string()
}
