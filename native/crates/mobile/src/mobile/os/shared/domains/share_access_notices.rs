//! `share.access` and `share.reconfirmAll` (plan 2026-11-06, R1/R3/R9/R10):
//! the same access facts as the desktop page, `se share status` and MCP.
//! `notices` are the rejected sessions the embedded worker reports (both
//! directions), `gaps` the paired devices that cannot open this device's
//! exports, `storage` whether the host can read shared storage at all.
use serde_json::{json, Value};

use super::share_state::{committed, default_home, reconfigure, wake};
use crate::mobile::{ApiError, Runtime};
use crate::share::connection_rights::FolderChoice;
use crate::share::lifecycle_view::access_gaps::{access_gaps, reconfirm_all_direct_grants};
use crate::share::ShareProfiles;

/// `share.access {}` → `{notices:[…], gaps:[…], reconfirm, storageAccess, sharesFolders, workerError}`.
/// A worker that cannot be read yields no notices, never an error: the gaps
/// come from the profiles alone.
pub(super) fn access() -> Result<Value, ApiError> {
    let profiles = ShareProfiles::load_checked(default_home())
        .map_err(|error| ApiError::new("invalid", error))?;
    let (notices, worker_error) = match crate::daemon::share_worker_snapshot() {
        Ok(snapshot) => (
            snapshot
                .routes
                .map(|routes| routes.access)
                .unwrap_or_default(),
            None,
        ),
        Err(error) => (Vec::new(), Some(error.to_string())),
    };
    let gaps: Vec<Value> = access_gaps(&profiles)
        .into_iter()
        .map(|gap| {
            json!({
                "kind": gap.kind.code(),
                "deviceId": gap.device_id,
                "deviceName": gap.device_name,
                "contactId": gap.contact_id,
                "message": gap.message,
                "action": gap.action,
            })
        })
        .collect();
    let reconfirm = profiles
        .direct_grants
        .iter()
        .filter(|grant| grant.state == crate::share::DirectGrantState::Reconfirm)
        .count();
    let shares_folders = !profiles.default_direct_exports.roots.is_empty()
        || profiles
            .rooms
            .iter()
            .any(|room| !room.exports.roots.is_empty());
    // R5: which folders each Direct grant sees, next to its write right.
    let exports = &profiles.default_direct_exports;
    let device_folders: Vec<Value> = profiles
        .direct_grants
        .iter()
        .map(|grant| {
            let (choice, count) =
                match exports.folder_choice_of(&grant.device_id, &grant.public_key) {
                    FolderChoice::All => ("all", exports.roots.len()),
                    FolderChoice::None => ("none", 0),
                    FolderChoice::Selected { paths, .. } => ("selected", paths.len()),
                };
            json!({
                "deviceId": grant.device_id,
                "publicKey": grant.public_key,
                "choice": choice,
                "count": count,
            })
        })
        .collect();
    Ok(json!({
        "deviceFolders": device_folders,
        "notices": notices,
        "sharesFolders": shares_folders,
        "gaps": gaps,
        "reconfirm": reconfirm,
        // R10: an Android host without „Zugriff auf alle Dateien“ lists
        // shared storage only partly; `null` off Android.
        "storageAccess": crate::daemon::storage_access(),
        "workerError": worker_error,
    }))
}

/// `share.reconfirmAll {}`: every grant waiting for re-confirmation is
/// active again (Exec off; a grant without rights waits for them).
pub(super) fn reconfirm_all(rt: &Runtime) -> Result<Value, ApiError> {
    let change = reconfirm_all_direct_grants(default_home())
        .map_err(|error| ApiError::new("invalid", error))?;
    committed(change.profiles);
    if change.changed {
        reconfigure(rt);
    } else {
        wake();
    }
    Ok(json!({"persisted": true, "changed": change.changed}))
}
