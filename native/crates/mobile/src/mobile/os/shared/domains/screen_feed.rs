//! `screen.feed {deviceId, publicKey}` (design 5.2, user rule r5 „wenn n
//! fenster offen ist sehen es andere se desktops“): the background
//! service's device feed of one device for the app's device dialogs, while
//! the app process runs. `families` keeps the native family spelling of
//! `screen.families`, so the app opens an entry exactly as before; the other
//! keys are camelCase. `control`: the device lets this one steer (it sends
//! services and the RDP state only then).
use serde_json::{json, Value};

use super::super::args::{invalid, str_arg};
use crate::mobile::ApiError;
use crate::share::peer_select::present_relation;

/// No feed for the device: it is absent or older than 0.5.189.
pub(crate) const NO_FEED: &str =
    "Das Gerät ist nicht anwesend oder hat eine ältere Version; seine Fenster erscheinen, sobald es erreichbar ist.";

pub(super) fn device(args: &Value) -> Result<Value, ApiError> {
    let device = str_arg(args, "deviceId")?;
    let key = str_arg(args, "publicKey")?;
    if device.is_empty() || key.is_empty() {
        return Err(invalid("Die Geräteidentität ist unvollständig."));
    }
    let snapshot = crate::daemon::share_worker_snapshot().map_err(ApiError::internal)?;
    let present = present_relation(&snapshot.profiles, device, key).is_some();
    let Some(feed) = snapshot
        .peer_feeds
        .iter()
        .find(|feed| feed.device_id == device && feed.public_key == key)
    else {
        return Ok(json!({"known": false, "present": present, "unavailable": NO_FEED}));
    };
    let rdp = feed.rdp.as_ref().map(|rdp| {
        json!({
            "startable": rdp.startable,
            "setup": rdp.setup,
            "setupText": rdp.setup.text(),
            "setupPossible": rdp.setup.setup_possible(),
            "remoteApps": rdp.remote_apps.iter().map(|app| json!({
                "alias": app.alias,
                "displayName": app.display_name,
                "collection": app.collection,
            })).collect::<Vec<_>>(),
        })
    });
    Ok(json!({
        "known": feed.unavailable.is_none(),
        "present": present,
        "families": feed.families,
        "familiesUnavailable": feed.families_unavailable,
        "services": feed.services,
        "servicesUnavailable": feed.services_unavailable,
        "rdp": rdp,
        "unavailable": feed.unavailable,
        "control": feed.rdp.is_some() || !feed.services.is_empty(),
    }))
}
