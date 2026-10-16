//! Share settings the desktop and the CLI change and the Android app now
//! changes the same way (api.md §5): per contact „Automatisch verbinden/
//! öffnen“ and „Vertrauen zurücksetzen“, room name and rejoining, a member's
//! cached presence, the order of exports, whether the direct code is
//! reachable, a new direct code, and finding paired devices on the LAN.
//! Each edit is one profile transaction of the shared core
//! (`ShareProfiles::{set_contact_options, reset_contact_trust, rename_room,
//! set_room_auto_join, reset_member_trust, move_export_root}`) followed by a
//! worker reload.
use serde_json::{json, Value};

use super::args::{bool_arg, i64_arg, invalid, str_arg};
use super::share_state::{committed, default_home, reconfigure, wake, with_state};
use crate::mobile::{ApiError, Runtime};
use crate::share::{ShareCmd, ShareProfiles};

/// One profile transaction; reloads the worker when it changed anything.
fn persist(
    rt: &Runtime,
    mut edit: impl FnMut(&mut ShareProfiles) -> Result<bool, String>,
) -> Result<Value, ApiError> {
    let mut changed = false;
    let profiles = ShareProfiles::mutate_persisted(default_home(), |profiles| {
        changed = edit(profiles)?;
        Ok(())
    })
    .map_err(|error| ApiError::new("invalid", error))?;
    committed(profiles);
    if changed {
        reconfigure(rt);
    } else {
        wake();
    }
    Ok(json!({ "persisted": true, "changed": changed }))
}

fn optional_bool(args: &Value, key: &str) -> Result<Option<bool>, ApiError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_bool()
            .map(Some)
            .ok_or_else(|| invalid(format!("{key} muss bool sein."))),
    }
}

/// `autoConnect`/`autoOpen` of `share.setContactOptions`; at least one.
pub(super) fn contact_options(args: &Value) -> Result<(Option<bool>, Option<bool>), ApiError> {
    let auto_connect = optional_bool(args, "autoConnect")?;
    let auto_open = optional_bool(args, "autoOpen")?;
    if auto_connect.is_none() && auto_open.is_none() {
        return Err(invalid("autoConnect oder autoOpen fehlt."));
    }
    Ok((auto_connect, auto_open))
}

/// `offset` of `share.moveExport`: one place up (`-1`) or down (`1`).
pub(super) fn export_offset(args: &Value) -> Result<isize, ApiError> {
    match i64_arg(args, "offset")? {
        -1 => Ok(-1),
        1 => Ok(1),
        _ => Err(invalid("offset muss -1 oder 1 sein.")),
    }
}

/// `share.setContactOptions {contactId, autoConnect?, autoOpen?}`.
pub(super) fn set_contact_options(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let contact_id = str_arg(args, "contactId")?;
    let (auto_connect, auto_open) = contact_options(args)?;
    persist(rt, |profiles| {
        profiles.set_contact_options(contact_id, auto_connect, auto_open)
    })
}

/// `share.resetContactTrust {contactId}`: forgets the pinned identity and
/// withdraws its authorization (Exec off); the contact stays.
pub(super) fn reset_contact_trust(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let contact_id = str_arg(args, "contactId")?;
    let now = crate::share::core_now_secs();
    persist(rt, |profiles| profiles.reset_contact_trust(contact_id, now))
}

/// `share.renameRoom {profileId, name}` (empty = „Raum“).
pub(super) fn rename_room(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let profile_id = str_arg(args, "profileId")?;
    let name = str_arg(args, "name")?;
    persist(rt, |profiles| profiles.rename_room(profile_id, name))
}

/// `share.rejoinRoom {profileId}`: a left room is joined automatically again.
pub(super) fn rejoin_room(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let profile_id = str_arg(args, "profileId")?;
    persist(rt, |profiles| profiles.set_room_auto_join(profile_id, true))
}

/// `share.resetMemberTrust {profileId, deviceId}`.
pub(super) fn reset_member_trust(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let profile_id = str_arg(args, "profileId")?;
    let device_id = str_arg(args, "deviceId")?;
    persist(rt, |profiles| {
        profiles.reset_member_trust(profile_id, device_id)
    })
}

/// `share.moveExport {scope, path, offset}`: `-1` one place up, `1` down.
pub(super) fn move_export(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let scope = str_arg(args, "scope")?;
    let path = str_arg(args, "path")?;
    let offset = export_offset(args)?;
    persist(rt, |profiles| {
        profiles.move_export_root(scope, path, offset)
    })
}

/// `share.setDirectOnline {online}`: whether the direct code is reachable for
/// new contacts until the worker restarts (desktop „Online/Offline
/// schalten“); paired devices and rooms stay connected.
pub(super) fn set_direct_online(args: &Value) -> Result<Value, ApiError> {
    let online = bool_arg(args, "online")?;
    crate::daemon::send_share_command(ShareCmd::SetDirectOnline { online }).map_err(|error| {
        ApiError::new(
            "network",
            format!("Direct-Code-Erreichbarkeit nicht geändert: {error}"),
        )
    })?;
    wake();
    Ok(json!({ "online": online }))
}

/// `share.setLanPresence {enabled}`: find paired devices in the local
/// network (mDNS), as `se share lan presence` and the desktop tab
/// „Netzwerk“.
pub(super) fn set_lan_presence(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let enabled = bool_arg(args, "enabled")?;
    crate::share::LanSettings::update(|settings| settings.presence_enabled = enabled)
        .map_err(|error| ApiError::internal(format!("LAN-Einstellung speichern: {error}")))?;
    reconfigure(rt);
    Ok(json!({ "enabled": enabled }))
}

/// `share.renewCode {}`: a new direct code. The worker stops, the code is
/// renewed and direct authorizations pause (Exec off) while it stays
/// stopped; then the worker returns (desktop „Code neu generieren“).
pub(super) fn renew_code(rt: &Runtime) -> Result<Value, ApiError> {
    let mut identity = super::share_state::identity()?;
    let worker_was_running = stop_worker()?;
    let rotation = identity.regenerate_direct_code();
    let rotation = match rotation {
        Ok(rotation) => rotation,
        Err(error) => {
            if worker_was_running {
                reconfigure(rt);
            }
            return Err(ApiError::internal(format!(
                "Direct-Code nicht erneuert: {error}"
            )));
        }
    };
    with_state(|state| state.identity = Some(identity.clone()));
    let profiles = identity
        .complete_pending_cleanup(default_home())
        .map_err(|error| {
            ApiError::internal(format!(
                "Direct-Code erneuert, aber Share bleibt aus Sicherheitsgründen gestoppt: {error}"
            ))
        })?;
    committed(profiles);
    if worker_was_running {
        reconfigure(rt);
    }
    if let Some(warning) = &rotation.cleanup_warning {
        rt.log_error("share", warning);
    }
    Ok(json!({ "directCode": identity.direct_code(), "warning": rotation.cleanup_warning }))
}

/// Stops a running Share worker and confirms the stop; `true` when it ran.
fn stop_worker() -> Result<bool, ApiError> {
    if !crate::daemon::is_running() {
        return Ok(false);
    }
    let snapshot = crate::daemon::share_worker_snapshot()
        .map_err(|error| ApiError::internal(format!("Share-Worker prüfen: {error}")))?;
    if !snapshot.running {
        return Ok(false);
    }
    crate::daemon::send_share_command(ShareCmd::Stop)
        .map_err(|error| ApiError::internal(format!("Share-Worker stoppen: {error}")))?;
    let stopped = crate::daemon::share_worker_snapshot()
        .map_err(|error| ApiError::internal(format!("Share-Worker-Stopp prüfen: {error}")))?;
    if stopped.running {
        return Err(ApiError::internal(
            "Share-Worker lief nach dem Stopp weiter.",
        ));
    }
    Ok(true)
}
