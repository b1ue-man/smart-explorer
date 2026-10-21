//! The running viewer sessions (SCR-M9, U28-2): the table, the session
//! thread that reads the relay's events, and their translation into bridge
//! events and queued pictures. A session ends once: the `ended` event, the
//! closed queue (`screenPull` answers −2) and the removal from the table.
use std::collections::BTreeMap;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::{json, Value};

use super::json_shapes::{accepted_fields, camel, event_value, split_refusal, stats_fields};
use crate::daemon::{StreamEvent, StreamRequest};
use crate::mobile::{ApiError, Runtime};
use crate::screen::core::gop_stream::now_us;
use crate::screen::core::input_model::PressedInputs;
use crate::screen::core::negotiate::H264Profile;
use crate::screen::core::session_state::KeyframeRequester;
use crate::screen::core::wire_control::{HostFrame, ViewerFrame};
use crate::screen::core::wire_hello::SessionMode;
use crate::screen::os::viewer_sink::{PullQueue, SinkOutput, ViewerSink, PULL_ENDED};
use crate::share::access_grant::{Capability, CapabilityDecision};
use crate::share::PeerOpenTarget;

pub(super) type SendFrame = Arc<dyn Fn(&ViewerFrame) -> io::Result<()> + Send + Sync>;
type CloseFn = Arc<dyn Fn() -> io::Result<()> + Send + Sync>;

static SESSIONS: Mutex<BTreeMap<u64, Arc<ViewSession>>> = Mutex::new(BTreeMap::new());
static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Link {
    send: SendFrame,
    close: CloseFn,
}

pub(super) struct ViewSession {
    pub id: u64,
    pub mode: SessionMode,
    queue: Arc<PullQueue>,
    link: Mutex<Option<Link>>,
    closed: AtomicBool,
    ended: AtomicBool,
    /// The host lets this session take control (`control` is not `off`).
    pub control_allowed: AtomicBool,
    pub keyframes: Arc<Mutex<KeyframeRequester>>,
    pub pressed: Mutex<PressedInputs>,
}

impl ViewSession {
    pub(super) fn send(&self, frame: &ViewerFrame) -> Result<(), ApiError> {
        let send = lock(&self.link)
            .as_ref()
            .map(|link| link.send.clone())
            .ok_or_else(|| ApiError::new("busy", "Die Sitzung ist noch nicht verbunden."))?;
        send(frame).map_err(|error| ApiError::new("network", error.to_string()))
    }

    /// Releases what is still pressed (the host does too, at the end).
    pub(super) fn release_pressed(&self) {
        let events = lock(&self.pressed).release_all();
        if !events.is_empty() {
            let _ = self.send(&ViewerFrame::Input(events));
        }
    }

    /// The app closes the session.
    pub(super) fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        lock(&SESSIONS).remove(&self.id);
        self.shut();
    }

    fn shut(&self) {
        self.release_pressed();
        self.queue.close();
        let link = lock(&self.link).take();
        if let Some(link) = link {
            let _ = (link.close)();
        }
    }
}

fn emit(session: u64, kind: &str, fields: Value) {
    if let Some(rt) = Runtime::installed() {
        rt.emit(event_value(session, kind, fields));
    }
}

/// The session `id`, if it runs.
pub(super) fn find(id: u64) -> Option<Arc<ViewSession>> {
    lock(&SESSIONS).get(&id).cloned()
}

/// `screenPull`: the next picture of `session`; −2 for an unknown or ended one.
pub(super) fn pull(session: u64, dst: &mut [u8], timeout: Duration) -> i32 {
    find(session).map_or(PULL_ENDED, |found| found.queue.pull_into(dst, timeout))
}

fn new_session(mode: SessionMode) -> Arc<ViewSession> {
    let id = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
    let session = Arc::new(ViewSession {
        id,
        mode,
        queue: Arc::new(PullQueue::default()),
        link: Mutex::new(None),
        closed: AtomicBool::new(false),
        ended: AtomicBool::new(false),
        control_allowed: AtomicBool::new(false),
        keyframes: Arc::new(Mutex::new(KeyframeRequester::default())),
        pressed: Mutex::new(PressedInputs::default()),
    });
    lock(&SESSIONS).insert(id, session.clone());
    session
}

/// Registers a session and starts its thread.
pub(super) fn start(
    mode: SessionMode,
    peer: PeerOpenTarget,
    request: StreamRequest,
    own: String,
) -> Result<u64, ApiError> {
    let session = new_session(mode);
    let id = session.id;
    std::thread::Builder::new()
        .name(format!("screen-view-{id}"))
        .spawn(move || run(&session, peer, request, &own))
        .map_err(|error| {
            lock(&SESSIONS).remove(&id);
            ApiError::internal(format!("Die Sitzung konnte nicht starten: {error}"))
        })?;
    Ok(id)
}

/// A registered session without a relay; frames go to `send` (tests).
#[cfg(test)]
pub(super) fn open_for_tests(mode: SessionMode, send: Option<SendFrame>) -> Arc<ViewSession> {
    let session = new_session(mode);
    if let Some(send) = send {
        *lock(&session.link) = Some(Link {
            send,
            close: Arc::new(|| Ok(())),
        });
    }
    session
}

/// Ends the session once: the `ended` event, the queue, the table entry.
fn finish(session: &ViewSession, code: &str, detail: Option<String>) {
    if !session.ended.swap(true, Ordering::SeqCst) {
        emit(
            session.id,
            "ended",
            json!({ "code": code, "detail": detail }),
        );
    }
    session.queue.close();
    lock(&SESSIONS).remove(&session.id);
}

fn run(session: &Arc<ViewSession>, peer: PeerOpenTarget, request: StreamRequest, own: &str) {
    let mut stream = match crate::daemon::screen_stream(peer, request) {
        Ok(stream) => stream,
        Err(message) => {
            let (code, detail) = split_refusal(&message);
            finish(session, &code, Some(detail));
            return;
        }
    };
    let control = stream.control();
    let (sender, closer) = (control.clone(), control);
    *lock(&session.link) = Some(Link {
        send: Arc::new(move |frame: &ViewerFrame| sender.send(frame)),
        close: Arc::new(move || closer.close()),
    });
    if session.closed.load(Ordering::SeqCst) {
        session.shut();
    }
    let mut sink = ViewerSink::new(session.queue.clone(), session.keyframes.clone());
    let mut encoder = (String::new(), false);
    let (code, detail) = loop {
        match stream.next_event() {
            Ok(StreamEvent::End(end)) => break (end.code, end.detail),
            Ok(event) => on_event(session, &mut sink, &mut encoder, own, event),
            Err(_) => break ("connection_lost".to_string(), None),
        }
    };
    finish(session, &code, detail);
}

fn on_event(
    session: &ViewSession,
    sink: &mut ViewerSink,
    encoder: &mut (String, bool),
    own: &str,
    event: StreamEvent,
) {
    let id = session.id;
    match event {
        StreamEvent::Pending(capability) => {
            let name = if matches!(capability, Capability::ScreenControl) {
                "screenControl"
            } else {
                "screenView"
            };
            emit(id, "pending", json!({ "capability": name }));
        }
        StreamEvent::Accepted { hello, path } => {
            let allowed = session.mode == SessionMode::Control
                && hello.authorization.control != CapabilityDecision::Off;
            session.control_allowed.store(allowed, Ordering::SeqCst);
            emit(id, "accepted", accepted_fields(&hello, &path));
        }
        StreamEvent::Host(frame) => on_host_frame(session, encoder, own, frame),
        StreamEvent::Record { header, record } => {
            for output in sink.record(&header, &record, now_us()) {
                on_sink_output(session, encoder, output);
            }
        }
        StreamEvent::Broken { gop_id } => {
            if let Some(output) = sink.broken(gop_id, now_us()) {
                on_sink_output(session, encoder, output);
            }
        }
        StreamEvent::Clock(_) | StreamEvent::End(_) => {}
    }
}

fn on_sink_output(session: &ViewSession, encoder: &(String, bool), output: SinkOutput) {
    match output {
        SinkOutput::NewStream {
            gop,
            codec,
            profile,
            size,
        } => emit(
            session.id,
            "stream",
            json!({
                "codec": codec.key(),
                "profile": profile.map(H264Profile::key),
                "width": size.width,
                "height": size.height,
                "gop": gop,
                "encoder": encoder.0,
                "hardware": encoder.1,
            }),
        ),
        SinkOutput::Keyframe(request) => {
            let _ = session.send(&ViewerFrame::KeyframeRequest(request));
        }
    }
}

fn on_host_frame(session: &ViewSession, encoder: &mut (String, bool), own: &str, frame: HostFrame) {
    let id = session.id;
    match frame {
        HostFrame::State(report) => {
            if let Some(stream) = &report.stream {
                *encoder = (stream.encoder.clone(), stream.hardware);
            }
            let fields = json!({ "state": report.state.key(), "reason": report.reason });
            emit(id, "state", fields);
        }
        HostFrame::Windows(list) => {
            let list = serde_json::to_value(&list).unwrap_or(Value::Null);
            emit(id, "windows", camel(list));
        }
        HostFrame::Stats(stats) => emit(id, "stats", stats_fields(&stats)),
        HostFrame::ControlState(state) => {
            let holder = state.holder.map(|holder| {
                if holder.device_id == own {
                    "viewer"
                } else {
                    "other"
                }
            });
            let blocked = state
                .blocked
                .and_then(|block| serde_json::to_value(block).ok());
            let fields = json!({ "holder": holder, "blocked": blocked });
            emit(id, "controlState", fields);
        }
        HostFrame::Error(error) => {
            let fields = json!({ "code": error.code, "detail": error.detail });
            emit(id, "error", fields);
        }
        HostFrame::Pong(_) => {}
    }
}
