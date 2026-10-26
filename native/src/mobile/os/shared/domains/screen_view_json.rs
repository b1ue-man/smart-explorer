//! The JSON shapes of the viewer bridge (SCR-M9, U28-3) that need no running
//! session: the camelCase conversion of core structures, the event envelope,
//! the device list from the stored profiles and the last probe results.
use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::screen::core::wire_control::HostStats;
use crate::screen::core::wire_hello::{ScreenErrorCode, ScreenHelloOk};
use crate::share::access_grant::CapabilityDecision;
use crate::share::{device_view, ShareProfiles};

/// What the last `screen.probe` of a device found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ProbeFacts {
    pub supported: bool,
    pub view: CapabilityDecision,
    pub control: CapabilityDecision,
}

/// The key of a device in the probe results.
pub(super) fn probe_key(relation: &str, relation_id: &str, device_id: &str) -> String {
    format!("{relation}/{relation_id}/{device_id}")
}

fn camel_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    let mut upper = false;
    for ch in key.chars() {
        if ch == '_' {
            upper = true;
        } else if upper {
            out.extend(ch.to_uppercase());
            upper = false;
        } else {
            out.push(ch);
        }
    }
    out
}

/// The keys of every object in `value` in camelCase (the bridge's spelling);
/// values stay as they are.
pub(super) fn camel(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| {
                    // Icon references are opaque signed/hash-bound native DTOs.
                    let value = if key == "icon" { value } else { camel(value) };
                    (camel_key(&key), value)
                })
                .collect(),
        ),
        Value::Array(list) => Value::Array(list.into_iter().map(camel).collect()),
        other => other,
    }
}

/// `{"type":"screenView","session","kind", …fields}`.
pub(super) fn event_value(session: u64, kind: &str, fields: Value) -> Value {
    let mut event = json!({ "type": "screenView", "session": session, "kind": kind });
    if let (Some(map), Value::Object(extra)) = (event.as_object_mut(), fields) {
        map.extend(extra);
    }
    event
}

/// The fields of the `accepted` event.
pub(super) fn accepted_fields(hello: &ScreenHelloOk, path: &str) -> Value {
    let mut fields = camel(serde_json::to_value(hello).unwrap_or(Value::Null));
    if let Some(map) = fields.as_object_mut() {
        map.insert("path".into(), json!(path));
        let limits=hello.frame_contract.map_or(crate::screen::core::frame_budget::FrameLimits::LEGACY,|c|c.limits);
        if let Ok(budget)=crate::screen::core::frame_size_caps::PacketBudget::for_contract(hello.frame_contract) {
            map.insert("bufferBudget".into(),json!({"workingBytes":limits.working_bytes,
                "imageBytes":budget.raw_bytes-8,"recordBytes":budget.record_bytes,
                "packetBytes":budget.encoded_bytes,"pullBytes":budget.pull_bytes,
                "recordOverheadBytes":crate::screen::core::frame_size_caps::RECORD_BUFFER_OVERHEAD,
                "legacy":hello.frame_contract.is_none()}));
        }
    }
    fields
}

/// The fields of the `stats` event; the relay reports no round trip, so
/// `rttMs` is `null`.
pub(super) fn stats_fields(stats: &HostStats) -> Value {
    let mut fields = camel(serde_json::to_value(stats).unwrap_or(Value::Null));
    if let Some(map) = fields.as_object_mut() {
        map.insert("rttMs".into(), Value::Null);
    }
    fields
}

/// Code and text of a failed open: the worker leads a screen refusal with its
/// code (`permission_denied: …`); anything else is a connection failure.
pub(super) fn split_refusal(message: &str) -> (String, String) {
    match message.split_once(": ") {
        Some((code, text)) if ScreenErrorCode::from_key(code).is_some() => {
            (code.to_string(), text.to_string())
        }
        _ => ("connection_failed".to_string(), message.to_string()),
    }
}

/// `screen.devices`: the Direct devices with a known device ID and the
/// admitted members of the joined rooms (other than this phone); `room`
/// limits it to one room by profile ID, room ID or name.
pub(super) fn devices_json(
    profiles: &ShareProfiles,
    room: Option<&str>,
    own: &str,
    now: i64,
    probes: &BTreeMap<String, ProbeFacts>,
) -> Value {
    let row = |relation: &str,
               relation_id: &str,
               device_id: &str,
               name: &str,
               room: Option<(&str, &str)>,
               online: bool| {
        let facts = probes.get(&probe_key(relation, relation_id, device_id));
        let known = facts.filter(|facts| facts.supported);
        json!({
            "relation": relation,
            "relationId": relation_id,
            "deviceId": device_id,
            "name": name,
            "roomId": room.map(|room| room.0),
            "roomName": room.map(|room| room.1),
            "online": online,
            "supported": facts.map(|facts| facts.supported),
            "view": known.map(|facts| facts.view.key()),
            "control": known.map(|facts| facts.control.key()),
        })
    };
    let mut devices = Vec::new();
    if room.is_none() {
        for contact in &profiles.direct_contacts {
            if let Some(device_id) = contact.remote_device_id.as_deref() {
                let online = device_view::online(contact, now);
                devices.push(row(
                    "direct",
                    &contact.id,
                    device_id,
                    &contact.display_name,
                    None,
                    online,
                ));
            }
        }
    }
    for joined in profiles.rooms.iter().filter(|joined| joined.auto_join) {
        let wanted = room.is_none_or(|wanted| {
            joined.id == wanted || joined.room_id == wanted || joined.name == wanted
        });
        if !wanted {
            continue;
        }
        for member in joined
            .members
            .iter()
            .filter(|member| member.is_admitted() && member.device_id != own)
        {
            devices.push(row(
                "room",
                &joined.id,
                &member.device_id,
                &member.device_name,
                Some((&joined.id, &joined.name)),
                device_view::reachable(&member.status),
            ));
        }
    }
    json!({ "devices": devices })
}
