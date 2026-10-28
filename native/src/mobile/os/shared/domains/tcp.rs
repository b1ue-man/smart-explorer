//! TCP controls use the platform-common daemon client. On Android that
//! client starts/uses the embedded worker, never a desktop process. A
//! further forwarding needs no acknowledgement and has no fixed lifetime
//! unless `lifetimeSecs` names one (design 4.4/4.5, user rule r7); the
//! services a device shares under steering are listed, opened and closed
//! on tap (`tcp_services.rs`, design 4.3).
use super::{
    args::{invalid, str_arg},
    share_state,
};
use crate::mobile::ApiError;
use crate::share::peer_select::present_relation;
use crate::share::tcp_forward::{TcpDestination, TcpStart, TunnelId};
use crate::share::{PeerOpenTarget, ShareProfiles};
use serde::Serialize;
use serde_json::{json, Value};

#[path = "tcp_services.rs"]
mod services;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Target {
    relation: String,
    relation_id: String,
    device_id: String,
    public_key: String,
    node_id: String,
    name: String,
    relation_label: String,
    can_start: bool,
    /// The relation se uses for this device now (design 2.2): present, a
    /// Direct contact before a room membership.
    present: bool,
}
fn targets() -> Result<Vec<Target>, ApiError> {
    let profiles =
        ShareProfiles::load_checked(share_state::default_home()).map_err(ApiError::internal)?;
    let mut targets = Vec::new();
    let chosen = |device: &str, key: &str| present_relation(&profiles, device, key);
    for contact in &profiles.direct_contacts {
        let (Some(device), Some(key)) = (&contact.remote_device_id, &contact.remote_public_key)
        else {
            continue;
        };
        let node = if contact.expected_node_id.is_empty() {
            contact
                .presence
                .as_ref()
                .map(|p| p.node_id.clone())
                .unwrap_or_default()
        } else {
            contact.expected_node_id.clone()
        };
        targets.push(Target {
            relation: "direct".into(),
            relation_id: contact.id.clone(),
            device_id: device.clone(),
            public_key: key.clone(),
            node_id: node,
            name: contact.display_name.clone(),
            relation_label: "Direktverbindung".into(),
            can_start: true,
            present: chosen(device, key)
                == Some(PeerOpenTarget::Direct {
                    contact_id: contact.id.clone(),
                }),
        });
    }
    for room in &profiles.rooms {
        for member in &room.members {
            if member.blocked {
                continue;
            }
            targets.push(Target {
                relation: "room".into(),
                relation_id: room.room_id.clone(),
                device_id: member.device_id.clone(),
                public_key: member.public_key.clone(),
                node_id: member.node_id.clone(),
                name: member.device_name.clone(),
                relation_label: format!("Raum „{}“", room.name),
                can_start: true,
                present: chosen(&member.device_id, &member.public_key)
                    == Some(PeerOpenTarget::RoomDevice {
                        room_id: room.room_id.clone(),
                        device_id: member.device_id.clone(),
                    }),
            });
        }
    }
    for grant in &profiles.direct_grants {
        if targets.iter().any(|target| {
            target.device_id == grant.device_id && target.public_key == grant.public_key
        }) {
            continue;
        }
        targets.push(Target {
            relation: "hosted".into(),
            relation_id: String::new(),
            device_id: grant.device_id.clone(),
            public_key: grant.public_key.clone(),
            node_id: grant.node_id.clone(),
            name: grant.device_name.clone(),
            relation_label: "Nur eingehender Zugriff; keine ausgehende Verbindung gespeichert"
                .into(),
            can_start: false,
            present: false,
        });
    }
    Ok(targets)
}
pub(super) fn handle(method: &str, args: &Value) -> Option<Result<Value, ApiError>> {
    Some(match method {
        "tcp.devices" => targets().map(|devices| json!({"devices": devices})),
        "tcp.start" => start(args),
        "tcp.services" => services::list(args),
        "tcp.openService" => services::open(args),
        "tcp.closeService" => services::close(args),
        "tcp.list" => crate::daemon::tcp_list()
            .map(|tunnels| json!({"tunnels": tunnels}))
            .map_err(ApiError::internal),
        "tcp.status" => status(args),
        "tcp.close" => close(args),
        _ => return None,
    })
}
fn number(args: &Value, key: &str, default: Option<u64>) -> Result<u64, ApiError> {
    match args.get(key) {
        Some(value) => value
            .as_u64()
            .ok_or_else(|| invalid(format!("{key} muss eine nichtnegative ganze Zahl sein."))),
        None => default.ok_or_else(|| invalid(format!("{key} fehlt."))),
    }
}
fn start(args: &Value) -> Result<Value, ApiError> {
    let relation = str_arg(args, "relation")?;
    let id = str_arg(args, "relationId")?;
    let device = str_arg(args, "deviceId")?;
    let key = str_arg(args, "publicKey")?;
    let node = str_arg(args, "nodeId")?;
    let target = targets()?
        .into_iter()
        .find(|target| {
            target.relation == relation
                && target.relation_id == id
                && target.device_id == device
                && target.public_key == key
                && target.node_id == node
        })
        .ok_or_else(|| invalid("Gerät oder Beziehung geändert. Die Geräteseite neu laden."))?;
    if !target.can_start || node.is_empty() {
        return Err(invalid(
            "Eine bestätigte ausgehende Verbindung zu diesem Gerät fehlt.",
        ));
    }
    let peer = if target.relation == "direct" {
        PeerOpenTarget::Direct {
            contact_id: target.relation_id,
        }
    } else {
        PeerOpenTarget::RoomDevice {
            room_id: target.relation_id,
            device_id: target.device_id,
        }
    };
    let request = start_request(args)?;
    let tunnel =
        crate::daemon::tcp_start_with(peer, request, bridge_admission(), None, None, || {})
            .map_err(ApiError::internal)?;
    Ok(json!({"tunnel": tunnel}))
}

/// The origin rule of the bridge's listeners (design 4.4): Android does not
/// reveal which process connects, so every local app may use a forwarding
/// opened by a tap there (`AnyLocal`, which the daemon admits only on
/// Android). Host test builds of the bridge keep the default rule.
fn bridge_admission() -> crate::share::tcp_forward::ListenerAdmission {
    if cfg!(target_os = "android") {
        crate::share::tcp_forward::ListenerAdmission::AnyLocal
    } else {
        crate::share::tcp_forward::ListenerAdmission::SameAccount
    }
}
/// The request of `tcp.start`: no acknowledgement argument needed (the
/// listener admits only this app's account) and no lifetime unless
/// `lifetimeSecs` names one; `acknowledgeLocalAccess` is ignored.
pub(super) fn start_request(args: &Value) -> Result<TcpStart, ApiError> {
    let port = u16::try_from(number(args, "targetPort", None)?)
        .map_err(|_| invalid("Zielport muss 1–65535 sein."))?;
    let host = match args.get("targetHost") {
        None => "loopback",
        Some(value) => value
            .as_str()
            .ok_or_else(|| invalid("targetHost muss ein Hostname oder eine IP als Text sein."))?,
    };
    let request = TcpStart {
        destination: TcpDestination::from_host_port(host, port).map_err(invalid)?,
        local_port: u16::try_from(number(args, "localPort", Some(0))?)
            .map_err(|_| invalid("Lokaler Port muss 0–65535 sein."))?,
        lifetime_secs: u32::try_from(number(args, "lifetimeSecs", Some(0))?)
            .map_err(|_| invalid("Lebensdauer muss 0 (ohne Frist) bis 86400 Sekunden sein."))?,
        acknowledge_local_access: true,
    };
    request.validate().map_err(invalid)?;
    Ok(request)
}
fn status(args: &Value) -> Result<Value, ApiError> {
    let id = TunnelId::parse(str_arg(args, "id")?).map_err(invalid)?;
    let tunnel = crate::daemon::tcp_status(id)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("Diese Weiterleitung ist beendet."))?;
    Ok(json!({"tunnel": tunnel}))
}
fn close(args: &Value) -> Result<Value, ApiError> {
    let id = TunnelId::parse(str_arg(args, "id")?).map_err(invalid)?;
    let closed = crate::daemon::tcp_close(id).map_err(ApiError::internal)?;
    if !closed {
        return Err(ApiError::not_found(
            "Diese Weiterleitung ist bereits beendet.",
        ));
    }
    Ok(json!({"closed": true}))
}

#[cfg(test)]
#[path = "einrichtung_bridge_task_tests.rs"]
mod einrichtung_bridge_task_tests;
