//! FC1 policy edits via the shared persisted boundary. This facade adds no
//! transport, denial store, profile fallback, or automatic authorization.
use serde_json::{json, Value};

use super::args::{invalid, opt_str, str_arg};
use super::share_state::{committed, default_home, reconfigure, wake};
use crate::mobile::{ApiError, Runtime};
use crate::share::{DirectPeerIdentity, ExportAccess, ShareProfiles};

fn error(message: String) -> ApiError { ApiError::new("invalid", message) }
fn profiles() -> Result<ShareProfiles, ApiError> {
    ShareProfiles::load_checked(default_home()).map_err(error)
}
fn optional_bool(args: &Value, name: &str) -> Result<Option<bool>, ApiError> {
    args.get(name).map(|value| value.as_bool().ok_or_else(|| ApiError::new("invalid", format!("{name} muss bool sein.")))).transpose()
}
fn bool_arg(args: &Value, name: &str) -> Result<bool, ApiError> {
    optional_bool(args, name)?.ok_or_else(|| ApiError::new("invalid", format!("{name} fehlt.")))
}
fn access(args: &Value) -> Result<Option<ExportAccess>, ApiError> {
    match args.get("access") {
        None => Ok(None),
        Some(Value::String(value)) if value == "read_only" => Ok(Some(ExportAccess::ReadOnly)),
        Some(Value::String(value)) if value == "read_write" => Ok(Some(ExportAccess::ReadWrite)),
        _ => Err(invalid("access muss read_only oder read_write sein.")),
    }
}
fn finish(rt: &Runtime, profiles: ShareProfiles, changed: bool) -> Value {
    committed(profiles);
    if changed { reconfigure(rt); } else { wake(); }
    json!({"persisted": true, "changed": changed})
}

pub(super) fn set_export(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let scope = str_arg(args, "scope")?;
    let path = str_arg(args, "path")?;
    let access = access(args)?.ok_or_else(|| invalid("access fehlt."))?;
    let system = optional_bool(args, "allowSystemWrites")?;
    let mut changed = false;
    let profiles = ShareProfiles::mutate_persisted(default_home(), |profiles| {
        changed = profiles.export_config_mut(scope)?.set_root_access(path, access, system)?;
        Ok(())
    }).map_err(error)?;
    Ok(finish(rt, profiles, changed))
}

pub(super) fn connections(args: &Value) -> Result<Value, ApiError> {
    let scope = str_arg(args, "scope")?;
    let mut profiles = profiles()?;
    let config = profiles.export_config_mut(scope).map_err(error)?;
    let saved = crate::creds::load_connections_checked().map_err(error)?;
    let connections = saved.iter().map(|saved| {
        let account = saved.account();
        let access = config.connection_access(&account);
        json!({"account": account, "label": saved.display(), "shared": access.is_some(), "access": access})
    }).collect::<Vec<_>>();
    Ok(json!({
        "connections": connections, "sharedConnections": config.shared_connections,
        "warning": "Freigegebene Verbindungen nutzen deine gespeicherten Zugangsdaten.",
    }))
}

pub(super) fn set_connection(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let scope = str_arg(args, "scope")?;
    let account = str_arg(args, "account")?;
    let shared = bool_arg(args, "shared")?;
    let requested = access(args)?;
    if !shared && requested.is_some() {
        return Err(invalid("Beim Entfernen wird kein access angegeben."));
    }
    if shared && !crate::creds::load_connections_checked().map_err(error)?.iter().any(|saved| saved.account() == account) {
        return Err(invalid("Gespeicherte Verbindung nicht gefunden; bitte neu laden."));
    }
    let mut changed = false;
    let profiles = ShareProfiles::mutate_persisted(default_home(), |profiles| {
        let config = profiles.export_config_mut(scope)?;
        let access = if shared { requested.or(config.connection_access(account)).or(Some(ExportAccess::ReadOnly)) } else { None };
        changed = config.set_connection_access(account, access)?;
        Ok(())
    }).map_err(error)?;
    Ok(finish(rt, profiles, changed))
}

pub(super) fn set_contact_write(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let peer = DirectPeerIdentity {
        device_id: str_arg(args, "deviceId")?.into(),
        device_name: opt_str(args, "name").unwrap_or("").into(),
        public_key: str_arg(args, "publicKey")?.into(),
        node_id: args.get("nodeId").and_then(Value::as_str)
            .ok_or_else(|| invalid("nodeId muss als String mitgegeben werden (Legacy darf leer sein)."))?.into(),
        fingerprint: str_arg(args, "fingerprint")?.into(),
    };
    let change = crate::share::set_direct_peer_write(default_home(), &peer, bool_arg(args, "write")?).map_err(error)?;
    Ok(finish(rt, change.profiles, change.changed))
}

pub(super) fn set_share_back(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let change = crate::share::set_direct_share_back(default_home(), str_arg(args, "contactId")?,
        bool_arg(args, "shareBack")?).map_err(error)?;
    Ok(finish(rt, change.profiles, change.changed))
}

pub(super) fn set_room_policy(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let profile_id = str_arg(args, "profileId")?;
    let room_id = str_arg(args, "roomId")?;
    let write = optional_bool(args, "membersMayWrite")?;
    let confirm = optional_bool(args, "confirmNewMembers")?;
    if write.is_none() && confirm.is_none() { return Err(invalid("Keine Raumpolicy angegeben.")); }
    let mut changed = false;
    let profiles = ShareProfiles::mutate_persisted(default_home(), |profiles| {
        if !profiles.rooms.iter().any(|room| room.id == profile_id && room.room_id == room_id) {
            return Err("Raumidentitaet wurde geaendert; bitte neu laden".into());
        }
        changed = profiles.set_room_policy(profile_id, write, confirm)?;
        Ok(())
    }).map_err(error)?;
    Ok(finish(rt, profiles, changed))
}

pub(super) fn policy() -> Result<Value, ApiError> {
    let result = crate::share::DirectRequestPolicy::load();
    let warning = result.as_ref().err().cloned();
    let requests = result.unwrap_or_default();
    Ok(json!({"requests": requests, "warning": warning}))
}
