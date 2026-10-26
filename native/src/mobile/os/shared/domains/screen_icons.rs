//! Bounded pending icon calls for one authenticated viewer session. No image bridge events.
use crate::mobile::ApiError;
use crate::screen::core::app_icon::{IconReply, IconRequest, IconResponse};
use crate::screen::core::icon_cache::{IconCache, IconCacheKey};
use crate::screen::core::wire_control::ViewerFrame;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{mpsc, Mutex, PoisonError};
use std::time::Duration;

#[derive(Default)]
struct State {
    cache: IconCache,
    pending: HashMap<IconRequest, mpsc::Sender<Option<IconReply>>>,
}
#[derive(Default)]
pub(super) struct IconState(Mutex<State>);

impl IconState {
    pub(super) fn deliver(&self, peer: &str, response: IconResponse) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(sender) = state.pending.remove(&response.request) else {
            return;
        };
        let content = if response.validate().is_ok() {
            response.content.filter(|reply| {
                state
                    .cache
                    .insert(
                        IconCacheKey {
                            peer: peer.into(),
                            icon: response.request.icon.clone(),
                        },
                        reply,
                    )
                    .is_ok()
            })
        } else {
            None
        };
        let _ = sender.send(content);
    }
    pub(super) fn clear(&self) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pending
            .clear();
    }
}

pub(super) fn parse(args: &Value) -> Result<IconRequest, ApiError> {
    let value = args
        .get("request")
        .cloned()
        .ok_or_else(|| ApiError::new("invalid", "Iconanfrage fehlt"))?;
    if value.to_string().len() > crate::screen::core::app_icon::MAX_ICON_REQUEST_BYTES {
        return Err(ApiError::new(
            "invalid",
            "Iconanfrage überschreitet ihr Bytebudget",
        ));
    }
    let request: IconRequest = serde_json::from_value(value)
        .map_err(|error| ApiError::new("invalid", error.to_string()))?;
    request
        .validate()
        .map_err(|error| ApiError::new("invalid", error.to_string()))?;
    Ok(request)
}

pub(super) fn request(session: &super::run::ViewSession, args: &Value) -> Result<Value, ApiError> {
    if !session
        .icons_supported
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        return Ok(Value::Null);
    }
    let request = parse(args)?;
    let peer = format!("session:{}", session.id);
    let (sender, receiver) = mpsc::channel();
    {
        let mut state = session
            .icons
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let key = IconCacheKey {
            peer: peer.clone(),
            icon: request.icon.clone(),
        };
        if let Some(bytes) = state.cache.get(&key) {
            return Ok(json!(IconReply {
                icon: request.icon,
                png_base64: STANDARD.encode(bytes)
            }));
        }
        if state.pending.len() >= 8 || state.pending.contains_key(&request) {
            return Ok(Value::Null);
        }
        state.pending.insert(request.clone(), sender);
    }
    if session
        .send(&ViewerFrame::IconRequest(request.clone()))
        .is_err()
    {
        session
            .icons
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pending
            .remove(&request);
        return Ok(Value::Null);
    }
    let reply = receiver
        .recv_timeout(Duration::from_secs(30))
        .ok()
        .flatten();
    session
        .icons
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .pending
        .remove(&request);
    Ok(serde_json::to_value(reply).unwrap_or(Value::Null))
}
