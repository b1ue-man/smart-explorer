//! Android reads the embedded worker's real snapshot through the daemon IPC.
use serde_json::{json, Value};
use crate::mobile::ApiError;

/// `share.routeStatus {contactId? | deviceId, publicKey}`. No route is probed or
/// authorized by this read. An empty selector returns the complete bounded view.
pub(super) fn status(args: &Value) -> Result<Value, ApiError> {
    let contact = optional_text(args, "contactId")?;
    let device = optional_text(args, "deviceId")?;
    let key = optional_text(args, "publicKey")?;
    if contact.is_some() && (device.is_some() || key.is_some()) {
        return Err(ApiError::new("invalid", "contactId oder deviceId mit publicKey angeben."));
    }
    if device.is_some() != key.is_some() {
        return Err(ApiError::new("invalid", "deviceId und publicKey müssen gemeinsam angegeben werden."));
    }
    let snapshot = crate::daemon::share_worker_snapshot()
        .map_err(|error| ApiError::new("unavailable", format!("Routediagnose: Share-Worker nicht erreichbar: {error}")))?;
    let Some(mut routes) = snapshot.routes else {
        return Ok(json!({ "available": false, "reason": "Der Worker liefert keine Routediagnose", "peers": [] }));
    };
    routes.peers.retain(|peer| match contact {
        Some(contact) => peer.contact_id.as_deref() == Some(contact),
        None => device.is_none_or(|device| peer.device_id == device && key == Some(peer.public_key.as_str())),
    });
    Ok(json!({ "available": true, "peers": routes.peers, "error": routes.error,
        "truncated": routes.truncated, "measurementNote": crate::share::ROUTE_PAYLOAD_MEASUREMENT_NOTE }))
}

fn optional_text<'a>(args: &'a Value, key: &str) -> Result<Option<&'a str>, ApiError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(Some(value)),
        _ => Err(ApiError::new("invalid", format!("{key} muss nichtleerer Text sein."))),
    }
}
