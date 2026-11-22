//! This phone as screen host (SCR-M8): the bridge methods of the app's
//! host side and the two binary calls of the JNI bridge (`screenPush`,
//! `screenControl`, `native/android-bridge/src/screen_jni.rs`).
//!
//! - `screen.hostPrereqs {accessibility?, usageAccess?, appWindowCapture?,
//!   display?, encoders?, apps?}` (`encoders`: `screen_host_encoders.rs`) registers the app as host and stores what
//!   it reports; answers `{"accessibility":"on|off|restricted",
//!   "usageAccess":bool,"appWindowCapture":bool}`.
//! - `screen.hostSessions` → `{"sessions":[…]}`.
//! - `screen.hostStop {session|all}` ends sessions as the host user.
//! - `screen.hostReady {session, width, height, encoder, hardware?,
//!   projection?}` after the system dialog or for still previews.
//! - `screen.hostEnded {session, reason}`.
//! - `screen.hostNotice {session, epoch, rateFloor}` (the lowest bit rate of
//!   the encoder of `epoch`), `{session, parameter, detail?}` (a parameter
//!   the encoder refused) or `{session, inputRefused}` (`gesture`, `text`,
//!   `action`, `timeout`, `button`, `secret`, `field`: an input Android did not perform): a notice for
//!   the viewer while encoder and session keep running
//!   (`HostStage::take_notice`, review 2026-11-19).
//!
//! Events: `screenHostStart {session, source, still, maxWidth, maxHeight,
//! viewerName, viewerDeviceId}`, `screenSession {session, state, reason}`.
use std::time::Duration;

use serde_json::{json, Value};

use super::args::{invalid, opt_bool, opt_i64, opt_str, str_arg};
use crate::mobile::{ApiError, Runtime};
use crate::screen::core::registry::StopSelector;
use crate::screen::core::session_state::EndReason;
use crate::screen::core::wire::MAX_CONTROL_BYTES;
use crate::screen::core::wire_control::ControlBlock;
use crate::screen::os::bridge_channels::{self as channels, Phase, Prereqs};

/// Longest wait of one `screenControl`.
const MAX_CONTROL_WAIT: Duration = Duration::from_secs(5);
/// Largest JSON form of the encoder name of `screen.hostReady` and of the
/// reason key of `screen.hostEnded`. Technical cause: both travel to the
/// viewer inside one control frame of at most `MAX_CONTROL_BYTES` (64 KiB;
/// the encoder in `STATE`, the reason as `ERROR` detail), so half the frame
/// leaves the other fields room. Longer texts are refused, never cut.
const MAX_WIRE_TEXT_BYTES: usize = MAX_CONTROL_BYTES / 2;

/// Whether `text` fits into a control frame next to the other fields.
fn fits_wire(text: &str) -> bool {
    serde_json::to_string(text).is_ok_and(|json| json.len() <= MAX_WIRE_TEXT_BYTES)
}

/// `screenPush` of the JNI bridge: one picture of session `session`.
pub fn screen_push(session: u64, data: &[u8], _pts_us: u64, flags: u32) -> i32 {
    channels::push(session, data, flags)
}
pub fn screen_push_admitted(
    session: u64,
    generation: u64,
    data: &[u8],
    _pts_us: u64,
    flags: u32,
) -> i32 {
    channels::push_admitted(session, generation, data, flags)
}
pub fn screen_capture_begin(session: u64, width: u32, height: u32, kind: u32) -> u64 {
    let generation = channels::channel(session)
        .and_then(|channel| {
            channel
                .begin_capture(
                    crate::screen::core::pixel_convert::FrameSize::new(width, height),
                    kind,
                )
                .ok()
        })
        .unwrap_or(0);
    #[cfg(all(target_os = "android", feature = "android-instrumentation-fixtures"))]
    if generation != 0 {
        crate::screen::os::android_capture_fixture::observed_begin(session, generation);
    }
    generation
}
pub fn screen_capture_claim(
    session: u64,
    generation: u64,
    owner: u32,
    bytes: u64,
    buffer: bool,
) -> u64 {
    channels::channel(session)
        .and_then(|channel| channel.capture.grant(generation).ok())
        .and_then(|grant| {
            crate::screen::os::android_capture_arena::claim(&grant, owner, bytes, buffer).ok()
        })
        .unwrap_or(0)
}
pub fn screen_capture_address(token: u64) -> Option<(usize, usize)> {
    crate::screen::os::android_capture_arena::address(token)
}
pub fn screen_capture_release(token: u64) {
    crate::screen::os::android_capture_arena::release(token);
}

/// `screenControl` of the JNI bridge: the next message for `session`
/// (0: input of the controlling viewer), or `None` after `timeout`.
pub fn screen_control(session: u64, timeout: Duration) -> Option<String> {
    channels::control(session, timeout.min(MAX_CONTROL_WAIT))
}

/// Handles `screen.host*`; `None` = not one of them.
pub(super) fn handle(method: &str, args: &Value) -> Option<Result<Value, ApiError>> {
    Some(match method {
        "screen.hostPrereqs" => prereqs(args),
        "screen.hostSessions" => Ok(sessions()),
        "screen.hostStop" => stop(args),
        "screen.hostReady" => ready(args),
        "screen.hostEnded" => ended(args),
        "screen.hostNotice" => notice(args),
        _ => return None,
    })
}

fn emit(event: Value) {
    if let Some(rt) = Runtime::installed() {
        rt.emit(event);
    }
}

fn session_arg(args: &Value) -> Result<u64, ApiError> {
    opt_i64(args, "session")
        .and_then(|id| u64::try_from(id).ok())
        .filter(|id| *id > 0)
        .ok_or_else(|| invalid("Parameter „session“ fehlt oder ist keine Sitzung."))
}

fn size_arg(args: &Value, key: &str) -> Result<u32, ApiError> {
    opt_i64(args, key)
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| (1..=16_384).contains(value))
        .ok_or_else(|| invalid(format!("Parameter „{key}“ fehlt oder ist ungültig.")))
}

pub(super) fn prereqs_json(prereqs: &Prereqs) -> Value {
    json!({
        "accessibility": if prereqs.accessibility.is_empty() { "off" } else { prereqs.accessibility.as_str() },
        "usageAccess": prereqs.usage_access,
        "appWindowCapture": prereqs.app_window_capture,
    })
}

fn prereqs(args: &Value) -> Result<Value, ApiError> {
    let working_bytes = match args.get("workingBytes") {
        None | Some(Value::Null) => None,
        Some(value) => {
            let bytes = value
                .as_u64()
                .ok_or_else(|| invalid("Ungültiges Bildspeicherbudget."))?;
            crate::screen::core::frame_budget::FrameLimits::RESOURCE
                .with_working_budget(bytes)
                .map_err(|error| invalid(error.to_string()))?;
            Some(bytes)
        }
    };
    let accessibility = match opt_str(args, "accessibility") {
        None => None,
        Some(state @ ("on" | "off" | "restricted")) => Some(state.to_string()),
        Some(_) => return Err(invalid("„accessibility“ ist on, off oder restricted.")),
    };
    let display = match args.get("display") {
        None | Some(Value::Null) => None,
        Some(display) => Some((size_arg(display, "width")?, size_arg(display, "height")?)),
    };
    // H.264, HEVC and AV1 encoders of MediaCodec (`screen_host_encoders.rs`).
    let encoders = args
        .get("encoders")
        .and_then(Value::as_array)
        .map(|list| encoders::encoders_arg(list));
    let apps = args.get("apps").and_then(Value::as_array).map(|list| {
        list.iter()
            .filter_map(|app| {
                let package = app.get("package").and_then(Value::as_str)?;
                let label = app.get("label").and_then(Value::as_str).unwrap_or(package);
                // Whole names (R15c): the JNI string has no length bound,
                // and the `WINDOWS` frame bounds itself (`window_list`).
                Some((package.to_string(), label.to_string()))
            })
            .collect::<Vec<(String, String)>>()
    });
    if let Some(list) = args.get("apps").and_then(Value::as_array) {
        let icons = list
            .iter()
            .take(128)
            .filter_map(|entry| {
                let package = entry.get("package")?.as_str()?;
                let icon: crate::screen::core::app_icon::IconRef =
                    serde_json::from_value(entry.get("icon")?.clone()).ok()?;
                if icon.app_id != package {
                    return None;
                }
                let png_base64 = entry.get("iconPngBase64")?.as_str()?;
                if png_base64.len() > crate::screen::core::app_icon::MAX_ICON_BASE64_BYTES {
                    return None;
                }
                Some(crate::screen::core::app_icon::IconReply {
                    icon,
                    png_base64: png_base64.into(),
                })
            })
            .collect();
        let _ = crate::screen::os::android_app_icons::replace_registered(icons);
    }
    if let Some(bytes) = working_bytes {
        crate::screen::os::image_memory::observe_capacity_ceiling(
            crate::screen::core::frame_memory::CapacityCeiling::bounded(bytes),
        )
        .map_err(|error| invalid(error.to_string()))?;
    }
    channels::set_event_sink(emit);
    let stored = channels::set_prereqs(|stored| {
        stored.registered = true;
        if let Some(bytes) = working_bytes {
            stored.working_bytes = Some(bytes);
        }
        if let Some(accessibility) = accessibility {
            stored.accessibility = accessibility;
        }
        if let Some(usage) = args.get("usageAccess").and_then(Value::as_bool) {
            stored.usage_access = usage;
        }
        if let Some(window) = args.get("appWindowCapture").and_then(Value::as_bool) {
            stored.app_window_capture = window;
        }
        if display.is_some() {
            stored.display = display;
        }
        if let Some(encoders) = encoders {
            // Other encoders than before: a refusal of the earlier ones no longer holds.
            if stored.encoders != encoders {
                stored.refused.clear();
            }
            stored.encoders = encoders;
        }
        if let Some(apps) = apps {
            stored.apps = apps;
        }
    });
    Ok(prereqs_json(&stored))
}

pub(super) fn sessions() -> Value {
    let list: Vec<Value> = channels::channels()
        .iter()
        .map(|channel| {
            let start = &channel.start;
            let (state, width, height, encoder, projection) = match channel.phase() {
                Phase::Consent => ("consent", 0, 0, String::new(), false),
                Phase::Ready {
                    width,
                    height,
                    encoder,
                    projection,
                    ..
                } => ("streaming", width, height, encoder, projection),
                Phase::Ended(_) => ("ended", 0, 0, String::new(), false),
            };
            json!({
                "session": channel.id,
                "source": if projection { "system-selected" } else { start.source.as_str() },
                "requestedSource": start.source,
                "still": start.still,
                "viewerName": start.viewer_name,
                "viewerDeviceId": start.viewer_device_id,
                "state": state,
                "width": width,
                "height": height,
                "encoder": encoder,
                "projection": projection,
            })
        })
        .collect();
    json!({ "sessions": list })
}

fn stop(args: &Value) -> Result<Value, ApiError> {
    let all = opt_str(args, "session") == Some("all") || opt_bool(args, "all", false);
    let targets: Vec<u64> = if all {
        channels::channels()
            .iter()
            .map(|channel| channel.id)
            .collect()
    } else {
        vec![session_arg(args)?]
    };
    let mut stopped = 0;
    for id in targets {
        if channels::ended(id, "host_stopped").is_ok() {
            stopped += 1;
        }
    }
    // The host user ends the sessions themselves too (C7: „Beenden“).
    if all {
        if let Some(registry) = channels::registry() {
            registry.stop(&StopSelector::All, EndReason::HostStopped);
        }
    }
    Ok(json!({ "stopped": stopped }))
}

fn ready(args: &Value) -> Result<Value, ApiError> {
    let id = session_arg(args)?;
    let encoder = str_arg(args, "encoder")?;
    if !fits_wire(encoder) {
        return Err(invalid(
            "„encoder“ ist länger, als ein Steuerrahmen (64 KiB) zum Betrachter trägt.",
        ));
    }
    let projection = opt_bool(args, "projection", false);
    let phase = Phase::Ready {
        width: size_arg(args, "width")?,
        height: size_arg(args, "height")?,
        encoder: encoder.to_string(),
        hardware: opt_bool(args, "hardware", false),
        projection,
    };
    channels::ready(id, phase).map_err(|message| ApiError::new("not_found", message))?;
    Ok(json!({"source": if projection { "system-selected" } else { "requested-source" }}))
}

fn ended(args: &Value) -> Result<Value, ApiError> {
    let id = session_arg(args)?;
    let reason = str_arg(args, "reason")?;
    if reason.is_empty()
        || !fits_wire(reason)
        || !reason
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
    {
        return Err(invalid(
            "„reason“ ist ein Schlüssel aus Kleinbuchstaben und _.",
        ));
    }
    channels::ended(id, reason).map_err(|message| ApiError::new("not_found", message))?;
    Ok(json!({}))
}

fn notice(args: &Value) -> Result<Value, ApiError> {
    let id = session_arg(args)?;
    let not_found = |message| ApiError::new("not_found", message);
    if let Some(floor) = args.get("rateFloor") {
        let bps = floor
            .as_u64()
            .filter(|bps| *bps > 0)
            .ok_or_else(|| invalid("„rateFloor“ ist eine Bitrate in bit/s."))?;
        let epoch = opt_i64(args, "epoch")
            .and_then(|epoch| u32::try_from(epoch).ok())
            .ok_or_else(|| invalid("Parameter „epoch“ fehlt oder ist ungültig."))?;
        channels::rate_floor(id, epoch, bps).map_err(not_found)?;
        return Ok(json!({}));
    }
    // Android did not perform an input of the session (Codex 2/19): a notice
    // for its viewer; the host session learns of input only when it queues it.
    if let Some(kind) = opt_str(args, "inputRefused") {
        let text = match kind {
            "gesture" => "the device cancelled a gesture of the input",
            "text" => "the device refused a text of the input (no focused field)",
            "action" => "the device refused a system action of the input",
            "timeout" => "the device did not answer an input in time",
            "button" => "the device has no counterpart for this mouse button (only left and back)",
            "field" => "the focused field did not take the text (it is read-only or refuses text)",
            "secret" => {
                "the device cannot see the content of this field (a password or web field); \
                 typing would replace it, so the text was not typed"
            }
            _ => {
                return Err(invalid(
                    "„inputRefused“ ist gesture, text, action, timeout, button, secret oder field.",
                ))
            }
        };
        channels::notice(id, text.into(), false).map_err(not_found)?;
        // The session reports it as a block too (CONTROL_STATE): a text or a
        // system action the device cannot take is not supported there, a
        // cancelled gesture or a late answer leaves the control unavailable.
        // A mouse button without counterpart or a field whose content is
        // hidden blocks nothing (the control works on): the notice alone
        // says it (round 10, B2).
        let block = match kind {
            "button" | "secret" | "field" => None,
            "text" | "action" => Some(ControlBlock::UnsupportedOnHost),
            _ => Some(ControlBlock::ControlUnavailable),
        };
        if let Some(block) = block {
            channels::input_refused(id, block).map_err(not_found)?;
        }
        return Ok(json!({}));
    }
    let name = str_arg(args, "parameter")?;
    let detail = opt_str(args, "detail").unwrap_or_default();
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(invalid(
            "„parameter“ ist ein Parametername aus Buchstaben, Ziffern, - und _.",
        ));
    }
    let text = if detail.is_empty() {
        format!("the encoder refused the parameter {name}")
    } else {
        format!("the encoder refused the parameter {name}: {detail}")
    };
    // The notice travels as a `STATE` reason: too long a text is refused, never cut.
    if !fits_wire(&text) {
        return Err(invalid("„detail“ ist zu lang."));
    }
    // `video-bitrate` is `MediaCodec.PARAMETER_KEY_VIDEO_BITRATE`.
    channels::notice(id, text, name == "video-bitrate").map_err(not_found)?;
    Ok(json!({}))
}

#[path = "screen_host_encoders.rs"]
mod encoders;

#[cfg(test)]
#[path = "screen_host_tests.rs"]
mod tests;
