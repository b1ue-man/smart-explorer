//! Android remote commands use the actual embedded worker's daemon IPC.
//! Local hostAdmission is a separate JNI-only one-use platform boundary.
use crate::mobile::ApiError;
use crate::share::{apps_wire::AppsCommand, PeerOpenTarget};
use serde_json::{json, Value};
pub(super) fn handle(method: &str, args: &Value) -> Option<Result<Value, ApiError>> {
    if method == "apps.localGrants" {
        return Some((|| {
            let field = |key| {
                args.get(key)
                    .and_then(Value::as_str)
                    .ok_or_else(|| ApiError::new("invalid", format!("Missing {key}")))
            };
            let snapshot =
                crate::daemon::apps_local_grants(field("deviceId")?, field("publicKey")?)
                    .map_err(ApiError::internal)?;
            serde_json::to_value(snapshot).map_err(|e| ApiError::internal(e.to_string()))
        })());
    }
    if method == "apps.setLocalGrant" {
        return Some((|| {
            let change = serde_json::from_value(args.get("change").cloned().unwrap_or(Value::Null))
                .map_err(|e| ApiError::new("invalid", e.to_string()))?;
            let result = crate::daemon::apps_set_local_grant(change).map_err(ApiError::internal)?;
            serde_json::to_value(result).map_err(|e| ApiError::internal(e.to_string()))
        })());
    }
    if method == "apps.hostAdmission" {
        return Some(host_admission(args));
    }
    if method == "apps.hostIconResult" {
        return Some(host_icon_result(args));
    }
    let command = match method {
        "apps.catalog" => Ok(AppsCommand::Catalog),
        "apps.rdpProbe" => Ok(AppsCommand::RdpProbe),
        "apps.icon" => (|| {
            Ok(AppsCommand::Icon {
                app_id: serde_json::from_value(args.get("appId").cloned().unwrap_or(Value::Null))
                    .map_err(|e| ApiError::new("invalid", e.to_string()))?,
                icon: serde_json::from_value(args.get("icon").cloned().unwrap_or(Value::Null))
                    .map_err(|e| ApiError::new("invalid", e.to_string()))?,
                catalog_revision: args
                    .get("catalogRevision")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ApiError::new("invalid", "Missing catalogRevision"))?
                    .into(),
            })
        })(),
        "apps.launch" | "apps.receipt" => {
            serde_json::from_value(args.get("request").cloned().unwrap_or(Value::Null))
                .map(|request| {
                    if method == "apps.launch" {
                        AppsCommand::Launch { request }
                    } else {
                        AppsCommand::Receipt { request }
                    }
                })
                .map_err(|e| ApiError::new("invalid", e.to_string()))
        }
        _ => return None,
    };
    Some((|| {
        let target: PeerOpenTarget =
            serde_json::from_value(args.get("target").cloned().unwrap_or(Value::Null))
                .map_err(|e| ApiError::new("invalid", e.to_string()))?;
        verify_ui_pin(&target, args)?;
        let wait = args
            .get("waitSeconds")
            .map(|n| {
                n.as_u64()
                    .filter(|n| *n > 0)
                    .ok_or_else(|| ApiError::new("invalid", "waitSeconds must be positive"))
            })
            .transpose()?
            .unwrap_or(30);
        let (node, device, result) = crate::daemon::apps_command(
            target,
            command?,
            Some(std::time::Duration::from_secs(wait)),
            || {},
        )
        .map_err(ApiError::internal)?;
        Ok(json!({"nodeId":node,"deviceId":device,"result":result}))
    })())
}
/// The picker keeps the exact device/key selected from the local relation list.
/// A changed contact/member must not silently retarget a still-open dialog.
fn verify_ui_pin(target: &PeerOpenTarget, args: &Value) -> Result<(), ApiError> {
    let field = |key| {
        args.get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::new("invalid", format!("Missing {key}")))
    };
    let device = field("expectedDeviceId")?;
    let key = field("expectedPublicKey")?;
    let node = field("expectedNodeId")?;
    let profiles = crate::share::ShareProfiles::load_checked(super::share_state::default_home())
        .map_err(ApiError::internal)?;
    let matches = match target {
        PeerOpenTarget::Direct { contact_id } => profiles.direct_contacts.iter().any(|c| {
            c.id == *contact_id
                && c.remote_device_id.as_deref() == Some(device)
                && c.remote_public_key.as_deref() == Some(key)
                && (node.is_empty()
                    || if c.expected_node_id.is_empty() {
                        c.presence.as_ref().is_some_and(|p| p.node_id == node)
                    } else {
                        c.expected_node_id == node
                    })
        }),
        PeerOpenTarget::RoomDevice { room_id, device_id } => {
            device_id == device
                && profiles.rooms.iter().any(|r| {
                    r.room_id == *room_id
                        && r.members.iter().any(|m| {
                            !m.blocked
                                && m.device_id == device
                                && m.public_key == key
                                && (node.is_empty() || m.node_id == node)
                        })
                })
        }
    };
    if !matches {
        return Err(ApiError::new(
            "identity_changed",
            "Gerät oder Beziehung geändert. Programmdialog schließen und Geräteliste neu laden.",
        ));
    }
    Ok(())
}
fn host_admission(args: &Value) -> Result<Value, ApiError> {
    let text = |key| {
        args.get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::new("invalid", format!("Missing {key}")))
    };
    crate::apps::os::host::claim_local_admission(text("dispatch_id")?,text("request_id")?)
        .map(|(end,valid_for)|json!({"allowed":true,"expires_at_unix_ms":end,"valid_for_ms":valid_for}))
        .map_err(|e|ApiError::new(if e.kind()==std::io::ErrorKind::Unsupported {"unsupported"} else {"denied"},e.to_string()))
}
fn host_icon_result(args: &Value) -> Result<Value, ApiError> {
    let challenge = args
        .get("challenge")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::new("invalid", "Missing icon challenge"))?;
    let reply = serde_json::from_value(args.get("reply").cloned().unwrap_or(Value::Null))
        .map_err(|e| ApiError::new("invalid", e.to_string()))?;
    crate::apps::os::host::finish_local_icon(challenge, reply)
        .map_err(|e| ApiError::new("app_icon", e.to_string()))?;
    Ok(json!({}))
}
