//! Bridge of the compact connect and share flow (plan 2026-10-31 §F,
//! F1/F4/F5/F6; contract in `docs/superpowers/plans/2026-09-25-android-apk/
//! api.md` §„Rechte beim Verbinden“): presets, open confirmations
//! („Rechte offen“), „Bestätigen“, „Abbrechen“ and the Direct code's
//! rights. The same core functions as the CLI and the daemon IPC; one
//! profile transaction each, then the worker reloads.
use serde_json::{json, Map, Value};

use super::args::{invalid, opt_bool};
use super::share_state::{default_home, reconfigure, wake};
use crate::mobile::{ApiError, Runtime};
use crate::share::access_grant::{AccountProfile, CapabilityDecision};
use crate::share::connection_rights::{
    ConnectionRights, FolderChoice, PresetView, RightsPreset, RightsTarget,
};
use crate::share::connection_rights_actions as actions;
use crate::share::connection_rights_apply::PendingRightsView;
use crate::share::ShareProfiles;

fn error(message: String) -> ApiError {
    ApiError::new("invalid", message)
}

fn load() -> Result<ShareProfiles, ApiError> {
    ShareProfiles::load_checked(default_home()).map_err(|error| ApiError::new("internal", error))
}

fn account_key(account: &AccountProfile) -> String {
    match account {
        AccountProfile::Own => "own".into(),
        AccountProfile::Remote => "remote".into(),
        AccountProfile::Account { name } => format!("account:{name}"),
    }
}

/// The bridge form of rights (camelCase), with `preset` and `summary`.
pub(super) fn rights_json(rights: &ConnectionRights) -> Value {
    let folders = match &rights.folders {
        FolderChoice::None => json!({"kind": "none"}),
        FolderChoice::All => json!({"kind": "all"}),
        FolderChoice::Selected { paths, connections } => {
            json!({"kind": "selected", "paths": paths, "connections": connections})
        }
    };
    json!({
        "folders": folders,
        "write": rights.write,
        "commands": rights.commands.key(),
        "terminal": rights.terminal.key(),
        "screenView": rights.screen_view.key(),
        "screenControl": rights.screen_control.key(),
        "validUntil": rights.valid_until,
        "account": account_key(&rights.account),
        "preset": rights.preset().map(RightsPreset::key),
        "summary": rights.summary(),
    })
}

fn decision(value: &Value, key: &str) -> Result<Option<CapabilityDecision>, ApiError> {
    match value.get(key).and_then(Value::as_str) {
        None => Ok(None),
        Some(text) => CapabilityDecision::from_key(text)
            .map(Some)
            .ok_or_else(|| invalid(format!("{key} muss off, ask oder allow sein."))),
    }
}

fn folders_of(value: &Value) -> Result<Option<FolderChoice>, ApiError> {
    let Some(folders) = value.get("folders") else {
        return Ok(None);
    };
    let paths = folders
        .get("paths")
        .and_then(Value::as_array)
        .map(|paths| {
            paths
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Ok(Some(match folders.get("kind").and_then(Value::as_str) {
        Some("none") => FolderChoice::None,
        Some("all") => FolderChoice::All,
        Some("selected") => FolderChoice::Selected {
            paths,
            connections: opt_bool(folders, "connections", false),
        },
        _ => return Err(invalid("folders.kind muss none, all oder selected sein.")),
    }))
}

fn account_of(text: &str) -> Result<AccountProfile, ApiError> {
    let account = match (text, text.strip_prefix("account:")) {
        ("own", _) => AccountProfile::Own,
        ("remote", _) => AccountProfile::Remote,
        (_, Some(name)) => AccountProfile::Account { name: name.into() },
        _ => return Err(invalid("account muss own, remote oder account:NAME sein.")),
    };
    account.validate().map_err(error)?;
    Ok(account)
}

/// The rights of `args`: `preset` (key of `share.rightsPresets`), else
/// `base`, else „Keine Rechte“, changed by every field of `rights`.
/// `None` when the call names neither.
pub(super) fn rights_arg(
    args: &Value,
    base: Option<ConnectionRights>,
) -> Result<Option<ConnectionRights>, ApiError> {
    let preset = args.get("preset").and_then(Value::as_str);
    let fields = args.get("rights").filter(|value| !value.is_null());
    if preset.is_none() && fields.is_none() {
        return Ok(None);
    }
    let mut rights = match preset {
        Some(key) => RightsPreset::from_key(key)
            .ok_or_else(|| invalid(format!("Unbekannte Voreinstellung: {key}")))?
            .rights(),
        None => base.unwrap_or_else(ConnectionRights::none),
    };
    if let Some(fields) = fields {
        if let Some(folders) = folders_of(fields)? {
            rights.folders = folders;
        }
        if let Some(write) = fields.get("write").and_then(Value::as_bool) {
            rights.write = write;
        }
        for (key, slot) in [
            ("commands", &mut rights.commands),
            ("terminal", &mut rights.terminal),
            ("screenView", &mut rights.screen_view),
            ("screenControl", &mut rights.screen_control),
        ] {
            if let Some(chosen) = decision(fields, key)? {
                *slot = chosen;
            }
        }
        match fields.get("validUntil") {
            Some(Value::Null) => rights.valid_until = None,
            Some(value) => {
                rights.valid_until = Some(
                    value
                        .as_i64()
                        .ok_or_else(|| invalid("validUntil muss eine Unix-Zeit sein."))?,
                )
            }
            None => {}
        }
        if let Some(account) = fields.get("account").and_then(Value::as_str) {
            rights.account = account_of(account)?;
        }
    }
    rights.validate().map_err(error)?;
    Ok(Some(rights))
}

fn target_json(target: &RightsTarget) -> Value {
    match target {
        RightsTarget::Direct {
            device_id,
            public_key,
        } => json!({"kind": "direct", "deviceId": device_id, "publicKey": public_key}),
        RightsTarget::Room { profile_id } => json!({"kind": "room", "profileId": profile_id}),
    }
}

fn target_arg(args: &Value) -> Result<RightsTarget, ApiError> {
    let target = args.get("target").ok_or_else(|| invalid("target fehlt."))?;
    let text = |key: &str| {
        target
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| invalid(format!("target.{key} fehlt.")))
    };
    match target.get("kind").and_then(Value::as_str) {
        Some("direct") => Ok(RightsTarget::Direct {
            device_id: text("deviceId")?,
            public_key: text("publicKey")?,
        }),
        Some("room") => Ok(RightsTarget::Room {
            profile_id: text("profileId")?,
        }),
        _ => Err(invalid("target.kind muss direct oder room sein.")),
    }
}

fn pending_json(view: &PendingRightsView) -> Value {
    json!({
        "target": target_json(&view.target),
        "origin": view.origin.key(),
        "since": view.since,
        "name": view.name,
        "rights": rights_json(&view.rights),
    })
}

/// `pendingRights` and `codeRights` of `share.status`.
pub(super) fn add_status(status: &mut Value, profiles: &ShareProfiles) {
    let pending = profiles.pending_rights_views();
    status["pendingRights"] = Value::Array(pending.iter().map(pending_json).collect());
    status["codeRights"] = json!(profiles.code_rights.as_ref().map(rights_json));
}

fn preset_json(view: &PresetView) -> Value {
    json!({
        "key": view.key,
        "label": view.label,
        "description": view.description,
        "rights": rights_json(&view.rights),
    })
}

/// `share.rightsPresets`.
pub(super) fn presets() -> Result<Value, ApiError> {
    let presets = crate::share::connection_rights::preset_views();
    Ok(json!({"presets": presets.iter().map(preset_json).collect::<Vec<_>>()}))
}

/// `share.pendingRights`.
pub(super) fn pending() -> Result<Value, ApiError> {
    let mut value = Value::Object(Map::new());
    add_status(&mut value, &load()?);
    Ok(value)
}

fn current(profiles: &ShareProfiles, target: &RightsTarget) -> Option<ConnectionRights> {
    match target {
        RightsTarget::Direct {
            device_id,
            public_key,
        } => profiles.current_direct_rights(device_id, public_key),
        RightsTarget::Room { profile_id } => profiles.current_room_rights(profile_id),
    }
}

fn finish(rt: &Runtime, profiles: &ShareProfiles, changed: bool) -> Value {
    reconfigure(rt);
    wake();
    let mut value = json!({"changed": changed, "persisted": true});
    add_status(&mut value, profiles);
    value
}

/// `share.confirmRights`: exactly the rights of `preset`/`rights` (the
/// current ones where `rights` names nothing) for `target`.
pub(super) fn confirm(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let target = target_arg(args)?;
    let profiles = load()?;
    let rights = rights_arg(args, current(&profiles, &target))?
        .ok_or_else(|| invalid("preset oder rights fehlt."))?;
    let change = actions::confirm_connection_rights(default_home(), &target, &rights)
        .map_err(|message| ApiError::new("conflict", message))?;
    Ok(finish(rt, &change.profiles, change.changed))
}

/// `share.cancelRights`: the connection stays without rights.
pub(super) fn cancel(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let target = target_arg(args)?;
    let change = actions::cancel_connection_rights(default_home(), &target)
        .map_err(|message| ApiError::new("conflict", message))?;
    Ok(finish(rt, &change.profiles, change.changed))
}

/// `share.setCodeRights`: `clear: true` removes them, else `preset`/`rights`
/// on top of the current code rights.
pub(super) fn set_code_rights(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let rights = if opt_bool(args, "clear", false) {
        None
    } else {
        Some(
            rights_arg(args, load()?.code_rights)?
                .ok_or_else(|| invalid("preset, rights oder clear fehlt."))?,
        )
    };
    let change = actions::set_code_rights(default_home(), rights).map_err(error)?;
    Ok(finish(rt, &change.profiles, change.changed))
}
