//! This phone as screen host (SCR-M8): the bridge methods of the app's
//! host side and the two binary calls of the JNI bridge (`screenPush`,
//! `screenControl`, `native/android-bridge/src/screen_jni.rs`).
//!
//! - `screen.hostPrereqs {accessibility?, usageAccess?, appWindowCapture?,
//!   display?, encoders?, apps?}` registers the app as host and stores what
//!   it reports; answers `{"accessibility":"on|off|restricted",
//!   "usageAccess":bool,"appWindowCapture":bool}`.
//! - `screen.hostSessions` → `{"sessions":[…]}`.
//! - `screen.hostStop {session|all}` ends sessions as the host user.
//! - `screen.hostReady {session, width, height, encoder, hardware?,
//!   projection?}` after the system dialog or for still previews.
//! - `screen.hostEnded {session, reason}`.
//!
//! Events: `screenHostStart {session, source, still, maxWidth, maxHeight,
//! viewerName, viewerDeviceId}`, `screenSession {session, state, reason}`.
use std::time::Duration;

use serde_json::{json, Value};

use super::args::{invalid, opt_bool, opt_i64, opt_str, str_arg};
use crate::mobile::{ApiError, Runtime};
use crate::screen::core::registry::StopSelector;
use crate::screen::core::session_state::EndReason;
use crate::screen::os::bridge_channels::{self as channels, Phase, Prereqs};

/// Longest wait of one `screenControl`.
const MAX_CONTROL_WAIT: Duration = Duration::from_secs(5);
/// Longest reason key of `screen.hostEnded`.
const MAX_REASON: usize = 64;

/// `screenPush` of the JNI bridge: one picture of session `session`.
pub fn screen_push(session: u64, data: &[u8], _pts_us: u64, flags: u32) -> i32 {
    channels::push(session, data, flags)
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
    let accessibility = match opt_str(args, "accessibility") {
        None => None,
        Some(state @ ("on" | "off" | "restricted")) => Some(state.to_string()),
        Some(_) => return Err(invalid("„accessibility“ ist on, off oder restricted.")),
    };
    let display = match args.get("display") {
        None | Some(Value::Null) => None,
        Some(display) => Some((size_arg(display, "width")?, size_arg(display, "height")?)),
    };
    let encoders = match args.get("encoders").and_then(Value::as_array) {
        None => None,
        Some(list) => Some(
            list.iter()
                .filter_map(|entry| {
                    let profile = entry.get("profile").and_then(Value::as_str)?;
                    matches!(profile, "cb" | "high")
                        .then(|| (profile.to_string(), opt_bool(entry, "hardware", false)))
                })
                .collect::<Vec<_>>(),
        ),
    };
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
    channels::set_event_sink(emit);
    let stored = channels::set_prereqs(|stored| {
        stored.registered = true;
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
                "source": start.source,
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
    let phase = Phase::Ready {
        width: size_arg(args, "width")?,
        height: size_arg(args, "height")?,
        encoder: str_arg(args, "encoder")?.chars().take(64).collect(),
        hardware: opt_bool(args, "hardware", false),
        projection: opt_bool(args, "projection", false),
    };
    channels::ready(id, phase).map_err(|message| ApiError::new("not_found", message))?;
    Ok(json!({}))
}

fn ended(args: &Value) -> Result<Value, ApiError> {
    let id = session_arg(args)?;
    let reason = str_arg(args, "reason")?;
    if reason.is_empty()
        || reason.len() > MAX_REASON
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

#[cfg(test)]
#[path = "screen_host_tests.rs"]
mod tests;
