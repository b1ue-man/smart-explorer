//! Direct devices, rooms and exports: the desktop `*_persisted` profile
//! operations and `crate::share::removal`, followed by the endpoint cleanup
//! (`connect::cleanup_removed_endpoint_state`) and a worker reload.
use serde_json::{json, Value};

use super::args::{invalid, opt_str, reject_app_internal, str_arg};
use super::locations::forget_endpoint;
use super::share_state::{committed, default_home, reconfigure, wake, with_state};
use crate::mobile::{ApiError, Runtime};
use crate::share::removal::{cleanup_notice, ProfileRemoval};
use crate::share::{ShareCmd, ShareExportConfig, ShareProfiles, SharedRoot};

fn internal(error: String) -> ApiError {
    ApiError::new("internal", error)
}

fn load_profiles() -> Result<ShareProfiles, ApiError> {
    ShareProfiles::load_checked(default_home()).map_err(internal)
}

/// Commits a removal's follow-ups and reports what the cleanup touched.
fn finish_removal(rt: &Runtime, removal: ProfileRemoval) -> Value {
    committed(removal.profiles);
    if let Some(warning) = &removal.warning {
        rt.log_error("share", warning);
    }
    let report = match &removal.scope {
        Some(scope) => {
            forget_endpoint(rt, scope);
            crate::connect::cleanup_removed_endpoint_state(scope)
        }
        None => crate::connect::CleanupReport::default(),
    };
    let notice = cleanup_notice(removal.headline, &report);
    if let Some(error) = &notice.error {
        rt.log_error("share", error);
    }
    with_state(|state| state.notice(notice.notice));
    if removal.changed {
        reconfigure(rt);
    } else {
        wake();
    }
    json!({
        "removedFavorites": report.favorites_removed,
        "orphanedJobs": report.orphaned_sync_jobs,
    })
}

pub(super) fn add_direct(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let code = str_arg(args, "code")?.trim().to_string();
    let name = opt_str(args, "name").unwrap_or("").trim().to_string();
    let (profiles, contact_id) =
        ShareProfiles::add_direct_from_code_persisted(default_home(), &code, &name)
            .map_err(|error| ApiError::new("invalid", error))?;
    committed(profiles);
    reconfigure(rt);
    Ok(json!({ "contactId": contact_id }))
}

pub(super) fn remove_device(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let contact_id = str_arg(args, "contactId")?;
    let current = load_profiles()?;
    if !current
        .direct_contacts
        .iter()
        .any(|contact| contact.id == contact_id)
    {
        return Err(ApiError::new("not_found", "Gerät nicht gefunden."));
    }
    let removal = crate::share::removal::remove_direct_peer(default_home(), &current, contact_id)
        .map_err(internal)?;
    Ok(finish_removal(rt, removal))
}

pub(super) fn readmit(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let device_id = str_arg(args, "deviceId")?;
    let removal = crate::share::removal::readmit_removed_device(default_home(), device_id)
        .map_err(internal)?;
    finish_removal(rt, removal);
    Ok(json!({}))
}

pub(super) fn create_room(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let name = str_arg(args, "name")?.trim().to_string();
    let code = ShareProfiles::new_room_code().map_err(internal)?;
    let (profiles, profile_id) =
        ShareProfiles::add_room_from_code_persisted(default_home(), &code, &name)
            .map_err(internal)?;
    committed(profiles);
    reconfigure(rt);
    Ok(json!({ "profileId": profile_id, "code": code }))
}

pub(super) fn join_room(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let code = str_arg(args, "code")?.trim().to_string();
    let name = opt_str(args, "name").unwrap_or("").trim().to_string();
    let (profiles, profile_id) =
        ShareProfiles::add_room_from_code_persisted(default_home(), &code, &name)
            .map_err(|error| ApiError::new("invalid", error))?;
    committed(profiles);
    reconfigure(rt);
    Ok(json!({ "profileId": profile_id }))
}

pub(super) fn room_code(args: &Value) -> Result<Value, ApiError> {
    let profile_id = str_arg(args, "profileId")?;
    let profiles = load_profiles()?;
    let room = profiles
        .rooms
        .iter()
        .find(|room| room.id == profile_id)
        .ok_or_else(|| ApiError::new("not_found", "Raum nicht gefunden."))?;
    let code = ShareProfiles::room_code_checked(room).map_err(internal)?;
    Ok(json!({ "code": code }))
}

/// Desktop "Verlassen": `auto_join` off (a user-owned field, merged onto the
/// newest profile), then the runtime leave signal.
pub(super) fn leave_room(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let profile_id = str_arg(args, "profileId")?;
    let previous = load_profiles()?;
    let mut edited = previous.clone();
    let room = edited
        .rooms
        .iter_mut()
        .find(|room| room.id == profile_id)
        .ok_or_else(|| ApiError::new("not_found", "Raum nicht gefunden."))?;
    room.auto_join = false;
    room.status = crate::share::ShareStatus::Offline;
    let room_id = room.room_id.clone();
    let profiles = ShareProfiles::mutate_persisted(default_home(), |latest| {
        crate::share::profile_edits::merge_user_edits(latest, &previous, &edited);
        Ok(())
    })
    .map_err(|error| internal(format!("Share-Profile speichern: {error}")))?;
    committed(profiles);
    reconfigure(rt);
    crate::daemon::send_share_command(ShareCmd::LeaveRoom { room_id })
        .map_err(|error| ApiError::new("network", format!("Raum verlassen: {error}")))?;
    Ok(json!({}))
}

pub(super) fn remove_room(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let profile_id = str_arg(args, "profileId")?;
    let current = load_profiles()?;
    if !current.rooms.iter().any(|room| room.id == profile_id) {
        return Err(ApiError::new("not_found", "Raum nicht gefunden."));
    }
    let removal = crate::share::removal::remove_room(default_home(), &current, profile_id)
        .map_err(internal)?;
    Ok(finish_removal(rt, removal))
}

fn exports_mut<'a>(
    profiles: &'a mut ShareProfiles,
    scope: &str,
) -> Result<&'a mut ShareExportConfig, String> {
    if scope == "direct" {
        return Ok(&mut profiles.default_direct_exports);
    }
    profiles
        .rooms
        .iter_mut()
        .find(|room| room.id == scope)
        .map(|room| &mut room.exports)
        .ok_or_else(|| "Raum nicht gefunden.".to_string())
}

fn canonical_directory(path: &str) -> Result<String, ApiError> {
    let not_a_folder = || invalid("Der Pfad muss ein vorhandener Ordner sein.");
    let canonical = std::fs::canonicalize(path).map_err(|_| not_a_folder())?;
    if !canonical.metadata().map_err(|_| not_a_folder())?.is_dir() {
        return Err(not_a_folder());
    }
    Ok(canonical.to_string_lossy().replace('\\', "/"))
}

pub(super) fn add_export(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let scope = str_arg(args, "scope")?.to_string();
    let raw = str_arg(args, "path")?;
    reject_app_internal(raw)?;
    let path = canonical_directory(raw)?;
    let label = opt_str(args, "label")
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            path.trim_end_matches('/')
                .rsplit('/')
                .next()
                .filter(|name| !name.is_empty())
                .unwrap_or("Freigabe")
                .to_string()
        });
    let profiles = ShareProfiles::mutate_persisted(default_home(), |profiles| {
        let config = exports_mut(profiles, &scope)?;
        if config.roots.iter().any(|root| root.path == path) {
            return Err("Dieser Ordner ist bereits freigegeben.".to_string());
        }
        config.roots.push(SharedRoot {
            label: label.clone(),
            path: path.clone(),
        });
        Ok(())
    })
    .map_err(|error| ApiError::new("invalid", error))?;
    committed(profiles);
    reconfigure(rt);
    Ok(json!({}))
}

pub(super) fn remove_export(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let scope = str_arg(args, "scope")?.to_string();
    let path = str_arg(args, "path")?.to_string();
    let profiles = ShareProfiles::mutate_persisted(default_home(), |profiles| {
        let config = exports_mut(profiles, &scope)?;
        let before = config.roots.len();
        config.roots.retain(|root| root.path != path);
        if config.roots.len() == before {
            return Err("Freigabe nicht gefunden.".to_string());
        }
        Ok(())
    })
    .map_err(|error| ApiError::new("not_found", error))?;
    committed(profiles);
    reconfigure(rt);
    Ok(json!({}))
}
