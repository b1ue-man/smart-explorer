//! One running terminal session in its owner process: the login shell of the
//! target in a pseudo-terminal, reached through a `Transport`. Its bytes pass
//! unchanged: a reader thread hands the output to every participant in order,
//! a writer thread hands the participants' input to the transport in order.
//!
//! The owner's front-end is never dropped (its full queue stops the reader and
//! so the transport: the flow control); up to eight attached terminals are
//! dropped after `ATTACHED_STALL` of not reading. A participant's input goes
//! to the remote terminal only, never to the others (a password stays with the
//! remote echo). The remote terminal has the size of the participant that was
//! active last (input, size report or join); only a change is passed on. The
//! state lock is never held while waiting for a queue or for the transport.
use super::transports::{Transport, TransportEnd, TransportEvent, TransportInput};
use super::types::{Dialect, Role, SessionKind, TerminalEvent, TerminalInfo, TerminalSize};
use crossbeam_channel::{Receiver, SendTimeoutError, Sender};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Attached terminals besides the owner.
const MAX_ATTACHED: usize = 8;
/// How long an attached terminal may leave its queue full before it is dropped.
const ATTACHED_STALL: Duration = Duration::from_secs(30);
/// How long the end of the session waits for each participant's queue.
const END_NOTICE_WAIT: Duration = Duration::from_secs(5);
/// Input operations waiting for the transport; a full queue stops the client
/// that wrote last.
const WRITE_QUEUE: usize = 256;

pub(super) struct TerminalMeta {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) target: String,
    pub(super) owner: String,
    pub(super) attachable: bool,
    pub(super) dialect: Option<Dialect>,
}

pub(super) struct Participant {
    role: Role,
    label: String,
    sender: Sender<TerminalEvent>,
    size: Option<TerminalSize>,
    last_active: u64,
}

/// What the lock guards; plain data, no I/O.
pub(super) struct State {
    pub(super) participants: BTreeMap<u64, Participant>,
    next_id: u64,
    tick: u64,
    pub(super) applied: TerminalSize,
    reason: Option<String>,
    ended: Option<(Option<i32>, String)>,
}

impl State {
    pub(super) fn new(size: TerminalSize) -> Self {
        Self {
            participants: BTreeMap::new(),
            next_id: 1,
            tick: 0,
            applied: size,
            reason: None,
            ended: None,
        }
    }

    /// Adds a participant that is active from now on.
    pub(super) fn add(&mut self, role: Role, label: String, sender: Sender<TerminalEvent>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.tick += 1;
        let last_active = self.tick;
        let size = None;
        self.participants.insert(
            id,
            Participant {
                role,
                label,
                sender,
                size,
                last_active,
            },
        );
        id
    }

    /// The participant did something (and maybe reported its size); false when
    /// it is not part of the session.
    pub(super) fn activity(&mut self, id: u64, size: Option<TerminalSize>) -> bool {
        self.tick += 1;
        let stamp = self.tick;
        let Some(participant) = self.participants.get_mut(&id) else {
            return false;
        };
        participant.last_active = stamp;
        participant.size = size.or(participant.size);
        true
    }

    /// The size of the participant that was active last, when it is known and
    /// differs from the remote terminal's; it counts as applied from now on.
    pub(super) fn follow_active(&mut self) -> Option<TerminalSize> {
        let wanted = self
            .participants
            .values()
            .max_by_key(|participant| participant.last_active)
            .and_then(|participant| participant.size)
            .filter(|size| *size != self.applied)?;
        self.applied = wanted;
        Some(wanted)
    }

    /// A line for every participant; dropped where a queue is full.
    fn notify(&self, text: &str) {
        for participant in self.participants.values() {
            let _ = participant
                .sender
                .try_send(TerminalEvent::Notice(text.to_string()));
        }
    }
}

type EndHook = Box<dyn FnOnce() + Send>;

pub(super) struct TerminalEngine {
    pub(super) meta: TerminalMeta,
    state: Arc<Mutex<State>>,
    writer: Sender<Vec<u8>>,
    input: Arc<dyn TransportInput>,
    /// Serializes deciding and applying the remote terminal's size.
    sizing: Mutex<()>,
    /// Read from `begin` on.
    transport: Mutex<Option<Box<dyn Transport>>>,
    end_hooks: Mutex<Vec<EndHook>>,
}

impl TerminalEngine {
    /// Prepares the session on `transport`, whose terminal already has `size`.
    /// Nothing is read before `begin`, so that nothing the shell prints first
    /// (banner, prompt) is lost before the owner joined.
    pub(super) fn start(
        transport: Box<dyn Transport>,
        meta: TerminalMeta,
        size: TerminalSize,
    ) -> Result<Arc<Self>, String> {
        let input = transport.input();
        let state = Arc::new(Mutex::new(State::new(size)));
        let (writer, writes) = crossbeam_channel::bounded::<Vec<u8>>(WRITE_QUEUE);
        let writing = input.clone();
        std::thread::Builder::new()
            .name("se-term-writer".into())
            .spawn(move || write_loop(writes, writing))
            .map_err(|error| {
                input.cancel();
                format!("cannot start the session writer: {error}")
            })?;
        Ok(Arc::new(Self {
            meta,
            state,
            writer,
            input,
            sizing: Mutex::new(()),
            transport: Mutex::new(Some(transport)),
            end_hooks: Mutex::new(Vec::new()),
        }))
    }

    /// Starts reading the transport; once, after the owner joined.
    pub(super) fn begin(self: &Arc<Self>) -> Result<(), String> {
        let taken = self.transport.lock().map_err(|_| poisoned())?.take();
        let transport = taken.ok_or_else(|| "the session already began".to_string())?;
        let reader = self.clone();
        std::thread::Builder::new()
            .name(format!("se-term-{}", self.meta.id))
            .spawn(move || reader.read_loop(transport))
            .map(|_| ())
            .map_err(|error| {
                self.input.cancel();
                format!("cannot start the session reader: {error}")
            })
    }

    pub(super) fn info(&self, role: Role) -> TerminalInfo {
        let size = match self.state.lock() {
            Ok(state) => state.applied,
            Err(poisoned) => poisoned.into_inner().applied,
        };
        TerminalInfo {
            kind: SessionKind::Terminal,
            id: self.meta.id.clone(),
            name: self.meta.name.clone(),
            target: self.meta.target.clone(),
            owner: self.meta.owner.clone(),
            attachable: self.meta.attachable,
            size,
            role,
        }
    }

    /// Adds a participant whose events go to `sender`.
    pub(super) fn join(
        &self,
        role: Role,
        label: &str,
        sender: Sender<TerminalEvent>,
    ) -> Result<u64, String> {
        let label = printable(label);
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        if let Some((_, message)) = &state.ended {
            return Err(format!("the session has ended: {message}"));
        }
        let count = |wanted: Role| {
            state
                .participants
                .values()
                .filter(|p| p.role == wanted)
                .count()
        };
        if role == Role::Owner && count(Role::Owner) > 0 {
            return Err("the session already has its owner".into());
        }
        if role == Role::Attached && count(Role::Attached) >= MAX_ATTACHED {
            return Err(format!(
                "{MAX_ATTACHED} clients are already attached to this session"
            ));
        }
        if role == Role::Attached {
            state.notify(&format!("{label} attached"));
        }
        Ok(state.add(role, label, sender))
    }

    /// An attached participant left (the owner's leaving is `close`).
    pub(super) fn leave(&self, id: u64) {
        let _ = self.with_size(|state| {
            if let Some(participant) = state.participants.remove(&id) {
                state.notify(&format!("{} detached", participant.label));
            }
        });
    }

    /// Input of participant `id` for the remote terminal.
    pub(super) fn input(&self, id: u64, bytes: Vec<u8>) -> Result<(), String> {
        self.active(id, None)?;
        if bytes.is_empty() {
            return Ok(());
        }
        // The terminal has the participant's size before these bytes arrive.
        self.writer
            .send(bytes)
            .map_err(|_| "the session has ended".to_string())
    }

    /// Participant `id` reports the size of its terminal.
    pub(super) fn resize(&self, id: u64, size: TerminalSize) -> Result<(), String> {
        self.active(id, Some(size))
    }

    /// A participant ends the whole session.
    pub(super) fn cancel(self: &Arc<Self>, by: &str) {
        self.terminate(&format!("the session was cancelled by {by}"));
    }

    /// The owner leaves: the session ends for everyone. An idle shell cannot
    /// be told from a program reading the terminal, so there is no `exit`.
    pub(super) fn close(self: &Arc<Self>) {
        self.terminate("the session was closed by its owner");
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

    /// Records that `id` was active, maybe with its size.
    fn active(&self, id: u64, size: Option<TerminalSize>) -> Result<(), String> {
        match self.with_size(|state| state.activity(id, size))? {
            true => Ok(()),
            false => Err("this client is no longer part of the session".into()),
        }
    }

    /// Changes the state, then gives the remote terminal the size of the
    /// participant that was active last. The lock keeps decisions and
    /// applications in one order.
    fn with_size<T>(&self, change: impl FnOnce(&mut State) -> T) -> Result<T, String> {
        let _ordered = self.sizing.lock().map_err(|_| poisoned())?;
        let (value, size) = {
            let mut state = self.state.lock().map_err(|_| poisoned())?;
            let value = change(&mut state);
            (value, state.follow_active())
        };
        if let Some(size) = size {
            let _ = self.input.resize(size);
        }
        Ok(value)
    }

    /// Ends the remote process tree and, without waiting for the transport
    /// (SSH needs up to 20 s for TERM and KILL), the session for everyone.
    fn terminate(self: &Arc<Self>, reason: &str) {
        let message = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            if state.ended.is_some() {
                return;
            }
            let first = state.reason.get_or_insert_with(|| printable(reason));
            first.clone()
        };
        self.input.cancel();
        let engine = self.clone();
        let announced = message.clone();
        let spawned = std::thread::Builder::new()
            .name("se-term-end".into())
            .spawn(move || engine.end_session(None, announced));
        if spawned.is_err() {
            self.end_session(None, message);
        }
    }

    fn read_loop(self: Arc<Self>, mut transport: Box<dyn Transport>) {
        loop {
            match transport.next_event() {
                TransportEvent::Stdout(bytes) | TransportEvent::Stderr(bytes) => self.output(bytes),
                TransportEvent::Ended(end) => {
                    let reason = self.state.lock().ok().and_then(|s| s.reason.clone());
                    let message = printable(&end_message(reason.as_deref(), &end));
                    self.end_session(end.exit_code, message);
                    break;
                }
            }
        }
    }

    /// Hands output to every participant in order and without loss.
    fn output(&self, mut bytes: Vec<u8>) {
        let targets: Vec<(u64, Role, Sender<TerminalEvent>)> = {
            let Ok(state) = self.state.lock() else {
                return;
            };
            let mut targets = Vec::new();
            for (id, participant) in &state.participants {
                targets.push((*id, participant.role, participant.sender.clone()));
            }
            targets
        };
        let count = targets.len();
        let mut failed = Vec::new();
        for (index, (id, role, sender)) in targets.into_iter().enumerate() {
            let chunk = if index + 1 == count {
                std::mem::take(&mut bytes)
            } else {
                bytes.clone()
            };
            let sent = match role {
                Role::Owner => sender.send(TerminalEvent::Output(chunk)).map_err(|_| false),
                Role::Attached => sender
                    .send_timeout(TerminalEvent::Output(chunk), ATTACHED_STALL)
                    .map_err(|error| matches!(error, SendTimeoutError::Timeout(_))),
            };
            if let Err(timed_out) = sent {
                failed.push((id, timed_out));
            }
        }
        if !failed.is_empty() {
            self.drop_participants(&failed);
        }
    }

    /// Removes participants whose queue is gone or stayed full.
    fn drop_participants(&self, failed: &[(u64, bool)]) {
        let _ = self.with_size(|state| {
            for (id, timed_out) in failed {
                let Some(participant) = state.participants.remove(id) else {
                    continue;
                };
                if participant.role == Role::Attached {
                    let why = if *timed_out {
                        "was disconnected because it read too slowly"
                    } else {
                        "detached"
                    };
                    state.notify(&format!("{} {why}", participant.label));
                }
            }
        });
    }

    /// Marks the session ended and tells everyone; only the first call acts.
    fn end_session(&self, exit_code: Option<i32>, message: String) {
        let participants = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            if state.ended.is_some() {
                return;
            }
            state.ended = Some((exit_code, message.clone()));
            std::mem::take(&mut state.participants)
        };
        let hooks = self
            .end_hooks
            .lock()
            .map(|mut hooks| std::mem::take(&mut *hooks))
            .unwrap_or_default();
        for hook in hooks {
            hook();
        }
        // Dropping each sender closes its queue once the end is written.
        for (_, participant) in participants {
            let ended = TerminalEvent::Ended {
                exit_code,
                message: message.clone(),
            };
            let _ = participant.sender.send_timeout(ended, END_NOTICE_WAIT);
        }
    }
}

/// Passes the participants' input to the transport in order; a transport that
/// cannot be written to ends the session.
fn write_loop(writes: Receiver<Vec<u8>>, input: Arc<dyn TransportInput>) {
    for bytes in writes {
        if input.stdin(&bytes).is_err() {
            input.cancel();
            return;
        }
    }
}

fn end_message(reason: Option<&str>, end: &TransportEnd) -> String {
    if let Some(reason) = reason {
        reason.to_string()
    } else if end.cancelled {
        "the session was cancelled".into()
    } else if let Some(error) = &end.error {
        error.clone()
    } else if let Some(code) = end.exit_code {
        format!("the shell exited with status {code}")
    } else {
        "the shell ended".into()
    }
}

/// Text for a participant's terminal: control characters become spaces, so
/// nothing the remote or a label contains steers the terminal.
pub(super) fn printable(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect();
    cleaned.trim().to_string()
}

fn poisoned() -> String {
    "the session state lock is poisoned".to_string()
}
