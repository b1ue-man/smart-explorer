//! Explicit pinned machine targets; this domain never mutates normal device grants.
use std::{collections::BTreeMap, sync::{Arc, Mutex, atomic::{AtomicU64, Ordering}}};
use serde_json::{json, Value};
use crate::{login::{FrameToken, GatewayTarget, ViewerSession}, mobile::ApiError, screen::core::input_model::{InputEvent, key_usage, MAX_INPUT_EVENTS}};
static SESSIONS: Mutex<BTreeMap<u64, Arc<ViewerSession>>> = Mutex::new(BTreeMap::new());
static NEXT: AtomicU64 = AtomicU64::new(1);
#[cfg(all(target_os = "android", feature = "android-instrumentation-fixtures"))]
pub(super) fn fixture_session_id() -> Result<u64, String> {
    NEXT.fetch_update(Ordering::SeqCst, Ordering::SeqCst,
        |id| id.checked_add(1).filter(|next| *next <= i64::MAX as u64))
        .map_err(|_| "login viewer IDs exhausted".into())
}
fn invalid(reason: impl ToString) -> ApiError { ApiError::new("invalid", reason.to_string()) }
fn session(args: &Value) -> Result<Arc<ViewerSession>, ApiError> {
    let id = args.get("session").and_then(Value::as_u64).filter(|id| *id != 0).ok_or_else(|| invalid("login session is required"))?;
    SESSIONS.lock().map_err(|_| ApiError::internal("login viewer registry is unavailable"))?.get(&id).cloned()
        .ok_or_else(|| ApiError::new("not_found", "login viewer session ended"))
}
fn token(args: &Value) -> Result<FrameToken, ApiError> {
    serde_json::from_value(args.get("token").cloned().ok_or_else(|| invalid("drawn login frame token is required"))?).map_err(invalid)
}
fn epoch(args: &Value) -> Result<u64, ApiError> { args.get("presentation_epoch").and_then(Value::as_u64).ok_or_else(|| invalid("actual presentation epoch is required")) }
fn target(value: Value) -> Result<GatewayTarget, ApiError> {
    if serde_json::to_vec(&value).map_err(invalid)?.len() > crate::login::core::gateway_types::MAX_NETWORK { return Err(invalid("public login target exceeds 32 KiB")); }
    let target: GatewayTarget = serde_json::from_value(value).map_err(invalid)?; target.validate().map_err(invalid)?; Ok(target)
}
pub(super) fn handle(method: &str, args: &Value) -> Option<Result<Value, ApiError>> {
    Some((|| match method {
        "login.target" => Ok(json!({"target":target(args.get("target").cloned().ok_or_else(|| invalid("explicit public machine target is required"))?)?,"pinning_required":true})),
        "login.status" => Ok(json!({"installation":crate::login::installation_status()})),
        "login.viewOpen" => {
            if args.get("acknowledge_machine_pin").and_then(Value::as_bool) != Some(true) { return Err(invalid("explicit administrative machine pin acknowledgement is required")); }
            let target = target(args.get("target").cloned().ok_or_else(|| invalid("explicit public machine target is required"))?)?;
            let user = args.get("target_user").and_then(Value::as_str).filter(|user| !user.is_empty() && user.len() <= 256).ok_or_else(|| invalid("explicit granted target SID/UID is required"))?;
            let id = NEXT.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |id| id.checked_add(1).filter(|next| *next <= i64::MAX as u64)).map_err(|_| invalid("login viewer IDs exhausted"))?;
            let mut sessions = SESSIONS.lock().map_err(|_| ApiError::internal("login viewer registry is unavailable"))?;
            sessions.retain(|_, session| session.snapshot().state != "ended");
            if sessions.len() >= 64 { return Err(ApiError::new("busy", "login viewer session admission is full")); }
            let owner = args.get("image_owner").and_then(Value::as_u64).ok_or_else(|| invalid("stable login image owner is required"))?;
            let heap = args.get("heap_offer").and_then(Value::as_u64).ok_or_else(|| invalid("Android heap offer is required"))?;
            let images = super::login_images::bind(owner, id, heap).map_err(invalid)?;
            let session = match ViewerSession::start_with_images(target, user.into(), true, images) {
                Ok(session) => session, Err(error) => {super::login_images::unbind(id); return Err(invalid(error));}
            }; sessions.insert(id, session);
            Ok(json!({"session":id}))
        }
        "login.viewStatus" => Ok(json!(session(args)?.snapshot())),
        "login.viewVisible" => { let active = session(args)?; if args.get("visible").and_then(Value::as_bool) == Some(true) { active.visible(); } else { active.close(); } Ok(json!({})) },
        "login.viewFocus" => { let active = session(args)?; active.focus(args.get("focused").and_then(Value::as_bool) == Some(true), args.get("control").and_then(Value::as_bool) == Some(true)); Ok(json!(active.snapshot())) },
        "login.viewPresented" => Ok(json!({"accepted":session(args)?.presented(&token(args)?, epoch(args)?)})),
        "login.viewInput" => {
            let values = args.get("events").and_then(Value::as_array).filter(|events| !events.is_empty() && events.len() <= MAX_INPUT_EVENTS).ok_or_else(|| invalid("login input needs 1..256 events"))?;
            let events = values.iter().map(|value| {
                let mut value = value.clone();
                if let Some(name) = value.get("key").and_then(Value::as_str) { let usage = key_usage(name).ok_or_else(|| invalid("unknown login key name"))?;
                    if let Some(map) = value.as_object_mut() { map.remove("key"); map.insert("hid_usage".into(), json!(usage)); } }
                serde_json::from_value::<InputEvent>(value).map_err(invalid)
            }).collect::<Result<Vec<_>,_>>()?;
            session(args)?.input(token(args)?, epoch(args)?, events).map_err(|error| ApiError::new("input_blocked", error))?;
            Ok(json!({"queued":values.len(),"retried":false}))
        }
        "login.viewSas" => { session(args)?.secure_attention(token(args)?, epoch(args)?).map_err(|error| ApiError::new("policy_blocked", error))?; Ok(json!({"queued":true})) },
        "login.viewRelease" => { session(args)?.release(); Ok(json!({})) },
        "login.viewClose" => { if let Some(id) = args.get("session").and_then(Value::as_u64) { login_close(id); } Ok(json!({})) },
        _ => return Err(invalid("unknown login viewer method")),
    })())
}
/// Whole immutable frame transfer; callers must pre-admit the bitmap destination.
pub(super) fn take_image_frame(id: u64) -> Option<crate::login::ViewerFrame> {
    #[cfg(all(target_os = "android", feature = "android-instrumentation-fixtures"))]
    if let Some(frame) = super::login_image_fixture::take(id) {return frame;}
    let session = SESSIONS.lock().ok()?.get(&id).cloned()?;
    session.take_frame()
}
pub fn login_close(id: u64) {
    #[cfg(all(target_os = "android", feature = "android-instrumentation-fixtures"))]
    super::login_image_fixture::close(id);
    super::login_images::unbind(id);
    if let Some(session) = SESSIONS.lock().ok().and_then(|mut sessions| sessions.remove(&id)) { session.close(); }
}
pub fn login_release(id: u64) {
    if let Some(session) = SESSIONS.lock().ok().and_then(|sessions| sessions.get(&id).cloned()) { session.release(); }
}
