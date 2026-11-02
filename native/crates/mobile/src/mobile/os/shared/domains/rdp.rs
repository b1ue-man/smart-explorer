//! Bridge RDP adapter. Android reports its absent host/client-verification
//! capability explicitly; it never substitutes an unverified app URI. On a
//! platform with an RDP program `rdp.open` needs only the device (design
//! 2.9, user rule r3).
use super::args::{invalid, opt_str, str_arg};
use crate::{
    mobile::ApiError,
    rdp::AuthenticatedProbe,
    share::{tcp_forward::TunnelId, PeerOpenTarget},
};
use serde_json::{json, Value};

pub(super) fn handle(
    method: &str,
    args: &Value,
    mut probe: impl FnMut(PeerOpenTarget) -> Result<AuthenticatedProbe, String>,
) -> Option<Result<Value, ApiError>> {
    Some(match method {
        "rdp.localProbe" => as_json(crate::rdp::probe_local_host()),
        "rdp.clients" => {
            let clients = crate::rdp::local_clients();
            let reason = clients.iter().all(|client| !client.available).then_some(
                "No installed client has a documented certificate-verification and credential-entry adapter in this app context; no unverified client deep link is started");
            Ok(camel(json!({"clients":clients,"unavailableReason":reason})))
        }
        "rdp.probe" => target(args)
            .and_then(|target| probe(target).map_err(invalid))
            .and_then(|probe| as_json(probe.report())),
        "rdp.open" => open(args, &mut probe),
        "rdp.list" => crate::daemon::tcp_list()
            .map(|tunnels| camel(json!({"tunnels":tunnels})))
            .map_err(invalid),
        "rdp.status" => str_arg(args, "id")
            .and_then(|id| TunnelId::parse(id).map_err(invalid))
            .and_then(|id| crate::daemon::tcp_status(id).map_err(invalid))
            .map(|tunnel| camel(json!({"tunnel":tunnel}))),
        "rdp.close" => str_arg(args, "id")
            .and_then(|id| TunnelId::parse(id).map_err(invalid))
            .and_then(|id| crate::rdp::close(id).map_err(invalid))
            .map(|closed| json!({"closed":closed})),
        _ => return None,
    })
}
fn target(args: &Value) -> Result<PeerOpenTarget, ApiError> {
    let relation = str_arg(args, "relation")?;
    let id = str_arg(args, "relationId")?;
    if id.is_empty() {
        return Err(invalid("RDP relationId is empty"));
    }
    // Selector facts are checked against current persisted profiles. They are
    // never imported as trusted host capability/certificate evidence.
    let device = str_arg(args, "deviceId")?;
    let key = str_arg(args, "publicKey")?;
    let node = str_arg(args, "nodeId")?;
    if device.is_empty() || key.is_empty() || node.is_empty() {
        return Err(invalid(
            "Die gepinnte RDP-Geräteidentität ist unvollständig.",
        ));
    }
    let profiles = crate::share::ShareProfiles::load_checked(super::share_state::default_home())
        .map_err(ApiError::internal)?;
    match relation {
        "direct" => {
            let contact = profiles
                .direct_contacts
                .iter()
                .find(|contact| contact.id == id)
                .ok_or_else(|| invalid("Der gespeicherte RDP-Kontakt ist nicht mehr vorhanden."))?;
            let current_node = if contact.expected_node_id.is_empty() {
                contact
                    .presence
                    .as_ref()
                    .map(|p| p.node_id.as_str())
                    .unwrap_or("")
            } else {
                &contact.expected_node_id
            };
            if contact.remote_device_id.as_deref() != Some(device)
                || contact.remote_public_key.as_deref() != Some(key)
                || current_node != node
            {
                return Err(invalid(
                    "Die RDP-Geräteidentität hat sich geändert. Gerätedaten neu laden.",
                ));
            }
            Ok(PeerOpenTarget::Direct {
                contact_id: id.into(),
            })
        }
        "room" => {
            let room = profiles
                .rooms
                .iter()
                .find(|room| room.room_id == id || room.id == id)
                .ok_or_else(|| invalid("Der RDP-Raum ist nicht mehr gespeichert."))?;
            if !room.members.iter().any(|member| {
                member.device_id == device
                    && member.public_key == key
                    && member.node_id == node
                    && !member.blocked
            }) {
                return Err(invalid("Die RDP-Raumidentität hat sich geändert oder ist gesperrt. Gerätedaten neu laden."));
            }
            Ok(PeerOpenTarget::RoomDevice {
                room_id: room.room_id.clone(),
                device_id: device.into(),
            })
        }
        _ => Err(invalid("RDP relation must be direct or room")),
    }
}
/// The Bridge refusal without a usable local RDP program (Android has none,
/// design 2.4); no TCP port is opened and no app link is started.
pub(crate) const NO_CLIENT: &str = "No installed local RDP client exposes a documented certificate-verification and credential-entry adapter in this app context; no TCP port was opened";

/// `rdp.open`: only `deviceId`/`publicKey` (and an optional `remoteApp`).
/// Relation, probe, host setup, service, client, trust and the session's
/// life are derived (design 2.9); older arguments are ignored.
fn open(
    args: &Value,
    _probe: &mut impl FnMut(PeerOpenTarget) -> Result<AuthenticatedProbe, String>,
) -> Result<Value, ApiError> {
    let clients = crate::rdp::local_clients();
    open_with(args, &clients, crate::rdp::open_device)
}

/// `rdp.open` with the local clients and the opener injected (tests).
pub(crate) fn open_with(
    args: &Value,
    clients: &[crate::rdp::core::ClientCapability],
    open: impl FnOnce(&str, &str, Option<&str>) -> Result<crate::rdp::SessionView, String>,
) -> Result<Value, ApiError> {
    if clients.iter().all(|client| !client.available) {
        return Err(invalid(NO_CLIENT));
    }
    let device = str_arg(args, "deviceId")?;
    let key = str_arg(args, "publicKey")?;
    if device.is_empty() || key.is_empty() {
        return Err(invalid(
            "Die gepinnte RDP-Geräteidentität ist unvollständig.",
        ));
    }
    let session = open(device, key, opt_str(args, "remoteApp")).map_err(invalid)?;
    as_json(json!({ "session": session }))
}
fn as_json(value: impl serde::Serialize) -> Result<Value, ApiError> {
    serde_json::to_value(value)
        .map(camel)
        .map_err(|e| invalid(e.to_string()))
}
fn camel(value: Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .map(|(key, value)| {
                    let mut result = String::new();
                    let mut upper = false;
                    for c in key.chars() {
                        if c == '_' {
                            upper = true;
                        } else if upper {
                            result.extend(c.to_uppercase());
                            upper = false;
                        } else {
                            result.push(c);
                        }
                    }
                    (result, camel(value))
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(camel).collect()),
        value => value,
    }
}
