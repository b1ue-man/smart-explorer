//! `tcp.services`, `tcp.openService` and `tcp.closeService` (design 4.3,
//! user rule r5 „wenn auf localhost was offen ist wird das frei gegeben für
//! andere geräte“): the locally listening services a device shares with
//! this one under „Bildschirm steuern: Erlauben“, from the background
//! service's device feed, each with its local access. Android cannot tell
//! which app opens a local connection (design 4.1, assumption A7), so a
//! service's listener exists only after a tap on „Öffnen“ and until
//! „Schließen“, the device going offline or the grant ending.
use super::super::args::{invalid, str_arg};
use crate::mobile::ApiError;
use serde_json::{json, Value};

/// No feed for the device: it is absent or older than 0.5.189.
pub(crate) const NO_FEED: &str =
    "Das Gerät ist nicht anwesend oder hat eine ältere Version; seine Dienste erscheinen, sobald es erreichbar ist.";

/// The device identity of a call.
fn identity(args: &Value) -> Result<(&str, &str), ApiError> {
    let device = str_arg(args, "deviceId")?;
    let key = str_arg(args, "publicKey")?;
    if device.is_empty() || key.is_empty() {
        return Err(invalid("Die Geräteidentität ist unvollständig."));
    }
    Ok((device, key))
}

/// The port argument of `openService`/`closeService`.
pub(crate) fn port_arg(args: &Value) -> Result<u16, ApiError> {
    args.get("port")
        .and_then(Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())
        .filter(|port| *port > 0)
        .ok_or_else(|| invalid("port muss 1–65535 sein."))
}

/// `{"services":[{port,scope,owner,process,pid,mapping}],"unavailable",
/// "control"}`; `mapping` is the local access of that port (address, link,
/// port note, error) while one exists.
pub(super) fn list(args: &Value) -> Result<Value, ApiError> {
    let (device, key) = identity(args)?;
    let snapshot = crate::daemon::share_worker_snapshot().map_err(ApiError::internal)?;
    let Some(feed) = snapshot
        .peer_feeds
        .iter()
        .find(|feed| feed.device_id == device && feed.public_key == key)
    else {
        return Ok(json!({"services": [], "unavailable": NO_FEED, "control": false}));
    };
    let rows: Vec<Value> = feed
        .services
        .iter()
        .map(|service| {
            let mapping = feed
                .mapped
                .iter()
                .find(|mapping| mapping.host_port == service.port && mapping.address.is_some());
            let mut row = serde_json::to_value(service).unwrap_or(Value::Null);
            if let Value::Object(map) = &mut row {
                map.insert("mapping".into(), json!(mapping));
            }
            row
        })
        .collect();
    let unavailable = feed
        .unavailable
        .clone()
        .or_else(|| feed.services_unavailable.clone());
    let control = feed.rdp.is_some() || !feed.services.is_empty();
    Ok(json!({"services": rows, "unavailable": unavailable, "control": control}))
}

/// Opens the local access to one shared service at the device's own
/// loopback address, the same port where free (design 4.3); every app of
/// this phone may use it (design 4.4 `AnyLocal`, Android only).
pub(super) fn open(args: &Value) -> Result<Value, ApiError> {
    let (device, key) = identity(args)?;
    let port = port_arg(args)?;
    let mapping = crate::share::auto_forward::open(device, key, port).map_err(invalid)?;
    Ok(json!({ "mapping": mapping }))
}

/// Ends the local access of one service.
pub(super) fn close(args: &Value) -> Result<Value, ApiError> {
    let (device, key) = identity(args)?;
    let port = port_arg(args)?;
    let closed = crate::share::auto_forward::close(device, key, port);
    Ok(json!({ "closed": closed }))
}
