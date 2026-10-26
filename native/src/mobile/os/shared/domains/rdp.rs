//! Bridge RDP adapter. Android reports its absent host/client-verification
//! capability explicitly; it never substitutes an unverified app URI.
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
fn open(
    args: &Value,
    probe: &mut impl FnMut(PeerOpenTarget) -> Result<AuthenticatedProbe, String>,
) -> Result<Value, ApiError> {
    if crate::rdp::local_clients()
        .iter()
        .all(|client| !client.available)
    {
        return Err(invalid("No installed local RDP client exposes a documented certificate-verification and credential-entry adapter in this app context; no TCP port was opened"));
    }
    let target = target(args)?;
    let report = probe(target.clone()).map_err(invalid)?;
    let policy = match opt_str(args, "certificatePolicy").unwrap_or("exact_peer_pin") {
        "exact_peer_pin" => crate::rdp::CertificatePolicy::ExactPeerPin,
        "peer_pin_or_native_trust" => crate::rdp::CertificatePolicy::PeerPinOrNativeTrust,
        "native_dialog" => crate::rdp::CertificatePolicy::NativeDialog,
        _ => return Err(invalid("Invalid RDP certificatePolicy")),
    };
    let flag = |name: &str| -> Result<bool, ApiError> {
        match args.get(name) {
            None | Some(Value::Null) => Ok(false),
            Some(Value::Bool(flag)) => Ok(*flag),
            _ => Err(invalid(format!("{name} must be boolean"))),
        }
    };
    let seconds = args.get("lifetimeSecs").map_or(Ok(900), |value| {
        value
            .as_u64()
            .ok_or_else(|| invalid("lifetimeSecs must be an integer"))
    })?;
    let request = crate::rdp::StartRequest {
        service_id: str_arg(args, "service")?.into(),
        client_id: str_arg(args, "client")?.into(),
        certificate_policy: policy,
        remote_app: opt_str(args, "remoteApp").map(str::to_string),
        acknowledge_local_access: flag("acknowledgeLocalAccess")?,
        acknowledge_session_effect: flag("acknowledgeSessionEffect")?,
        acknowledge_certificate_dialog: flag("acknowledgeCertificateDialog")?,
        lifetime_secs: u32::try_from(seconds)
            .map_err(|_| invalid("lifetimeSecs must be 1..86400"))?,
    };
    let session = crate::rdp::start(
        target,
        &report,
        &request,
        std::time::Duration::from_secs(30),
        || {},
    )
    .map_err(invalid)?;
    as_json(json!({"session":crate::rdp::own_in_background(session).map_err(invalid)?}))
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
