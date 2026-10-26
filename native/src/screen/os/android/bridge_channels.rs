//! The channels between the Android app and the screen host of this process
//! (SCR-M8, C1 Android, C2): the app captures with MediaProjection or the
//! accessibility service and encodes with MediaCodec; Rust keeps the session
//! and its picture streams. Per host session one channel: control messages
//! from Rust to the app (`screenControl`, JSON) and pictures from the app to
//! Rust (`screenPush`). Channel 0 carries the input of the controlling
//! viewer to the accessibility service. Starts and ends reach the app as the
//! bridge events `screenHostStart` and `screenSession`.
//!
//! A pushed picture carries `flags`: bit 0 key frame, bit 1 a still picture
//! (8 bytes width and height, little endian, then RGBA rows), bits 16–31 the
//! low 16 bits of the encoder epoch it belongs to (`configure`). Nothing of a
//! picture is ever logged; events carry sizes, names and states only.
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::screen::core::registry::ScreenRegistry;
use crate::screen::core::wire_binary::MAX_RECORD_BYTES;
use crate::screen::core::capture_bytes::AdmittedBytes;
use crate::screen::core::capture_admission::CaptureOwner;

pub(crate) const FLAG_KEYFRAME: u32 = 0x1;
pub(crate) const FLAG_STILL: u32 = 0x2;
/// Captured RGBA and its MediaCodec access unit, stamped with one requested frame id.
pub(crate) const FLAG_CAPTURED_VIDEO: u32 = 0x4;
pub(crate) const EPOCH_SHIFT: u32 = 16;
/// The channel of the accessibility input (`screenControl(0, …)`).
pub(crate) const INPUT_CHANNEL: u64 = 0;
/// `screenPush` answers.
pub(crate) const PUSH_OK: i32 = 0;
pub(crate) const PUSH_UNKNOWN: i32 = -1;
pub(crate) const PUSH_INVALID: i32 = -2;
/// Pictures kept per session; a full queue drops its oldest delta picture.
const MAX_QUEUED: usize = 8;
/// Control messages kept per channel (the app polls at least once a second).
const MAX_CONTROL: usize = 64;

/// A session start as the app needs it (`screenHostStart`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HostStart {
    /// `monitor:0`, `window:<package>` or `synthetic`.
    pub source: String,
    /// Stills through the accessibility service only (tile previews).
    pub still: bool,
    pub viewer_device_id: String,
    pub viewer_name: String,
    pub max_width: u32,
    pub max_height: u32,
}

/// One picture of the app.
pub(crate) struct Picture {
    pub data: AdmittedBytes,
    pub flags: u32,
}

impl Picture {
    pub(crate) fn keyframe(&self) -> bool {
        self.flags & FLAG_KEYFRAME != 0
    }

    pub(crate) fn still(&self) -> bool {
        self.flags & FLAG_STILL != 0
    }

    pub(crate) fn captured_video(&self) -> bool { self.flags & FLAG_CAPTURED_VIDEO != 0 }

    pub(crate) fn epoch(&self) -> u32 {
        self.flags >> EPOCH_SHIFT
    }
}

/// Where a session stands for the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    /// The app asks its user (system dialog „Bildschirm teilen“) or prepares.
    Consent,
    Ready {
        width: u32,
        height: u32,
        encoder: String,
        hardware: bool,
        /// MediaProjection runs (granted in the system dialog).
        projection: bool,
    },
    /// Ended by the app or the host; the text is the reason key.
    Ended(String),
}

#[derive(Default)]
struct Queues {
    control: VecDeque<String>,
    pictures: VecDeque<Picture>,
    waiting_keyframe: bool,
    encoder_failure: Option<String>,
    active_epoch: Option<u32>,
}

/// One host session as the app sees it.
pub(crate) struct Channel {
    pub(crate) id: u64,
    pub(crate) start: HostStart,
    state: Mutex<(Phase, Queues)>,
    changed: Condvar,
    pub(crate) capture: super::android_capture_admission::CaptureSession,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Channel {
    fn new(id: u64, start: HostStart) -> Self {
        Self {
            id,
            start,
            state: Mutex::new((Phase::Consent, Queues::default())),
            changed: Condvar::new(),
            capture: super::android_capture_admission::CaptureSession::default(),
        }
    }

    pub(crate) fn phase(&self) -> Phase {
        lock(&self.state).0.clone()
    }

    /// Queues `message` for the app.
    pub(crate) fn send(&self, message: Value) {
        let mut state = lock(&self.state);
        if message.get("op").and_then(Value::as_str)==Some("configure") {
            // Retire old queued storage atomically with its epoch. A producer
            // cannot enqueue an old-epoch owner between clear and rebinding.
            state.1.pictures.clear();
            state.1.active_epoch=message.get("epoch").and_then(Value::as_u64).and_then(|epoch|u32::try_from(epoch).ok());
            state.1.encoder_failure=None;
        }
        if state.1.control.len() >= MAX_CONTROL {
            state.1.control.pop_front();
        }
        state.1.control.push_back(message.to_string());
        self.changed.notify_all();
    }

    /// Drops every queued picture (a new encoder epoch begins).
    pub(crate) fn clear_pictures(&self) {
        lock(&self.state).1.pictures.clear();
    }

    /// The next picture that `accept` takes, waiting at most `timeout`;
    /// pictures it refuses are dropped. `Err` with the reason once ended.
    pub(crate) fn next_picture(
        &self,
        timeout: Duration,
        accept: impl Fn(&Picture) -> bool,
    ) -> Result<Option<Picture>, String> {
        let deadline = Instant::now() + timeout;
        let mut state = lock(&self.state);
        loop {
            if let Phase::Ended(reason) = &state.0 {
                return Err(reason.clone());
            }
            if let Some(reason) = state.1.encoder_failure.take() { return Err(format!("encoder_failed: {reason}")); }
            while let Some(picture) = state.1.pictures.pop_front() {
                if accept(&picture) {
                    return Ok(Some(picture));
                }
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            state = self
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Waits at most `timeout` for a phase other than `Consent`.
    pub(crate) fn wait_started(&self, timeout: Duration) -> Phase {
        let state = lock(&self.state);
        let (state, _) = self
            .changed
            .wait_timeout_while(state, timeout, |state| state.0 == Phase::Consent)
            .unwrap_or_else(PoisonError::into_inner);
        state.0.clone()
    }

    fn set_phase(&self,phase:Phase)->bool {
        let mut state=lock(&self.state);
        if matches!(state.0,Phase::Ended(_)) {return false;}
        if matches!(phase,Phase::Ended(_)) {state.1.pictures.clear();self.capture.retire();}
        state.0=phase;
        drop(state);self.changed.notify_all();true
    }

    fn push(&self, generation: u64, data: &[u8], flags: u32) -> i32 {
        let mut state = lock(&self.state);
        if matches!(state.0, Phase::Ended(_)) {
            return PUSH_UNKNOWN;
        }
        let Ok(grant) = self.capture.grant(generation) else {return PUSH_INVALID;};
        let epoch=flags>>EPOCH_SHIFT;
        if flags & (FLAG_STILL|FLAG_CAPTURED_VIDEO|FLAG_KEYFRAME)!=0 && epoch!=0
            && state.1.active_epoch.is_some_and(|active|active!=epoch) {return PUSH_OK;}
        if flags & 0x8 !=0 {
            if data.len()>4096 {return PUSH_INVALID;}
            if state.1.active_epoch.is_some_and(|active|active!=epoch) {return PUSH_OK;}
            state.1.encoder_failure=Some(String::from_utf8_lossy(data).into_owned());
            state.1.pictures.clear();self.changed.notify_all();return PUSH_OK;
        }
        if flags & 0x20 !=0 {
            if data.len()!=9 || data[0]>1 {return PUSH_INVALID;}
            state.1.pictures.clear();state.1.encoder_failure=None;state.1.waiting_keyframe=true;
            let Ok(data)=self.capture.activity_ack(data) else {return PUSH_INVALID;};
            let picture=Picture {data,flags};
            state.1.pictures.push_back(picture);self.changed.notify_all();return PUSH_OK;
        }
        let queued_bytes: usize = state.1.pictures.iter().map(|p|p.data.len()).sum();
        let raw=flags & (FLAG_STILL|FLAG_CAPTURED_VIDEO)!=0;
        let overflow=if raw {!state.1.pictures.is_empty()} else {queued_bytes.saturating_add(data.len())>16*1024*1024};
        if state.1.pictures.len() >= MAX_QUEUED || overflow {
            state.1.pictures.clear();
            state.1.waiting_keyframe = true;
            if state.1.control.len() < MAX_CONTROL { state.1.control.push_back(json!({"op":"keyframe"}).to_string()); }
        }
        if flags & FLAG_STILL == 0 {
            if flags & FLAG_KEYFRAME != 0 { state.1.waiting_keyframe = false; }
            else if state.1.waiting_keyframe { return PUSH_OK; }
        }
        // Queue eviction and grant validation happen under this lock, BEFORE
        // the replacement copy. Producer + queued + consumer remain three J.
        let Ok(data)=AdmittedBytes::copy(&grant,CaptureOwner::Transport,data) else {return PUSH_INVALID;};
        let picture=Picture {data,flags};
        state.1.pictures.push_back(picture);
        self.changed.notify_all();
        PUSH_OK
    }

    pub(crate) fn begin_capture(&self, size: crate::screen::core::pixel_convert::FrameSize, kind: u32)
        -> std::io::Result<u64> {
        let state=lock(&self.state);
        if matches!(state.0,Phase::Ended(_)) {return Err(std::io::Error::other("Android capture session ended"));}
        self.capture.begin(size,kind)
    }

    fn take_control(&self, timeout: Duration) -> Option<String> {
        let state = lock(&self.state);
        let (mut state, _) = self
            .changed
            .wait_timeout_while(state, timeout, |state| {
                state.1.control.is_empty() && !matches!(state.0, Phase::Ended(_))
            })
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(message) = state.1.control.pop_front() {
            return Some(message);
        }
        match &state.0 {
            Phase::Ended(reason) => Some(json!({"op": "stop", "reason": reason}).to_string()),
            _ => None,
        }
    }
}

/// How the app reports its prerequisites (`screen.hostPrereqs`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Prereqs {
    /// The app registered as screen host in this process.
    pub registered: bool,
    /// `on`, `off` or `restricted` („Eingeschränkte Einstellungen“).
    pub accessibility: String,
    pub usage_access: bool,
    pub app_window_capture: bool,
    pub display: Option<(u32, u32)>,
    /// Available Java allocator bytes observed before this source opens; absent means legacy.
    pub working_bytes:Option<u64>,
    /// Reported MediaCodec profiles, hardware and explicit dimensional capability evidence.
    pub encoders: Vec<crate::screen::core::negotiate::CodecOffer>,
    /// Running apps: (package, label).
    pub apps: Vec<(String, String)>,
}

impl Prereqs {
    pub(crate) fn accessibility_on(&self) -> bool {
        self.accessibility == "on"
    }
}

/// Where the bridge events go (the mobile event hub).
pub(crate) type EventSink = fn(Value);

#[derive(Default)]
struct Register {
    next_id: u64,
    channels: BTreeMap<u64, Arc<Channel>>,
    prereqs: Prereqs,
    sink: Option<EventSink>,
    registry: Weak<ScreenRegistry>,
}

static REGISTER: Mutex<Option<Register>> = Mutex::new(None);
static INPUT: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
static INPUT_READY: Condvar = Condvar::new();

fn with<R>(apply: impl FnOnce(&mut Register) -> R) -> R {
    let mut register = lock(&REGISTER);
    apply(register.get_or_insert_with(Register::default))
}

fn emit(event: Value) {
    if let Some(sink) = with(|register| register.sink) {
        sink(event);
    }
}

fn session_event(id: u64, state: &str, reason: Option<&str>) {
    emit(json!({"type": "screenSession", "session": id, "state": state, "reason": reason}));
}

pub(crate) fn set_event_sink(sink: EventSink) {
    with(|register| register.sink = Some(sink));
}

/// The screen registry of this process's share node (from its indicator).
pub(crate) fn set_registry(registry: &Arc<ScreenRegistry>) {
    with(|register| register.registry = Arc::downgrade(registry));
}

pub(crate) fn registry() -> Option<Arc<ScreenRegistry>> {
    with(|register| register.registry.upgrade())
}

pub(crate) fn prereqs() -> Prereqs {
    with(|register| register.prereqs.clone())
}

pub(crate) fn set_prereqs(change: impl FnOnce(&mut Prereqs)) -> Prereqs {
    with(|register| {
        change(&mut register.prereqs);
        register.prereqs.clone()
    })
}

/// Opens the channel of a new host session and tells the app.
pub(crate) fn open(start: HostStart) -> Arc<Channel> {
    let channel = register_channel(start);
    let start = &channel.start;
    emit(json!({
        "type": "screenHostStart",
        "session": channel.id,
        "source": start.source,
        "still": start.still,
        "maxWidth": start.max_width,
        "maxHeight": start.max_height,
        "viewerName": start.viewer_name,
        "viewerDeviceId": start.viewer_device_id,
    }));
    session_event(channel.id, "consent", None);
    channel
}

fn register_channel(start: HostStart) -> Arc<Channel> {
    with(|register| {
        register.next_id += 1;
        let channel = Arc::new(Channel::new(register.next_id, start));
        register.channels.insert(channel.id, channel.clone());
        channel
    })
}

/// Instrumentation owns its producer; never emit a product controller start/stop.
#[cfg(all(target_os = "android", feature = "android-instrumentation-fixtures"))]
pub(crate) fn open_fixture(start: HostStart) -> Arc<Channel> {register_channel(start)}
#[cfg(all(target_os = "android", feature = "android-instrumentation-fixtures"))]
pub(crate) fn close_fixture(id: u64) {
    if let Some(channel) = with(|register| register.channels.remove(&id)) {
        channel.set_phase(Phase::Ended("fixture_retired".into()));
    }
}

/// The source ended on the host side: the app stops on its next poll.
pub(crate) fn close(id: u64, reason: &str) {
    if let Some(channel) = with(|register| register.channels.remove(&id)) {
        if !matches!(channel.phase(), Phase::Ended(_)) {
            channel.set_phase(Phase::Ended(reason.into()));
            session_event(id, "ended", Some(reason));
        }
    }
}

pub(crate) fn channel(id: u64) -> Option<Arc<Channel>> {
    with(|register| register.channels.get(&id).cloned())
}

pub(crate) fn channels() -> Vec<Arc<Channel>> {
    with(|register| register.channels.values().cloned().collect())
}

/// `screenPush`: one picture of session `id`. The app's presentation time
/// paces only its own encoder; the stream takes the time of arrival.
pub(crate) fn push(id: u64, data: &[u8], flags: u32) -> i32 {
    // Legacy entrypoint carries only bounded lifecycle messages. Product
    // pixels/access units must identify their granted generation explicitly.
    if flags & (0x8|0x20)==0 {return PUSH_INVALID;}
    let Some(channel)=channel(id) else {return PUSH_UNKNOWN;};
    let Some(grant)=channel.capture.current() else {return PUSH_INVALID;};
    push_admitted(id,grant.generation().value(),data,flags)
}
pub(crate) fn push_admitted(id:u64,generation:u64,data:&[u8],flags:u32)->i32 {
    let raw = flags & (FLAG_STILL | FLAG_CAPTURED_VIDEO) != 0;
    let limit=if raw {64*1024*1024+MAX_RECORD_BYTES+20} else {MAX_RECORD_BYTES};
    if data.is_empty() || data.len() > limit {
        return PUSH_INVALID;
    }
    if raw {
        let Some(header) = data.get(..8) else { return PUSH_INVALID; };
        let width = u32::from_le_bytes([header[0],header[1],header[2],header[3]]);
        let height = u32::from_le_bytes([header[4],header[5],header[6],header[7]]);
        let size = crate::screen::core::pixel_convert::FrameSize::new(width,height);
        let Ok(bytes)=crate::screen::core::frame_budget::FrameLimits::RESOURCE.check(size) else {return PUSH_INVALID;};
        let expected = bytes.checked_add(if flags & FLAG_CAPTURED_VIDEO != 0 { 20 } else { 8 });
        let unchanged=flags & 0x10 != 0 && flags & FLAG_CAPTURED_VIDEO != 0 && data.len()==20;
        if !unchanged {
            let video=if flags & FLAG_CAPTURED_VIDEO!=0 {
                let Some(len)=data.get(16..20) else {return PUSH_INVALID;};
                u32::from_le_bytes([len[0],len[1],len[2],len[3]]) as usize
            } else {0};
            if video>MAX_RECORD_BYTES || expected.and_then(|n|n.checked_add(video))!=Some(data.len()) {return PUSH_INVALID;}
        }
    }
    let Some(channel) = channel(id) else { return PUSH_UNKNOWN; };
    channel.push(generation,data,flags)
}

/// `screenControl`: the next message for session `id` (channel 0: input),
/// waiting at most `timeout`; an unknown session answers `stop`.
pub(crate) fn control(id: u64, timeout: Duration) -> Option<String> {
    if id == INPUT_CHANNEL {
        let queue = lock(&INPUT);
        let (mut queue, _) = INPUT_READY
            .wait_timeout_while(queue, timeout, |queue| queue.is_empty())
            .unwrap_or_else(PoisonError::into_inner);
        return queue.pop_front();
    }
    match channel(id) {
        Some(channel) => channel.take_control(timeout),
        None => Some(json!({"op": "stop", "reason": "unknown_session"}).to_string()),
    }
}

/// Queues the input of the controlling viewer for the accessibility service.
pub(crate) fn send_input(message: Value) {
    let mut queue = lock(&INPUT);
    if queue.len() >= MAX_CONTROL {
        queue.pop_front();
    }
    queue.push_back(message.to_string());
    INPUT_READY.notify_all();
}

/// `screen.hostReady`: the app captures (after the system dialog) or serves
/// stills.
pub(crate) fn ready(id: u64, phase: Phase) -> Result<(), String> {
    let channel = channel(id).ok_or_else(|| format!("Keine Bildschirmsitzung {id}"))?;
    if !channel.set_phase(phase) {return Err(format!("Bildschirmsitzung {id} ist beendet; Codecbestätigung verworfen"));}
    session_event(id, "streaming", None);
    Ok(())
}

/// `screen.hostEnded`: the app ended the session (system dialog refused,
/// device locked, „Beenden“ in its notification).
pub(crate) fn ended(id: u64, reason: &str) -> Result<(), String> {
    let channel = channel(id).ok_or_else(|| format!("Keine Bildschirmsitzung {id}"))?;
    if channel.set_phase(Phase::Ended(reason.into())) {session_event(id,"ended",Some(reason));}
    Ok(())
}

/// Serializes the tests that use the process-wide register.
#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = Mutex::new(());

/// A fresh register for one test (hold `TEST_LOCK`).
#[cfg(test)]
pub(crate) fn reset_for_tests() {
    *lock(&REGISTER) = None;
    lock(&INPUT).clear();
}
