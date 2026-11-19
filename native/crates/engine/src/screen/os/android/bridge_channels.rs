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
#[path = "bridge_input_queue.rs"]
mod input_queue;
pub use input_queue::{send_controlling_input, send_input};

pub const FLAG_KEYFRAME: u32 = 0x1;
pub const FLAG_STILL: u32 = 0x2;
/// Captured RGBA and its MediaCodec access unit, stamped with one requested frame id.
pub const FLAG_CAPTURED_VIDEO: u32 = 0x4;
pub const EPOCH_SHIFT: u32 = 16;
/// The channel of the accessibility input (`screenControl(0, …)`).
pub const INPUT_CHANNEL: u64 = 0;
/// `screenPush` answers.
pub const PUSH_OK: i32 = 0;
pub const PUSH_UNKNOWN: i32 = -1;
pub const PUSH_INVALID: i32 = -2;
/// Pictures kept per session; a full queue drops its oldest delta picture.
const MAX_QUEUED: usize = 8;
/// Control messages kept per channel (the app polls at least once a second).
const MAX_CONTROL: usize = 64;
/// Notices of the app waiting for the session (`Channel::push_notice`): the
/// session takes one per frame, so this only bounds a burst.
const MAX_NOTICES: usize = 8;

/// A session start as the app needs it (`screenHostStart`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostStart {
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
pub struct Picture {
    pub data: AdmittedBytes,
    pub flags: u32,
}

impl Picture {
    pub fn keyframe(&self) -> bool {
        self.flags & FLAG_KEYFRAME != 0
    }

    pub fn still(&self) -> bool {
        self.flags & FLAG_STILL != 0
    }

    pub fn captured_video(&self) -> bool {
        self.flags & FLAG_CAPTURED_VIDEO != 0
    }

    pub fn epoch(&self) -> u32 {
        self.flags >> EPOCH_SHIFT
    }
}

/// Where a session stands for the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
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
    /// The lowest bit rate the encoder of `active_epoch` runs at (the app's
    /// MediaCodec range, `screen.hostNotice`); `None` until the app said it.
    rate_floor: Option<u64>,
    /// Notices of the app for the viewer (`HostStage::take_notice`), oldest
    /// first; each is given once.
    notices: VecDeque<String>,
    /// The app's encoder refused the last bit rate sent (`video-bitrate`,
    /// Codex 3/16): the stage no longer counts it as running.
    bitrate_refused: bool,
}

/// One host session as the app sees it.
pub struct Channel {
    pub id: u64,
    pub start: HostStart,
    state: Mutex<(Phase, Queues)>,
    changed: Condvar,
    pub capture: super::android_capture_admission::CaptureSession,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How the app reports its prerequisites (`screen.hostPrereqs`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Prereqs {
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
    /// HEVC and AV1 whose MediaCodec encoder the app refused to open in this run
    /// (`encoder_failed` of a session of that codec): not offered again until the app
    /// reports other encoders, so the next session negotiates H.264 (BK7b, B-26).
    pub refused: Vec<crate::screen::core::negotiate::CodecKind>,
    /// Running apps: (package, label).
    pub apps: Vec<(String, String)>,
}

impl Prereqs {
    pub fn accessibility_on(&self) -> bool {
        self.accessibility == "on"
    }
}

/// Where the bridge events go (the mobile event hub).
pub type EventSink = fn(Value);

#[derive(Default)]
struct Register {
    next_id: u64,
    channels: BTreeMap<u64, Arc<Channel>>,
    prereqs: Prereqs,
    sink: Option<EventSink>,
    registry: Weak<ScreenRegistry>,
}

static REGISTER: Mutex<Option<Register>> = Mutex::new(None);

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

pub fn set_event_sink(sink: EventSink) {
    with(|register| register.sink = Some(sink));
}

/// The screen registry of this process's share node (from its indicator).
pub fn set_registry(registry: &Arc<ScreenRegistry>) {
    with(|register| register.registry = Arc::downgrade(registry));
}

pub fn registry() -> Option<Arc<ScreenRegistry>> {
    with(|register| register.registry.upgrade())
}

pub fn prereqs() -> Prereqs {
    with(|register| register.prereqs.clone())
}

/// Notes that the app's encoder of `codec` (HEVC or AV1) failed to open: it is left out of the host's offers.
pub fn refuse_codec(codec: crate::screen::core::negotiate::CodecKind) {
    if codec.is_legacy() {
        return;
    }
    set_prereqs(|prereqs| {
        if !prereqs.refused.contains(&codec) {
            prereqs.refused.push(codec);
        }
    });
}

pub fn set_prereqs(change: impl FnOnce(&mut Prereqs)) -> Prereqs {
    with(|register| {
        change(&mut register.prereqs);
        register.prereqs.clone()
    })
}

/// Opens the channel of a new host session and tells the app.
pub fn open(start: HostStart) -> Arc<Channel> {
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
pub fn open_fixture(start: HostStart) -> Arc<Channel> {
    register_channel(start)
}
#[cfg(all(target_os = "android", feature = "android-instrumentation-fixtures"))]
pub fn close_fixture(id: u64) {
    if let Some(channel) = with(|register| register.channels.remove(&id)) {
        channel.set_phase(Phase::Ended("fixture_retired".into()));
    }
}

/// The source ended on the host side: the app stops on its next poll.
pub fn close(id: u64, reason: &str) {
    let (removed, last) = with(|register| {
        let removed = register.channels.remove(&id);
        (removed, register.channels.is_empty())
    });
    if let Some(channel) = removed {
        if !matches!(channel.phase(), Phase::Ended(_)) {
            channel.set_phase(Phase::Ended(reason.into()));
            session_event(id, "ended", Some(reason));
        }
        input_queue::drop_session(id);
        if last {
            drop_pending_input();
        }
    }
}

/// The last session ended: the app's input reader stops with it, so input
/// still queued belongs to no session any more. It is dropped here instead of
/// being performed when the next session starts the reader again (review
/// 2026-11-19, Codex 2/1).
fn drop_pending_input() {
    input_queue::clear();
}

pub fn channel(id: u64) -> Option<Arc<Channel>> {
    with(|register| register.channels.get(&id).cloned())
}

pub fn channels() -> Vec<Arc<Channel>> {
    with(|register| register.channels.values().cloned().collect())
}

/// `screenPush`: one picture of session `id`. The app's presentation time
/// paces only its own encoder; the stream takes the time of arrival.
pub fn push(id: u64, data: &[u8], flags: u32) -> i32 {
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
pub fn push_admitted(id: u64, generation: u64, data: &[u8], flags: u32) -> i32 {
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
pub fn control(id: u64, timeout: Duration) -> Option<String> {
    if id == INPUT_CHANNEL {
        return input_queue::take(timeout);
    }
    match channel(id) {
        Some(channel) => channel.take_control(timeout),
        None => Some(json!({"op": "stop", "reason": "unknown_session"}).to_string()),
    }
}

/// `screen.hostNotice` with `rateFloor`: the lowest bit rate of the
/// encoder of `epoch` (the `epoch` of its `configure`).
pub fn rate_floor(id: u64, epoch: u32, bps: u64) -> Result<(), String> {
    let channel = channel(id).ok_or_else(|| format!("Keine Bildschirmsitzung {id}"))?;
    channel.set_rate_floor(epoch, bps);
    Ok(())
}

/// `screen.hostNotice` with a sentence for the viewer (an encoder that
/// refused a parameter); `bitrate`: the refused parameter was the bit rate.
pub fn notice(id: u64, text: String, bitrate: bool) -> Result<(), String> {
    let channel = channel(id).ok_or_else(|| format!("Keine Bildschirmsitzung {id}"))?;
    if bitrate {
        channel.note_bitrate_refused();
    }
    channel.push_notice(text);
    Ok(())
}

/// `screen.hostReady`: the app captures (after the system dialog) or serves
/// stills.
pub fn ready(id: u64, phase: Phase) -> Result<(), String> {
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
pub fn ended(id: u64, reason: &str) -> Result<(), String> {
    let channel = channel(id).ok_or_else(|| format!("Keine Bildschirmsitzung {id}"))?;
    if channel.set_phase(Phase::Ended(reason.into())) {
        session_event(id, "ended", Some(reason));
    }
    Ok(())
}

/// Serializes the tests that use the process-wide register.
#[cfg(any(test, feature = "test-support"))]
pub static TEST_LOCK: Mutex<()> = Mutex::new(());

/// A fresh register for one test (hold `TEST_LOCK`).
#[cfg(any(test, feature = "test-support"))]
pub fn reset_for_tests() {
    *lock(&REGISTER) = None;
    input_queue::clear();
}
