//! This phone as screen viewer (SCR-M9, U28): the bridge methods of the
//! app's viewer side and the binary call `screenPull` of the JNI bridge
//! (`native/android-bridge/src/screen_jni.rs`). A session runs through the
//! background worker's relay (`daemon::screen_stream`, as `share.exec` runs
//! through its Exec stream); `screen_view_run.rs` keeps the sessions,
//! `screen/os/android/viewer_sink.rs` the queue of pictures.
//!
//! - `screen.devices {roomId?}` → `{"devices":[…]}` (profiles and the last
//!   `probe`; `null` = unknown).
//! - `screen.feed {deviceId, publicKey}` → the device feed (design 5.2).
//! - `screen.probe {relation, relationId, deviceId}` and `screen.windows
//!   {…}` answer like `ProbeReport` and `WindowList` with camelCase keys.
//! - `screen.viewOpen {…, mode, source?, decoders, displayWidth,
//!   displayHeight, maxFps, maxRate?, codec?, decoder?}` → `{"session"}`
//!   (`maxRate`: „Höchste Datenrate“ in bit/s, absent = automatic; `codec`
//!   and `decoder`: `screen_view_codecs.rs`; the event `accepted` carries
//!   `rateCeiling`: `automatic`, `applied` or `unsupported_by_host`);
//!   progress comes as events
//!   `{"type":"screenView","session","kind":"pending|accepted|state|stream|
//!   stats|controlState|windows|error|ended", …}`.
//! - `screen.viewSetMode {…, maxRate?}` (`maxRate`: „Höchste Datenrate“ changed live, 0 =
//!   automatic), `viewInput {session, events}` (an event is the
//!   serde form of `InputEvent`; a `key` event may name the key instead of its
//!   `hid_usage`), `viewControl {session, action}`, `viewListWindows`,
//!   `viewKeyframe`, `viewClose` (idempotent), `viewSetDecoders` (a new
//!   „Codec“/„Dekodierer“ in the session, `screen_view_switch.rs`).
use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};

use super::args::{invalid, now_secs, opt_bool, opt_i64, opt_str, str_arg};
use super::share_state;
use crate::daemon::StreamRequest;
use crate::mobile::{ApiError, Runtime};
use crate::screen::core::gop_stream::now_us;
use crate::screen::core::input_model::{key_usage, InputEvent, MAX_INPUT_EVENTS};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::window_list::SourceId;
use crate::screen::core::wire_control::{
    ControlAction, ControlRequest, KeyframeReason, KeyframeRequest, SetMode, StreamMode,
    ViewerFrame,
};
use crate::screen::core::wire_hello::{SessionMode, MAX_DISPLAY_SIDE};
use crate::share::{PeerOpenTarget, ShareProfiles};

#[path = "screen_view_codecs.rs"]
mod codecs;
#[path = "screen_family.rs"]
mod family;
#[path = "screen_feed.rs"]
mod feed;
#[path = "screen_icons.rs"]
mod icons;
#[path = "screen_view_json.rs"]
mod json_shapes;
#[path = "screen_view_presented.rs"]
mod presented;
#[path = "screen_view_run.rs"]
mod run;
#[path = "screen_view_switch.rs"]
mod switch;
use json_shapes::{camel, devices_json, probe_key, ProbeFacts};
use run::{find, lock, ViewSession};

static PROBES: Mutex<BTreeMap<String, ProbeFacts>> = Mutex::new(BTreeMap::new());

/// `screenPull` of the JNI bridge: the next picture of `session` in `dst`
/// (layout in `viewer_sink.rs`), or one of the `PULL_*` codes.
pub fn screen_pull(session: u64, dst: &mut [u8], timeout: Duration) -> i32 {
    run::pull(session, dst, timeout)
}

/// Handles `screen.*` methods of the viewer; `None` = not one of them.
pub(super) fn handle(method: &str, args: &Value) -> Option<Result<Value, ApiError>> {
    if let Some(result) = family::handle(method, args) {
        return Some(result);
    }
    Some(match method {
        "screen.devices" => Ok(devices(args)),
        "screen.feed" => feed::device(args),
        "screen.probe" => probe(args),
        "screen.windows" => windows(args),
        "screen.icon" => icon(args),
        "screen.viewIcon" => session_arg(args).and_then(|session| icons::request(&session, args)),
        "screen.viewOpen" => view_open(args),
        "screen.viewSetMode" => set_mode(args),
        "screen.viewSetDecoders" => switch::set_decoders(args),
        "screen.viewInput" => input(args),
        "screen.viewVisibility" => visibility(args),
        "screen.viewPresented" => presented::confirm(args),
        "screen.viewControl" => control(args),
        "screen.viewListWindows" => session_arg(args)
            .and_then(|session| session.send(&ViewerFrame::ListWindows))
            .map(|()| json!({})),
        "screen.viewKeyframe" => keyframe(args),
        "screen.viewClose" => close(args),
        _ => return None,
    })
}

fn profiles() -> ShareProfiles {
    share_state::with_state(|state| {
        if state.profiles.is_none() {
            state.profiles = ShareProfiles::load_checked(share_state::default_home()).ok();
        }
        state.profiles.clone()
    })
    .unwrap_or_default()
}

fn own_device() -> String {
    share_state::identity().map_or_else(|_| String::new(), |identity| identity.device_id)
}

fn devices(args: &Value) -> Value {
    let probes = lock(&PROBES).clone();
    let room = opt_str(args, "roomId").filter(|room| !room.is_empty());
    devices_json(&profiles(), room, &own_device(), now_secs(), &probes)
}

/// The device a call names.
struct Target {
    relation: String,
    relation_id: String,
    device_id: Option<String>,
}

impl Target {
    fn parse(args: &Value) -> Result<Self, ApiError> {
        let relation = str_arg(args, "relation")?.to_string();
        let relation_id = str_arg(args, "relationId")?.trim().to_string();
        let device_id = opt_str(args, "deviceId")
            .filter(|device| !device.is_empty())
            .map(str::to_string);
        if relation_id.is_empty() {
            return Err(invalid("Ungültiges Bildschirmziel: relationId fehlt."));
        }
        match relation.as_str() {
            "direct" => {}
            "room" if device_id.is_some() => {}
            "room" => return Err(invalid("Das Raumgerät fehlt im Bildschirmziel.")),
            _ => return Err(invalid("„relation“ ist direct oder room.")),
        }
        Ok(Self {
            relation,
            relation_id,
            device_id,
        })
    }

    fn key(&self) -> String {
        let device = self.device_id.as_deref().unwrap_or("");
        probe_key(&self.relation, &self.relation_id, device)
    }

    fn peer(&self) -> PeerOpenTarget {
        match &self.device_id {
            Some(device_id) if self.relation == "room" => PeerOpenTarget::RoomDevice {
                room_id: self.relation_id.clone(),
                device_id: device_id.clone(),
            },
            _ => PeerOpenTarget::Direct {
                contact_id: self.relation_id.clone(),
            },
        }
    }

    /// A device that is gone or changed is refused (the texts of the desktop).
    fn check(&self, profiles: &ShareProfiles) -> Result<(), ApiError> {
        const CHANGED: &str = "Das Gerät hat sich geändert. Bitte die Verbindung zum Gerät prüfen.";
        if self.relation == "direct" {
            let contact = profiles
                .direct_contacts
                .iter()
                .find(|contact| contact.id == self.relation_id)
                .ok_or_else(|| invalid("Das Gerät ist nicht mehr in der Geräteliste."))?;
            return match (&self.device_id, &contact.remote_device_id) {
                (Some(named), Some(current)) if named == current => Ok(()),
                (Some(_), _) => Err(invalid(CHANGED)),
                (None, _) => Ok(()),
            };
        }
        let room = profiles
            .rooms
            .iter()
            .find(|room| room.id == self.relation_id || room.room_id == self.relation_id)
            .ok_or_else(|| invalid("Der Raum ist nicht mehr gespeichert."))?;
        let device = self.device_id.as_deref().unwrap_or_default();
        if room.members.iter().any(|member| member.device_id == device) {
            Ok(())
        } else {
            Err(invalid("Das Gerät gehört nicht mehr zum Raum."))
        }
    }
}

fn probe(args: &Value) -> Result<Value, ApiError> {
    let target = Target::parse(args)?;
    target.check(&profiles())?;
    let report = crate::daemon::screen_probe(target.peer())
        .map_err(|message| ApiError::new("network", message))?;
    let facts = ProbeFacts {
        supported: report.supported,
        view: report.view,
        control: report.control,
    };
    lock(&PROBES).insert(target.key(), facts);
    Ok(camel(serde_json::to_value(&report).unwrap_or(Value::Null)))
}

fn windows(args: &Value) -> Result<Value, ApiError> {
    let target = Target::parse(args)?;
    target.check(&profiles())?;
    let list = crate::daemon::screen_windows(target.peer(), |_| {})
        .map_err(|message| ApiError::new("network", message))?;
    Ok(camel(serde_json::to_value(&list).unwrap_or(Value::Null)))
}

fn icon(args: &Value) -> Result<Value, ApiError> {
    let target = Target::parse(args)?;
    target.check(&profiles())?;
    let reply = crate::daemon::screen_icon(target.peer(), icons::parse(args)?)
        .map_err(|message| ApiError::new("network", message))?;
    Ok(serde_json::to_value(reply).unwrap_or(Value::Null))
}

fn source_arg(args: &Value) -> Result<Option<SourceId>, ApiError> {
    match opt_str(args, "source").filter(|source| !source.is_empty()) {
        None => Ok(None),
        Some(text) => SourceId::parse(text)
            .map(Some)
            .map_err(|_| invalid("„source“ ist monitor:<id>, window:<id> oder synthetic.")),
    }
}

fn side_arg(value: Option<i64>) -> Option<u32> {
    value
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| (1..=MAX_DISPLAY_SIDE).contains(value))
}

fn size_args(args: &Value, width: &str, height: &str) -> Option<FrameSize> {
    match (
        side_arg(opt_i64(args, width)),
        side_arg(opt_i64(args, height)),
    ) {
        (Some(width), Some(height)) => Some(FrameSize::new(width, height)),
        _ => None,
    }
}

fn fps_arg(args: &Value, key: &str) -> Result<Option<u16>, ApiError> {
    let Some(value) = opt_i64(args, key) else {
        return Ok(None);
    };
    u16::try_from(value)
        .ok()
        .filter(|fps| *fps >= 1)
        .map(Some)
        .ok_or_else(|| invalid(format!("„{key}“ liegt zwischen 1 und 65535.")))
}

fn view_open(args: &Value) -> Result<Value, ApiError> {
    if Runtime::installed().is_none() {
        return Err(ApiError::internal("Die Kernlaufzeit läuft nicht."));
    }
    let target = Target::parse(args)?;
    target.check(&profiles())?;
    let mode = match str_arg(args, "mode")? {
        "view" => SessionMode::View,
        "control" => SessionMode::Control,
        "preview" => SessionMode::Preview,
        _ => return Err(invalid("„mode“ ist view, control oder preview.")),
    };
    let frame_capabilities = size_args(args, "textureWidth", "textureHeight")
        .map(|texture| {
            use crate::screen::core::frame_budget::FrameCapabilities;
            let capabilities = match args.get("workingBytes") {
                None | Some(Value::Null) => FrameCapabilities::legacy_resources(texture),
                Some(value) => FrameCapabilities::resources(texture)
                    .with_working_budget(
                        value
                            .as_u64()
                            .ok_or_else(|| invalid("Ungültiges Bildspeicherbudget."))?,
                    )
                    .map_err(|error| invalid(error.to_string()))?,
            };
            capabilities
                .validate()
                .map_err(|error| invalid(error.to_string()))?;
            Ok::<_, ApiError>(capabilities)
        })
        .transpose()?;
    if let Some(bytes) = args.get("workingBytes").and_then(Value::as_u64) {
        crate::screen::core::frame_budget::FrameLimits::RESOURCE
            .with_working_budget(bytes)
            .map_err(|error| invalid(error.to_string()))?;
        crate::screen::os::image_memory::observe_capacity_ceiling(
            crate::screen::core::frame_memory::CapacityCeiling::bounded(bytes),
        )
        .map_err(|error| invalid(error.to_string()))?;
    }
    // „Codec“ and „Dekodierer“ (plan 2026-11-14 „Bildkette“ 4.10, BK6).
    let (codec, decoder) = codecs::codec_args(args)?;
    let request = StreamRequest {
        mode,
        source: source_arg(args)?,
        decoders: codecs::decoders_arg(args, mode)?,
        display_size: size_args(args, "displayWidth", "displayHeight"),
        max_fps: fps_arg(args, "maxFps")?.unwrap_or(30),
        max_rate_bps: match opt_i64(args, "maxRate") {
            None => None,
            Some(rate) => Some(
                u64::try_from(rate)
                    .ok()
                    .filter(|rate| *rate > 0)
                    .ok_or_else(|| invalid("„maxRate“ ist eine positive Zahl (bit/s)."))?,
            ),
        },
        frame_capabilities,
        expected_node: opt_str(args, "expectedNodeId").map(str::to_string),
        codec,
        decoder,
        // B-36: the Android sink opens a decoder per picture group.
        gop_continue: false,
        gop_continue_decoders: Vec::new(),
    };
    let id = run::start(mode, target.peer(), request, own_device())?;
    Ok(json!({ "session": id }))
}

fn session_arg(args: &Value) -> Result<std::sync::Arc<ViewSession>, ApiError> {
    let id = opt_i64(args, "session")
        .and_then(|id| u64::try_from(id).ok())
        .filter(|id| *id > 0)
        .ok_or_else(|| invalid("Parameter „session“ fehlt oder ist keine Sitzung."))?;
    find(id).ok_or_else(|| ApiError::not_found("Die Sitzung gibt es nicht (mehr)."))
}

fn set_mode(args: &Value) -> Result<Value, ApiError> {
    let session = session_arg(args)?;
    let mode = match str_arg(args, "mode")? {
        "preview" => StreamMode::Preview,
        "full" => StreamMode::Full,
        _ => return Err(invalid("„mode“ ist preview oder full.")),
    };
    // „Höchste Datenrate“ changed live (BK7b): 0 is automatic. The frame leaves only for a
    // host that announced `rate_ceiling_v1` (`viewer_client_tasks.rs`).
    let ceiling = opt_i64(args, "maxRate")
        .map(|rate| {
            u64::try_from(rate)
                .map(|rate| (rate > 0).then_some(rate))
                .map_err(|_| {
                    invalid("„maxRate“ ist 0 (automatisch) oder eine positive Zahl (bit/s).")
                })
        })
        .transpose()?;
    session.send(&ViewerFrame::SetMode(SetMode {
        mode,
        source: source_arg(args)?,
        size: size_args(args, "width", "height"),
        fps: fps_arg(args, "fps")?,
        sharpness: opt_bool(args, "sharpness", false),
        lossless: opt_bool(args, "lossless", false),
    }))?;
    if let Some(ceiling) = ceiling {
        session.send(&ViewerFrame::RateCeiling(ceiling))?;
    }
    Ok(json!({}))
}

fn visibility(args: &Value) -> Result<Value, ApiError> {
    let session = session_arg(args)?;
    let report: crate::screen::core::visibility::ViewerVisibility = serde_json::from_value(
        args.get("report")
            .cloned()
            .ok_or_else(|| invalid("Sichtbarkeitsbericht fehlt"))?,
    )
    .map_err(|error| invalid(error.to_string()))?;
    report
        .validate()
        .map_err(|error| invalid(error.to_string()))?;
    if report.hidden() {
        session.release_pressed();
    }
    session.send(&ViewerFrame::Visibility(report))?;
    Ok(json!({}))
}

/// An input event from JSON; a `key` event may carry `key` (a name of
/// `key_usage`) instead of `hid_usage`.
fn input_event(value: &Value) -> Result<InputEvent, ApiError> {
    let mut value = value.clone();
    if let Some(map) = value.as_object_mut() {
        if let Some(name) = map.get("key").and_then(Value::as_str).map(str::to_string) {
            let usage =
                key_usage(&name).ok_or_else(|| invalid(format!("Unbekannte Taste „{name}“.")))?;
            map.insert("hid_usage".into(), json!(usage));
        }
    }
    serde_json::from_value(value)
        .map_err(|error| invalid(format!("Ungültiges Eingabeereignis: {error}")))
}

fn no_control() -> ApiError {
    ApiError::new("permission", "Das Gerät erlaubt hier kein Steuern.")
}

/// The events of a `screen.viewInput` call.
fn parse_events(args: &Value) -> Result<Vec<InputEvent>, ApiError> {
    let list = args
        .get("events")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("Parameter „events“ fehlt."))?;
    if list.len() > MAX_INPUT_EVENTS {
        return Err(invalid(format!(
            "Höchstens {MAX_INPUT_EVENTS} Ereignisse je Aufruf."
        )));
    }
    list.iter().map(input_event).collect()
}

fn input(args: &Value) -> Result<Value, ApiError> {
    let session = session_arg(args)?;
    if lock(&session.family).is_some() {
        return family::route_input(&session, parse_events(args)?).map(|()| json!({}));
    }
    if !session.control_allowed.load(Ordering::SeqCst) {
        return Err(no_control());
    }
    let events = parse_events(args)?;
    {
        let mut pressed = lock(&session.pressed);
        events.iter().for_each(|event| pressed.observe(event));
    }
    let sent = events.len();
    if sent > 0 {
        session.send(&ViewerFrame::Input(events))?;
    }
    Ok(json!({ "sent": sent }))
}

fn control(args: &Value) -> Result<Value, ApiError> {
    let session = session_arg(args)?;
    let action = match str_arg(args, "action")? {
        "take" if session.control_allowed.load(Ordering::SeqCst) => ControlAction::Take,
        "take" => return Err(no_control()),
        "release" => {
            session.release_pressed();
            ControlAction::Release
        }
        _ => return Err(invalid("„action“ ist take oder release.")),
    };
    session.send(&ViewerFrame::ControlRequest(ControlRequest { action }))?;
    Ok(json!({}))
}

/// The app could not decode: one key frame request (`DecodeError`), unless
/// one is open.
fn keyframe(args: &Value) -> Result<Value, ApiError> {
    let session = session_arg(args)?;
    let due = lock(&session.keyframes).should_request(now_us());
    if due {
        session.send(&ViewerFrame::KeyframeRequest(KeyframeRequest {
            reason: KeyframeReason::DecodeError,
            gop_id: None,
        }))?;
    }
    Ok(json!({}))
}

fn close(args: &Value) -> Result<Value, ApiError> {
    match session_arg(args) {
        Ok(session) => session.close(),
        Err(error) if error.kind != "not_found" => return Err(error),
        Err(_) => {}
    }
    Ok(json!({}))
}

#[cfg(test)]
#[path = "screen_view_tests.rs"]
mod tests;
