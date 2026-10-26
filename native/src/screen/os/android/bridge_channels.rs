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

use crate::screen::core::capture_admission::CaptureOwner;
use crate::screen::core::capture_bytes::AdmittedBytes;
use crate::screen::core::registry::ScreenRegistry;
use crate::screen::core::wire_binary::MAX_RECORD_BYTES;

#[path = "bridge_channel_state.rs"]
mod channel_state;

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

    pub(crate) fn captured_video(&self) -> bool {
        self.flags & FLAG_CAPTURED_VIDEO != 0
    }

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
    pub working_bytes: Option<u64>,
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
pub(crate) fn open_fixture(start: HostStart) -> Arc<Channel> {
    register_channel(start)
}
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
    if flags & (0x8 | 0x20) == 0 {
        return PUSH_INVALID;
    }
    let Some(channel) = channel(id) else {
        return PUSH_UNKNOWN;
    };
    let Some(grant) = channel.capture.current() else {
        return PUSH_INVALID;
    };
    push_admitted(id, grant.generation().value(), data, flags)
}
pub(crate) fn push_admitted(id: u64, generation: u64, data: &[u8], flags: u32) -> i32 {
    let raw = flags & (FLAG_STILL | FLAG_CAPTURED_VIDEO) != 0;
    let limit = if raw {
        64 * 1024 * 1024 + MAX_RECORD_BYTES + 20
    } else {
        MAX_RECORD_BYTES
    };
    if data.is_empty() || data.len() > limit {
        return PUSH_INVALID;
    }
    if raw {
        let Some(header) = data.get(..8) else {
            return PUSH_INVALID;
        };
        let width = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
        let height = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
        let size = crate::screen::core::pixel_convert::FrameSize::new(width, height);
        let Ok(bytes) = crate::screen::core::frame_budget::FrameLimits::RESOURCE.check(size) else {
            return PUSH_INVALID;
        };
        let expected = bytes.checked_add(if flags & FLAG_CAPTURED_VIDEO != 0 {
            20
        } else {
            8
        });
        let unchanged = flags & 0x10 != 0 && flags & FLAG_CAPTURED_VIDEO != 0 && data.len() == 20;
        if !unchanged {
            let video = if flags & FLAG_CAPTURED_VIDEO != 0 {
                let Some(len) = data.get(16..20) else {
                    return PUSH_INVALID;
                };
                u32::from_le_bytes([len[0], len[1], len[2], len[3]]) as usize
            } else {
                0
            };
            if video > MAX_RECORD_BYTES
                || expected.and_then(|n| n.checked_add(video)) != Some(data.len())
            {
                return PUSH_INVALID;
            }
        }
    }
    let Some(channel) = channel(id) else {
        return PUSH_UNKNOWN;
    };
    channel.push(generation, data, flags)
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
    if !channel.set_phase(phase) {
        return Err(format!(
            "Bildschirmsitzung {id} ist beendet; Codecbestätigung verworfen"
        ));
    }
    session_event(id, "streaming", None);
    Ok(())
}

/// `screen.hostEnded`: the app ended the session (system dialog refused,
/// device locked, „Beenden“ in its notification).
pub(crate) fn ended(id: u64, reason: &str) -> Result<(), String> {
    let channel = channel(id).ok_or_else(|| format!("Keine Bildschirmsitzung {id}"))?;
    if channel.set_phase(Phase::Ended(reason.into())) {
        session_event(id, "ended", Some(reason));
    }
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
